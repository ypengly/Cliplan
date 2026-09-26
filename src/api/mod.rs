pub mod clipboard;
pub mod devices;
pub mod files;
pub mod transfers;

use axum::{
    extract::{ConnectInfo, FromRequestParts},
    http::{header, request::Parts},
};
use std::net::SocketAddr;

use crate::error::AppError;
use crate::pairing::identity::hash_token;
use crate::server::AppState;
use crate::storage::database;
use crate::storage::models::Device;

/// Extractor that requires a valid `Authorization: Bearer <token>` header
/// matching a paired device. Every device-facing endpoint (clipboard,
/// files, transfers, device self-management) uses this -- there is no
/// "trust everyone on the LAN" path, per section 22.
pub struct AuthedDevice(pub Device);

impl FromRequestParts<AppState> for AuthedDevice {
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let header_val = parts
            .headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .ok_or(AppError::Unauthorized)?;

        let token = header_val
            .strip_prefix("Bearer ")
            .ok_or(AppError::Unauthorized)?;

        let token_hash = hash_token(token);
        let device = {
            let conn = state.db.lock().unwrap();
            database::find_device_by_token_hash(&conn, &token_hash)?
        };

        let device = device.ok_or(AppError::Unauthorized)?;

        // Refresh last-seen on every authenticated request; cheap and keeps
        // the "online/offline" view in the Devices page honest even
        // between WebSocket (re)connects.
        {
            let conn = state.db.lock().unwrap();
            let now = crate::now_iso();
            let _ = database::touch_device(&conn, &device.id, &now, "online");
        }

        Ok(AuthedDevice(device))
    }
}

/// Extractor that only allows requests originating from the machine
/// running the server itself (127.0.0.1 / ::1). Used to gate pairing
/// approval: any device on the LAN may *request* pairing, but only
/// someone with physical/local access to the host can approve it. This is
/// the crux of "passwordless does not mean unauthenticated" (section 14).
pub struct LoopbackOnly;

impl FromRequestParts<AppState> for LoopbackOnly {
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, _state: &AppState) -> Result<Self, Self::Rejection> {
        let ConnectInfo(addr) = parts
            .extensions
            .get::<ConnectInfo<SocketAddr>>()
            .copied()
            .ok_or_else(|| AppError::Forbidden("could not determine client address".into()))?;

        if addr.ip().is_loopback() {
            Ok(LoopbackOnly)
        } else {
            Err(AppError::Forbidden(
                "pairing approval is only allowed from the host machine".into(),
            ))
        }
    }
}
