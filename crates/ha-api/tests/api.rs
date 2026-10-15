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
        Self::with_editor(true).await
    }

    /// `editable` : l'éditeur d'automatisations accepte-t-il les modifications ?
    async fn with_editor(editable: bool) -> Self {
        let store = Store::open_in_memory().await.unwrap();
        let auth = Auth::new(&store).await.unwrap();
        auth.create_user("david", PASSWORD).await.unwrap();
        let core = Arc::new(Core::new());
        let recorder = Recorder::start(store.clone(), Arc::clone(&core));
        let areas = ha_areas::parse(
            "- {area: Cuisine, entities: [{entity: switch.prise_cafe, name: Cafetière}]}",
        )
        .unwrap();
        let file = ha_automation::parse_configs(
            "- {id: matin, triggers: [{trigger: time, at: '07:30'}], \
               actions: [{action: cover.open_cover, target: {entity_id: cover.salon}}]}",
        )
        .unwrap();
        let editor = ha_automation::Editor::new(store.clone(), file, false);
        let automations = ha_api::AutomationEditor {
            editor: Arc::new(editor),
            editable,
        };
        let state = ApiState::new(core, store, Arc::new(auth), areas, Some(automations));
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

    /// Crée un compte avec ce rôle et renvoie son jeton.
    async fn login_as(&self, username: &str, role: ha_auth::Role) -> String {
        self.state
            .auth
            .create_user_as(username, PASSWORD, role)
            .await
            .unwrap();
        let (status, body) = self
            .request(
                "POST",
                "/api/auth/login",
                None,
                Some(json!({"username": username, "password": PASSWORD})),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body["access_token"].as_str().unwrap().to_owned()
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
        ("GET", "/api/logbook"),
        ("GET", "/api/statistics/sensor.temperature"),
        ("GET", "/api/areas"),
        ("GET", "/api/history/light.salon/series"),
        ("POST", "/api/auth/confirm"),
        ("GET", "/api/automations"),
        ("POST", "/api/automations/check"),
        ("PUT", "/api/automations/soir"),
        ("DELETE", "/api/automations/soir"),
        ("GET", "/api/automations/soir/versions"),
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
        origin,
    } = event.as_ref()
    else {
        panic!("CallService attendu, reçu {event:?}");
    };
    assert_eq!((domain.as_str(), service.as_str()), ("light", "turn_on"));
    assert_eq!(*origin, ha_core::Origin::User("david".into()));
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
        ("/fonts/geist.woff2", "font/woff2"),
        ("/fonts/geist-mono.woff2", "font/woff2"),
        ("/fonts/bricolage-grotesque.woff2", "font/woff2"),
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
async fn embedded_fonts_are_real_woff2_and_allowed_by_the_policy() {
    let app = TestApp::new().await;
    let response = app
        .router()
        .oneshot(
            Request::builder()
                .uri("/fonts/geist.woff2")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let policy = response.headers()[header::CONTENT_SECURITY_POLICY]
        .to_str()
        .unwrap()
        .to_owned();
    // Les polices viennent du serveur lui-même, jamais d'un site extérieur.
    assert!(policy.contains("font-src 'self'"), "{policy}");
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    assert!(body.starts_with(b"wOF2"), "pas un fichier WOFF2");
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

#[tokio::test]
async fn areas_are_served() {
    let app = TestApp::new().await;
    let token = app.login().await;
    let (status, body) = app.request("GET", "/api/areas", Some(&token), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body,
        json!({"areas": [{"name": "Cuisine",
                          "entities": [{"entity_id": "switch.prise_cafe", "name": "Cafetière"}]}]})
    );
}

#[tokio::test]
async fn history_series_for_a_chart() {
    let mut app = TestApp::new().await;
    let token = app.login().await;
    for value in ["20.5", "21.5", "unavailable", "22"] {
        let (status, _) = app
            .request(
                "POST",
                "/api/states/sensor.temperature",
                Some(&token),
                Some(json!({"state": value})),
            )
            .await;
        assert!(status.is_success());
    }
    // L'enregistreur écrit tout avant de s'arrêter.
    app.recorder.take().unwrap().stop().await;

    let (status, body) = app
        .request(
            "GET",
            "/api/history/sensor.temperature/series?hours=24",
            Some(&token),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    // Les mesures, très proches dans le temps, tombent dans la même tranche.
    let points = body["numeric"].as_array().unwrap();
    assert_eq!(points.len(), 1, "{body}");
    assert_eq!(points[0]["min"], 20.5);
    assert_eq!(points[0]["max"], 22.0);
    let changes: Vec<&str> = body["changes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["state"].as_str().unwrap())
        .collect();
    assert_eq!(changes, ["20.5", "21.5", "unavailable", "22"]);

    for bad in ["hours=0", "hours=10000", "hours=abc", "jours=2"] {
        let (status, _) = app
            .request(
                "GET",
                &format!("/api/history/sensor.temperature/series?{bad}"),
                Some(&token),
                None,
            )
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}");
    }
}

const SOIR: &str = r#"
id: soir
alias: Lampe le soir
triggers:
  - trigger: state
    entity_id: binary_sensor.porte
    to: "on"
actions:
  - action: light.turn_on
    target: {entity_id: light.salon}
"#;

#[tokio::test]
async fn automations_are_listed_and_checked() {
    let app = TestApp::new().await;
    let token = app.login().await;

    let (status, body) = app
        .request("GET", "/api/automations", Some(&token), None)
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["editable"], true);
    assert_eq!(body["confirmed_for"], Value::Null);
    assert_eq!(body["automations"][0]["id"], "matin");
    assert_eq!(body["automations"][0]["source"], "file");
    assert!(
        body["automations"][0]["yaml"]
            .as_str()
            .unwrap()
            .starts_with("id: matin\n")
    );

    // YAML → formulaire (JSON), avec les entités inconnues signalées.
    app.state
        .core
        .states
        .set("light.salon".parse().unwrap(), "off", Attributes::new())
        .unwrap();
    let (status, body) = app
        .request(
            "POST",
            "/api/automations/check",
            Some(&token),
            Some(json!({"yaml": SOIR})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["config"]["triggers"][0]["to"], "on");
    assert_eq!(
        body["warnings"],
        json!(["binary_sensor.porte n'existe pas (pour l'instant)"])
    );
    // … et retour : formulaire → YAML.
    let (status, back) = app
        .request(
            "POST",
            "/api/automations/check",
            Some(&token),
            Some(json!({"config": body["config"]})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{back}");
    assert_eq!(back["yaml"], body["yaml"]);

    // Les erreurs sont expliquées.
    for request in [
        json!({"yaml": SOIR.replace("light.turn_on", "allume")}),
        json!({"yaml": SOIR.replace("id: soir", "id: matin")}),
        json!({"config": {"id": "x"}}),
        json!({"yaml": SOIR, "config": {}}),
        json!({}),
    ] {
        let (status, body) = app
            .request(
                "POST",
                "/api/automations/check",
                Some(&token),
                Some(request),
            )
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert!(body["error"].as_str().is_some_and(|e| !e.is_empty()));
    }
}

#[tokio::test]
async fn changing_automations_requires_a_recent_confirmation() {
    let app = TestApp::new().await;
    let token = app.login().await;
    let save = json!({"yaml": SOIR});

    let (status, body) = app
        .request(
            "PUT",
            "/api/automations/soir",
            Some(&token),
            Some(save.clone()),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["confirm"], true);

    let (status, _) = app
        .request(
            "POST",
            "/api/auth/confirm",
            Some(&token),
            Some(json!({"password": "mauvais mot de passe"})),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "le jeton reste valide");
    let (status, body) = app
        .request(
            "POST",
            "/api/auth/confirm",
            Some(&token),
            Some(json!({"password": PASSWORD})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["confirmed_for"], 900);

    let (status, body) = app
        .request(
            "PUT",
            "/api/automations/soir",
            Some(&token),
            Some(save.clone()),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let (status, _) = app
        .request(
            "PUT",
            "/api/automations/soir",
            Some(&token),
            Some(save.clone()),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "modification");
    let (status, _) = app
        .request(
            "PUT",
            "/api/automations/autre",
            Some(&token),
            Some(save.clone()),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "identifiant différent de l'adresse"
    );
    let (status, _) = app
        .request("DELETE", "/api/automations/matin", Some(&token), None)
        .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "le fichier est en lecture seule"
    );

    let (_, body) = app
        .request("GET", "/api/automations", Some(&token), None)
        .await;
    assert!(body["confirmed_for"].as_u64().unwrap() > 800);
    assert_eq!(body["automations"][1]["id"], "soir");
    assert_eq!(body["automations"][1]["updated_by"], "david");

    let (status, _) = app
        .request("DELETE", "/api/automations/soir", Some(&token), None)
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = app
        .request("DELETE", "/api/automations/soir", Some(&token), None)
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, body) = app
        .request("GET", "/api/automations/soir/versions", Some(&token), None)
        .await;
    assert_eq!(status, StatusCode::OK);
    let deleted: Vec<bool> = body["versions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["deleted"].as_bool().unwrap())
        .collect();
    assert_eq!(deleted, [true, false, false]);
    assert_eq!(body["versions"][1]["config"]["id"], "soir");

    // Tout est dans le journal d'audit.
    let (_, body) = app.request("GET", "/api/audit", Some(&token), None).await;
    let actions: Vec<&str> = body["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["action"].as_str().unwrap())
        .collect();
    for expected in [
        "confirm_failed",
        "confirm",
        "automation_create",
        "automation_update",
        "automation_delete",
    ] {
        assert!(actions.contains(&expected), "{expected} : {actions:?}");
    }

    // La confirmation est liée au jeton : un autre jeton doit confirmer à
    // son tour, et elle disparaît à la déconnexion.
    let other = app.login().await;
    let (status, _) = app
        .request(
            "PUT",
            "/api/automations/soir",
            Some(&other),
            Some(save.clone()),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    app.request("POST", "/api/auth/logout", Some(&token), None)
        .await;
    let (status, _) = app
        .request("PUT", "/api/automations/soir", Some(&token), Some(save))
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_read_only_editor_refuses_changes() {
    let app = TestApp::with_editor(false).await;
    let token = app.login().await;
    app.request(
        "POST",
        "/api/auth/confirm",
        Some(&token),
        Some(json!({"password": PASSWORD})),
    )
    .await;
    let (status, body) = app
        .request(
            "PUT",
            "/api/automations/soir",
            Some(&token),
            Some(json!({"yaml": SOIR})),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(
        body["error"].as_str().unwrap().contains("lecture seule"),
        "{body}"
    );
    let (_, body) = app
        .request("GET", "/api/automations", Some(&token), None)
        .await;
    assert_eq!(body["editable"], false);
}

#[tokio::test]
async fn the_logbook_is_read_with_filters() {
    let app = TestApp::new().await;
    let token = app.login().await;
    let door: ha_core::EntityId = "binary_sensor.porte".parse().unwrap();
    for (entity_id, name) in [(Some(&door), "Porte"), (None, "Bonne nuit")] {
        app.state
            .store
            .add_logbook(ha_storage::NewLogbookEntry {
                entity_id,
                name,
                message: "message",
                cause: Some("par david"),
            })
            .await
            .unwrap();
    }

    let (status, body) = app.request("GET", "/api/logbook", Some(&token), None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let names: Vec<&str> = body["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["Bonne nuit", "Porte"]);
    assert_eq!(body["entries"][1]["entity_id"], "binary_sensor.porte");
    assert_eq!(body["entries"][1]["cause"], "par david");

    let (_, body) = app
        .request(
            "GET",
            "/api/logbook?hours=1&entity_id=binary_sensor.porte",
            Some(&token),
            None,
        )
        .await;
    assert_eq!(body["entries"].as_array().unwrap().len(), 1);

    for bad in [
        "/api/logbook?hours=0",
        "/api/logbook?hours=100000",
        "/api/logbook?entity_id=Porte",
        "/api/logbook?inconnu=1",
    ] {
        let (status, _) = app.request("GET", bad, Some(&token), None).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}");
    }
}

/// La maison des tests de rôles : une lumière, une porte de garage, un volet,
/// une personne, l'alarme.
fn furnish(app: &TestApp) {
    let set = |entity: &str, value: &str, attributes: Value| {
        let Value::Object(attributes) = attributes else {
            panic!("un objet est attendu")
        };
        app.state
            .core
            .states
            .set(entity.parse().unwrap(), value, attributes)
            .unwrap();
    };
    set("light.salon", "off", json!({}));
    set("cover.garage", "closed", json!({"device_class": "garage"}));
    set("cover.salon", "open", json!({"device_class": "shutter"}));
    set("person.david", "home", json!({}));
    set(
        "sensor.alarme",
        "disarmed",
        json!({"device_class": "alarm"}),
    );
    set("lock.entree", "locked", json!({}));
}

#[tokio::test]
async fn a_guest_sees_and_controls_only_safe_devices() {
    let app = TestApp::new().await;
    furnish(&app);
    let guest = app.login_as("visiteur", ha_auth::Role::Guest).await;

    let (_, body) = app.request("GET", "/api/", Some(&guest), None).await;
    assert_eq!(body["role"], "guest");
    assert_eq!(body["permissions"]["history"], false);
    // Le tableau de bord reçoit ce que l'invité peut commander, pour cacher
    // le reste.
    assert_eq!(
        body["permissions"]["controls"],
        json!({
            "domains": ["light", "switch", "fan", "scene", "cover"],
            "forbidden_cover_classes": ["garage", "gate", "door"],
        })
    );
    let family = app.login_as("alice", ha_auth::Role::Family).await;
    let (_, body) = app.request("GET", "/api/", Some(&family), None).await;
    assert_eq!(body["permissions"]["controls"], Value::Null);

    // La présence et l'alarme sont cachées, même en les demandant directement.
    let (_, listed) = app.request("GET", "/api/states", Some(&guest), None).await;
    let mut ids: Vec<&str> = listed
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["entity_id"].as_str().unwrap())
        .collect();
    ids.sort_unstable();
    assert_eq!(ids, ["cover.garage", "cover.salon", "light.salon"]);
    for hidden in ["person.david", "sensor.alarme", "lock.entree"] {
        let (status, _) = app
            .request("GET", &format!("/api/states/{hidden}"), Some(&guest), None)
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{hidden}");
    }

    // Lumière et volet : oui. Garage, serrure, alarme : non.
    let call = |service: &str, entity: &str| {
        let body = json!({"entity_id": entity});
        let uri = format!("/api/services/{service}");
        let app = &app;
        let guest = guest.clone();
        async move { app.request("POST", &uri, Some(&guest), Some(body)).await.0 }
    };
    assert_eq!(call("light/turn_on", "light.salon").await, StatusCode::OK);
    assert_eq!(
        call("cover/close_cover", "cover.salon").await,
        StatusCode::OK
    );
    for (service, entity) in [
        ("cover/open_cover", "cover.garage"),
        ("lock/unlock", "lock.entree"),
        ("alarm_control_panel/alarm_disarm", "sensor.alarme"),
        ("notify/send_message", "notify.telephone"),
    ] {
        assert_eq!(
            call(service, entity).await,
            StatusCode::FORBIDDEN,
            "{entity}"
        );
    }

    for uri in [
        "/api/history/light.salon",
        "/api/history/light.salon/series",
        "/api/logbook",
        "/api/audit",
        "/api/automations",
    ] {
        let (status, _) = app.request("GET", uri, Some(&guest), None).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{uri}");
    }
    let (status, _) = app
        .request(
            "POST",
            "/api/states/light.salon",
            Some(&guest),
            Some(json!({"state": "on"})),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn the_family_controls_everything_but_cannot_edit_automations() {
    let app = TestApp::new().await;
    furnish(&app);
    let family = app.login_as("alice", ha_auth::Role::Family).await;

    let (_, listed) = app.request("GET", "/api/states", Some(&family), None).await;
    assert_eq!(listed.as_array().unwrap().len(), 6, "tout est visible");
    let (status, _) = app
        .request(
            "POST",
            "/api/services/cover/open_cover",
            Some(&family),
            Some(json!({"entity_id": "cover.garage"})),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    for uri in ["/api/history/light.salon", "/api/logbook"] {
        let (status, _) = app.request("GET", uri, Some(&family), None).await;
        assert_eq!(status, StatusCode::OK, "{uri}");
    }
    // Les automatisations : lecture seule.
    let (status, body) = app
        .request("GET", "/api/automations", Some(&family), None)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["editable"], false);
    let (status, _) = app
        .request("DELETE", "/api/automations/matin", Some(&family), None)
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    // Ni journal d'audit, ni écriture directe d'un état.
    let (status, _) = app.request("GET", "/api/audit", Some(&family), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = app
        .request("DELETE", "/api/states/light.salon", Some(&family), None)
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn a_guest_websocket_never_receives_hidden_devices() {
    let app = TestApp::new().await;
    furnish(&app);
    let guest = app.login_as("visiteur", ha_auth::Role::Guest).await;
    let addr = spawn_server(&app).await;
    let (mut socket, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/api/websocket"))
        .await
        .unwrap();
    assert_eq!(ws_recv(&mut socket).await["type"], "auth_required");
    ws_send(&mut socket, json!({"type": "auth", "access_token": guest})).await;
    assert_eq!(ws_recv(&mut socket).await["type"], "auth_ok");

    ws_send(&mut socket, json!({"id": 1, "type": "get_states"})).await;
    let reply = ws_recv(&mut socket).await;
    let ids: Vec<&str> = reply["result"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["entity_id"].as_str().unwrap())
        .collect();
    assert!(!ids.contains(&"person.david"), "{ids:?}");

    ws_send(&mut socket, json!({"id": 2, "type": "subscribe_events"})).await;
    assert_eq!(ws_recv(&mut socket).await["success"], true);
    // David part (caché), une commande passe (cachée), puis la lumière s'allume.
    let set = |entity: &str, value: &str| {
        app.state
            .core
            .states
            .set(entity.parse().unwrap(), value, Attributes::new())
            .unwrap();
    };
    set("person.david", "not_home");
    app.state
        .core
        .call_service(
            "light",
            "turn_on",
            "light.salon".parse().unwrap(),
            Attributes::new(),
        )
        .unwrap();
    set("light.salon", "on");
    let event = ws_recv(&mut socket).await;
    assert_eq!(
        event["event"]["data"]["entity_id"], "light.salon",
        "le premier événement reçu est la lumière : {event}"
    );

    app.state.shutdown();
}

#[tokio::test]
async fn statistics_are_checked_and_protected() {
    let app = TestApp::new().await;
    let token = app.login().await;
    let (status, body) = app
        .request(
            "GET",
            "/api/statistics/sensor.temperature?period=day&days=365",
            Some(&token),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["points"], json!([]));
    for bad in [
        "/api/statistics/sensor.temperature?period=week",
        "/api/statistics/sensor.temperature?period=hour&days=60",
        "/api/statistics/sensor.temperature?days=0",
        "/api/statistics/sensor.temperature?days=99999",
        "/api/statistics/Sensor?days=1",
    ] {
        let (status, _) = app.request("GET", bad, Some(&token), None).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}");
    }
    let guest = app.login_as("visiteur", ha_auth::Role::Guest).await;
    let (status, _) = app
        .request(
            "GET",
            "/api/statistics/sensor.temperature",
            Some(&guest),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn unlocking_a_door_asks_for_the_password_again() {
    let app = TestApp::new().await;
    furnish(&app);
    let token = app.login().await;
    let mut events = app.state.core.bus.subscribe();
    let unlock = json!({"entity_id": "lock.entree"});

    // Verrouiller : tout de suite. Déverrouiller : il faut confirmer.
    let (status, _) = app
        .request(
            "POST",
            "/api/services/lock/lock",
            Some(&token),
            Some(unlock.clone()),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(events.try_recv().is_ok());
    for service in ["unlock", "open"] {
        let (status, body) = app
            .request(
                "POST",
                &format!("/api/services/lock/{service}"),
                Some(&token),
                Some(unlock.clone()),
            )
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{service}");
        assert_eq!(body["confirm"], true, "{service}");
    }
    // Rien n'est parti vers la serrure.
    assert!(events.try_recv().is_err());

    let (status, _) = app
        .request(
            "POST",
            "/api/auth/confirm",
            Some(&token),
            Some(json!({"password": PASSWORD})),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = app
        .request(
            "POST",
            "/api/services/lock/unlock",
            Some(&token),
            Some(unlock.clone()),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let event = events.try_recv().unwrap();
    assert!(matches!(
        event.as_ref(),
        ha_core::Event::CallService { domain, service, .. } if domain == "lock" && service == "unlock"
    ));

    // Un invité n'y a jamais droit, même avec son mot de passe.
    let guest = app.login_as("visiteur", ha_auth::Role::Guest).await;
    let (status, body) = app
        .request(
            "POST",
            "/api/services/lock/lock",
            Some(&guest),
            Some(unlock),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_ne!(body["confirm"], true);
}
