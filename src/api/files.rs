use axum::extract::{Multipart, Path, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use uuid::Uuid;

use super::AuthedDevice;
use crate::error::{AppError, AppResult};
use crate::server::AppState;
use crate::storage::database;
use crate::storage::models::TransferStatus;
use crate::transfer::{download, integrity, upload};
use crate::websocket::events::WsEvent;

#[derive(Debug, Serialize)]
pub struct UploadResponse {
    pub transfer_id: String,
    pub filename: String,
    pub size: u64,
    pub sha256: String,
}

/// Handle `POST /api/files/upload`. Expects a multipart body with a `file`
/// part (any other parts, e.g. a `to_device` hint, are read but optional).
/// The file is streamed straight to disk under the configured shared
/// directory -- never buffered whole in memory, never written under a
/// client-controlled path (section 22: "no arbitrary filesystem access").
pub async fn upload_file(
    State(state): State<AppState>,
    AuthedDevice(me): AuthedDevice,
    mut multipart: Multipart,
) -> AppResult<Json<UploadResponse>> {
    let mut to_device: Option<String> = None;
    let mut result: Option<UploadResponse> = None;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::BadRequest(format!("malformed multipart body: {e}")))?
    {
        let name = field.name().unwrap_or("").to_string();

        if name == "to_device" {
            to_device = field.text().await.ok().filter(|s| !s.is_empty());
            continue;
        }

        if name != "file" {
            continue; // ignore unknown fields rather than erroring
        }

        let original_name = field
            .file_name()
            .map(|s| s.to_string())
            .unwrap_or_else(|| "upload.bin".to_string());
        let safe_name = integrity::sanitize_filename(&original_name);

        let transfer_id = Uuid::new_v4().to_string();
        let storage_file_name = format!("{transfer_id}_{safe_name}");
        let dest = state.config.files_dir().join(&storage_file_name);

        // Defense in depth: the joined path must still live inside the
        // configured files directory. UUID-prefixed names make this
        // effectively impossible to violate, but we check anyway.
        let files_dir = state.config.files_dir();
        if dest.parent() != Some(files_dir.as_path()) {
            return Err(AppError::BadRequest("invalid destination path".into()));
        }

        let now = crate::now_iso();
        {
            let conn = state.db.lock().unwrap();
            database::insert_transfer(
                &conn,
                &transfer_id,
                &safe_name,
                0,
                Some(me.id.as_str()),
                to_device.as_deref(),
                dest.to_string_lossy().as_ref(),
                &now,
            )?;
        }
        let _ = state.ws_tx.send(WsEvent::FileStarted {
            transfer_id: transfer_id.clone(),
            filename: safe_name.clone(),
            size: 0,
        });

        let max_size = state.config.max_upload_size;
        let last_broadcast = Arc::new(AtomicI64::new(0));
        let ws_tx = state.ws_tx.clone();
        let progress_id = transfer_id.clone();

        let upload_result = upload::receive_upload(field, &dest, max_size, move |received| {
            // Throttle progress events to roughly every 256 KiB so a fast
            // LAN transfer of a large file doesn't flood the WebSocket.
            let received_i = received as i64;
            let last = last_broadcast.load(Ordering::Relaxed);
            if received_i - last >= 256 * 1024 || received_i == 0 {
                last_broadcast.store(received_i, Ordering::Relaxed);
                let _ = ws_tx.send(WsEvent::FileProgress {
                    transfer_id: progress_id.clone(),
                    bytes_received: received_i,
                    size: 0,
                });
            }
        })
        .await;

        let upload_result = match upload_result {
            Ok(r) => r,
            Err(e) => {
                let conn = state.db.lock().unwrap();
                let _ = database::fail_transfer(&conn, &transfer_id);
                drop(conn);
                let _ = state.ws_tx.send(WsEvent::FileFailed {
                    transfer_id: transfer_id.clone(),
                    reason: e.to_string(),
                });
                return Err(e);
            }
        };

        let completed_at = crate::now_iso();
        {
            let conn = state.db.lock().unwrap();
            conn.execute(
                "UPDATE transfers SET size = ?1 WHERE id = ?2",
                rusqlite::params![upload_result.bytes_written as i64, transfer_id],
            )?;
            database::complete_transfer(&conn, &transfer_id, &upload_result.sha256, &completed_at)?;
        }

        let _ = state.ws_tx.send(WsEvent::FileCompleted {
            transfer_id: transfer_id.clone(),
            sha256: upload_result.sha256.clone(),
        });

        tracing::info!(
            file = %safe_name,
            size = upload_result.bytes_written,
            sha256 = %upload_result.sha256,
            "transfer completed"
        );

        result = Some(UploadResponse {
            transfer_id,
            filename: safe_name,
            size: upload_result.bytes_written,
            sha256: upload_result.sha256,
        });
    }

    result
        .map(Json)
        .ok_or_else(|| AppError::BadRequest("no file part found in upload".into()))
}

pub async fn download_file(
    State(state): State<AppState>,
    _me: AuthedDevice,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> AppResult<Response> {
    let transfer = {
        let conn = state.db.lock().unwrap();
        database::get_transfer(&conn, &id)?
    };
    let transfer = transfer.ok_or(AppError::NotFound)?;

    if TransferStatus::from_str(&transfer.status) != TransferStatus::Completed {
        return Err(AppError::Conflict("file is not ready for download".into()));
    }

    let storage_path = {
        let conn = state.db.lock().unwrap();
        database::get_transfer_storage_path(&conn, &id)?
    }
    .ok_or(AppError::NotFound)?;

    let path = std::path::PathBuf::from(&storage_path);

    // Defense in depth, mirroring the upload-side check.
    let files_dir = state.config.files_dir();
    let canonical_files_dir = files_dir.canonicalize().unwrap_or(files_dir);
    let canonical_path = path
        .canonicalize()
        .map_err(|_| AppError::NotFound)?;
    if !canonical_path.starts_with(&canonical_files_dir) {
        return Err(AppError::Forbidden("refusing to serve file outside shared directory".into()));
    }

    let response = download::serve_file(&path, &transfer.filename, transfer.size as u64, &headers).await?;
    Ok(response.into_response())
}
