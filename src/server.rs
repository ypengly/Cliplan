use axum::extract::DefaultBodyLimit;
use axum::response::{Html, IntoResponse};
use axum::routing::{delete, get, patch, post};
use axum::Router;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Instant;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use uuid::Uuid;

use crate::api;
use crate::config::Config;
use crate::discovery::lan::{self, DiscoveryMap};
use crate::pairing::PairingManager;
use crate::storage::Db;
use crate::websocket::{self, events::WsEvent};

const INDEX_HTML: &str = include_str!("../web/index.html");
const STYLE_CSS: &str = include_str!("../web/style.css");
const APP_JS: &str = include_str!("../web/app.js");

#[derive(Clone)]
pub struct AppState {
    pub db: Db,
    pub config: Arc<Config>,
    pub pairing: Arc<PairingManager>,
    pub ws_tx: tokio::sync::broadcast::Sender<WsEvent>,
    pub discovered: DiscoveryMap,
    pub pending_tokens: Arc<Mutex<HashMap<String, (String, Instant)>>>,
    pub local_ip: String,
    pub device_id: String,
}


pub async fn run(config: Config) -> anyhow::Result<()> {
    let (app, state) = build(config, true).await?;

    let addr = SocketAddr::from(([0, 0, 0, 0], state.config.port));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!(%addr, "cliplan listening");

    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;

    Ok(())
}

/// Assemble the AppState and router without binding a socket or starting
/// background tasks. `with_background_tasks` controls whether the
/// pairing-code banner is printed and the LAN discovery/rotation tasks are
/// spawned -- integration tests build the app the same way production does,
/// just with those side effects switched off so parallel test runs don't
/// fight over the discovery UDP port or spam stdout.
pub async fn build(config: Config, with_background_tasks: bool) -> anyhow::Result<(Router, AppState)> {
    config.ensure_dirs()?;
    let conn = crate::storage::database::open(&config.db_path())?;
    let db: Db = Arc::new(Mutex::new(conn));

    let local_ip = local_ip_address::local_ip()
        .map(|ip| ip.to_string())
        .unwrap_or_else(|_| "127.0.0.1".to_string());

    let device_name = config.device_name.clone().unwrap_or_else(default_device_name);
    let device_id = Uuid::new_v4().to_string();

    let (ws_tx, _rx) = tokio::sync::broadcast::channel(256);

    let state = AppState {
        db,
        config: Arc::new(config.clone()),
        pairing: Arc::new(PairingManager::new()),
        ws_tx,
        discovered: Arc::new(RwLock::new(HashMap::new())),
        pending_tokens: Arc::new(Mutex::new(HashMap::new())),
        local_ip: local_ip.clone(),
        device_id: device_id.clone(),
    };

    // Always keep one pairing session alive so the QR/code shown in the
    // banner and dashboard are always valid; rotate on expiry.
    {
        let pairing = state.pairing.clone();
        let session = pairing.start_session();
        if with_background_tasks {
            print_banner(&config, &local_ip, &session.code);
            tokio::spawn(async move {
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(4 * 60)).await;
                    pairing.start_session();
                }
            });
        }
    }

    if with_background_tasks && config.discovery {
        let map = state.discovered.clone();
        let id1 = device_id.clone();
        let id2 = device_id.clone();
        let name = device_name.clone();
        let port = config.port;
        let ip = local_ip.clone();
        tokio::spawn(lan::run_announcer(id1, name, port, ip));
        tokio::spawn(lan::run_listener(id2, map));
    }

    #[cfg(feature = "agent")]
    if with_background_tasks && config.clipboard_sync {
        let mut rx = crate::clipboard::watcher::spawn_watcher(std::time::Duration::from_millis(750));
        let watcher_state = state.clone();
        tokio::spawn(async move {
            while let Some(change) = rx.recv().await {
                if let Err(e) = crate::api::clipboard::publish_entry(&watcher_state, &change.content, None).await {
                    tracing::debug!(error = ?e, "clipboard agent: failed to publish change");
                }
            }
        });
    }

    let app = build_router(state.clone());
    Ok((app, state))
}

