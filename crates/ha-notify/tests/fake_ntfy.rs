//! Tests avec un **faux serveur ntfy** : un petit serveur HTTP local qui
//! enregistre ce qu'il reçoit.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "dans un test, un échec doit simplement faire échouer le test"
)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use ha_core::{Attributes, Core};
use ha_notify::{Config, Notifier};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use zeroize::Zeroizing;

/// Requêtes reçues : (en-têtes, corps JSON).
type Received = Arc<Mutex<Vec<(String, Value)>>>;

async fn fake_ntfy() -> (String, Received) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let received: Received = Arc::default();
    let shared = Arc::clone(&received);
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let shared = Arc::clone(&shared);
            tokio::spawn(async move {
                let mut data = Vec::new();
                let mut buf = [0u8; 4096];
                loop {
                    let n = stream.read(&mut buf).await.unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    data.extend_from_slice(&buf[..n]);
                    let text = String::from_utf8_lossy(&data).to_string();
                    if let Some((head, body)) = text.split_once("\r\n\r\n") {
                        let length: usize = head
                            .lines()
                            .find_map(|l| {
                                l.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .map(|v| v.trim().parse().unwrap())
                            })
                            .unwrap_or(0);
                        if body.len() >= length {
                            let json = serde_json::from_str(body).unwrap();
                            shared.lock().unwrap().push((head.to_owned(), json));
                            break;
                        }
                    }
                }
                let _ = stream
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}",
                    )
                    .await;
            });
        }
    });
    (url, received)
}

fn data(value: Value) -> Attributes {
    match value {
        Value::Object(map) => map,
        _ => Attributes::new(),
    }
}

async fn wait_for(received: &Received, count: usize) {
    for _ in 0..200 {
        if received.lock().unwrap().len() >= count {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("{count} notification(s) attendue(s)");
}

#[tokio::test]
async fn notifications_reach_ntfy_with_the_token() {
    let (url, received) = fake_ntfy().await;
    let token = Zeroizing::new("tk_jeton_secret".to_owned());
    let config = Config::new(&url, "maison-test", Some(token)).unwrap();
    let core = Arc::new(Core::new());
    let notifier = Notifier::start(config, Arc::clone(&core)).unwrap();
    let entity = "notify.telephone".parse().unwrap();
    assert_eq!(core.states.get(&entity).unwrap().state(), "unknown");

    core.call_service(
        "notify",
        "send_message",
        entity.clone(),
        data(json!({"title": "Fumée !", "message": "Détecteur du salon", "priority": 5})),
    )
    .unwrap();
    wait_for(&received, 1).await;
    {
        let requests = received.lock().unwrap();
        let (head, body) = &requests[0];
        assert!(head.starts_with("POST / HTTP/1.1"), "{head}");
        assert!(
            head.contains("authorization: Bearer tk_jeton_secret"),
            "{head}"
        );
        assert_eq!(
            body,
            &json!({"topic": "maison-test", "title": "Fumée !",
                    "message": "Détecteur du salon", "priority": 5})
        );
    }

    // D'autres appels ne nous concernent pas, ou n'ont pas de message.
    core.call_service(
        "notify",
        "send_message",
        "notify.autre".parse().unwrap(),
        data(json!({"message": "x"})),
    )
    .unwrap();
    core.call_service(
        "notify",
        "send_message",
        entity.clone(),
        data(json!({"title": "vide"})),
    )
    .unwrap();

    // Une rafale (automatisation qui boucle) : au plus 10 en 10 minutes.
    for n in 0..20 {
        core.call_service(
            "notify",
            "send_message",
            entity.clone(),
            data(json!({"message": format!("message {n}")})),
        )
        .unwrap();
    }
    wait_for(&received, 10).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(received.lock().unwrap().len(), 10, "limite anti-rafale");
    assert_ne!(core.states.get(&entity).unwrap().state(), "unknown");

    notifier.stop().await;
}
