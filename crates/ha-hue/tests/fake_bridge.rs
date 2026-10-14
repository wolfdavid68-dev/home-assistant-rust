//! Tests avec un **faux pont Hue** : un petit serveur HTTPS local, avec un
//! certificat auto-signé comme le vrai pont, qui répond comme l'API v2.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use ha_core::{Attributes, Core, EntityId};
use ha_hue::{Hue, HueError, pairing};
use ha_storage::Store;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::broadcast;
use tokio_rustls::TlsAcceptor;
use tokio_rustls::rustls::ServerConfig;
use tokio_rustls::rustls::pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer};

const LIGHT_ID: &str = "3f8d6e6a-1b2c-4d5e-8f90-0123456789ab";
const APP_KEY: &str = "cle-de-test-0123456789abcdefghijklmnop";

/// État partagé du faux pont.
struct FakeBridge {
    host: String,
    fingerprint: [u8; 32],
    /// Corps des commandes `PUT` reçues.
    commands: Arc<Mutex<Vec<Value>>>,
    /// Pour pousser des événements dans le flux.
    events: broadcast::Sender<String>,
}

async fn start_fake_bridge() -> FakeBridge {
    let certified = rcgen::generate_simple_self_signed(vec!["localhost".to_owned()]).unwrap();
    let cert_der = certified.cert.der().clone();
    let fingerprint: [u8; 32] = Sha256::digest(cert_der.as_ref()).into();
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(
        certified.signing_key.serialize_der(),
    ));
    let config = ServerConfig::builder_with_provider(Arc::new(
        tokio_rustls::rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(vec![cert_der], key)
    .unwrap();
    let acceptor = TlsAcceptor::from(Arc::new(config));

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let host = listener.local_addr().unwrap().to_string();
    let commands = Arc::new(Mutex::new(Vec::new()));
    let (events, _) = broadcast::channel(16);
    let link_attempts = Arc::new(Mutex::new(0u32));

    let state = (Arc::clone(&commands), events.clone(), link_attempts);
    tokio::spawn(async move {
        loop {
            let Ok((tcp, _)) = listener.accept().await else {
                return;
            };
            let acceptor = acceptor.clone();
            let (commands, events, link_attempts) = state.clone();
            tokio::spawn(async move {
                let Ok(mut tls) = acceptor.accept(tcp).await else {
                    return; // Client qui refuse notre certificat : normal dans un test.
                };
                let (head, body) = read_request(&mut tls).await;
                let request_line = head.lines().next().unwrap_or_default().to_owned();
                let authorized = head
                    .to_ascii_lowercase()
                    .contains(&format!("hue-application-key: {APP_KEY}"));

                let reply = match request_line.split(' ').take(2).collect::<Vec<_>>()[..] {
                    ["GET", "/api/0/config"] => {
                        json!({"bridgeid": "001788FFFE123456", "name": "Faux pont"})
                    }
                    ["POST", "/api"] => {
                        let mut attempts = link_attempts.lock().unwrap();
                        *attempts += 1;
                        if *attempts == 1 {
                            json!([{"error": {"type": 101, "address": "", "description": "link button not pressed"}}])
                        } else {
                            json!([{"success": {"username": APP_KEY, "clientkey": "00"}}])
                        }
                    }
                    [_, _] if !authorized => {
                        json!({"errors": [{"description": "unauthorized user"}], "data": []})
                    }
                    ["GET", "/clip/v2/resource/light"] => json!({"errors": [], "data": [{
                        "id": LIGHT_ID, "type": "light",
                        "metadata": {"name": "Lampe salon", "archetype": "sultan_bulb"},
                        "on": {"on": true}, "dimming": {"brightness": 100.0, "min_dim_level": 0.2},
                    }]}),
                    ["PUT", path] if path == format!("/clip/v2/resource/light/{LIGHT_ID}") => {
                        let command: Value = serde_json::from_str(&body).unwrap();
                        commands.lock().unwrap().push(command.clone());
                        // Le vrai pont confirme le changement dans le flux d'événements.
                        let mut update = command;
                        update["id"] = json!(LIGHT_ID);
                        update["type"] = json!("light");
                        let _ =
                            events.send(json!([{"type": "update", "data": [update]}]).to_string());
                        json!({"errors": [], "data": [{"rid": LIGHT_ID, "rtype": "light"}]})
                    }
                    ["GET", "/eventstream/clip/v2"] => {
                        let mut rx = events.subscribe();
                        let _ = tls
                            .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n: hi\n\n")
                            .await;
                        let mut n = 0;
                        while let Ok(event) = rx.recv().await {
                            n += 1;
                            let chunk = format!("id: {n}:0\ndata: {event}\n\n");
                            if tls.write_all(chunk.as_bytes()).await.is_err() {
                                return;
                            }
                        }
                        return;
                    }
                    _ => json!({"errors": [{"description": "not found"}], "data": []}),
                };
                let body = reply.to_string();
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = tls.write_all(response.as_bytes()).await;
                let _ = tls.shutdown().await;
            });
        }
    });

    FakeBridge {
        host,
        fingerprint,
        commands,
        events,
    }
}

/// Lit une requête HTTP/1.1 : en-têtes puis corps (`Content-Length`).
async fn read_request<S: AsyncReadExt + Unpin>(stream: &mut S) -> (String, String) {
    let mut data = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        if let Some(end) = data.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&data[..end]).to_string();
            let length = head
                .lines()
                .find_map(|l| {
                    l.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                })
                .unwrap_or(0);
            while data.len() < end + 4 + length {
                let n = stream.read(&mut buf).await.unwrap_or(0);
                if n == 0 {
                    break;
                }
                data.extend_from_slice(&buf[..n]);
            }
            let body = String::from_utf8_lossy(&data[end + 4..]).to_string();
            return (head, body);
        }
        let n = stream.read(&mut buf).await.unwrap_or(0);
        if n == 0 {
            return (String::from_utf8_lossy(&data).to_string(), String::new());
        }
        data.extend_from_slice(&buf[..n]);
    }
}

