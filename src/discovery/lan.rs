use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};
use tokio::net::UdpSocket;

pub const DISCOVERY_PORT: u16 = 47872;
const ANNOUNCE_INTERVAL: Duration = Duration::from_secs(3);
const STALE_AFTER: Duration = Duration::from_secs(15);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Announcement {
    pub app: String, // always "cliplan", used to ignore unrelated broadcast traffic
    pub device_id: String,
    pub device_name: String,
    pub ip: String,
    pub port: u16,
    pub version: String,
    pub capabilities: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiscoveredDevice {
    #[serde(flatten)]
    pub announcement: Announcement,
    #[serde(skip)]
    pub last_seen: Instant,
}

pub type DiscoveryMap = Arc<RwLock<HashMap<String, DiscoveredDevice>>>;

/// Periodically broadcast an announcement so other ClipLAN instances on the
/// same LAN segment can find this one without any manual configuration
/// (fallback: manual `ip:port` entry in the UI, per section 12).
pub async fn run_announcer(local_device_id: String, device_name: String, http_port: u16, local_ip: String) {
    let socket = match UdpSocket::bind("0.0.0.0:0").await {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(error = %e, "discovery: failed to bind announce socket");
            return;
        }
    };
    if let Err(e) = socket.set_broadcast(true) {
        tracing::warn!(error = %e, "discovery: failed to enable broadcast");
        return;
    }

    let announcement = Announcement {
        app: "cliplan".to_string(),
        device_id: local_device_id,
        device_name,
        ip: local_ip,
        port: http_port,
        version: env!("CARGO_PKG_VERSION").to_string(),
        capabilities: vec!["clipboard".into(), "files".into()],
    };

    let payload = match serde_json::to_vec(&announcement) {
        Ok(p) => p,
        Err(_) => return,
    };
    let dest: SocketAddr = format!("255.255.255.255:{DISCOVERY_PORT}").parse().unwrap();

    loop {
        if let Err(e) = socket.send_to(&payload, dest).await {
            tracing::debug!(error = %e, "discovery: broadcast send failed");
        }
        tokio::time::sleep(ANNOUNCE_INTERVAL).await;
    }
}

/// Listen for announcements from other instances and maintain a map of
/// currently-visible peers, expiring entries that haven't been heard from
/// recently.
pub async fn run_listener(local_device_id: String, map: DiscoveryMap) {
    let socket = match UdpSocket::bind(("0.0.0.0", DISCOVERY_PORT)).await {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(error = %e, "discovery: failed to bind listen socket (port {DISCOVERY_PORT} in use?)");
            return;
        }
    };

    let mut buf = [0u8; 2048];
    loop {
        match socket.recv_from(&mut buf).await {
            Ok((len, _addr)) => {
                if let Ok(ann) = serde_json::from_slice::<Announcement>(&buf[..len]) {
                    if ann.app != "cliplan" || ann.device_id == local_device_id {
                        continue;
                    }
                    let mut m = map.write().unwrap();
                    m.insert(
                        ann.device_id.clone(),
                        DiscoveredDevice { announcement: ann, last_seen: Instant::now() },
                    );
                    m.retain(|_, d| d.last_seen.elapsed() < STALE_AFTER);
                }
            }
            Err(e) => {
                tracing::debug!(error = %e, "discovery: recv error");
            }
        }
    }
}
