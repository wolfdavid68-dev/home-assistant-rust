//! Tests avec une **fausse box TaHoma** et un **faux serveur Somfy** :
//! de petits serveurs HTTPS locaux, avec certificat auto-signé, qui répondent
//! comme l'API Overkiz.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "dans un test, un échec doit simplement faire échouer le test"
)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use ha_core::{Attributes, Core, EntityId};
use ha_somfy::cloud::CloudSession;
use ha_somfy::{Pairing, Somfy, SomfyError, pairing};
use ha_storage::Store;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;
use tokio_rustls::rustls::ServerConfig;
use tokio_rustls::rustls::pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer};
use zeroize::Zeroizing;

const TOKEN: &str = "0123456789abcdef0123";
const API: &str = "/enduser-mobile-web/1/enduserAPI";
const SHUTTER: &str = "io://1234-5678-9012/1111111";

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

/// État de la fausse box : position du volet, événements en attente, commandes reçues.
#[derive(Default)]
struct BoxState {
    closure: u64,
    pending: Vec<Value>,
    commands: Vec<Value>,
}

fn devices_json(closure: u64) -> Value {
    json!([
        {"deviceURL": SHUTTER, "label": "Volet salon",
         "controllableName": "io:RollerShutterGenericIOComponent",
         "definition": {"uiClass": "RollerShutter", "commands": [
             {"commandName": "open"}, {"commandName": "close"},
             {"commandName": "stop"}, {"commandName": "setClosure"}]},
         "states": [{"name": "core:ClosureState", "type": 1, "value": closure}]},
        {"deviceURL": "io://1234-5678-9012/2222222", "label": "Fenêtre cuisine",
         "definition": {"uiClass": "ContactSensor", "commands": []},
         "states": [{"name": "core:ContactState", "type": 3, "value": "closed"}]},
        {"deviceURL": "io://1234-5678-9012/3333333#2", "label": "Température séjour",
         "definition": {"uiClass": "TemperatureSensor"},
         "states": [{"name": "core:TemperatureState", "type": 2, "value": "20.47"}]},
        {"deviceURL": "internal://1234-5678-9012/alarm/0", "label": "Alarme",
         "definition": {"uiClass": "Alarm"},
         "states": [{"name": "internal:CurrentAlarmModeState", "type": 3, "value": "off"}]},
        {"deviceURL": "io://1234-5678-9012/4444444", "label": "Radiateur chambre",
         "definition": {"uiClass": "HeatingSystem"},
         "states": [{"name": "core:TemperatureState", "value": 19.0},
                    {"name": "core:TargetTemperatureState", "value": 20.5}]},
        // Ignoré volontairement : on n'ouvre pas une porte sans réflexion dédiée.
        {"deviceURL": "io://1234-5678-9012/5555555", "label": "Porte de garage",
         "definition": {"uiClass": "GarageDoor", "commands": [{"commandName": "open"}]}},
        // Adresse piégée : refusée.
        {"deviceURL": "io://x/../../exec", "label": "Piège",
         "definition": {"uiClass": "RollerShutter"}}
    ])
}

async fn start_fake_box() -> (String, [u8; 32], Arc<Mutex<BoxState>>) {
    let state = Arc::new(Mutex::new(BoxState {
        closure: 100,
        ..BoxState::default()
    }));
    let shared = Arc::clone(&state);
    let handler: Handler = Arc::new(move |request| {
        let authorized = request.head.contains(&format!("Bearer {TOKEN}"));
        let path = request.path.strip_prefix(API).unwrap_or_default();
        let mut box_state = shared.lock().unwrap();
        match (request.method.as_str(), path) {
            ("GET", "/apiVersion") => (200, String::new(), json!({"protocolVersion": "2026.1.0"})),
            _ if !authorized => (401, String::new(), json!({"error": "Not authenticated"})),
            ("GET", "/setup/devices") => (200, String::new(), devices_json(box_state.closure)),
            ("POST", "/events/register") => (200, String::new(), json!({"id": "listener-1"})),
            ("POST", "/events/listener-1/fetch") => {
                let events = std::mem::take(&mut box_state.pending);
                (200, String::new(), Value::Array(events))
            }
            ("POST", "/exec/apply") => {
                let body: Value = serde_json::from_str(&request.body).unwrap();
                let command = body["actions"][0]["commands"][0].clone();
                box_state.commands.push(command.clone());
                // Le volet « bouge » et la box le signale par un événement.
                box_state.closure = match command["name"].as_str() {
                    Some("open") => 0,
                    Some("close") => 100,
                    Some("setClosure") => command["parameters"][0].as_u64().unwrap(),
                    _ => box_state.closure,
                };
                let closure = box_state.closure;
                box_state.pending.push(json!({
                    "name": "DeviceStateChangedEvent", "deviceURL": SHUTTER,
                    "deviceStates": [{"name": "core:ClosureState", "type": 1, "value": closure}]
                }));
                (200, String::new(), json!({"execId": "exec-1"}))
            }
            _ => (404, String::new(), json!({"error": "not found"})),
        }
    });
    let (host, fingerprint) = https_server(handler).await;
    (host, fingerprint, state)
}

