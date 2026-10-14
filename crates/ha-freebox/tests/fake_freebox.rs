//! Tests avec une **fausse Freebox** : un petit serveur HTTPS local, avec
//! certificat auto-signé, qui répond comme l'API de Freebox OS.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "dans un test, un échec doit simplement faire échouer le test"
)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use ha_core::Core;
use ha_freebox::{Freebox, FreeboxError, pairing};
use ha_storage::Store;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;
use tokio_rustls::rustls::ServerConfig;
use tokio_rustls::rustls::pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer};

const APP_TOKEN: &str = "jeton-application-tres-secret";
const CHALLENGE: &str = "defi-aleatoire-42";
const SESSION: &str = "session-1";
const PHONE: &str = "AA:BB:CC:DD:EE:01";

/// Une requête reçue par un faux serveur.
struct Request {
    method: String,
    path: String,
    head: String,
    body: String,
}

type Handler = Arc<dyn Fn(&Request) -> (u16, String, Value) + Send + Sync>;

/// Démarre un serveur HTTPS local. Renvoie (`hôte:port`, empreinte du certificat).
async fn https_server(handler: Handler) -> (String, [u8; 32]) {
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

    tokio::spawn(async move {
        while let Ok((tcp, _)) = listener.accept().await {
            let acceptor = acceptor.clone();
            let handler = Arc::clone(&handler);
            tokio::spawn(async move {
                let Ok(mut tls) = acceptor.accept(tcp).await else {
                    return;
                };
                let request = read_request(&mut tls).await;
                let (status, extra_headers, body) = handler(&request);
                let body = body.to_string();
                let response = format!(
                    "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{extra_headers}Connection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = tls.write_all(response.as_bytes()).await;
                let _ = tls.shutdown().await;
            });
        }
    });
    (host, fingerprint)
}

async fn read_request<S: AsyncReadExt + Unpin>(stream: &mut S) -> Request {
    let mut data = Vec::new();
    let mut buf = [0u8; 4096];
    let end = loop {
        if let Some(end) = data.windows(4).position(|w| w == b"\r\n\r\n") {
            break end;
        }
        let n = stream.read(&mut buf).await.unwrap_or(0);
        if n == 0 {
            break data.len();
        }
        data.extend_from_slice(&buf[..n]);
    };
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
    let body = String::from_utf8_lossy(data.get(end + 4..).unwrap_or_default()).to_string();
    let mut first = head.lines().next().unwrap_or_default().split(' ');
    Request {
        method: first.next().unwrap_or_default().to_owned(),
        path: first.next().unwrap_or_default().to_owned(),
        head,
        body,
    }
}

/// État de la fausse Freebox.
struct FreeboxState {
    /// Réponse à l'autorisation sur l'écran : `granted` ou `denied`.
    decision: &'static str,
    /// Nombre de fois où l'on a demandé l'état de l'autorisation.
    polls: u32,
    /// Prochaine requête authentifiée : session expirée.
    expire_session: bool,
    sessions_opened: u32,
    hosts: Vec<Value>,
}

fn lan_host(mac: &str, name: &str, active: bool) -> Value {
    json!({"id": format!("ether-{mac}"), "primary_name": name, "host_type": "smartphone",
           "vendor_name": "Fabricant", "active": active, "reachable": active,
           "l2ident": {"id": mac.to_lowercase(), "type": "mac_address"}})
}

fn ok(result: &Value) -> (u16, String, Value) {
    (
        200,
        String::new(),
        json!({"success": true, "result": result}),
    )
}

fn error(code: &str) -> (u16, String, Value) {
    (
        403,
        String::new(),
        json!({"success": false, "error_code": code, "msg": "refusé"}),
    )
}

async fn start_fake_freebox(
    decision: &'static str,
) -> (String, [u8; 32], Arc<Mutex<FreeboxState>>) {
    let state = Arc::new(Mutex::new(FreeboxState {
        decision,
        polls: 0,
        expire_session: false,
        sessions_opened: 0,
        hosts: vec![
            lan_host(PHONE, "iPhone de David", true),
            lan_host("AA:BB:CC:DD:EE:02", "Télé", false),
        ],
    }));
    let shared = Arc::clone(&state);
    let handler: Handler = Arc::new(move |request| {
        let mut state = shared.lock().unwrap();
        let authorized = request.head.contains(&format!("X-Fbx-App-Auth: {SESSION}"))
            || request.head.contains(&format!("x-fbx-app-auth: {SESSION}"));
        match (request.method.as_str(), request.path.as_str()) {
            ("GET", "/api_version") => (
                200,
                String::new(),
                json!({
                "uid": "0123456789abcdef", "device_name": "Freebox Server",
                "api_version": "12.0", "device_type": "FreeboxServer7,1",
                "api_base_url": "/api/"}),
            ),
            ("POST", "/api/v6/login/authorize/") => {
                let body: Value = serde_json::from_str(&request.body).unwrap();
                assert_eq!(body["app_id"], "fr.ha-rust");
                ok(&json!({"app_token": APP_TOKEN, "track_id": 7}))
            }
            ("GET", "/api/v6/login/authorize/7") => {
                state.polls += 1;
                let status = if state.polls < 2 {
                    "pending"
                } else {
                    state.decision
                };
                ok(&json!({"status": status, "challenge": "x"}))
            }
            ("GET", "/api/v6/login/") => ok(&json!({"logged_in": false, "challenge": CHALLENGE})),
            ("POST", "/api/v6/login/session/") => {
                let body: Value = serde_json::from_str(&request.body).unwrap();
                // Le jeton ne doit jamais circuler : seulement HMAC(jeton, défi).
                assert!(!request.body.contains(APP_TOKEN));
                let expected = ha_freebox::api::challenge_password(APP_TOKEN, CHALLENGE);
                if body["password"] == expected.as_str() {
                    state.sessions_opened += 1;
                    ok(&json!({"session_token": SESSION,
                              "permissions": {"settings": false, "explorer": true}}))
                } else {
                    error("invalid_token")
                }
            }
            _ if !authorized => error("auth_required"),
            _ if state.expire_session => {
                state.expire_session = false;
                error("auth_required")
            }
            ("GET", "/api/v6/lan/browser/interfaces/") => {
                ok(&json!([{"name": "pub", "host_count": 2}]))
            }
            ("GET", "/api/v6/lan/browser/pub/") => ok(&Value::Array(state.hosts.clone())),
            ("GET", "/api/v6/connection/") => ok(&json!({
                "state": "up", "media": "ftth", "rate_down": 62_500_000, "rate_up": 2_500_000})),
            _ => (
                404,
                String::new(),
                json!({"success": false, "error_code": "not_found"}),
            ),
        }
    });
    let (host, fingerprint) = https_server(handler).await;
    (host, fingerprint, state)
}

async fn wait_for(core: &Core, entity: &str, expected: &str) {
    for _ in 0..200 {
        if core
            .states
            .get(&entity.parse().unwrap())
            .is_some_and(|state| state.state() == expected)
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("{entity} n'est jamais passé à {expected:?}");
}

#[tokio::test]
async fn freebox_is_identified_authorized_and_saved() {
    let (host, fingerprint, state) = start_fake_freebox("granted").await;
    let (version, seen) = pairing::identify(&host).await.unwrap();
    assert_eq!(version.device_name, "Freebox Server");
    assert_eq!(seen, fingerprint);

    let paired = pairing::request_token(&host, &version.device_name, seen, Duration::from_secs(10))
        .await
        .unwrap();
    assert!(state.lock().unwrap().polls >= 2, "attend la validation");
    assert!(
        !format!("{paired:?}").contains(APP_TOKEN),
        "jeton visible dans Debug"
    );
    let (_, permissions) = pairing::connect(&paired).await.unwrap();
    assert_eq!(permissions, ["explorer"]);

    let store = Store::open_in_memory().await.unwrap();
    pairing::save(&store, &paired).await.unwrap();
    let raw: String = sqlx::query_scalar("SELECT app_token FROM freebox_box")
        .fetch_one(store.pool())
        .await
        .unwrap();
    assert!(!raw.contains(APP_TOKEN), "jeton en clair dans la base");
    let loaded = pairing::load(&store).await.unwrap().unwrap();
    assert_eq!(loaded.app_token.as_str(), APP_TOKEN);
    assert_eq!(loaded.fingerprint, fingerprint);
    assert!(pairing::forget(&store).await.unwrap());
    assert!(pairing::load(&store).await.unwrap().is_none());
}

#[tokio::test]
async fn refused_authorization_and_wrong_certificate() {
    let (host, fingerprint, _) = start_fake_freebox("denied").await;
    assert!(matches!(
        pairing::request_token(&host, "Freebox", fingerprint, Duration::from_secs(10)).await,
        Err(FreeboxError::Denied)
    ));
    // Une autre « Freebox » (autre certificat) à la même adresse : refusée.
    let (other_host, _, _) = start_fake_freebox("granted").await;
    let result =
        pairing::request_token(&other_host, "Freebox", fingerprint, Duration::from_secs(10)).await;
    let message = result.unwrap_err().to_string();
    assert!(message.contains("certificat inattendu"), "{message}");
}

#[tokio::test]
async fn presence_internet_and_new_devices() {
    let (host, fingerprint, state) = start_fake_freebox("granted").await;
    let store = Store::open_in_memory().await.unwrap();
    let paired = pairing::request_token(&host, "Freebox", fingerprint, Duration::from_secs(10))
        .await
        .unwrap();
    pairing::save(&store, &paired).await.unwrap();
    pairing::track(&store, "aa-bb-cc-dd-ee-01", "telephone_david")
        .await
        .unwrap();
    // Noms et adresses invalides refusés.
    assert!(pairing::track(&store, "pas une mac", "x").await.is_err());
    assert!(
        pairing::track(&store, PHONE, "Téléphone David")
            .await
            .is_err()
    );

    let core = Arc::new(Core::new());
    let freebox = Freebox::start_with_interval(
        pairing::load(&store).await.unwrap().unwrap(),
        store.clone(),
        Arc::clone(&core),
        Duration::from_millis(100),
    );
    wait_for(&core, "device_tracker.telephone_david", "home").await;
    wait_for(&core, "binary_sensor.freebox_internet", "on").await;
    wait_for(&core, "sensor.freebox_debit_descendant", "500.0").await;

    // La session expire : le serveur en rouvre une tout seul.
    state.lock().unwrap().expire_session = true;
    // Un inconnu rejoint le Wi-Fi (avec un nom piégé).
    state
        .lock()
        .unwrap()
        .hosts
        .push(lan_host("AA:BB:CC:DD:EE:99", "Inconnu\u{1b}[2J", true));
    wait_for(
        &core,
        "sensor.freebox_nouvel_appareil",
        "Inconnu\u{fffd}[2J",
    )
    .await;
    assert!(
        state.lock().unwrap().sessions_opened >= 2,
        "session rouverte"
    );

    // Le nouvel appareil est dans le journal d'audit, une seule fois.
    tokio::time::sleep(Duration::from_millis(300)).await;
    let log = store.audit_log(50).await.unwrap();
    let alerts: Vec<_> = log
        .iter()
        .filter(|e| e.action == "freebox_new_device")
        .collect();
    assert_eq!(alerts.len(), 1, "{log:?}");
    assert_eq!(alerts[0].target.as_deref(), Some("AA:BB:CC:DD:EE:99"));

    // Les appareils du premier relevé n'ont pas été signalés, mais mémorisés.
    assert_eq!(pairing::known(&store).await.unwrap().len(), 3);
    freebox.stop().await;
}
