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
            traces: ha_automation::Traces::new(),
        };
        let state = ApiState::new(
            core,
            store,
            Arc::new(auth),
            areas,
            Some(automations),
            ha_energy::EnergyConfig::default(),
        );
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

    /// Envoie des octets bruts (une image) et renvoie (statut, corps).
    async fn send_bytes(
        &self,
        method: &str,
        uri: &str,
        token: &str,
        body: Vec<u8>,
    ) -> (StatusCode, Vec<u8>, Option<String>) {
        let request = Request::builder()
            .method(method)
            .uri(uri)
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .body(Body::from(body))
            .unwrap();
        let response = self.router().oneshot(request).await.unwrap();
        let status = response.status();
        let content_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .map(|v| v.to_str().unwrap().to_owned());
        let bytes = axum::body::to_bytes(response.into_body(), 8 << 20)
            .await
            .unwrap();
        (status, bytes.to_vec(), content_type)
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
        ("GET", "/api/automations/matin/traces"),
        ("PUT", "/api/automations/matin/layout"),
        ("GET", "/api/energy"),
        ("GET", "/api/home/drawing"),
        ("PUT", "/api/home/drawing"),
        ("GET", "/api/home/images/jour"),
        ("PUT", "/api/home/images/jour"),
        ("DELETE", "/api/home/images/jour"),
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
async fn orders_get_an_answer_from_integrations_that_give_one() {
    let app = TestApp::new().await;
    let token = app.login().await;
    let core = Arc::clone(&app.state.core);
    let lamp = |name: &str| -> ha_core::EntityId { format!("light.{name}").parse().unwrap() };
    core.answers_commands("hue");
    core.report("hue", "Hue", ha_core::Health::Connected);
    for name in ["salon", "cuisine"] {
        core.states
            .set_by("hue", lamp(name), "off", ha_core::Attributes::new())
            .unwrap();
    }
    // Un faux pont : il accepte le salon et refuse la cuisine.
    let mut events = core.bus.subscribe();
    let bridge = Arc::clone(&core);
    tokio::spawn(async move {
        while let Ok(event) = events.recv().await {
            if let ha_core::Event::CallService {
                entity_id, command, ..
            } = event.as_ref()
            {
                let result = if entity_id.as_str() == "light.salon" {
                    Ok(())
                } else {
                    Err("le pont Hue n'a pas accepté l'ordre".to_owned())
                };
                bridge.command_done(*command, result);
            }
        }
    });
    let order = |entity: &'static str| {
        let app = &app;
        let token = token.clone();
        async move {
            app.request(
                "POST",
                "/api/services/light/turn_on",
                Some(&token),
                Some(json!({"entity_id": entity})),
            )
            .await
        }
    };
    let (status, body) = order("light.salon").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["result"], "done");
    let (status, body) = order("light.cuisine").await;
    assert_eq!(status, StatusCode::BAD_GATEWAY, "{body}");
    assert_eq!(body["error"], "le pont Hue n'a pas accepté l'ordre");

    // Un appareil d'une intégration qui ne répond pas : simplement envoyé.
    let (status, body) = order("switch.prise_cafe").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["result"], "sent");

    // Hue en panne depuis plus d'une minute : l'ordre n'est même pas envoyé.
    core.report(
        "hue",
        "Hue",
        ha_core::Health::Retrying {
            error: "pont injoignable".into(),
            retry_at: std::time::SystemTime::now(),
        },
    );
    core.check_integrations(std::time::SystemTime::now() + ha_core::integrations::GRACE * 2);
    let mut watch = core.bus.subscribe();
    let (status, body) = order("light.salon").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .starts_with("Hue ne répond plus depuis"),
        "{body}"
    );
    assert!(
        !std::iter::from_fn(|| watch.try_recv().ok())
            .any(|event| matches!(event.as_ref(), ha_core::Event::CallService { .. })),
        "aucun ordre ne part"
    );
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
        ..
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
        ("/canvas.js", "text/javascript; charset=utf-8"),
        ("/house.js", "text/javascript; charset=utf-8"),
        ("/house.svg", "image/svg+xml"),
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
    for (name, script) in [
        ("app.js", include_str!("../web/app.js")),
        ("canvas.js", include_str!("../web/canvas.js")),
        ("house.js", include_str!("../web/house.js")),
    ] {
        dashboard_script_is_safe(name, script);
    }
}

