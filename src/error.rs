use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

/// Central error type for ClipLAN. Every fallible operation in the app
/// converges on this so API handlers can turn errors into proper HTTP
/// responses without leaking internals (paths, SQL, stack info) to clients.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("not found")]
    NotFound,

    #[error("unauthorized")]
    Unauthorized,

    #[error("forbidden: {0}")]
    Forbidden(String),

    #[error("bad request: {0}")]
    BadRequest(String),

    #[error("conflict: {0}")]
    Conflict(String),

    #[error("payload too large")]
    PayloadTooLarge,

    #[error("rate limited")]
    RateLimited,

    #[error("database error")]
    Database(#[from] rusqlite::Error),

    #[error("io error")]
    Io(#[from] std::io::Error),

    #[error("internal error")]
    Internal(#[from] anyhow::Error),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        // Log the detailed error server-side; never echo internals to the client.
        match &self {
            AppError::Database(e) => tracing::error!(error = %e, "database error"),
            AppError::Io(e) => tracing::error!(error = %e, "io error"),
            AppError::Internal(e) => tracing::error!(error = %e, "internal error"),
            _ => {}
        }

        let (status, message) = match self {
            AppError::NotFound => (StatusCode::NOT_FOUND, "not found".to_string()),
            AppError::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized".to_string()),
            AppError::Forbidden(m) => (StatusCode::FORBIDDEN, m),
            AppError::BadRequest(m) => (StatusCode::BAD_REQUEST, m),
            AppError::Conflict(m) => (StatusCode::CONFLICT, m),
            AppError::PayloadTooLarge => (
                StatusCode::PAYLOAD_TOO_LARGE,
                "payload too large".to_string(),
            ),
            AppError::RateLimited => {
                (StatusCode::TOO_MANY_REQUESTS, "rate limited".to_string())
            }
            AppError::Database(_) | AppError::Io(_) | AppError::Internal(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal server error".to_string(),
            ),
        };

        (status, Json(json!({ "error": message }))).into_response()
    }
}

pub type AppResult<T> = Result<T, AppError>;
