mod common;

#[tokio::test]
async fn path_traversal_filename_is_sanitized_on_upload() {
    let server = common::spawn().await;
    let token = common::pair_device(&server.base_url, "Attacker").await;
    let client = reqwest::Client::new();

    for malicious_name in ["../../secret.txt", "..\\..\\etc\\passwd", "/etc/passwd"] {
        let part = reqwest::multipart::Part::bytes(b"payload".to_vec()).file_name(malicious_name);
        let form = reqwest::multipart::Form::new().part("file", part);

        let upload: serde_json::Value = client
            .post(format!("{}/api/files/upload", server.base_url))
            .bearer_auth(&token)
            .multipart(form)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();

        let filename = upload["filename"].as_str().unwrap();
        assert!(!filename.contains(".."), "sanitized name leaked traversal: {filename}");
        assert!(!filename.contains('/') && !filename.contains('\\'), "sanitized name leaked a separator: {filename}");
    }
}

#[tokio::test]
async fn invalid_bearer_token_is_rejected() {
    let server = common::spawn().await;
    let client = reqwest::Client::new();

    let resp = client
        .get(format!("{}/api/devices", server.base_url))
        .bearer_auth("not-a-real-token")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn missing_authorization_header_is_rejected() {
    let server = common::spawn().await;
    let client = reqwest::Client::new();

    let resp = client.get(format!("{}/api/devices", server.base_url)).send().await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn downloading_an_unknown_transfer_id_is_not_found() {
    let server = common::spawn().await;
    let token = common::pair_device(&server.base_url, "Viewer").await;
    let client = reqwest::Client::new();

    let resp = client
        .get(format!("{}/api/files/does-not-exist/download", server.base_url))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn pairing_approval_endpoints_are_loopback_only() {
    // We can't easily simulate a non-loopback source address from within a
    // single-machine test suite, so this test documents and locks in the
    // *mechanism* instead: the endpoint is reachable over loopback (which
    // is exactly what our test client uses) and requires a real pending
    // request id to succeed -- see `pairing.rs` for the full approve/
    // reject flows exercised over loopback. The `LoopbackOnly` extractor
    // itself (src/api/mod.rs) is what rejects non-loopback callers in
    // production, by inspecting the TCP peer address axum provides via
    // `ConnectInfo`, which a same-host test client cannot spoof.
    let server = common::spawn().await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{}/api/devices/pairing/unknown-id/approve", server.base_url))
        .send()
        .await
        .unwrap();
    // Reachable (not 403) from loopback, but 404s because the id is bogus.
    assert_eq!(resp.status(), reqwest::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn oversized_clipboard_content_is_rejected() {
    let server = common::spawn().await;
    let token = common::pair_device(&server.base_url, "Spammer").await;
    let client = reqwest::Client::new();

    let huge = "a".repeat(300 * 1024); // over the 256 KiB clipboard cap
    let resp = client
        .post(format!("{}/api/clipboard", server.base_url))
        .bearer_auth(&token)
        .json(&serde_json::json!({ "content": huge }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::BAD_REQUEST);
}