fn dashboard_script_is_safe(name: &str, script: &str) {
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
            "{forbidden} interdit dans {name}"
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
async fn integrations_report_their_health() {
    let app = TestApp::new().await;
    let token = app.login().await;
    let core = &app.state.core;
    core.report("hue", "Hue", ha_core::Health::Connected);
    core.states
        .set_by(
            "hue",
            "light.salon".parse().unwrap(),
            "on",
            ha_core::Attributes::new(),
        )
        .unwrap();
    core.report(
        "somfy",
        "Somfy",
        ha_core::Health::Retrying {
            error: "box injoignable".into(),
            retry_at: std::time::SystemTime::now(),
        },
    );
    let (status, body) = app
        .request("GET", "/api/integrations", Some(&token), None)
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let list = body["integrations"].as_array().unwrap();
    assert_eq!(list.len(), 2);
    assert_eq!(list[0]["id"], "hue");
    assert_eq!(list[0]["status"], "connected");
    assert_eq!(list[0]["entities"], 1);
    assert_eq!(list[1]["status"], "retrying");
    assert_eq!(list[1]["error"], "box injoignable");
    assert!(list[1]["retry_at"].is_string());

    // Un invité ne voit pas l'état des intégrations (les erreurs citent le réseau).
    let guest = app.login_as("invite", ha_auth::Role::Guest).await;
    let (status, _) = app
        .request("GET", "/api/integrations", Some(&guest), None)
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn the_system_page_measures_the_server() {
    let app = TestApp::new().await;
    let token = app.login().await;
    // Quelques requêtes à mesurer.
    for _ in 0..3 {
        app.request("GET", "/api/states", Some(&token), None).await;
    }
    let (status, body) = app.request("GET", "/api/system", Some(&token), None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body["api"]["samples"].as_u64().unwrap() >= 3, "{body}");
    assert!(body["api"]["p95_ms"].as_f64().unwrap() >= 0.0);
    assert!(body["database_bytes"].as_u64().unwrap() > 0);
    assert!(body["uptime_seconds"].is_u64());
    assert_eq!(body["budget"]["memory_mib"], 64);
    if cfg!(target_os = "linux") {
        assert!(body["memory"]["resident_bytes"].as_u64().unwrap() > 0);
        assert!(body["cpu_percent"].is_number());
    }

    // Réservé à l'administrateur.
    let family = app.login_as("alice", ha_auth::Role::Family).await;
    let (status, _) = app.request("GET", "/api/system", Some(&family), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
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
async fn the_place_of_the_blocks_is_kept_without_confirmation() {
    let app = TestApp::new().await;
    let token = app.login().await;
    // Pas de confirmation d'identité : seule la place des blocs change.
    let layout = json!({"déclencheur 1": [40.4, 80], "1": [320, 80], "1 › alors › 1": [600, -40]});
    let (status, body) = app
        .request(
            "PUT",
            "/api/automations/matin/layout",
            Some(&token),
            Some(layout),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    let (_, body) = app
        .request("GET", "/api/automations", Some(&token), None)
        .await;
    assert_eq!(
        body["automations"][0]["layout"],
        json!({"déclencheur 1": [40.0, 80.0], "1": [320.0, 80.0], "1 › alors › 1": [600.0, -40.0]})
    );

    for bad in [
        json!([1, 2]),
        json!({"1": [1]}),
        json!({"1": ["a", 2]}),
        json!({"1": [1, 1e9]}),
        json!({"": [1, 2]}),
        json!({"1": {"x": 1}}),
    ] {
        let (status, body) = app
            .request(
                "PUT",
                "/api/automations/matin/layout",
                Some(&token),
                Some(bad.clone()),
            )
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad} : {body}");
    }
    let (status, _) = app
        .request(
            "PUT",
            "/api/automations/inconnue/layout",
            Some(&token),
            Some(json!({})),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // Un membre de la famille ne modifie pas les automatisations.
    let family = app.login_as("alice", ha_auth::Role::Family).await;
    let (status, _) = app
        .request(
            "PUT",
            "/api/automations/matin/layout",
            Some(&family),
            Some(json!({})),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
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
    // Désarmer l'alarme aussi.
    let (status, body) = app
        .request(
            "POST",
            "/api/services/alarm_control_panel/alarm_disarm",
            Some(&token),
            Some(json!({"entity_id": "alarm_control_panel.maison"})),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["confirm"], true);
    // Rien n'est parti vers la serrure ni l'alarme.
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

#[tokio::test]
async fn a_guest_cannot_open_the_garage_through_a_group() {
    let app = TestApp::new().await;
    furnish(&app);
    // Un groupe de volets qui contient la porte du garage.
    let Value::Object(group) = json!({"entity_id": ["cover.salon", "cover.garage"]}) else {
        panic!("un objet est attendu")
    };
    app.state
        .core
        .states
        .set("cover.tout".parse().unwrap(), "closed", group)
        .unwrap();
    let Value::Object(safe) = json!({"entity_id": ["cover.salon"]}) else {
        panic!("un objet est attendu")
    };
    app.state
        .core
        .states
        .set("cover.salon_seul".parse().unwrap(), "closed", safe)
        .unwrap();
    let guest = app.login_as("visiteur", ha_auth::Role::Guest).await;
    let open = |entity: &str| json!({"entity_id": entity});
    let (status, _) = app
        .request(
            "POST",
            "/api/services/cover/open_cover",
            Some(&guest),
            Some(open("cover.tout")),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = app
        .request(
            "POST",
            "/api/services/cover/open_cover",
            Some(&guest),
            Some(open("cover.salon_seul")),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn the_run_history_is_readable_by_the_family_only() {
    let app = TestApp::new().await;
    let traces = &app.state.automations.as_ref().unwrap().traces;
    let recorder = traces.begin(
        "matin",
        "Matin",
        Some("il est 07:30".into()),
        Some(0),
        vec![ha_automation::trace::Check {
            description: "après 07:00".into(),
            passed: true,
        }],
    );
    recorder.step(
        "1",
        jiff::Timestamp::now(),
        "cover.open_cover sur cover.salon".into(),
        ha_automation::StepStatus::Done,
        None,
    );
    recorder.finish(ha_automation::Outcome::Finished);

    let token = app.login().await;
    let (status, body) = app
        .request("GET", "/api/automations/matin/traces", Some(&token), None)
        .await;
    assert_eq!(status, StatusCode::OK);
    let trace = &body["traces"][0];
    assert_eq!(trace["trigger"], "il est 07:30");
    assert_eq!(trace["trigger_index"], 0);
    assert_eq!(trace["outcome"], "finished");
    assert_eq!(trace["conditions"][0]["passed"], true);
    assert_eq!(trace["steps"][0]["status"], "done");
    assert_eq!(
        trace["steps"][0]["description"],
        "cover.open_cover sur cover.salon"
    );

    // La liste dit quand chaque automatisation a tourné pour la dernière fois.
    let (_, body) = app
        .request("GET", "/api/automations", Some(&token), None)
        .await;
    let matin = body["automations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["id"] == "matin")
        .unwrap();
    assert_eq!(matin["last_run"]["outcome"], "finished");

    // Une automatisation sans exécution : une liste vide.
    let (_, body) = app
        .request(
            "GET",
            "/api/automations/inconnue/traces",
            Some(&token),
            None,
        )
        .await;
    assert_eq!(body["traces"], json!([]));

    let guest = app.login_as("visiteur", ha_auth::Role::Guest).await;
    let (status, _) = app
        .request("GET", "/api/automations/matin/traces", Some(&guest), None)
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn the_energy_report_is_for_the_family() {
    let app = TestApp::new().await;
    let mut attributes = serde_json::Map::new();
    attributes.insert("friendly_name".into(), json!("Linky heures creuses"));
    attributes.insert("state_class".into(), json!("total_increasing"));
    app.state
        .core
        .states
        .set(
            "sensor.linky_heures_creuses".parse().unwrap(),
            "12.5",
            attributes,
        )
        .unwrap();

    let token = app.login().await;
    let (status, body) = app
        .request("GET", "/api/energy?days=7", Some(&token), None)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["configured"], false);
    assert_eq!(body["sources"][0]["name"], "Linky heures creuses");
    assert_eq!(body["sources"][0]["kind"], "consumption");
    assert_eq!(body["today"]["consumed_kwh"], 0.0);
    assert_eq!(body["days"], json!([]));

    let (status, _) = app
        .request("GET", "/api/energy?days=0", Some(&token), None)
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = app
        .request("GET", "/api/energy?jours=3", Some(&token), None)
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let guest = app.login_as("visiteur", ha_auth::Role::Guest).await;
    let (status, _) = app.request("GET", "/api/energy", Some(&guest), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

/// Une image PNG (seuls les premiers octets comptent pour le serveur).
fn png(content: &[u8]) -> Vec<u8> {
    let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
    bytes.extend_from_slice(content);
    bytes
}

#[tokio::test]
async fn the_house_drawing_is_assembled_from_checked_images() {
    let app = TestApp::new().await;
    let token = app.login().await;

    // Pas encore de dessin : un assemblage vide, que l'administrateur peut régler.
    let (status, body) = app
        .request("GET", "/api/home/drawing", Some(&token), None)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["config"],
        json!({"pages": [{"id": "maison", "name": "Maison", "areas": [], "layers": [], "labels": [], "actions": [], "charts": [], "zones": []}]})
    );
    assert_eq!(body["editable"], true);

    // Des images : PNG, JPEG ou WebP, reconnus à leur contenu.
    for name in ["jour", "garage_ouvert"] {
        let (status, _, _) = app
            .send_bytes(
                "PUT",
                &format!("/api/home/images/{name}"),
                &token,
                png(name.as_bytes()),
            )
            .await;
        assert_eq!(status, StatusCode::NO_CONTENT, "{name}");
    }
    let (status, bytes, content_type) = app
        .send_bytes("GET", "/api/home/images/jour", &token, Vec::new())
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(bytes, png(b"jour"));
    assert_eq!(content_type.as_deref(), Some("image/png"));

    // Un SVG (qui peut contenir du script), un HTML, un mauvais nom, une image trop grosse.
    for (uri, body) in [
        ("/api/home/images/piege", b"<svg onload=alert(1)>".to_vec()),
        (
            "/api/home/images/page",
            b"<html><script>alert(1)</script>".to_vec(),
        ),
        ("/api/home/images/..%2Fha.key", png(b"x")),
        ("/api/home/images/Majuscule", png(b"x")),
        ("/api/home/images/enorme", png(&vec![0; 3 * 1024 * 1024])),
    ] {
        let (status, _, _) = app.send_bytes("PUT", uri, &token, body).await;
        assert!(
            status == StatusCode::BAD_REQUEST || status == StatusCode::PAYLOAD_TOO_LARGE,
            "{uri} : {status}"
        );
    }

    // Deux onglets : la maison (un fond, des calques, des valeurs), et la
    // cuisine seule.
    let home = json!({
        "id": "maison",
        "name": "Maison",
        "background": "jour",
        "ambiances": {"jour-pluie": "garage_ouvert"},
        "forced": "jour-pluie",
        "effects": "neige",
        "effects_level": "discret",
        "layers": [
            {"image": "garage_ouvert", "entity_id": "cover.garage", "states": ["open", "opening"]},
            {"image": "jour", "entity_id": "person.alice", "states": ["home"], "when": "night"},
        ],
        "labels": [
            {"entity_id": "sensor.dehors", "x": 90.04, "y": 8, "name": "Dehors"},
            {"entity_id": "person.alice", "x": 10, "y": 10},
        ],
        "actions": ["light.porche", "person.alice"],
        "charts": [
            {"entity_id": "sensor.nas", "name": "NAS", "color": "violet", "hours": 24},
            {"entity_id": "person.alice", "color": "vert", "hours": 6},
        ],
        "zones": [
            {"entity_id": "light.porche", "x": 40.04, "y": 55, "w": 8, "h": 10, "name": "Porche"},
            {"entity_id": "person.alice", "x": 0, "y": 0, "w": 5, "h": 5},
            {"area": "Cuisine", "x": 50, "y": 50, "w": 10, "h": 10},
        ],
    });
    let kitchen = json!({"id": "cuisine", "name": "Cuisine", "areas": ["Cuisine"]});
    let (status, body) = app
        .request(
            "PUT",
            "/api/home/drawing",
            Some(&token),
            Some(json!({"pages": [home, kitchen]})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["config"]["pages"][0]["labels"][0]["x"], 90.0);
    assert_eq!(body["config"]["pages"][0]["layers"][0]["when"], "always");
    assert_eq!(body["config"]["pages"][1]["areas"], json!(["Cuisine"]));
    assert_eq!(
        body["config"]["pages"][0]["ambiances"],
        json!({"jour-pluie": "garage_ouvert"})
    );
    assert_eq!(body["config"]["pages"][0]["forced"], "jour-pluie");
    assert_eq!(body["config"]["pages"][0]["effects"], "neige");
    assert_eq!(body["config"]["pages"][0]["effects_level"], "discret");
    assert_eq!(body["config"]["pages"][0]["zones"][0]["x"], 40.0);
    let page = |extra: Value| {
        let mut page = json!({"id": "maison", "name": "Maison"});
        page.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        json!({"pages": [page]})
    };
    for bad in [
        page(json!({"background": "absente"})),
        page(json!({"layers": [{"image": "jour", "entity_id": "cover.garage", "states": []}]})),
        page(json!({"labels": [{"entity_id": "sensor.a", "x": -1, "y": 0}]})),
        page(json!({"areas": ["Grenier"]})),
        page(json!({"zones": [{"entity_id": "light.porche", "x": 95, "y": 0, "w": 10, "h": 10}]})),
        page(json!({"zones": [{"area": "Grenier", "x": 0, "y": 0, "w": 10, "h": 10}]})),
        page(json!({"effects": "tornade"})),
        page(json!({"background": "jour", "ambiances": {"jour-tempete": "jour"}})),
        json!({"pages": []}),
        json!({"script": "alert(1)"}),
    ] {
        let (status, body) = app
            .request("PUT", "/api/home/drawing", Some(&token), Some(bad.clone()))
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad} : {body}");
    }

    // Une image qui sert encore ne se retire pas.
    let (status, _, _) = app
        .send_bytes(
            "DELETE",
            "/api/home/images/garage_ouvert",
            &token,
            Vec::new(),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // La famille voit le dessin, mais ne le change pas.
    let family = app.login_as("alice", ha_auth::Role::Family).await;
    let (status, body) = app
        .request("GET", "/api/home/drawing", Some(&family), None)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["editable"], false);
    assert_eq!(body["config"]["pages"][0]["background"], "jour");
    let (status, _) = app
        .request("PUT", "/api/home/drawing", Some(&family), Some(json!({})))
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _, _) = app
        .send_bytes("PUT", "/api/home/images/autre", &family, png(b"x"))
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Un invité ne voit ni les calques ni les valeurs des appareils qui lui
    // sont cachés (où est Alice).
    let guest = app.login_as("invite", ha_auth::Role::Guest).await;
    let (_, body) = app
        .request("GET", "/api/home/drawing", Some(&guest), None)
        .await;
    let home = &body["config"]["pages"][0];
    let layers = home["layers"].as_array().unwrap();
    assert_eq!(layers.len(), 1);
    assert_eq!(layers[0]["entity_id"], "cover.garage");
    assert_eq!(home["labels"].as_array().unwrap().len(), 1);
    assert_eq!(home["labels"][0]["entity_id"], "sensor.dehors");
    assert_eq!(home["actions"], json!(["light.porche"]));
    assert_eq!(home["charts"].as_array().unwrap().len(), 1);
    assert_eq!(home["charts"][0]["entity_id"], "sensor.nas");
    assert_eq!(home["zones"].as_array().unwrap().len(), 2);
    assert_eq!(home["zones"][0]["entity_id"], "light.porche");
    assert_eq!(home["zones"][1]["area"], "Cuisine");

    // Chaque changement est dans le journal d'audit.
    let (_, body) = app.request("GET", "/api/audit", Some(&token), None).await;
    let actions: Vec<&str> = body["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|entry| entry["action"].as_str())
        .collect();
    assert!(actions.contains(&"drawing_set"), "{actions:?}");
    assert!(actions.contains(&"drawing_image"), "{actions:?}");
}
