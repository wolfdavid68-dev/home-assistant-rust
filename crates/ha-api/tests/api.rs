//! Tests de l'API : requêtes HTTP simulées, et un vrai serveur local pour le WebSocket.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "dans un test, un échec doit simplement faire échouer le test"
)]

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::extract::connect_info::MockConnectInfo;
use axum::http::{Request, StatusCode, header};
use futures_util::{SinkExt, StreamExt};
use ha_api::ApiState;
use ha_auth::Auth;
use ha_core::{Attributes, Core};
use ha_storage::{Recorder, Store};
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message;
use tower::ServiceExt;

const PASSWORD: &str = "cafe chaud sur la terrasse";

struct TestApp {
    state: Arc<ApiState>,
    recorder: Option<Recorder>,
}

impl TestApp {
    async fn new() -> Self {
        let store = Store::open_in_memory().await.unwrap();
        let auth = Auth::new(&store).await.unwrap();
        auth.create_user("david", PASSWORD).await.unwrap();
        let core = Arc::new(Core::new());
        let recorder = Recorder::start(store.clone(), Arc::clone(&core));
        let state = ApiState::new(core, store, Arc::new(auth));
        Self {
            state,
            recorder: Some(recorder),
        }
    }

    fn router(&self) -> Router {
        ha_api::router(Arc::clone(&self.state))
            .layer(MockConnectInfo(SocketAddr::from(([127, 0, 0, 1], 50_000))))
    }

