mod common;

#[tokio::test]
async fn pairing_flow_succeeds_and_grants_access() {
    let server = common::spawn().await;
    let token = common::pair_device(&server.base_url, "Test Phone").await;
    assert!(!token.is_empty());

    let client = reqwest::Client::new();
    let devices: serde_json::Value = client
        .get(format!("{}/api/devices", server.base_url))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let devices = devices.as_array().unwrap();
    assert_eq!(devices.len(), 1);
    assert_eq!(devices[0]["name"], "Test Phone");
    assert_eq!(devices[0]["is_self"], true);
}

#[tokio::test]
async fn wrong_pairing_code_is_rejected() {
    let server = common::spawn().await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{}/api/devices/pair", server.base_url))
        .json(&serde_json::json!({ "code": "000000", "device_name": "Intruder" }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn unapproved_pairing_request_grants_no_token() {
    let server = common::spawn().await;
    let client = reqwest::Client::new();

    let session: serde_json::Value = client
        .get(format!("{}/api/devices/pairing/session", server.base_url))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let code = session["code"].as_str().unwrap();

    let init: serde_json::Value = client
        .post(format!("{}/api/devices/pair", server.base_url))
        .json(&serde_json::json!({ "code": code, "device_name": "Waiting Phone" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let request_id = init["request_id"].as_str().unwrap();

    // No approval happened -- polling should show "pending" and no token.
    let status: serde_json::Value = client
        .get(format!("{}/api/devices/pair/{}", server.base_url, request_id))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(status["status"], "pending");
    assert!(status["token"].is_null());
}

#[tokio::test]
async fn rejected_pairing_request_never_yields_a_token() {
    let server = common::spawn().await;
    let client = reqwest::Client::new();

    let session: serde_json::Value = client
        .get(format!("{}/api/devices/pairing/session", server.base_url))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let code = session["code"].as_str().unwrap();

    let init: serde_json::Value = client
        .post(format!("{}/api/devices/pair", server.base_url))
        .json(&serde_json::json!({ "code": code, "device_name": "Rejected Phone" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let request_id = init["request_id"].as_str().unwrap();

    let reject = client
        .post(format!("{}/api/devices/pairing/{}/reject", server.base_url, request_id))
        .send()
        .await
        .unwrap();
    assert!(reject.status().is_success());

    let status: serde_json::Value = client
        .get(format!("{}/api/devices/pair/{}", server.base_url, request_id))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(status["status"], "rejected");
    assert!(status["token"].is_null());
}
