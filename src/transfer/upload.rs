use axum::extract::multipart::Field;
use sha2::{Digest, Sha256};
use std::path::Path;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;

use crate::error::{AppError, AppResult};

pub struct UploadResult {
    pub bytes_written: u64,
    pub sha256: String,
}

/// Stream a multipart field to disk, hashing as we go (so we never buffer
/// the whole file in memory -- section 30, "stream files rather than
/// loading entire files into memory") and enforcing `max_size` (section 22,
/// "upload size limits"). `on_progress` is called after each chunk with the
/// cumulative byte count so the caller can broadcast `file.progress`
/// events.
pub async fn receive_upload(
    mut field: Field<'_>,
    dest: &Path,
    max_size: u64,
    mut on_progress: impl FnMut(u64),
) -> AppResult<UploadResult> {
    let mut file = File::create(dest).await.map_err(AppError::Io)?;
    let mut hasher = Sha256::new();
    let mut total: u64 = 0;

    loop {
        let chunk = field
            .chunk()
            .await
            .map_err(|e| AppError::BadRequest(format!("malformed upload: {e}")))?;
        let Some(bytes) = chunk else { break };

        total += bytes.len() as u64;
        if total > max_size {
            // Stop immediately, clean up the partial file, and reject.
            drop(file);
            let _ = tokio::fs::remove_file(dest).await;
            return Err(AppError::PayloadTooLarge);
        }

        hasher.update(&bytes);
        file.write_all(&bytes).await.map_err(AppError::Io)?;
        on_progress(total);
    }

    file.flush().await.map_err(AppError::Io)?;

    Ok(UploadResult {
        bytes_written: total,
        sha256: hex::encode(hasher.finalize()),
    })
}
