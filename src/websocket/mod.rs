pub mod events;

use axum::{
    extract::ws::{Message, WebSocket, WebSocketUpgrade},
    extract::{Query, State},
    response::IntoResponse,
};
use futures::{SinkExt, StreamExt};
use serde::Deserialize;

use crate::pairing::identity::hash_token;
use crate::server::AppState;
use crate::storage::database;
use events::WsEvent;

#[derive(Debug, Deserialize)]
pub struct WsAuthQuery {
    /// Browsers cannot set custom headers on the WebSocket handshake, so the
    /// device token is passed as a query parameter over what is, in
    /// production, meant to eventually be wss:// (see section 23 on HTTPS).
    /// The token itself is never logged (see tracing calls below).
    token: String,
}

pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
    Query(query): Query<WsAuthQuery>,
) -> impl IntoResponse {
    let token_hash = hash_token(&query.token);
    let device = {
        let conn = state.db.lock().unwrap();
        database::find_device_by_token_hash(&conn, &token_hash)
    };

    match device {
        Ok(Some(device)) => {
            ws.on_upgrade(move |socket| handle_socket(socket, state, device.id, device.name))
        }
        _ => {
            // Reject unknown/invalid tokens outright rather than upgrading
            // and then closing, so no unauthenticated socket is ever handed
            // access to the event stream.
            axum::http::StatusCode::UNAUTHORIZED.into_response()
        }
    }
}

async fn handle_socket(socket: WebSocket, state: AppState, device_id: String, device_name: String) {
    let (mut sender, mut receiver) = socket.split();
    let mut rx = state.ws_tx.subscribe();

    {
        let conn = state.db.lock().unwrap();
        let now = crate::now_iso();
        let _ = database::touch_device(&conn, &device_id, &now, "online");
    }
    let _ = state.ws_tx.send(WsEvent::DeviceConnected {
        device_id: device_id.clone(),
        name: device_name.clone(),
    });
    tracing::info!(device = %device_name, "device connected");

    let mut send_task = tokio::spawn(async move {
        while let Ok(event) = rx.recv().await {
            // Every event is broadcast to every connected device; clients
            // filter locally. For a LAN app with a handful of devices this
            // is simpler and more robust than per-device routing tables.
            if let Ok(json) = serde_json::to_string(&event) {
                if sender.send(Message::Text(json)).await.is_err() {
                    break;
                }
            }
        }
    });

    let mut recv_task = tokio::spawn(async move {
        while let Some(Ok(msg)) = receiver.next().await {
            match msg {
                Message::Close(_) => break,
                Message::Ping(_) | Message::Pong(_) => {}
                // Clients may send lightweight "ping" text frames to keep
                // NAT/proxy connections alive; we don't expect commands
                // over the socket -- all mutations go through the REST API
                // so they get consistent auth/validation/rate-limiting.
                Message::Text(_) | Message::Binary(_) => {}
            }
        }
    });

    tokio::select! {
        _ = (&mut send_task) => { recv_task.abort(); }
        _ = (&mut recv_task) => { send_task.abort(); }
    }

    {
        let conn = state.db.lock().unwrap();
        let now = crate::now_iso();
        let _ = database::touch_device(&conn, &device_id, &now, "offline");
    }
    let _ = state.ws_tx.send(WsEvent::DeviceDisconnected { device_id: device_id.clone() });
    tracing::info!(device = %device_name, "device disconnected");
}
