//! Le WebSocket, sur un vrai serveur local.

use std::time::Duration;

use futures_util::StreamExt;
use ha_core::Attributes;
use serde_json::json;
use tokio_tungstenite::tungstenite::Message;

use crate::common::{TestApp, spawn_server, ws_recv, ws_send};

#[tokio::test]
async fn websocket_auth_and_events() {
    let app = TestApp::new().await;
    let token = app.login().await;
    let addr = spawn_server(&app).await;

    let (mut socket, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/api/websocket"))
        .await
        .unwrap();
    assert_eq!(ws_recv(&mut socket).await["type"], "auth_required");
    ws_send(&mut socket, json!({"type": "auth", "access_token": token})).await;
    assert_eq!(ws_recv(&mut socket).await["type"], "auth_ok");

    ws_send(&mut socket, json!({"id": 1, "type": "subscribe_events"})).await;
    let reply = ws_recv(&mut socket).await;
    assert_eq!(reply["success"], true);

    app.state
        .core
        .states
        .set("light.salon".parse().unwrap(), "on", Attributes::new())
        .unwrap();
    let event = ws_recv(&mut socket).await;
    assert_eq!(event["id"], 1);
    assert_eq!(event["event"]["event_type"], "state_changed");
    assert_eq!(event["event"]["data"]["new_state"]["state"], "on");

    ws_send(&mut socket, json!({"id": 2, "type": "get_states"})).await;
    let reply = ws_recv(&mut socket).await;
    assert_eq!(reply["result"][0]["entity_id"], "light.salon");

    ws_send(&mut socket, json!({"id": 3, "type": "rm -rf"})).await;
    assert_eq!(
        ws_recv(&mut socket).await["error"]["code"],
        "unknown_command"
    );

    app.state.shutdown();
}

#[tokio::test]
async fn websocket_rejects_a_bad_token() {
    let app = TestApp::new().await;
    let addr = spawn_server(&app).await;

    let (mut socket, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/api/websocket"))
        .await
        .unwrap();
    assert_eq!(ws_recv(&mut socket).await["type"], "auth_required");
    let fake = format!("hat_{}", "b".repeat(64));
    ws_send(&mut socket, json!({"type": "auth", "access_token": fake})).await;
    assert_eq!(ws_recv(&mut socket).await["type"], "auth_invalid");

    // Le serveur ferme ensuite la connexion.
    let next = tokio::time::timeout(Duration::from_secs(5), socket.next())
        .await
        .unwrap();
    assert!(matches!(next, Some(Ok(Message::Close(_)) | Err(_)) | None));

    app.state.shutdown();
}
