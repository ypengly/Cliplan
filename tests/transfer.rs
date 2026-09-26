mod common;

use sha2::{Digest, Sha256};

#[tokio::test]
async fn upload_download_roundtrip_preserves_bytes_and_hash() {
    let server = common::spawn().await;
    let token = common::pair_device(&server.base_url, "Uploader").await;
    let client = reqwest::Client::new();

    let content = b"Hello from ClipLAN!".repeat(1000);
    let mut hasher = Sha256::new();
    hasher.update(&content);
    let expected_hash = hex::encode(hasher.finalize());

    let part = reqwest::multipart::Part::bytes(content.clone()).file_name("greeting.txt");
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

    assert_eq!(upload["filename"], "greeting.txt");
    assert_eq!(upload["sha256"], expected_hash);
    assert_eq!(upload["size"], content.len());

    let transfer_id = upload["transfer_id"].as_str().unwrap();

    let downloaded = client
        .get(format!("{}/api/files/{}/download", server.base_url, transfer_id))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert!(downloaded.status().is_success());
    let bytes = downloaded.bytes().await.unwrap();
    assert_eq!(bytes.as_ref(), content.as_slice());

    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    assert_eq!(hex::encode(hasher.finalize()), expected_hash);

    let history: serde_json::Value = client
        .get(format!("{}/api/transfers", server.base_url))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let history = history.as_array().unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0]["status"], "completed");
}

#[tokio::test]
async fn oversized_upload_is_rejected() {
    let server = common::spawn_custom(|cfg| cfg.max_upload_size = 1024).await;
    let token = common::pair_device(&server.base_url, "Uploader").await;
    let client = reqwest::Client::new();

    let content = vec![0u8; 4096]; // larger than the 1 KiB limit
    let part = reqwest::multipart::Part::bytes(content).file_name("big.bin");
    let form = reqwest::multipart::Form::new().part("file", part);

    let resp = client
        .post(format!("{}/api/files/upload", server.base_url))
        .bearer_auth(&token)
        .multipart(form)
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), reqwest::StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn deleting_a_transfer_removes_the_file() {
    let server = common::spawn().await;
    let token = common::pair_device(&server.base_url, "Uploader").await;
    let client = reqwest::Client::new();

    let part = reqwest::multipart::Part::bytes(b"temp file".to_vec()).file_name("temp.txt");
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
    let transfer_id = upload["transfer_id"].as_str().unwrap();

    let del = client
        .delete(format!("{}/api/transfers/{}", server.base_url, transfer_id))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert!(del.status().is_success());

    let after_delete = client
        .get(format!("{}/api/files/{}/download", server.base_url, transfer_id))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(after_delete.status(), reqwest::StatusCode::NOT_FOUND);
}
