use cliplan::config::Config;
use std::net::SocketAddr;
use tempfile::TempDir;

pub struct TestServer {
    pub base_url: String,
    // Held for its Drop impl, which deletes the temp directory once the
    // test is done. Not read directly, hence the leading underscore.
    _tmp: TempDir,
}

/// Boot a real ClipLAN server on an ephemeral loopback port, backed by a
/// throwaway temp directory (its own sqlite db + files dir), with
/// discovery/banner/rotation background tasks disabled. Tests then talk to
/// it over real HTTP exactly like a phone or PC would.
pub async fn spawn() -> TestServer {
    spawn_custom(|_| {}).await
}

/// Same as `spawn`, but lets the test tweak the config (e.g. a tiny
/// `max_upload_size` to exercise the oversized-upload rejection path)
/// before the server starts.
pub async fn spawn_custom(configure: impl FnOnce(&mut Config)) -> TestServer {
    let tmp = tempfile::tempdir().expect("create temp dir");

    let mut config = Config::default();
    config.port = 0;
    config.data_dir = tmp.path().to_string_lossy().to_string();
    config.discovery = false;
    configure(&mut config);

    let (app, _state) = cliplan::server::build(config, false)
        .await
        .expect("build app");

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral port");
    let addr = listener.local_addr().expect("local addr");

    tokio::spawn(async move {
        axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
            .await
            .ok();
    });

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    TestServer {
        base_url: format!("http://{addr}"),
        _tmp: tmp,
    }
}

/// Run the full pairing flow against a `TestServer` (fetch the current
/// pairing code, submit it as a new device, approve it as the host would)
/// and return the resulting device token.
pub async fn pair_device(base_url: &str, device_name: &str) -> String {
    let client = reqwest::Client::new();

    let session: serde_json::Value = client
        .get(format!("{base_url}/api/devices/pairing/session"))
        .send()
        .await
        .expect("get session")
        .json()
        .await
        .expect("parse session");

    let code = session["code"].as_str().expect("code present");

    let init: serde_json::Value = client
        .post(format!("{base_url}/api/devices/pair"))
        .json(&serde_json::json!({ "code": code, "device_name": device_name }))
        .send()
        .await
        .expect("init pairing")
        .json()
        .await
        .expect("parse init response");

    let request_id = init["request_id"].as_str().expect("request_id present");

    let approve = client
        .post(format!("{base_url}/api/devices/pairing/{request_id}/approve"))
        .send()
        .await
        .expect("approve pairing");
    assert!(approve.status().is_success(), "approval should succeed");

    let status: serde_json::Value = client
        .get(format!("{base_url}/api/devices/pair/{request_id}"))
        .send()
        .await
        .expect("poll pairing status")
        .json()
        .await
        .expect("parse status");

    assert_eq!(status["status"], "approved");
    status["token"].as_str().expect("token present").to_string()
}
