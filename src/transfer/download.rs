use axum::body::Body;
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use std::path::Path;
use tokio::fs::File;
use tokio::io::{AsyncSeekExt, SeekFrom};
use tokio_util::io::ReaderStream;

use crate::error::{AppError, AppResult};

/// Serve a file, honoring a single-range `Range: bytes=START-END` header if
/// present so that browsers/clients can resume interrupted downloads
/// (section 11) without ClipLAN needing a bespoke resume protocol on top of
/// plain HTTP.
pub async fn serve_file(
    path: &Path,
    filename: &str,
    total_size: u64,
    headers: &HeaderMap,
) -> AppResult<Response> {
    let range = headers.get(header::RANGE).and_then(|v| v.to_str().ok());

    let (start, end, status) = match range.and_then(parse_range) {
        Some((s, e)) if s < total_size => {
            let e = e.min(total_size.saturating_sub(1));
            (s, e, StatusCode::PARTIAL_CONTENT)
        }
        Some(_) => {
            // Unsatisfiable range.
            return Ok((
                StatusCode::RANGE_NOT_SATISFIABLE,
                [(header::CONTENT_RANGE, format!("bytes */{total_size}"))],
            )
                .into_response());
        }
        None => (0, total_size.saturating_sub(1), StatusCode::OK),
    };

    let mut file = File::open(path).await.map_err(AppError::Io)?;
    file.seek(SeekFrom::Start(start)).await.map_err(AppError::Io)?;

    let len = end - start + 1;
    let limited = tokio::io::AsyncReadExt::take(file, len);
    let stream = ReaderStream::new(limited);
    let body = Body::from_stream(stream);

    let disposition = format!("attachment; filename=\"{}\"", sanitize_header_value(filename));

    let mut response = Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .header(header::CONTENT_LENGTH, len.to_string())
        .header(header::ACCEPT_RANGES, "bytes")
        .header(
            header::CONTENT_DISPOSITION,
            HeaderValue::from_str(&disposition)
                .unwrap_or_else(|_| HeaderValue::from_static("attachment")),
        )
        .body(body)
        .unwrap();

    if status == StatusCode::PARTIAL_CONTENT {
        response.headers_mut().insert(
            header::CONTENT_RANGE,
            HeaderValue::from_str(&format!("bytes {start}-{end}/{total_size}")).unwrap(),
        );
    }

    Ok(response)
}

fn parse_range(value: &str) -> Option<(u64, u64)> {
    let value = value.strip_prefix("bytes=")?;
    let (start_s, end_s) = value.split_once('-')?;
    let start: u64 = start_s.parse().ok()?;
    let end: u64 = if end_s.is_empty() { u64::MAX } else { end_s.parse().ok()? };
    Some((start, end))
}

/// Header values can't contain control characters or raw quotes; strip
/// anything that isn't a normal printable char to keep this well-formed.
fn sanitize_header_value(s: &str) -> String {
    s.chars().filter(|c| !c.is_control() && *c != '"').collect()
}
