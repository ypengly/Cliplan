//! Desktop clipboard monitoring agent (section 5, "Mode B").
//!
//! Compiled only with `--features agent`, since it pulls in platform
//! clipboard bindings (X11/Wayland on Linux, etc.) that a headless LAN
//! server doesn't need. When enabled, it polls the OS clipboard and feeds
//! changes into the same code path a `POST /api/clipboard` call would use,
//! so there is exactly one place that implements "a new clipboard entry
//! arrived" (see `api::clipboard::publish_entry`).

use std::time::Duration;
use tokio::sync::mpsc;

pub struct ClipboardChange {
    pub content: String,
}

/// Spawn a background task that polls the system clipboard every
/// `interval` and sends a `ClipboardChange` whenever the content differs
/// from the last observed value. Returns the receiving end; the caller
/// (server.rs) wires this into the same publish path used by the API.
pub fn spawn_watcher(interval: Duration) -> mpsc::Receiver<ClipboardChange> {
    let (tx, rx) = mpsc::channel(16);

    tokio::task::spawn_blocking(move || {
        let mut clipboard = match arboard::Clipboard::new() {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(error = %e, "clipboard agent: unable to access system clipboard");
                return;
            }
        };
        let mut last: Option<String> = None;
        loop {
            if let Ok(text) = clipboard.get_text() {
                if !text.is_empty() && Some(&text) != last.as_ref() {
                    last = Some(text.clone());
                    if tx.blocking_send(ClipboardChange { content: text }).is_err() {
                        break; // receiver dropped, shut down
                    }
                }
            }
            std::thread::sleep(interval);
        }
    });

    rx
}
