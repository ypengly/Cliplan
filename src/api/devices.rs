use axum::extract::{Path, State};
use axum::Json;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{AuthedDevice, LoopbackOnly};
use crate::error::{AppError, AppResult};
use crate::pairing::identity::{generate_fingerprint, generate_token, hash_token};
use crate::server::AppState;
use crate::storage::database;
use crate::storage::models::Device;
use crate::websocket::events::WsEvent;

// ---------- Public: initiate pairing ----------

#[derive(Debug, Deserialize)]
pub struct PairInitRequest {
    /// Present when the device arrived via the QR-encoded `/pair/:session_id`
    /// link. Absent for the manual "just type the code" flow -- in that
    /// case the currently-active session is used, since only one session
    /// is ever active at a time.
    #[serde(default)]
    pub session_id: Option<String>,
    pub code: String,
    pub device_name: String,
}

#[derive(Debug, Serialize)]
pub struct PairInitResponse {
    pub request_id: String,
    pub fingerprint: String,
    pub status: &'static str,
}

/// A device scans the QR code (which encodes `session_id`) and submits the
/// code shown on the host's terminal/dashboard along with a display name.
/// This creates a *pending* request -- it does NOT grant access. Access is
/// only granted once a human at the host machine approves it (see
/// `approve_pairing` below), which is the actual security boundary here.
pub async fn init_pairing(
    State(state): State<AppState>,
    Json(body): Json<PairInitRequest>,
) -> AppResult<Json<PairInitResponse>> {
    let name = body.device_name.trim();
    if name.is_empty() || name.len() > 64 {
        return Err(AppError::BadRequest("device_name must be 1-64 characters".into()));
    }

    let valid = match &body.session_id {
        Some(sid) => state.pairing.validate(sid, &body.code),
        None => state.pairing.validate_code(&body.code),
    };
    if !valid {
        return Err(AppError::Unauthorized);
    }

    let request_id = Uuid::new_v4().to_string();
    let fingerprint = generate_fingerprint();
    let now = crate::now_iso();
    let expires_at = crate::iso_in(std::time::Duration::from_secs(10 * 60));

    {
        let conn = state.db.lock().unwrap();
        database::insert_pairing_request(&conn, &request_id, name, &fingerprint, &body.code, &now, &expires_at)?;
    }

    tracing::info!(device = %name, fingerprint = %fingerprint, "pairing request received, awaiting host approval");

    Ok(Json(PairInitResponse {
        request_id,
        fingerprint,
        status: "pending",
    }))
}

#[derive(Debug, Serialize)]
pub struct PairStatusResponse {
    pub status: String,
    /// Present exactly once: the first successful poll after approval
    /// returns the device token, then it is purged from server memory.
    pub token: Option<String>,
    pub device_id: Option<String>,
}

/// The connecting device polls this endpoint waiting for approval. Kept
/// deliberately simple (short poll) rather than a websocket, since the
/// device has no credentials yet to authenticate a websocket with.
pub async fn pairing_status(
    State(state): State<AppState>,
    Path(request_id): Path<String>,
) -> AppResult<Json<PairStatusResponse>> {
    let record = {
        let conn = state.db.lock().unwrap();
        database::get_pairing_request(&conn, &request_id)?
    };
    let record = record.ok_or(AppError::NotFound)?;

    if record.status == "approved" {
        // Hand over the one-time token, if it hasn't already been claimed.
        let token = {
            let mut pending = state.pending_tokens.lock().unwrap();
            pending.remove(&request_id)
        };
        let device_id = {
            let conn = state.db.lock().unwrap();
            let mut stmt = conn.prepare("SELECT id FROM devices WHERE fingerprint = ?1 ORDER BY created_at DESC LIMIT 1")?;
            stmt.query_row(rusqlite::params![&record.fingerprint], |r| r.get::<_, String>(0)).ok()
        };
        return Ok(Json(PairStatusResponse {
            status: record.status,
            token: token.map(|(t, _)| t),
            device_id,
        }));
    }

    Ok(Json(PairStatusResponse { status: record.status, token: None, device_id: None }))
}

// ---------- Host-only: review pending requests ----------

pub async fn list_pending(
    State(state): State<AppState>,
    _admin: LoopbackOnly,
) -> AppResult<Json<Vec<crate::storage::models::PairingRequest>>> {
    let conn = state.db.lock().unwrap();
    Ok(Json(database::list_pending_pairing_requests(&conn)?))
}

