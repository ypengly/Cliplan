mod common;

#[tokio::test]
async fn status_endpoint_is_public() {
    let server = common::spawn().await;
    let resp = reqwest::get(format!("{}/api/status", server.base_url)).await.unwrap();
    assert!(resp.status().is_success());
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["name"], "ClipLAN");
}

#[tokio::test]
async fn clipboard_endpoints_require_auth() {
    let server = common::spawn().await;
    let client = reqwest::Client::new();

    let resp = client
        .get(format!("{}/api/clipboard", server.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);

    let resp = client
        .post(format!("{}/api/clipboard", server.base_url))
        .json(&serde_json::json!({ "content": "hi" }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn clipboard_roundtrip() {
    let server = common::spawn().await;
    let token = common::pair_device(&server.base_url, "PC").await;
    let client = reqwest::Client::new();

    let created: serde_json::Value = client
        .post(format!("{}/api/clipboard", server.base_url))
        .bearer_auth(&token)
        .json(&serde_json::json!({ "content": "https://example.com" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(created["content_type"], "url");
    let id = created["id"].as_str().unwrap().to_string();

    let list: serde_json::Value = client
        .get(format!("{}/api/clipboard", server.base_url))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let list = list.as_array().unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["content"], "https://example.com");

    let del = client
        .delete(format!("{}/api/clipboard/{}", server.base_url, id))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert!(del.status().is_success());

    let list: serde_json::Value = client
        .get(format!("{}/api/clipboard", server.base_url))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list.as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn empty_clipboard_content_is_rejected() {
    let server = common::spawn().await;
    let token = common::pair_device(&server.base_url, "PC").await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{}/api/clipboard", server.base_url))
        .bearer_auth(&token)
        .json(&serde_json::json!({ "content": "" }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn device_can_be_renamed() {
    let server = common::spawn().await;
    let token = common::pair_device(&server.base_url, "Old Name").await;
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
    let id = devices[0]["id"].as_str().unwrap().to_string();

    let resp = client
        .patch(format!("{}/api/devices/{}", server.base_url, id))
        .bearer_auth(&token)
        .json(&serde_json::json!({ "name": "New Name" }))
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success());

    let devices: serde_json::Value = client
        .get(format!("{}/api/devices", server.base_url))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(devices[0]["name"], "New Name");
}