fn id(value: &str) -> EntityId {
    value.parse().unwrap()
}

async fn wait_for(core: &Core, entity: &str, check: impl Fn(&ha_core::State) -> bool) {
    let entity = id(entity);
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
async fn box_is_identified_paired_and_saved() {
    let (host, fingerprint, _) = start_fake_box().await;

    let (version, seen) = pairing::identify(&host).await.unwrap();
    assert_eq!(version, "2026.1.0");
    assert_eq!(seen, fingerprint);

    let paired = Pairing {
        host,
        gateway_id: "1234-5678-9012".into(),
        fingerprint,
        token: Zeroizing::new(TOKEN.into()),
    };
    assert_eq!(pairing::verify(&paired).await.unwrap().len(), 7);
    assert!(
        !format!("{paired:?}").contains(TOKEN),
        "jeton visible dans Debug"
    );

    let store = Store::open_in_memory().await.unwrap();
    pairing::save(&store, &paired).await.unwrap();
    // Dans la base, le jeton est chiffré.
    let raw: String = sqlx::query_scalar("SELECT token FROM somfy_box")
        .fetch_one(store.pool())
        .await
        .unwrap();
    assert!(!raw.contains(TOKEN), "jeton en clair dans la base");
    let loaded = pairing::load(&store).await.unwrap().unwrap();
    assert_eq!(loaded.token.as_str(), TOKEN);
    assert_eq!(loaded.fingerprint, fingerprint);

    // Une association faite avant le chiffrement (jeton en clair) est
    // relue, puis chiffrée.
    sqlx::query("UPDATE somfy_box SET token = ?1")
        .bind(TOKEN)
        .execute(store.pool())
        .await
        .unwrap();
    let loaded = pairing::load(&store).await.unwrap().unwrap();
    assert_eq!(loaded.token.as_str(), TOKEN);
    let raw: String = sqlx::query_scalar("SELECT token FROM somfy_box")
        .fetch_one(store.pool())
        .await
        .unwrap();
    assert!(ha_storage::is_sealed(&raw), "ancien jeton pas rechiffré");

    assert!(pairing::forget(&store).await.unwrap());
}

#[tokio::test]
async fn wrong_token_and_wrong_certificate_are_refused() {
    let (host, fingerprint, _) = start_fake_box().await;

    let wrong_token = Pairing {
        host: host.clone(),
        gateway_id: "1234-5678-9012".into(),
        fingerprint,
        token: Zeroizing::new("mauvais".into()),
    };
    assert!(matches!(
        pairing::verify(&wrong_token).await,
        Err(SomfyError::Unauthorized)
    ));

    let impostor = Pairing {
        host,
        gateway_id: "1234-5678-9012".into(),
        fingerprint: [9; 32],
        token: Zeroizing::new(TOKEN.into()),
    };
    let Err(SomfyError::Http(error)) = pairing::verify(&impostor).await else {
        panic!("le faux certificat aurait dû être refusé");
    };
    let mut cause: Option<&dyn std::error::Error> = Some(&error);
    let mut found = false;
    while let Some(current) = cause {
        found |= current.to_string().contains("certificat inattendu");
        cause = current.source();
    }
    assert!(found, "refus attendu par l'épinglage : {error}");
}

#[tokio::test]
async fn devices_become_entities_and_shutters_obey() {
    let (host, fingerprint, box_state) = start_fake_box().await;
    let core = Arc::new(Core::new());
    let somfy = Somfy::start(
        Pairing {
            host,
            gateway_id: "1234-5678-9012".into(),
            fingerprint,
            token: Zeroizing::new(TOKEN.into()),
        },
        Arc::clone(&core),
    );

    // Les appareils apparaissent, avec des états compréhensibles.
    wait_for(&core, "cover.volet_salon", |s| s.state() == "closed").await;
    let shutter = core.states.get(&id("cover.volet_salon")).unwrap();
    assert_eq!(shutter.attributes()["current_position"], 0.0);
    assert_eq!(
        core.states
            .get(&id("binary_sensor.fenetre_cuisine"))
            .unwrap()
            .state(),
        "off"
    );
    assert_eq!(
        core.states
            .get(&id("sensor.temperature_sejour"))
            .unwrap()
            .state(),
        "20.5"
    );
    assert_eq!(
        core.states.get(&id("sensor.alarme")).unwrap().state(),
        "off"
    );
    let heating = core.states.get(&id("sensor.radiateur_chambre")).unwrap();
    assert_eq!(heating.state(), "19");
    assert_eq!(heating.attributes()["target_temperature"], 20.5);
    // Porte de garage et appareil piégé : ignorés.
    assert!(core.states.get(&id("cover.porte_de_garage")).is_none());
    assert!(core.states.get(&id("cover.piege")).is_none());

    // Ouvrir à 30 % : Somfy reçoit « fermé à 70 % ».
    let data: Attributes = json!({"position": 30}).as_object().unwrap().clone();
    core.call_service("cover", "set_cover_position", id("cover.volet_salon"), data)
        .unwrap();
    wait_for(&core, "cover.volet_salon", |s| {
        s.state() == "open" && s.attributes()["current_position"] == 30.0
    })
    .await;
    assert_eq!(
        box_state.lock().unwrap().commands.last().unwrap(),
        &json!({"name": "setClosure", "parameters": [70]})
    );

    // Fermer complètement. Juste après un ordre, la box est relevée toutes
    // les 0,5 s (étape 63) : le volet s'affiche fermé en 0,5 s environ, au
    // lieu de 2 s (la marge couvre une machine de CI chargée).
    let sent = std::time::Instant::now();
    core.call_service(
        "cover",
        "close_cover",
        id("cover.volet_salon"),
        Attributes::new(),
    )
    .unwrap();
    wait_for(&core, "cover.volet_salon", |s| s.state() == "closed").await;
    assert!(
        sent.elapsed() < Duration::from_millis(1_500),
        "fermeture vue après {:?}",
        sent.elapsed()
    );

    // Rien n'est envoyé pour une position invalide ou un capteur.
    let before = box_state.lock().unwrap().commands.len();
    // Une position hors plage est refusée par le noyau, avant l'envoi.
    let bad: Attributes = json!({"position": 150}).as_object().unwrap().clone();
    let refused = core
        .call_service("cover", "set_cover_position", id("cover.volet_salon"), bad)
        .unwrap_err();
    assert_eq!(
        refused.to_string(),
        "Volet salon : position entre 0 et 100 (reçu 150)"
    );
    core.call_service(
        "cover",
        "open_cover",
        id("sensor.alarme"),
        Attributes::new(),
    )
    .unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(box_state.lock().unwrap().commands.len(), before);

    somfy.stop().await;
}

#[tokio::test]
async fn token_is_created_with_the_somfy_account() {
    let calls = Arc::new(Mutex::new(Vec::<String>::new()));
    let log = Arc::clone(&calls);
    let handler: Handler = Arc::new(move |request| {
        log.lock()
            .unwrap()
            .push(format!("{} {}", request.method, request.path));
        let logged_in = request.head.contains("JSESSIONID=session-42");
        match (request.method.as_str(), request.path.as_str()) {
            ("POST", "/login")
                if request.body == "userId=moi%40exemple.fr&userPassword=p%40ss%20word" =>
            {
                (
                    200,
                    "Set-Cookie: JSESSIONID=session-42; Path=/; Secure; HttpOnly\r\n".into(),
                    json!({"success": true}),
                )
            }
            ("POST", "/login") => (401, String::new(), json!({"error": "Bad credentials"})),
            _ if !logged_in => (401, String::new(), json!({"error": "Not authenticated"})),
            ("GET", "/setup/gateways") => {
                (200, String::new(), json!([{"gatewayId": "1234-5678-9012"}]))
            }
            ("GET", "/config/1234-5678-9012/local/tokens/generate") => {
                (200, String::new(), json!({"token": TOKEN}))
            }
            ("POST", "/config/1234-5678-9012/local/tokens") => {
                let body: Value = serde_json::from_str(&request.body).unwrap();
                assert_eq!(body["scope"], "devmode");
                assert_eq!(body["token"], TOKEN);
                (200, String::new(), json!({"requestId": "r-1"}))
            }
            ("POST", "/logout") => (200, String::new(), json!({})),
            _ => (404, String::new(), json!({})),
        }
    });
    let (host, _) = https_server(handler).await;
    let base = format!("https://{host}");
    // Le vrai serveur Somfy est vérifié avec les autorités du système ; le
    // faux a un certificat auto-signé, d'où un client « tout accepter » ici.
    let (http, _) = ha_tls::client(None).unwrap();

    assert!(matches!(
        CloudSession::login(http.clone(), &base, "moi@exemple.fr", "mauvais").await,
        Err(SomfyError::BadCredentials)
    ));

    let session = CloudSession::login(http, &base, "moi@exemple.fr", "p@ss word")
        .await
        .unwrap();
    let gateways = session.gateways().await.unwrap();
    assert_eq!(gateways, ["1234-5678-9012"]);
    let token = session.create_local_token(&gateways[0]).await.unwrap();
    assert_eq!(token.as_str(), TOKEN);
    session.logout().await;
    assert_eq!(calls.lock().unwrap().last().unwrap(), "POST /logout");
}