pub async fn approve_pairing(
    State(state): State<AppState>,
    _admin: LoopbackOnly,
    Path(request_id): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    let record = {
        let conn = state.db.lock().unwrap();
        database::get_pairing_request(&conn, &request_id)?
    };
    let record = record.ok_or(AppError::NotFound)?;
    if record.status != "pending" {
        return Err(AppError::Conflict(format!("request is already {}", record.status)));
    }
    if crate::now_iso().as_str() > record.expires_at.as_str() {
        let conn = state.db.lock().unwrap();
        let _ = database::set_pairing_status(&conn, &request_id, "expired");
        return Err(AppError::Conflict("pairing request expired".into()));
    }

    let device_id = Uuid::new_v4().to_string();
    let token = generate_token();
    let token_hash = hash_token(&token);
    let now = crate::now_iso();

    {
        let conn = state.db.lock().unwrap();
        database::insert_device(&conn, &device_id, &record.device_name, &record.fingerprint, &token_hash, &now)?;
        database::set_pairing_status(&conn, &request_id, "approved")?;
    }

    state
        .pending_tokens
        .lock()
        .unwrap()
        .insert(request_id.clone(), (token, std::time::Instant::now()));

    tracing::info!(device = %record.device_name, device_id = %device_id, "device paired");
    let _ = state.ws_tx.send(WsEvent::PairingApproved { request_id });

    Ok(Json(serde_json::json!({ "status": "approved", "device_id": device_id })))
}

pub async fn reject_pairing(
    State(state): State<AppState>,
    _admin: LoopbackOnly,
    Path(request_id): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    let conn = state.db.lock().unwrap();
    let updated = database::set_pairing_status(&conn, &request_id, "rejected")?;
    if updated == 0 {
        return Err(AppError::NotFound);
    }
    drop(conn);
    let _ = state.ws_tx.send(WsEvent::PairingRejected { request_id });
    Ok(Json(serde_json::json!({ "status": "rejected" })))
}

/// Host-only: fetch the currently active pairing session (code + session id
/// for the QR) so the CLI / local dashboard can render it.
pub async fn current_session(
    State(state): State<AppState>,
    _admin: LoopbackOnly,
) -> AppResult<Json<serde_json::Value>> {
    let session = state.pairing.current().unwrap_or_else(|| state.pairing.start_session());
    Ok(Json(serde_json::json!({
        "session_id": session.session_id,
        "code": session.code,
        "pair_url": format!("http://{}:{}/pair/{}", state.local_ip, state.config.port, session.session_id),
    })))
}

// ---------- Authenticated: device list & management ----------

#[derive(Debug, Serialize)]
pub struct DeviceView {
    #[serde(flatten)]
    pub device: Device,
    pub is_self: bool,
}

pub async fn list_devices(
    State(state): State<AppState>,
    AuthedDevice(me): AuthedDevice,
) -> AppResult<Json<Vec<DeviceView>>> {
    let conn = state.db.lock().unwrap();
    let devices = database::list_devices(&conn)?
        .into_iter()
        .map(|d| {
            let is_self = d.id == me.id;
            DeviceView { device: d, is_self }
        })
        .collect();
    Ok(Json(devices))
}

#[derive(Debug, Deserialize)]
pub struct RenameRequest {
    pub name: String,
}

pub async fn rename_device(
    State(state): State<AppState>,
    _me: AuthedDevice,
    Path(id): Path<String>,
    Json(body): Json<RenameRequest>,
) -> AppResult<Json<serde_json::Value>> {
    let name = body.name.trim();
    if name.is_empty() || name.len() > 64 {
        return Err(AppError::BadRequest("name must be 1-64 characters".into()));
    }
    let conn = state.db.lock().unwrap();
    let updated = database::rename_device(&conn, &id, name)?;
    if updated == 0 {
        return Err(AppError::NotFound);
    }
    drop(conn);
    let _ = state.ws_tx.send(WsEvent::DeviceUpdated { device_id: id });
    Ok(Json(serde_json::json!({ "status": "renamed" })))
}

pub async fn remove_device(
    State(state): State<AppState>,
    _me: AuthedDevice,
    Path(id): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    let conn = state.db.lock().unwrap();
    let updated = database::delete_device(&conn, &id)?;
    if updated == 0 {
        return Err(AppError::NotFound);
    }
    drop(conn);
    let _ = state.ws_tx.send(WsEvent::DeviceDisconnected { device_id: id });
    Ok(Json(serde_json::json!({ "status": "removed" })))
}

/// Devices announcing themselves over LAN discovery but not yet paired --
/// shown in the "Available devices" list (section 12) as a convenience;
/// selecting one just pre-fills the pairing flow, it does not grant access.
pub async fn discovered_devices(State(state): State<AppState>) -> Json<Vec<crate::discovery::lan::Announcement>> {
    let map = state.discovered.read().unwrap();
    Json(map.values().map(|d| d.announcement.clone()).collect())
}
