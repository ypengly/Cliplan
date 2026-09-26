use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub fingerprint: String,
    pub created_at: String,
    pub last_seen: String,
    pub status: String, // "online" | "offline"
}

#[derive(Debug, Clone, Serialize)]
pub struct PairingRequest {
    pub id: String,
    pub device_name: String,
    pub fingerprint: String,
    pub status: String, // "pending" | "approved" | "rejected" | "expired"
    pub created_at: String,
    pub expires_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ClipboardEntry {
    pub id: String,
    pub content: String,
    pub content_type: String, // "text" | "url" | "code"
    pub device_id: Option<String>,
    pub device_name: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum TransferStatus {
    #[serde(rename = "pending")]
    Pending,
    #[serde(rename = "transferring")]
    Transferring,
    #[serde(rename = "verifying")]
    Verifying,
    #[serde(rename = "completed")]
    Completed,
    #[serde(rename = "failed")]
    Failed,
    #[serde(rename = "cancelled")]
    Cancelled,
}

impl TransferStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            TransferStatus::Pending => "pending",
            TransferStatus::Transferring => "transferring",
            TransferStatus::Verifying => "verifying",
            TransferStatus::Completed => "completed",
            TransferStatus::Failed => "failed",
            TransferStatus::Cancelled => "cancelled",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "transferring" => TransferStatus::Transferring,
            "verifying" => TransferStatus::Verifying,
            "completed" => TransferStatus::Completed,
            "failed" => TransferStatus::Failed,
            "cancelled" => TransferStatus::Cancelled,
            _ => TransferStatus::Pending,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Transfer {
    pub id: String,
    pub filename: String,
    pub size: i64,
    pub sha256: Option<String>,
    pub from_device: Option<String>,
    pub to_device: Option<String>,
    pub status: String,
    pub bytes_received: i64,
    pub created_at: String,
    pub completed_at: Option<String>,
}
