pub mod api;
pub mod clipboard;
pub mod config;
pub mod discovery;
pub mod error;
pub mod pairing;
pub mod server;
pub mod storage;
pub mod transfer;
pub mod websocket;

/// Current UTC time as RFC3339, used for every timestamp stored in the
/// database so ordering ("Today" / "Yesterday" grouping etc.) is
/// unambiguous regardless of client timezone.
pub fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339()
}

pub fn iso_in(duration: std::time::Duration) -> String {
    let d = chrono::Duration::from_std(duration).unwrap_or_default();
    (chrono::Utc::now() + d).to_rfc3339()
}
