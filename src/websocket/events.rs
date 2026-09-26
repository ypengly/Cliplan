use serde::Serialize;

/// Real-time events pushed to connected devices. Kept deliberately thin --
/// per section 16, event payloads never carry sensitive information
/// (clipboard *contents*, tokens, keys); clients fetch full details via the
/// REST API using their own auth once notified something changed.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
pub enum WsEvent {
    #[serde(rename = "clipboard.updated")]
    ClipboardUpdated { id: String, timestamp: String },

    #[serde(rename = "clipboard.deleted")]
    ClipboardDeleted { id: String },

    #[serde(rename = "clipboard.cleared")]
    ClipboardCleared,

    #[serde(rename = "device.connected")]
    DeviceConnected { device_id: String, name: String },

    #[serde(rename = "device.disconnected")]
    DeviceDisconnected { device_id: String },

    #[serde(rename = "device.updated")]
    DeviceUpdated { device_id: String },

    #[serde(rename = "file.started")]
    FileStarted { transfer_id: String, filename: String, size: i64 },

    #[serde(rename = "file.progress")]
    FileProgress { transfer_id: String, bytes_received: i64, size: i64 },

    #[serde(rename = "file.completed")]
    FileCompleted { transfer_id: String, sha256: String },

    #[serde(rename = "file.failed")]
    FileFailed { transfer_id: String, reason: String },

    #[serde(rename = "pairing.request")]
    PairingRequest { request_id: String, device_name: String, fingerprint: String },

    #[serde(rename = "pairing.approved")]
    PairingApproved { request_id: String },

    #[serde(rename = "pairing.rejected")]
    PairingRejected { request_id: String },
}