    /// Envoie une requête et renvoie (statut, corps JSON).
    async fn request(
        &self,
        method: &str,
        uri: &str,
        token: Option<&str>,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut request = Request::builder().method(method).uri(uri);
        if let Some(token) = token {
            request = request.header(header::AUTHORIZATION, format!("Bearer {token}"));
        }
        let body = body.map_or_else(Body::empty, |b| Body::from(b.to_string()));
        let response = self
            .router()
            .oneshot(request.body(body).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
            .await
            .unwrap();
        let json = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        (status, json)
    }

    async fn login(&self) -> String {
        let (status, body) = self
            .request(
                "POST",
                "/api/auth/login",
                None,
                Some(json!({"username": "david", "password": PASSWORD})),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body["access_token"].as_str().unwrap().to_owned()
    }
}

#[tokio::test]
async fn every_route_requires_a_token() {
    let app = TestApp::new().await;
    for (method, uri) in [
        ("GET", "/api/"),
        ("GET", "/api/states"),
        ("GET", "/api/states/light.salon"),
        ("POST", "/api/states/light.salon"),
        ("DELETE", "/api/states/light.salon"),
        ("GET", "/api/history/light.salon"),
        ("POST", "/api/auth/logout"),
        ("POST", "/api/services/switch/turn_on"),
        ("GET", "/api/audit"),
    ] {
        let (status, _) = app.request(method, uri, None, None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{method} {uri}");

        let fake = format!("hat_{}", "a".repeat(64));
        let (status, _) = app.request(method, uri, Some(&fake), None).await;
        assert_eq!(
            status,
            StatusCode::UNAUTHORIZED,
            "{method} {uri} (faux jeton)"
        );
    }
}

#[tokio::test]
async fn login_then_read_and_write_states() {
    let app = TestApp::new().await;
    let token = app.login().await;

    let (status, body) = app.request("GET", "/api/", Some(&token), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["user"], "david");

    let (status, body) = app
        .request(
            "POST",
            "/api/states/light.salon",
            Some(&token),
            Some(json!({"state": "on", "attributes": {"brightness": 200}})),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(body["state"], "on");
    assert!(body["last_changed"].as_str().unwrap().ends_with('Z'));

    let (status, body) = app
        .request("GET", "/api/states/light.salon", Some(&token), None)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["attributes"]["brightness"], 200);

    let (status, body) = app.request("GET", "/api/states", Some(&token), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.as_array().unwrap().len(), 1);

    let (status, _) = app
        .request("DELETE", "/api/states/light.salon", Some(&token), None)
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = app
        .request("GET", "/api/states/light.salon", Some(&token), None)
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn history_is_available_through_the_api() {
    let mut app = TestApp::new().await;
    let token = app.login().await;
    for value in ["on", "off"] {
        app.request(
            "POST",
            "/api/states/switch.prise",
            Some(&token),
            Some(json!({"state": value})),
        )
        .await;
    }
    app.recorder.take().unwrap().stop().await;

    let (status, body) = app
        .request(
            "GET",
            "/api/history/switch.prise?limit=10",
            Some(&token),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let values: Vec<&str> = body["history"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["state"].as_str().unwrap())
        .collect();
    assert_eq!(values, ["off", "on"]);
}

#[tokio::test]
async fn bad_input_is_rejected_cleanly() {
    let app = TestApp::new().await;
    let token = app.login().await;

    // Identifiant invalide.
    let (status, body) = app
        .request("GET", "/api/states/pas-valide", Some(&token), None)
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["error"].is_string());

    // Champ inconnu : refusé grâce à `deny_unknown_fields`.
    let (status, _) = app
        .request(
            "POST",
            "/api/states/light.salon",
            Some(&token),
            Some(json!({"state": "on", "admin": true})),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // État trop long.
    let (status, _) = app
        .request(
            "POST",
            "/api/states/light.salon",
            Some(&token),
            Some(json!({"state": "x".repeat(300)})),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // Corps trop gros (> 64 Kio).
    let (status, _) = app
        .request(
            "POST",
            "/api/states/light.salon",
            Some(&token),
            Some(json!({"state": "on", "attributes": {"x": "y".repeat(100_000)}})),
        )
        .await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn failed_logins_are_limited() {
    let app = TestApp::new().await;
    let wrong = json!({"username": "david", "password": "mauvais mot de passe"});
    for _ in 0..ha_auth::limiter::MAX_FAILURES {
        let (status, body) = app
            .request("POST", "/api/auth/login", None, Some(wrong.clone()))
            .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(
            body["error"],
            "nom d'utilisateur, mot de passe ou code incorrect"
        );
    }

    let right = json!({"username": "david", "password": PASSWORD});
    let (status, _) = app
        .request("POST", "/api/auth/login", None, Some(right))
        .await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn logout_revokes_the_token() {
    let app = TestApp::new().await;
    let token = app.login().await;

    let (status, _) = app
        .request("POST", "/api/auth/logout", Some(&token), None)
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, _) = app.request("GET", "/api/", Some(&token), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

/// Démarre un vrai serveur sur un port libre de 127.0.0.1.
async fn spawn_server(app: &TestApp) -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(ha_api::serve(listener, Arc::clone(&app.state)));
    addr
}

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn ws_send(socket: &mut Socket, value: Value) {
    socket
        .send(Message::Text(value.to_string().into()))
        .await
        .unwrap();
}

async fn ws_recv(socket: &mut Socket) -> Value {
    let message = tokio::time::timeout(Duration::from_secs(5), socket.next())
        .await
        .expect("pas de message reçu à temps")
        .unwrap()
        .unwrap();
    match message {
        Message::Text(text) => serde_json::from_str(&text).unwrap(),
        other => panic!("message inattendu : {other:?}"),
    }
}

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

#[tokio::test]
async fn service_calls_are_published_on_the_bus() {
    let app = TestApp::new().await;
    let token = app.login().await;
    let mut events = app.state.core.bus.subscribe();

    let (status, _) = app
        .request(
            "POST",
            "/api/services/light/turn_on",
            Some(&token),
            Some(json!({"entity_id": "light.salon", "brightness": 128})),
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    let event = events.try_recv().unwrap();
    let ha_core::Event::CallService {
        domain,
        service,
        entity_id,
        data,
    } = event.as_ref()
    else {
        panic!("CallService attendu, reçu {event:?}");
    };
    assert_eq!((domain.as_str(), service.as_str()), ("light", "turn_on"));
    assert_eq!(entity_id.as_str(), "light.salon");
    assert_eq!(data["brightness"], 128);

    // Sans entity_id, ou avec un nom de service invalide : refusé.
    for (uri, body) in [
        ("/api/services/light/turn_on", json!({"brightness": 1})),
        ("/api/services/light/turn_on", json!({"entity_id": 42})),
        (
            "/api/services/Light/turn_on",
            json!({"entity_id": "light.salon"}),
        ),
        (
            "/api/services/light/turn%20on",
            json!({"entity_id": "light.salon"}),
        ),
    ] {
        let (status, _) = app.request("POST", uri, Some(&token), Some(body)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{uri}");
    }
}

#[tokio::test]
async fn dashboard_files_are_served_with_security_headers() {
    let app = TestApp::new().await;
    for (uri, content_type) in [
        ("/", "text/html; charset=utf-8"),
        ("/app.js", "text/javascript; charset=utf-8"),
        ("/style.css", "text/css; charset=utf-8"),
        ("/icon.svg", "image/svg+xml"),
        ("/manifest.webmanifest", "application/manifest+json"),
    ] {
        let response = app
            .router()
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{uri}");
        let headers = response.headers();
        assert_eq!(headers[header::CONTENT_TYPE], content_type, "{uri}");
        assert!(
            headers[header::CONTENT_SECURITY_POLICY]
                .to_str()
                .unwrap()
                .contains("script-src 'self'"),
            "{uri}"
        );
        assert_eq!(headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
        assert_eq!(headers[header::X_FRAME_OPTIONS], "DENY");
    }
}

#[tokio::test]
async fn api_errors_also_carry_security_headers() {
    let app = TestApp::new().await;
    let response = app
        .router()
        .oneshot(
            Request::builder()
                .uri("/api/states")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert!(
        response
            .headers()
            .contains_key(header::CONTENT_SECURITY_POLICY)
    );
    assert_eq!(response.headers()[header::REFERRER_POLICY], "no-referrer");
}

#[test]
fn dashboard_never_uses_inner_html() {
    // Règle de sécurité : les noms d'appareils viennent du réseau. La page ne
    // doit jamais les insérer comme du HTML.
    let script = include_str!("../web/app.js");
    for forbidden in [
        ".innerHTML",
        ".outerHTML",
        ".insertAdjacentHTML(",
        "document.write(",
        "eval(",
        "new Function(",
    ] {
        assert!(
            !script.contains(forbidden),
            "{forbidden} interdit dans app.js"
        );
    }
}

#[tokio::test]
async fn login_accepts_an_optional_two_factor_code() {
    let app = TestApp::new().await;
    // Sans double authentification activée, un code envoyé est ignoré.
    let (status, body) = app
        .request(
            "POST",
            "/api/auth/login",
            None,
            Some(json!({"username": "david", "password": PASSWORD, "code": "123456"})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    // Un champ inconnu reste refusé.
    let (status, _) = app
        .request(
            "POST",
            "/api/auth/login",
            None,
            Some(json!({"username": "david", "password": PASSWORD, "otp": "123456"})),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn actions_are_recorded_in_the_audit_log() {
    let app = TestApp::new().await;
    // Un échec de connexion, avec un nom piégé (séquence d'échappement).
    let (status, _) = app
        .request(
            "POST",
            "/api/auth/login",
            None,
            Some(json!({"username": "x\u{1b}[2J", "password": "mauvais mot de passe"})),
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let token = app.login().await;
    let (status, _) = app
        .request(
            "POST",
            "/api/services/switch/turn_on",
            Some(&token),
            Some(json!({"entity_id": "switch.prise_cafe"})),
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    let (status, body) = app
        .request("GET", "/api/audit?limit=10", Some(&token), None)
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let entries = body["entries"].as_array().unwrap();
    let actions: Vec<&str> = entries
        .iter()
        .map(|e| e["action"].as_str().unwrap())
        .collect();
    assert_eq!(actions, ["service", "login", "login_failed"]);
    assert_eq!(entries[0]["actor"], "david");
    assert_eq!(entries[0]["target"], "switch.prise_cafe");
    assert_eq!(entries[0]["detail"], "switch.turn_on");
    assert_eq!(entries[0]["source"], "127.0.0.1");
    // Les caractères de contrôle ont été neutralisés.
    let failed_actor = entries[2]["actor"].as_str().unwrap();
    assert!(!failed_actor.contains('\u{1b}'), "{failed_actor:?}");
    // Le mot de passe n'apparaît nulle part.
    assert!(!body.to_string().contains("mauvais mot de passe"));

    // Les tentatives bloquées (après trop d'échecs) ne remplissent pas le journal.
    for _ in 0..10 {
        app.request(
            "POST",
            "/api/auth/login",
            None,
            Some(json!({"username": "david", "password": "encore faux encore"})),
        )
        .await;
    }
    let (_, body) = app
        .request("GET", "/api/audit?limit=100", Some(&token), None)
        .await;
    let failures = body["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["action"] == "login_failed")
        .count();
    // 1 échec au début, puis 5 avant le blocage ; les 5 suivants, bloqués,
    // ne sont pas enregistrés.
    assert_eq!(failures, 1 + ha_auth::limiter::MAX_FAILURES as usize);
    assert_eq!(
        app.state.store.verify_audit().await.unwrap(),
        ha_storage::AuditCheck::Intact {
            entries: 3 + u64::from(ha_auth::limiter::MAX_FAILURES)
        }
    );
}
