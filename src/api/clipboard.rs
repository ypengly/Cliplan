use axum::extract::{Path, State};
use axum::Json;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::AuthedDevice;
use crate::clipboard::{detect_content_type, MAX_CLIPBOARD_BYTES};
use crate::error::{AppError, AppResult};
use crate::server::AppState;
use crate::storage::database;
use crate::storage::models::ClipboardEntry;
use crate::websocket::events::WsEvent;

pub async fn list(
    State(state): State<AppState>,
    _me: AuthedDevice,
) -> AppResult<Json<Vec<ClipboardEntry>>> {
    let conn = state.db.lock().unwrap();
    let limit = if state.config.clipboard_history { state.config.history_size() } else { 1 };
    Ok(Json(database::list_clipboard_entries(&conn, limit)?))
}

#[derive(Debug, Deserialize)]
pub struct NewClipboardEntry {
    pub content: String,
}

#[derive(Debug, Serialize)]
pub struct NewClipboardResponse {
    pub id: String,
    pub content_type: String,
}

pub async fn create(
    State(state): State<AppState>,
    AuthedDevice(me): AuthedDevice,
    Json(body): Json<NewClipboardEntry>,
) -> AppResult<Json<NewClipboardResponse>> {
    let resp = publish_entry(&state, &body.content, Some(me.id.as_str())).await?;
    Ok(Json(resp))
}

/// Shared path for anything that produces a new clipboard entry: the REST
/// API above, and (when built with `--features agent`) the desktop
/// clipboard watcher. Keeping one function means validation, trimming, and
/// the broadcast event can't drift between the two sources.
pub async fn publish_entry(
    state: &AppState,
    content: &str,
    device_id: Option<&str>,
) -> AppResult<NewClipboardResponse> {
    if content.is_empty() {
        return Err(AppError::BadRequest("content must not be empty".into()));
    }
    if content.len() > MAX_CLIPBOARD_BYTES {
        return Err(AppError::BadRequest(format!(
            "content exceeds {MAX_CLIPBOARD_BYTES} byte limit"
        )));
    }
    if !state.config.clipboard_sync {
        return Err(AppError::Forbidden("clipboard sync is disabled on this server".into()));
    }

    let content_type = detect_content_type(content);
    let id = Uuid::new_v4().to_string();
    let now = crate::now_iso();

    {
        let conn = state.db.lock().unwrap();
        database::insert_clipboard_entry(&conn, &id, content, content_type, device_id, &now)?;
        let keep = if state.config.clipboard_history { state.config.history_size() } else { 1 };
        database::trim_clipboard_history(&conn, keep)?;
    }

    let _ = state.ws_tx.send(WsEvent::ClipboardUpdated { id: id.clone(), timestamp: now });

    Ok(NewClipboardResponse { id, content_type: content_type.to_string() })
}

pub async fn delete(
    State(state): State<AppState>,
    _me: AuthedDevice,
    Path(id): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    let conn = state.db.lock().unwrap();
    let updated = database::delete_clipboard_entry(&conn, &id)?;
    if updated == 0 {
        return Err(AppError::NotFound);
    }
    drop(conn);
    let _ = state.ws_tx.send(WsEvent::ClipboardDeleted { id });
    Ok(Json(serde_json::json!({ "status": "deleted" })))
}

pub async fn clear(State(state): State<AppState>, _me: AuthedDevice) -> AppResult<Json<serde_json::Value>> {
    let conn = state.db.lock().unwrap();
    database::clear_clipboard_history(&conn)?;
    drop(conn);
    let _ = state.ws_tx.send(WsEvent::ClipboardCleared);
    Ok(Json(serde_json::json!({ "status": "cleared" })))
}
