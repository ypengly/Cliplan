use axum::extract::{Path, State};
use axum::Json;

use super::AuthedDevice;
use crate::error::{AppError, AppResult};
use crate::server::AppState;
use crate::storage::database;
use crate::storage::models::Transfer;

pub async fn list(State(state): State<AppState>, _me: AuthedDevice) -> AppResult<Json<Vec<Transfer>>> {
    let conn = state.db.lock().unwrap();
    Ok(Json(database::list_transfers(&conn, 200)?))
}

pub async fn get(
    State(state): State<AppState>,
    _me: AuthedDevice,
    Path(id): Path<String>,
) -> AppResult<Json<Transfer>> {
    let conn = state.db.lock().unwrap();
    let transfer = database::get_transfer(&conn, &id)?.ok_or(AppError::NotFound)?;
    Ok(Json(transfer))
}

/// Delete a transfer's metadata *and* its file on disk. Deliberately
/// synchronous-then-async in that order: if the DB delete fails we leave
/// the file alone rather than risking an orphaned metadata row pointing at
/// a missing file.
pub async fn delete(
    State(state): State<AppState>,
    _me: AuthedDevice,
    Path(id): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    let storage_path = {
        let conn = state.db.lock().unwrap();
        database::get_transfer_storage_path(&conn, &id)?
    };
    let Some(storage_path) = storage_path else {
        return Err(AppError::NotFound);
    };

    {
        let conn = state.db.lock().unwrap();
        database::delete_transfer(&conn, &id)?;
    }

    let _ = tokio::fs::remove_file(storage_path).await;

    Ok(Json(serde_json::json!({ "status": "deleted" })))
}

pub async fn clear(State(state): State<AppState>, _me: AuthedDevice) -> AppResult<Json<serde_json::Value>> {
    // Clearing history removes both the metadata rows and their files --
    // once the metadata is gone the file would otherwise be orphaned and
    // permanently inaccessible via the API anyway.
    let paths = {
        let conn = state.db.lock().unwrap();
        let paths = database::list_clearable_storage_paths(&conn)?;
        database::clear_transfer_history(&conn)?;
        paths
    };
    for path in paths {
        let _ = tokio::fs::remove_file(path).await;
    }
    Ok(Json(serde_json::json!({ "status": "cleared" })))
}