pub fn build_router(state: AppState) -> Router {
    let api_routes = Router::new()
        .route("/status", get(status))
        .route("/devices", get(api::devices::list_devices))
        .route("/devices/discovered", get(api::devices::discovered_devices))
        .route(
            "/devices/:id",
            patch(api::devices::rename_device).delete(api::devices::remove_device),
        )
        .route("/devices/pair", post(api::devices::init_pairing))
        .route("/devices/pair/:request_id", get(api::devices::pairing_status))
        .route("/devices/pairing/session", get(api::devices::current_session))
        .route("/devices/pairing/pending", get(api::devices::list_pending))
        .route("/devices/pairing/:request_id/approve", post(api::devices::approve_pairing))
        .route("/devices/pairing/:request_id/reject", post(api::devices::reject_pairing))
        .route("/clipboard", get(api::clipboard::list).post(api::clipboard::create))
        .route("/clipboard/clear", post(api::clipboard::clear))
        .route("/clipboard/:id", delete(api::clipboard::delete))
        // POST /api/transfers is an alias for /api/files/upload (see
        // section 21's endpoint list) -- both accept the same multipart
        // body and share one implementation so behavior can't drift.
        .route("/transfers", get(api::transfers::list).post(api::files::upload_file))
        .route("/transfers/clear", post(api::transfers::clear))
        .route(
            "/transfers/:id",
            get(api::transfers::get).delete(api::transfers::delete),
        )
        .route("/files/upload", post(api::files::upload_file))
        .route("/files/:id/download", get(api::files::download_file))
        .layer(DefaultBodyLimit::max(state.config.max_upload_size as usize + 8192));

    Router::new()
        .route("/", get(index))
        .route("/pair/:session_id", get(index))
        .route("/app.js", get(app_js))
        .route("/style.css", get(style_css))
        .route("/ws", get(websocket::ws_handler))
        .nest("/api", api_routes)
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

async fn index() -> Html<&'static str> {
    Html(INDEX_HTML)
}

async fn app_js() -> impl IntoResponse {
    (
        [(axum::http::header::CONTENT_TYPE, "application/javascript; charset=utf-8")],
        APP_JS,
    )
}

async fn style_css() -> impl IntoResponse {
    ([(axum::http::header::CONTENT_TYPE, "text/css; charset=utf-8")], STYLE_CSS)
}

async fn status(axum::extract::State(state): axum::extract::State<AppState>) -> axum::Json<serde_json::Value> {
    let device_count = {
        let conn = state.db.lock().unwrap();
        crate::storage::database::list_devices(&conn).map(|d| d.len()).unwrap_or(0)
    };
    axum::Json(serde_json::json!({
        "name": "ClipLAN",
        "version": env!("CARGO_PKG_VERSION"),
        "address": format!("http://{}:{}", state.local_ip, state.config.port),
        "devices": device_count,
        "clipboard_sync": state.config.clipboard_sync,
        "discovery": state.config.discovery,
    }))
}

fn default_device_name() -> String {
    hostname_or_default()
}

fn hostname_or_default() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "ClipLAN Host".to_string())
}

pub fn print_banner(config: &Config, local_ip: &str, code: &str) {
    let addr = format!("http://{local_ip}:{}", config.port);
    println!("\n\u{256d}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{256e}");
    println!("\u{2502}           \u{1F4CB} ClipLAN               \u{2502}");
    println!("\u{2570}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{256f}\n");
    println!("Status:       \u{25CF} Running");
    println!("Port:         {}", config.port);
    println!("Address:      {local_ip}");
    println!("Clipboard:    {} Enabled", if config.clipboard_sync { "\u{25CF}" } else { "\u{25CB}" });
    println!("Discovery:    {} Enabled", if config.discovery { "\u{25CF}" } else { "\u{25CB}" });
    println!("\nWeb:\n{addr}\n");
    println!("Pairing code (enter this on the connecting device): {code}");
    println!("(A QR code that skips typing the address is at /pair on the dashboard.)");
    print_qr(&addr);
    println!();
}

fn print_qr(url: &str) {
    use qrcode::render::unicode;
    use qrcode::QrCode;
    match QrCode::new(url.as_bytes()) {
        Ok(code) => {
            let rendered = code
                .render::<unicode::Dense1x2>()
                .quiet_zone(true)
                .build();
            println!("{rendered}");
        }
        Err(e) => {
            tracing::warn!(error = %e, "failed to render QR code");
        }
    }
}