/// Attend qu'une condition sur l'état d'une entité soit vraie (5 s au plus).
async fn wait_for(core: &Core, entity: &str, check: impl Fn(&ha_core::State) -> bool) {
    let entity: EntityId = entity.parse().unwrap();
    for _ in 0..100 {
        if core.states.get(&entity).is_some_and(|s| check(&s)) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!(
        "condition jamais remplie pour {entity} : {:?}",
        core.states.get(&entity)
    );
}

#[tokio::test]
async fn pairing_pins_the_certificate_and_waits_for_the_button() {
    let bridge = start_fake_bridge().await;

    let (bridge_id, fingerprint) = pairing::identify(&bridge.host).await.unwrap();
    assert_eq!(bridge_id, "001788FFFE123456");
    assert_eq!(fingerprint, bridge.fingerprint);

    // Le premier essai répond « bouton non pressé », le second donne la clé.
    let paired = pairing::request_key(&bridge.host, bridge_id, fingerprint)
        .await
        .unwrap();
    assert_eq!(paired.app_key.as_str(), APP_KEY);
    assert!(
        !format!("{paired:?}").contains(APP_KEY),
        "clé visible dans Debug"
    );

    let store = Store::open_in_memory().await.unwrap();
    pairing::save(&store, &paired).await.unwrap();
    let loaded = pairing::load(&store).await.unwrap().unwrap();
    assert_eq!(loaded.fingerprint, bridge.fingerprint);
    assert_eq!(loaded.app_key.as_str(), APP_KEY);
    assert!(pairing::forget(&store).await.unwrap());
    assert!(pairing::load(&store).await.unwrap().is_none());
}

#[tokio::test]
async fn another_certificate_is_refused() {
    let bridge = start_fake_bridge().await;
    // Empreinte d'un autre certificat : un imposteur sur le réseau.
    let result = pairing::request_key(&bridge.host, "001788FFFE123456".into(), [7; 32]).await;
    let Err(HueError::Http(error)) = result else {
        panic!("refus attendu, reçu {result:?}");
    };
    // C'est bien notre vérification d'empreinte qui a refusé la connexion.
    let mut cause: Option<&dyn std::error::Error> = Some(&error);
    let mut messages = Vec::new();
    while let Some(current) = cause {
        messages.push(current.to_string());
        cause = current.source();
    }
    assert!(
        messages
            .iter()
            .any(|m| m.contains("certificat du pont Hue inattendu")),
        "{messages:?}"
    );
}

#[tokio::test]
async fn lights_are_synced_controlled_and_updated_live() {
    let bridge = start_fake_bridge().await;
    let core = Arc::new(Core::new());
    let hue = Hue::start(
        pairing::Pairing {
            host: bridge.host.clone(),
            bridge_id: "001788FFFE123456".into(),
            fingerprint: bridge.fingerprint,
            app_key: zeroize::Zeroizing::new(APP_KEY.to_owned()),
        },
        Arc::clone(&core),
    );

    // 1. La lampe apparaît, allumée à 100 %.
    wait_for(&core, "light.lampe_salon", |s| {
        s.state() == "on" && s.attributes()["brightness"] == 255
    })
    .await;

    // 2. Commande depuis le noyau : luminosité 128 (≈ 50 %).
    let data = json!({"brightness": 128}).as_object().unwrap().clone();
    core.call_service(
        "light",
        "turn_on",
        "light.lampe_salon".parse().unwrap(),
        data,
    )
    .unwrap();
    wait_for(&core, "light.lampe_salon", |s| {
        s.attributes()["brightness"] == 128
    })
    .await;
    assert_eq!(
        bridge.commands.lock().unwrap().last().unwrap(),
        &json!({"on": {"on": true}, "dimming": {"brightness": 50.2}})
    );

    // 3. Changement fait ailleurs (appli Hue, Alexa…) : reçu en direct.
    bridge
        .events
        .send(json!([{"type": "update", "data": [{"id": LIGHT_ID, "type": "light", "on": {"on": false}}]}]).to_string())
        .unwrap();
    wait_for(&core, "light.lampe_salon", |s| s.state() == "off").await;

    // 4. Les services d'autres domaines ou entités ne partent pas vers le pont.
    let before = bridge.commands.lock().unwrap().len();
    core.call_service(
        "switch",
        "turn_on",
        "light.lampe_salon".parse().unwrap(),
        Attributes::new(),
    )
    .unwrap();
    core.call_service(
        "light",
        "turn_on",
        "light.autre".parse().unwrap(),
        Attributes::new(),
    )
    .unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(bridge.commands.lock().unwrap().len(), before);

    hue.stop().await;
}
