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
        Self::build(
            editable,
            "- {area: Cuisine, entities: [{entity: switch.prise_cafe, name: Cafetière}]}",
        )
        .await
    }

    /// Avec ces pièces (le contenu d'un `areas.yaml`).
    async fn with_areas(areas: &str) -> Self {
        Self::build(true, areas).await
    }

    async fn build(editable: bool, areas: &str) -> Self {
        let store = Store::open_in_memory().await.unwrap();
        let auth = Auth::new(&store).await.unwrap();
        auth.create_user("david", PASSWORD).await.unwrap();
        let core = Arc::new(Core::new());
        let recorder = Recorder::start(store.clone(), Arc::clone(&core));
        let areas = ha_areas::parse(areas).unwrap();
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
        ("GET", "/api/automations/presets"),
        ("PUT", "/api/automations/presets/nuit"),
        ("GET", "/api/energy"),
        ("GET", "/api/home/drawing"),
        ("PUT", "/api/home/drawing"),
        ("GET", "/api/home/images/jour"),
        ("PUT", "/api/home/images/jour"),
        ("DELETE", "/api/home/images/jour"),
        ("GET", "/api/approvals"),
        ("GET", "/api/push"),
        ("POST", "/api/push/subscriptions"),
        ("PATCH", "/api/push/subscriptions/1"),
        ("DELETE", "/api/push/subscriptions/1"),
        ("POST", "/api/push/test"),
        ("POST", "/api/approvals/1"),
        ("GET", "/api/camera_proxy/camera.porte"),
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
        ("/utils.js", "text/javascript; charset=utf-8"),
        ("/app.js", "text/javascript; charset=utf-8"),
        ("/tiles.js", "text/javascript; charset=utf-8"),
        ("/dialogs.js", "text/javascript; charset=utf-8"),
        ("/editor.js", "text/javascript; charset=utf-8"),
        ("/account.js", "text/javascript; charset=utf-8"),
        ("/start.js", "text/javascript; charset=utf-8"),
        ("/canvas.js", "text/javascript; charset=utf-8"),
        ("/house.js", "text/javascript; charset=utf-8"),
        ("/agenda.js", "text/javascript; charset=utf-8"),
        ("/hub.js", "text/javascript; charset=utf-8"),
        ("/sonnette.js", "text/javascript; charset=utf-8"),
        ("/colis.js", "text/javascript; charset=utf-8"),
        ("/voix.js", "text/javascript; charset=utf-8"),
        ("/pretes.js", "text/javascript; charset=utf-8"),
        ("/musique.js", "text/javascript; charset=utf-8"),
        ("/ecoute.js", "text/javascript; charset=utf-8"),
        ("/house.svg", "image/svg+xml"),
        ("/style.css", "text/css; charset=utf-8"),
        ("/icon.svg", "image/svg+xml"),
        ("/manifest.webmanifest", "application/manifest+json"),
        ("/fonts/geist.woff2", "font/woff2"),
        ("/fonts/geist-mono.woff2", "font/woff2"),
        ("/fonts/bricolage-grotesque.woff2", "font/woff2"),
        ("/sw.js", "text/javascript; charset=utf-8"),
        ("/smartthings", "text/html; charset=utf-8"),
        ("/icon-192.png", "image/png"),
        ("/icon-512.png", "image/png"),
        ("/apple-touch-icon.png", "image/png"),
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
        // Le service worker des notifications, et rien d'autre.
        assert!(
            headers[header::CONTENT_SECURITY_POLICY]
                .to_str()
                .unwrap()
                .contains("worker-src 'self';"),
            "{uri}"
        );
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
    let scripts = [
        ("utils.js", include_str!("../web/utils.js")),
        ("app.js", include_str!("../web/app.js")),
        ("tiles.js", include_str!("../web/tiles.js")),
        ("dialogs.js", include_str!("../web/dialogs.js")),
        ("editor.js", include_str!("../web/editor.js")),
        ("account.js", include_str!("../web/account.js")),
        ("start.js", include_str!("../web/start.js")),
        ("canvas.js", include_str!("../web/canvas.js")),
        ("house.js", include_str!("../web/house.js")),
        ("agenda.js", include_str!("../web/agenda.js")),
        ("hub.js", include_str!("../web/hub.js")),
        ("sonnette.js", include_str!("../web/sonnette.js")),
        ("colis.js", include_str!("../web/colis.js")),
        ("orbe.js", include_str!("../web/orbe.js")),
        ("cartes.js", include_str!("../web/cartes.js")),
        ("voix.js", include_str!("../web/voix.js")),
        ("pretes.js", include_str!("../web/pretes.js")),
        ("musique.js", include_str!("../web/musique.js")),
        ("ecoute.js", include_str!("../web/ecoute.js")),
        ("sw.js", include_str!("../web/sw.js")),
    ];
    for (name, script) in scripts {
        dashboard_script_is_safe(name, script);
    }
    // Chaque script chargé par la page est vérifié ci-dessus : un nouveau
    // fichier ne peut pas être oublié.
    let index = include_str!("../web/index.html");
    for src in index.split("<script src=\"/").skip(1) {
        let name = src.split('"').next().unwrap();
        assert!(
            scripts.iter().any(|(checked, _)| *checked == name),
            "{name} chargé par index.html mais pas vérifié"
        );
    }
}

/// Une demande du tableau de bord, avec des en-têtes choisis.
async fn get_dashboard_file(
    app: &TestApp,
    uri: &str,
    headers: &[(header::HeaderName, &str)],
) -> axum::response::Response {
    let mut request = Request::builder().uri(uri);
    for (name, value) in headers {
        request = request.header(name, *value);
    }
    app.router()
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap()
}

#[tokio::test]
async fn dashboard_text_files_are_gzipped_for_browsers_that_accept_it() {
    use std::io::Read as _;

    let app = TestApp::new().await;
    let gzip = [(header::ACCEPT_ENCODING, "gzip, deflate, br")];
    let original = include_bytes!("../web/app.js");

    let response = get_dashboard_file(&app, "/app.js", &gzip).await;
    assert_eq!(response.status(), StatusCode::OK);
    let headers = response.headers().clone();
    assert_eq!(headers[header::CONTENT_ENCODING], "gzip");
    assert_eq!(headers[header::VARY], "Accept-Encoding");
    assert_eq!(headers[header::CACHE_CONTROL], "no-cache");
    assert_eq!(
        headers[header::CONTENT_TYPE],
        "text/javascript; charset=utf-8"
    );
    // Les en-têtes de sécurité restent là.
    assert!(headers.contains_key(header::CONTENT_SECURITY_POLICY));
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    assert!(body.len() * 2 < original.len(), "{} octets", body.len());
    let mut unpacked = Vec::new();
    flate2::read::GzDecoder::new(&body[..])
        .read_to_end(&mut unpacked)
        .unwrap();
    assert_eq!(unpacked, original);

    // Sans gzip annoncé (ou refusé), le fichier part tel quel.
    for headers in [&[][..], &[(header::ACCEPT_ENCODING, "gzip;q=0")][..]] {
        let response = get_dashboard_file(&app, "/app.js", headers).await;
        assert!(!response.headers().contains_key(header::CONTENT_ENCODING));
        assert_eq!(response.headers()[header::VARY], "Accept-Encoding");
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(&body[..], original);
    }

    // Les polices et les images sont déjà compressées : jamais deux fois.
    for uri in ["/fonts/geist.woff2", "/icon-192.png"] {
        let response = get_dashboard_file(&app, uri, &gzip).await;
        assert!(
            !response.headers().contains_key(header::CONTENT_ENCODING),
            "{uri}"
        );
        assert!(!response.headers().contains_key(header::VARY), "{uri}");
        assert_eq!(
            response.headers()[header::CACHE_CONTROL],
            "public, max-age=604800",
            "{uri}"
        );
    }
}

#[tokio::test]
async fn dashboard_files_are_not_sent_again_while_unchanged() {
    let app = TestApp::new().await;
    for accept in ["gzip", "identity"] {
        let response =
            get_dashboard_file(&app, "/style.css", &[(header::ACCEPT_ENCODING, accept)]).await;
        let etag = response.headers()[header::ETAG]
            .to_str()
            .unwrap()
            .to_owned();
        assert!(etag.starts_with('"') && etag.ends_with('"'), "{etag}");

        // Le navigateur redemande avec son empreinte : rien à renvoyer.
        let again = get_dashboard_file(
            &app,
            "/style.css",
            &[
                (header::ACCEPT_ENCODING, accept),
                (header::IF_NONE_MATCH, &etag),
            ],
        )
        .await;
        assert_eq!(again.status(), StatusCode::NOT_MODIFIED, "{accept}");
        assert_eq!(again.headers()[header::ETAG], etag.as_str());
        assert!(
            again
                .headers()
                .contains_key(header::CONTENT_SECURITY_POLICY)
        );
        let body = axum::body::to_bytes(again.into_body(), usize::MAX)
            .await
            .unwrap();
        assert!(body.is_empty());

        // Une autre empreinte (ancienne version) : le fichier est renvoyé.
        let old = get_dashboard_file(
            &app,
            "/style.css",
            &[
                (header::ACCEPT_ENCODING, accept),
                (header::IF_NONE_MATCH, "\"ancienne\""),
            ],
        )
        .await;
        assert_eq!(old.status(), StatusCode::OK, "{accept}");
    }

    // La version compressée et la version brute n'ont pas la même empreinte.
    let gz = get_dashboard_file(&app, "/style.css", &[(header::ACCEPT_ENCODING, "gzip")]).await;
    let raw = get_dashboard_file(&app, "/style.css", &[]).await;
    assert_ne!(gz.headers()[header::ETAG], raw.headers()[header::ETAG]);
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
    assert_eq!(body["devices"], json!([]), "aucun ordre encore");
    assert_eq!(body["slow_ms"], 3000);
    if cfg!(target_os = "linux") {
        assert!(body["memory"]["resident_bytes"].as_u64().unwrap() > 0);
        assert!(body["cpu_percent"].is_number());
    }

    // Un ordre à une lampe Hue, et sa réponse 150 ms plus tard : la page
    // donne la rapidité de Hue, sous son nom.
    let core = &app.state.core;
    core.report("hue", "Hue", ha_core::Health::Connected);
    let lamp: ha_core::EntityId = "light.salon".parse().unwrap();
    core.states
        .set_by("hue", lamp.clone(), "off", ha_core::Attributes::new())
        .unwrap();
    let sent = std::time::Instant::now();
    core.latencies.observe(
        &ha_core::Event::CallService {
            domain: "light".into(),
            service: "turn_on".into(),
            entity_id: lamp.clone(),
            data: ha_core::Attributes::new(),
            origin: ha_core::Origin::System,
            command: None,
            issued_at: sent,
        },
        &core.states,
        sent,
    );
    core.latencies.observe(
        &ha_core::Event::StateChanged {
            entity_id: lamp.clone(),
            old_state: None,
            new_state: core.states.get(&lamp),
        },
        &core.states,
        sent + std::time::Duration::from_millis(150),
    );
    let (_, body) = app.request("GET", "/api/system", Some(&token), None).await;
    assert_eq!(
        body["devices"],
        json!([{"integration": "hue", "name": "Hue", "p50_ms": 150.0, "p95_ms": 150.0, "samples": 1, "slow": 0}])
    );

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

/// Les automatisations toutes prêtes (étape 85) : cochées, elles deviennent
/// des automatisations ordinaires, fabriquées d'après les appareils.
#[tokio::test]
async fn ready_made_automations_are_ticked() {
    let app = TestApp::new().await;
    furnish(&app);
    let set = |id: &str, value: &str, attributes: Value| {
        let Value::Object(attributes) = attributes else {
            panic!()
        };
        app.state
            .core
            .states
            .set(id.parse().unwrap(), value, attributes)
            .unwrap();
    };
    set("zone.home", "1", json!({}));
    set("climate.salon", "heat", json!({"temperature": 20}));
    set("light.prise_frigo", "on", json!({"device_class": "outlet"}));
    set(
        "binary_sensor.fenetre",
        "off",
        json!({"device_class": "window", "friendly_name": "Fenêtre"}),
    );
    let token = app.login().await;

    let (status, body) = app
        .request("GET", "/api/automations/presets", Some(&token), None)
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let names: Vec<&str> = body["presets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        ["depart", "retour", "nuit", "matin", "vacances", "lessive"]
    );
    let depart = &body["presets"][0];
    assert_eq!(depart["enabled"], false);
    assert_eq!(depart["options"]["chauffage"], 17.0);
    // La prise n'est pas une lumière ; le garage est une ouverture.
    assert_eq!(
        body["house"]["lights"],
        json!([{"entity_id": "light.salon", "name": "salon"}])
    );
    assert_eq!(body["house"]["openings"], 2);
    assert_eq!(body["house"]["people"], true);

    let tick = json!({"enabled": true, "options": {"volets": true}});
    let (status, body) = app
        .request(
            "PUT",
            "/api/automations/presets/depart",
            Some(&token),
            Some(tick.clone()),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "le mot de passe d'abord");
    assert_eq!(body["confirm"], true);
    let (status, _) = app
        .request(
            "POST",
            "/api/auth/confirm",
            Some(&token),
            Some(json!({"password": PASSWORD})),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = app
        .request(
            "PUT",
            "/api/automations/presets/depart",
            Some(&token),
            Some(tick),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let actions: Vec<&str> = body["config"]["actions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["action"].as_str().unwrap_or("if"))
        .collect();
    assert_eq!(
        actions,
        [
            "light.turn_off",
            "cover.close_cover",
            "climate.set_temperature",
            "if"
        ]
    );

    // Une automatisation ordinaire, active, et les réglages gardés.
    let (_, body) = app
        .request("GET", "/api/automations", Some(&token), None)
        .await;
    let entry = body["automations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["id"] == "pret_depart")
        .unwrap()
        .clone();
    assert_eq!(entry["enabled"], true);
    assert_eq!(entry["source"], "editor");
    let (_, body) = app
        .request("GET", "/api/automations/presets", Some(&token), None)
        .await;
    assert_eq!(body["presets"][0]["enabled"], true);
    assert_eq!(body["presets"][0]["options"]["volets"], true);

    // Décochée : gardée, désactivée.
    let (status, _) = app
        .request(
            "PUT",
            "/api/automations/presets/depart",
            Some(&token),
            Some(json!({"enabled": false, "options": {"volets": true}})),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (_, body) = app
        .request("GET", "/api/automations/presets", Some(&token), None)
        .await;
    assert_eq!(body["presets"][0]["enabled"], false);
    assert_eq!(body["presets"][0]["exists"], true);

    // Des réglages faux, une automatisation inconnue.
    let (status, body) = app
        .request(
            "PUT",
            "/api/automations/presets/nuit",
            Some(&token),
            Some(json!({"enabled": true, "options": {"heure": "25:00"}})),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["error"].as_str().unwrap().contains("heure"), "{body}");
    let (status, _) = app
        .request(
            "PUT",
            "/api/automations/presets/cafe",
            Some(&token),
            Some(json!({"enabled": true})),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = app
        .request(
            "PUT",
            "/api/automations/presets/nuit",
            Some(&token),
            Some(json!({"enabled": true, "options": {"cafe": true}})),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "réglage inconnu refusé");

    // Un invité ne coche rien.
    let guest = app.login_as("visiteur", ha_auth::Role::Guest).await;
    let (status, _) = app
        .request(
            "PUT",
            "/api/automations/presets/nuit",
            Some(&guest),
            Some(json!({"enabled": true})),
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
            "domains": ["light", "switch", "fan", "scene", "cover", "media_player"],
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
async fn a_guest_never_sees_nor_controls_the_server_switches() {
    // La simulation de présence dit quand la maison est vide ; le délestage
    // protège le compteur : pas pour un invité, même si ce sont des
    // « switch ». Un rappel (étape 69) dit que le garage est ouvert.
    let app = TestApp::new().await;
    for (entity, attributes) in [
        ("switch.simulation_presence", json!({"en_cours": true})),
        ("switch.delestage", json!({"coupes": []})),
        ("alert.garage_ouvert", json!({"surveille": "cover.garage"})),
        ("switch.protection_solaire", json!({"baisses": []})),
        ("switch.prise_salon", json!({})),
    ] {
        let Value::Object(attributes) = attributes else {
            panic!("un objet est attendu")
        };
        app.state
            .core
            .states
            .set(entity.parse().unwrap(), "on", attributes)
            .unwrap();
    }
    let guest = app.login_as("visiteur", ha_auth::Role::Guest).await;
    let (_, listed) = app.request("GET", "/api/states", Some(&guest), None).await;
    let ids: Vec<&str> = listed
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["entity_id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["switch.prise_salon"]);
    for hidden in [
        "switch.simulation_presence",
        "switch.delestage",
        "alert.garage_ouvert",
        "switch.protection_solaire",
    ] {
        let (status, _) = app
            .request("GET", &format!("/api/states/{hidden}"), Some(&guest), None)
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{hidden}");
        let domain = hidden.split('.').next().unwrap();
        let (status, _) = app
            .request(
                "POST",
                &format!("/api/services/{domain}/turn_off"),
                Some(&guest),
                Some(json!({"entity_id": hidden})),
            )
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{hidden}");
    }
    // La famille, elle, les commande.
    let family = app.login_as("alice", ha_auth::Role::Family).await;
    let (status, _) = app
        .request(
            "POST",
            "/api/services/switch/turn_off",
            Some(&family),
            Some(json!({"entity_id": "switch.delestage"})),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = app
        .request(
            "POST",
            "/api/services/alert/turn_off",
            Some(&family),
            Some(json!({"entity_id": "alert.garage_ouvert"})),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn a_guest_order_in_a_protected_room_waits_for_the_family() {
    let app = TestApp::new().await;
    furnish(&app);
    let core = &app.state.core;
    core.guard.configure(
        ha_core::Policy {
            protected: [("light.salon".parse().unwrap(), "Salon".to_owned())].into(),
            quiet_hours: None,
        },
        || 0,
    );
    let guest = app.login_as("visiteur", ha_auth::Role::Guest).await;
    let family = app.login_as("alice", ha_auth::Role::Family).await;
    let mut events = core.bus.subscribe();
    let lamp = json!({"entity_id": "light.salon"});

    // L'invité : l'ordre attend.
    let (status, body) = app
        .request(
            "POST",
            "/api/services/light/turn_on",
            Some(&guest),
            Some(lamp.clone()),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    assert_eq!(body["result"], "held");
    assert!(
        body["message"]
            .as_str()
            .unwrap()
            .contains("«\u{a0}Salon\u{a0}» est une pièce protégée")
    );
    let (status, _) = app
        .request("GET", "/api/approvals", Some(&guest), None)
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = app
        .request(
            "POST",
            "/api/approvals/1",
            Some(&guest),
            Some(json!({"approve": true})),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "un invité ne se valide pas");
    let (_, listed) = app
        .request("GET", "/api/approvals", Some(&family), None)
        .await;
    let held = &listed["approvals"][0];
    assert_eq!(held["entity_id"], "light.salon");
    assert_eq!(held["by"], "par visiteur");
    let id = held["id"].as_u64().unwrap();

    // La famille valide : l'ordre part, avec son origine.
    let (status, body) = app
        .request(
            "POST",
            &format!("/api/approvals/{id}"),
            Some(&family),
            Some(json!({"approve": true})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["result"], "sent");
    loop {
        let event = events.recv().await.unwrap();
        if let ha_core::Event::CallService {
            service, origin, ..
        } = event.as_ref()
        {
            assert_eq!(service, "turn_on");
            assert_eq!(*origin, ha_core::Origin::User("visiteur".into()));
            break;
        }
    }
    // Déjà traité.
    let (status, _) = app
        .request(
            "POST",
            &format!("/api/approvals/{id}"),
            Some(&family),
            Some(json!({"approve": false})),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Refusé : rien ne part.
    app.request(
        "POST",
        "/api/services/light/turn_off",
        Some(&guest),
        Some(lamp.clone()),
    )
    .await;
    let id = core.held_orders()[0].id;
    let (_, body) = app
        .request(
            "POST",
            &format!("/api/approvals/{id}"),
            Some(&family),
            Some(json!({"approve": false})),
        )
        .await;
    assert_eq!(body["result"], "refused");
    assert_eq!(core.held_orders().len(), 0);

    // La famille n'est jamais retenue, et le volet n'est pas protégé.
    let (status, body) = app
        .request(
            "POST",
            "/api/services/light/turn_on",
            Some(&family),
            Some(lamp),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, _) = app
        .request(
            "POST",
            "/api/services/cover/close_cover",
            Some(&guest),
            Some(json!({"entity_id": "cover.salon"})),
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    let journal = app.state.store.audit_log(20).await.unwrap();
    let actions: Vec<(&str, &str)> = journal
        .iter()
        .filter(|e| ["held", "approve", "deny"].contains(&e.action.as_str()))
        .map(|e| (e.actor.as_str(), e.action.as_str()))
        .collect();
    assert_eq!(
        actions,
        [
            ("alice", "deny"),
            ("visiteur", "held"),
            ("alice", "approve"),
            ("visiteur", "held"),
        ]
    );
}

/// Un abonnement comme celui d'un navigateur (clés de l'exemple de la RFC 8291).
fn browser_subscription(endpoint: &str) -> Value {
    json!({
        "endpoint": endpoint,
        "expirationTime": null,
        "keys": {
            "p256dh": "BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4",
            "auth": "BTBZMqHH6r4Tts7J_aSIgg",
        },
        "name": "Pixel de David",
    })
}

#[tokio::test]
async fn phones_subscribe_to_notifications() {
    let app = TestApp::new().await;
    let guest = app.login_as("visiteur", ha_auth::Role::Guest).await;
    let alice = app.login_as("alice", ha_auth::Role::Family).await;
    let david = app.login().await;
    let fcm = "https://fcm.googleapis.com/fcm/send/abc:def";

    // Un invité ne reçoit pas de notifications.
    for (method, uri) in [("GET", "/api/push"), ("POST", "/api/push/test")] {
        let (status, _) = app.request(method, uri, Some(&guest), None).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{uri}");
    }
    let (status, _) = app
        .request(
            "POST",
            "/api/push/subscriptions",
            Some(&guest),
            Some(browser_subscription(fcm)),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Le service n'est pas démarré : pas de clé.
    let (_, body) = app.request("GET", "/api/push", Some(&david), None).await;
    assert_eq!(body["available"], false);
    let (status, _) = app
        .request(
            "POST",
            "/api/push/subscriptions",
            Some(&david),
            Some(browser_subscription(fcm)),
        )
        .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);

    let vapid = ha_push::load_or_create_vapid(&app.state.store)
        .await
        .unwrap();
    let (_, body) = app.request("GET", "/api/push", Some(&david), None).await;
    assert_eq!(body["available"], true);
    assert_eq!(body["public_key"], vapid.public_key());
    assert_eq!(body["devices"], json!([]));

    let (status, body) = app
        .request(
            "POST",
            "/api/push/subscriptions",
            Some(&david),
            Some(browser_subscription(fcm)),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let id = body["id"].as_i64().unwrap();
    let (_, body) = app.request("GET", "/api/push", Some(&david), None).await;
    let device = &body["devices"][0];
    assert_eq!(device["name"], "Pixel de David");
    assert_eq!(device["endpoint"], fcm);
    assert_eq!(device["categories"], 7);
    assert!(
        device.get("auth").is_none() && device.get("p256dh").is_none(),
        "jamais les clés"
    );

    // Une adresse qui n'est pas un service de notifications est refusée :
    // le serveur ne doit pas appeler une adresse choisie par la page.
    for endpoint in [
        "http://fcm.googleapis.com/fcm/send/x",
        "https://192.168.1.1/admin",
        "https://exemple.fr/push",
    ] {
        let (status, _) = app
            .request(
                "POST",
                "/api/push/subscriptions",
                Some(&david),
                Some(browser_subscription(endpoint)),
            )
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{endpoint}");
    }
    let mut bad_keys = browser_subscription("https://fcm.googleapis.com/fcm/send/y");
    bad_keys["keys"]["auth"] = json!("court");
    let (status, _) = app
        .request(
            "POST",
            "/api/push/subscriptions",
            Some(&david),
            Some(bad_keys),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // Ce qu'il reçoit ; personne d'autre ne touche à ses appareils.
    let uri = format!("/api/push/subscriptions/{id}");
    let (status, _) = app
        .request("PATCH", &uri, Some(&alice), Some(json!({"categories": 1})))
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = app
        .request("PATCH", &uri, Some(&david), Some(json!({"categories": 9})))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, body) = app
        .request("PATCH", &uri, Some(&david), Some(json!({"categories": 1})))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, _) = app.request("DELETE", &uri, Some(&alice), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Un essai : vers ses propres appareils seulement.
    let mut events = app.state.core.bus.subscribe();
    let (status, _) = app
        .request("POST", "/api/push/test", Some(&david), None)
        .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let event = events.recv().await.unwrap();
    let ha_core::Event::CallService {
        entity_id,
        data,
        origin,
        ..
    } = event.as_ref()
    else {
        panic!("un appel de service est attendu");
    };
    assert_eq!(entity_id.as_str(), "notify.appli");
    assert_eq!(data["test"], true);
    assert_eq!(*origin, ha_core::Origin::User("david".into()));

    // Dix appareils au plus par compte.
    for n in 0..9 {
        let endpoint = format!("https://updates.push.services.mozilla.com/wpush/v2/{n}");
        let (status, _) = app
            .request(
                "POST",
                "/api/push/subscriptions",
                Some(&alice),
                Some(browser_subscription(&endpoint)),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED);
    }
    let last = "https://web.push.apple.com/dixieme";
    let (status, _) = app
        .request(
            "POST",
            "/api/push/subscriptions",
            Some(&alice),
            Some(browser_subscription(last)),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, body) = app
        .request(
            "POST",
            "/api/push/subscriptions",
            Some(&alice),
            Some(browser_subscription("https://web.push.apple.com/onze")),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    // Le même navigateur qui se réabonne ne compte pas deux fois.
    let (status, _) = app
        .request(
            "POST",
            "/api/push/subscriptions",
            Some(&alice),
            Some(browser_subscription(last)),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, _) = app.request("DELETE", &uri, Some(&david), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = app.request("DELETE", &uri, Some(&david), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let journal = app.state.store.audit_log(50).await.unwrap();
    assert!(
        journal
            .iter()
            .any(|e| e.action == "push_subscribe" && e.actor == "david")
    );
    assert!(journal.iter().any(|e| e.action == "push_unsubscribe"));
}

#[tokio::test]
async fn an_order_the_device_cannot_follow_is_refused_before_it_leaves() {
    let app = TestApp::new().await;
    furnish(&app);
    let core = &app.state.core;
    core.guard.configure(
        ha_core::Policy {
            protected: [("cover.salon".parse().unwrap(), "Salon".to_owned())].into(),
            quiet_hours: None,
        },
        || 0,
    );
    let david = app.login().await;
    let guest = app.login_as("visiteur", ha_auth::Role::Guest).await;
    let mut events = core.bus.subscribe();
    for (token, data) in [
        (&david, json!({"entity_id": "cover.salon", "position": 150})),
        (&david, json!({"entity_id": "cover.salon"})),
        // Un invité dans une pièce protégée : refusé tout de suite, pas
        // retenu pour être validé puis refusé.
        (&guest, json!({"entity_id": "cover.salon", "position": -5})),
    ] {
        let (status, body) = app
            .request(
                "POST",
                "/api/services/cover/set_cover_position",
                Some(token),
                Some(data),
            )
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert!(
            body["error"]
                .as_str()
                .unwrap()
                .starts_with("cover.salon : position"),
            "{body}"
        );
    }
    assert_eq!(core.held_orders().len(), 0);
    assert!(
        events.try_recv().is_err(),
        "aucun ordre n'est parti sur le bus"
    );
    let (status, body) = app
        .request(
            "POST",
            "/api/services/cover/set_cover_position",
            Some(&david),
            Some(json!({"entity_id": "cover.salon", "position": 40})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

#[tokio::test]
async fn assist_understands_french_and_obeys_the_same_rules() {
    let app = TestApp::new().await;
    furnish(&app);
    let core = &app.state.core;
    for (id, name) in [
        ("cover.salon", "Volet salon"),
        ("light.salon", "Lampe salon"),
        ("lock.entree", "Porte d'entrée"),
        ("person.david", "David"),
    ] {
        let state = core.states.get(&id.parse().unwrap()).unwrap();
        let mut attributes = state.attributes().clone();
        attributes.insert("friendly_name".into(), name.into());
        core.states
            .set(id.parse().unwrap(), state.state(), attributes)
            .unwrap();
    }
    core.guard.configure(
        ha_core::Policy {
            protected: [("light.salon".parse().unwrap(), "Salon".to_owned())].into(),
            quiet_hours: None,
        },
        || 0,
    );
    let david = app.login().await;
    let guest = app.login_as("visiteur", ha_auth::Role::Guest).await;
    let say = |token: &String, text: &str| {
        let token = token.clone();
        let text = text.to_owned();
        let app = &app;
        async move {
            let (status, body) = app
                .request(
                    "POST",
                    "/api/conversation/process",
                    Some(&token),
                    Some(json!({"text": text, "language": "fr"})),
                )
                .await;
            assert_eq!(status, StatusCode::OK, "{body}");
            (
                body["response"]["response_type"]
                    .as_str()
                    .unwrap()
                    .to_owned(),
                body["response"]["speech"]["plain"]["speech"]
                    .as_str()
                    .unwrap()
                    .to_owned(),
            )
        }
    };

    let mut events = core.bus.subscribe();
    let (kind, speech) = say(&david, "Ferme le volet du salon").await;
    assert_eq!(kind, "action_done");
    assert_eq!(speech, "C'est fait : fermer Volet salon.");
    let event = events.recv().await.unwrap();
    let ha_core::Event::CallService {
        service, origin, ..
    } = event.as_ref()
    else {
        panic!("un ordre est attendu");
    };
    assert_eq!(service, "close_cover");
    assert_eq!(*origin, ha_core::Origin::User("david".into()));

    // Les valeurs sont vérifiées comme ailleurs.
    let (kind, speech) = say(&david, "ouvre le volet du salon à 150 %").await;
    assert_eq!(kind, "error");
    assert!(
        speech.contains("position entre 0 et 100 (reçu 150)"),
        "{speech}"
    );
    // Ouvrir la maison redemande le mot de passe.
    let (kind, speech) = say(&david, "déverrouille la porte d'entrée").await;
    assert_eq!(kind, "error");
    assert!(speech.contains("redonne ton mot de passe"), "{speech}");
    // Une question.
    let (kind, speech) = say(&david, "Qui est à la maison ?").await;
    assert_eq!(
        (kind.as_str(), speech.as_str()),
        ("query_answer", "À la maison : David.")
    );
    // Pas compris.
    let (kind, _) = say(&david, "fais-moi un café").await;
    assert_eq!(kind, "error");

    // L'invité : la pièce protégée attend une validation ; la présence et la
    // serrure lui restent invisibles.
    let (kind, speech) = say(&guest, "allume la lampe du salon").await;
    assert_eq!(kind, "action_done");
    assert!(
        speech.starts_with("En attente de validation : allumer Lampe salon"),
        "{speech}"
    );
    assert_eq!(core.held_orders().len(), 1);
    let (_, speech) = say(&guest, "qui est à la maison ?").await;
    assert_eq!(speech, "Je ne peux pas te le dire.");
    let (kind, _) = say(&guest, "verrouille la porte d'entrée").await;
    assert_eq!(kind, "error", "la serrure n'existe pas pour lui");

    // Sans connexion : refusé.
    let (status, _) = app
        .request(
            "POST",
            "/api/conversation/process",
            None,
            Some(json!({"text": "ferme le volet du salon"})),
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

/// Les autres noms (étape 94) : compris par Assist comme le nom, servis par
/// le WebSocket, sans jamais donner plus de droits que le nom.
#[tokio::test]
async fn other_names_work_like_names_and_never_give_more_rights() {
    let app = TestApp::with_areas(
        "
- area: Salon
  entities:
    - {entity: light.salon, name: Lampe salon, aliases: [la lampe du canapé]}
    - {entity: cover.salon, name: Volet salon, aliases: [le store]}
- area: Entrée
  entities:
    - {entity: lock.entree, name: Serrure, aliases: [la grosse porte]}
    - {entity: cover.garage, name: Porte du garage, aliases: [le store]}
",
    )
    .await;
    furnish(&app);
    let core = &app.state.core;
    core.guard.configure(
        ha_core::Policy {
            protected: [("light.salon".parse().unwrap(), "Salon".to_owned())].into(),
            quiet_hours: None,
        },
        || 0,
    );
    let david = app.login().await;
    let guest = app.login_as("visiteur", ha_auth::Role::Guest).await;
    let say = |token: &String, text: &str| {
        let token = token.clone();
        let text = text.to_owned();
        let app = &app;
        async move {
            let (_, body) = app
                .request(
                    "POST",
                    "/api/conversation/process",
                    Some(&token),
                    Some(json!({"text": text})),
                )
                .await;
            (
                body["response"]["response_type"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned(),
                body["response"]["speech"]["plain"]["speech"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned(),
            )
        }
    };

    // L'autre nom et la pièce : le store du salon, pas celui de l'entrée.
    let mut events = core.bus.subscribe();
    let (kind, _) = say(&david, "Ferme le store du salon").await;
    assert_eq!(kind, "action_done");
    let event = events.recv().await.unwrap();
    let ha_core::Event::CallService { entity_id, .. } = event.as_ref() else {
        panic!("un ordre est attendu");
    };
    assert_eq!(entity_id.as_str(), "cover.salon");
    // Deux appareils portent ce nom : Assist demande lequel.
    let (kind, speech) = say(&david, "ferme le store").await;
    assert_eq!(kind, "error");
    assert!(
        speech.starts_with("Lequel ?")
            && speech.contains("Volet salon")
            && speech.contains("Porte du garage"),
        "{speech}"
    );

    // L'invité : la pièce protégée reste protégée sous un autre nom…
    let (kind, speech) = say(&guest, "allume la lampe du canapé").await;
    assert_eq!(kind, "action_done");
    assert!(
        speech.starts_with("En attente de validation : allumer light.salon"),
        "{speech}"
    );
    assert_eq!(core.held_orders().len(), 1);
    // … et la serrure, invisible pour lui, ne se trouve pas par son autre nom.
    let mut events = core.bus.subscribe();
    let (kind, _) = say(&guest, "déverrouille la grosse porte").await;
    assert_eq!(kind, "error");
    assert!(events.try_recv().is_err(), "aucun ordre ne doit être parti");
    assert_eq!(
        core.states
            .get(&"lock.entree".parse().unwrap())
            .unwrap()
            .state(),
        "locked"
    );

    // Le WebSocket, comme le registre des appareils de Home Assistant.
    let addr = spawn_server(&app).await;
    let connect = |token: String| async move {
        let (mut socket, _) =
            tokio_tungstenite::connect_async(format!("ws://{addr}/api/websocket"))
                .await
                .unwrap();
        assert_eq!(ws_recv(&mut socket).await["type"], "auth_required");
        ws_send(&mut socket, json!({"type": "auth", "access_token": token})).await;
        assert_eq!(ws_recv(&mut socket).await["type"], "auth_ok");
        socket
    };
    let mut socket = connect(david.clone()).await;
    ws_send(
        &mut socket,
        json!({"id": 1, "type": "config/entity_registry/get", "entity_id": "lock.entree"}),
    )
    .await;
    let reply = ws_recv(&mut socket).await;
    assert_eq!(reply["success"], true, "{reply}");
    assert_eq!(reply["result"]["name"], "Serrure");
    assert_eq!(reply["result"]["aliases"], json!(["la grosse porte"]));
    assert_eq!(reply["result"]["area_id"], "entree");

    let mut socket = connect(guest.clone()).await;
    ws_send(
        &mut socket,
        json!({"id": 1, "type": "config/entity_registry/get", "entity_id": "lock.entree"}),
    )
    .await;
    assert_eq!(ws_recv(&mut socket).await["error"]["code"], "not_found");
    ws_send(
        &mut socket,
        json!({
            "id": 2,
            "type": "config/entity_registry/get_entries",
            "entity_ids": ["light.salon", "lock.entree", "pas un appareil"],
        }),
    )
    .await;
    let reply = ws_recv(&mut socket).await;
    assert_eq!(
        reply["result"]["light.salon"]["aliases"],
        json!(["la lampe du canapé"])
    );
    assert_eq!(reply["result"]["lock.entree"], Value::Null);
    assert_eq!(reply["result"]["pas un appareil"], Value::Null);

    app.state.shutdown();
}

/// Une pièce donnée par l'intégration (SmartThings, étape 86) compte pour
/// Assist comme une pièce d'areas.yaml.
#[tokio::test]
async fn assist_knows_the_room_given_by_an_integration() {
    let app = TestApp::new().await;
    for (id, name) in [
        ("light.bureau_haut", "Plafonnier"),
        ("light.bureau_lampe", "Lampe"),
    ] {
        let mut attributes = Attributes::new();
        attributes.insert("friendly_name".into(), name.into());
        attributes.insert("piece".into(), "Bureau".into());
        app.state
            .core
            .states
            .set(id.parse().unwrap(), "on", attributes)
            .unwrap();
    }
    let david = app.login().await;
    let (status, body) = app
        .request(
            "POST",
            "/api/conversation/process",
            Some(&david),
            Some(json!({"text": "éteins les lumières du bureau"})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let speech = body["response"]["speech"]["plain"]["speech"]
        .as_str()
        .unwrap();
    assert!(
        speech.contains("Plafonnier") && speech.contains("Lampe"),
        "{speech}"
    );
}

/// « Ajoute du pain à la liste de courses » (étape 84) : un ordre à la liste.
#[tokio::test]
async fn assist_adds_to_the_shopping_list() {
    let app = TestApp::new().await;
    let mut attributes = Attributes::new();
    attributes.insert("friendly_name".into(), "Liste de courses".into());
    app.state
        .core
        .states
        .set("todo.courses".parse().unwrap(), "0", attributes)
        .unwrap();
    let david = app.login().await;
    let mut events = app.state.core.bus.subscribe();
    let (status, body) = app
        .request(
            "POST",
            "/api/conversation/process",
            Some(&david),
            Some(json!({"text": "Ajoute du pain de mie à la liste de courses"})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["response"]["speech"]["plain"]["speech"],
        "C'est fait : Pain de mie ajouté à « Liste de courses »."
    );
    let event = events.recv().await.unwrap();
    let ha_core::Event::CallService {
        domain,
        service,
        data,
        ..
    } = event.as_ref()
    else {
        panic!("un ordre est attendu");
    };
    assert_eq!((domain.as_str(), service.as_str()), ("todo", "add_item"));
    assert_eq!(data["item"], "Pain de mie");
}

/// Une phrase dans une conversation (étape 96) : `(type, réponse,
/// conversation_id, continuer)`.
async fn say_in(
    app: &TestApp,
    token: &str,
    text: &str,
    conversation_id: Option<&str>,
) -> (String, String, Option<String>, bool) {
    let mut body = json!({"text": text, "language": "fr"});
    if let Some(id) = conversation_id {
        body["conversation_id"] = json!(id);
    }
    let (status, body) = app
        .request("POST", "/api/conversation/process", Some(token), Some(body))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    (
        body["response"]["response_type"]
            .as_str()
            .unwrap()
            .to_owned(),
        body["response"]["speech"]["plain"]["speech"]
            .as_str()
            .unwrap()
            .to_owned(),
        body["conversation_id"].as_str().map(str::to_owned),
        body["continue_conversation"].as_bool().unwrap(),
    )
}

/// Deux lampes rangées dans deux pièces.
fn two_rooms(app: &TestApp) {
    for (id, name, room) in [
        ("light.lampe_salon", "Lampe", "Salon"),
        ("light.plafonnier_cuisine", "Plafonnier", "Cuisine"),
    ] {
        let mut attributes = Attributes::new();
        attributes.insert("friendly_name".into(), name.into());
        attributes.insert("piece".into(), room.into());
        app.state
            .core
            .states
            .set(id.parse().unwrap(), "off", attributes)
            .unwrap();
    }
}

/// L'ordre suivant sur le bus : `(service, entité)`.
async fn next_order(
    events: &mut tokio::sync::broadcast::Receiver<Arc<ha_core::Event>>,
) -> (String, String) {
    loop {
        let event = tokio::time::timeout(Duration::from_secs(5), events.recv())
            .await
            .expect("un ordre est attendu")
            .unwrap();
        if let ha_core::Event::CallService {
            service, entity_id, ..
        } = event.as_ref()
        {
            return (service.clone(), entity_id.to_string());
        }
    }
}

/// Enchaîner les phrases (étape 96) : « et la cuisine aussi »,
/// « éteins-la », puis « merci » qui termine la conversation.
#[tokio::test]
async fn assist_follows_the_conversation() {
    let app = TestApp::new().await;
    two_rooms(&app);
    let david = app.login().await;
    let mut events = app.state.core.bus.subscribe();

    let (kind, _, id, more) = say_in(&app, &david, "allume la lumière du salon", None).await;
    assert_eq!(kind, "action_done");
    assert!(more, "la tablette peut écouter la suite");
    let id = id.expect("un conversation_id");
    assert_eq!(id.len(), 32);
    assert_eq!(
        next_order(&mut events).await,
        ("turn_on".into(), "light.lampe_salon".into())
    );

    let (kind, speech, same, _) = say_in(&app, &david, "et la cuisine aussi", Some(&id)).await;
    assert_eq!(kind, "action_done", "{speech}");
    assert_eq!(same.as_deref(), Some(id.as_str()));
    assert_eq!(
        next_order(&mut events).await,
        ("turn_on".into(), "light.plafonnier_cuisine".into())
    );

    let (kind, speech, _, _) = say_in(&app, &david, "Éteins-la.", Some(&id)).await;
    assert_eq!(kind, "action_done", "{speech}");
    assert_eq!(
        next_order(&mut events).await,
        ("turn_off".into(), "light.plafonnier_cuisine".into())
    );

    // « Merci » : une réponse courte, et c'est fini.
    let (kind, speech, _, more) = say_in(&app, &david, "Merci !", Some(&id)).await;
    assert_eq!(
        (kind.as_str(), speech.as_str()),
        ("action_done", "Avec plaisir.")
    );
    assert!(!more);
    // Le contexte est oublié.
    let (kind, _, _, _) = say_in(&app, &david, "éteins-la", Some(&id)).await;
    assert_eq!(kind, "error");

    // Sans conversation_id : pas de suite possible.
    let (kind, _, _, _) = say_in(&app, &david, "et la cuisine aussi", None).await;
    assert_eq!(kind, "error");
}

/// Une conversation appartient à son compte, et une suite passe par les
/// mêmes droits qu'une phrase complète.
#[tokio::test]
async fn a_conversation_keeps_the_rights_of_its_account() {
    let app = TestApp::new().await;
    two_rooms(&app);
    furnish(&app);
    let david = app.login().await;
    let guest = app.login_as("visiteur", ha_auth::Role::Guest).await;

    let (_, _, id, _) = say_in(&app, &david, "allume la lumière du salon", None).await;
    let id = id.unwrap();
    // Un autre compte qui présente l'identifiant : une nouvelle conversation,
    // sans le contexte de David.
    let (kind, _, other, _) = say_in(&app, &guest, "éteins-la", Some(&id)).await;
    assert_eq!(kind, "error");
    assert_ne!(other.as_deref(), Some(id.as_str()));

    // L'invité ferme le volet du salon, puis « et le garage » : refusé,
    // comme s'il l'avait demandé en entier.
    let (kind, speech, id, _) = say_in(&app, &guest, "ferme le volet du salon", None).await;
    assert_eq!(kind, "action_done", "{speech}");
    let (kind, speech, _, _) = say_in(&app, &guest, "et le garage", id.as_deref()).await;
    assert_eq!(kind, "error", "{speech}");
    assert!(speech.starts_with("Pas fait"), "{speech}");

    // Un identifiant mal formé est ignoré.
    let (kind, _, fresh, _) = say_in(&app, &david, "éteins-la", Some("../../etc")).await;
    assert_eq!(kind, "error");
    assert_eq!(fresh.map(|id| id.len()), Some(32));
}

/// Un faux service d'écoute (comme sherpa-onnx, par WebSocket) : il répond
/// les phrases données, une par son reçu, et compte les sons.
async fn fake_listener(
    sentences: Vec<&'static str>,
) -> (String, Arc<std::sync::atomic::AtomicUsize>) {
    slow_listener(sentences, Duration::ZERO).await
}

/// Le même, qui prend `delay` pour écouter chaque son.
async fn slow_listener(
    sentences: Vec<&'static str>,
    delay: Duration,
) -> (String, Arc<std::sync::atomic::AtomicUsize>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let heard = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let count = Arc::clone(&heard);
    tokio::spawn(async move {
        for sentence in sentences {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            let Some(Ok(Message::Binary(_))) = socket.next().await else {
                panic!("un son est attendu");
            };
            count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            tokio::time::sleep(delay).await;
            socket
                .send(Message::Text(json!({"text": sentence}).to_string().into()))
                .await
                .unwrap();
            let _ = socket.next().await;
        }
    });
    (format!("ws://{address}"), heard)
}

/// Enchaîner à la voix (étape 96) : `?conversation_id=` sur
/// `/api/conversation/audio`.
#[tokio::test]
async fn the_tablet_follows_a_spoken_conversation() {
    let app = TestApp::new().await;
    two_rooms(&app);
    let david = app.login().await;
    let (url, _) = fake_listener(vec!["Allume la lumière du salon.", "Et la cuisine."]).await;
    app.state
        .set_voice(Some(ha_stt::Stt::from_settings(&url).unwrap()), None);
    let mut events = app.state.core.bus.subscribe();

    let (status, bytes, _) = app
        .send_bytes("POST", "/api/conversation/audio", &david, spoken_wav())
        .await;
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["continue_conversation"], true);
    let id = body["conversation_id"].as_str().unwrap().to_owned();
    assert_eq!(next_order(&mut events).await.1, "light.lampe_salon");

    let (status, bytes, _) = app
        .send_bytes(
            "POST",
            &format!("/api/conversation/audio?conversation_id={id}"),
            &david,
            spoken_wav(),
        )
        .await;
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["text"], "Et la cuisine.");
    assert_eq!(body["conversation_id"], id.as_str());
    assert_eq!(next_order(&mut events).await.1, "light.plafonnier_cuisine");

    // Un paramètre inconnu : refusé.
    let (status, _, _) = app
        .send_bytes(
            "POST",
            "/api/conversation/audio?autre=1",
            &david,
            spoken_wav(),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

/// La confirmation après un ordre (étape 95) : « C'est fait » seulement si
/// l'appareil a changé d'état, sinon on le dit honnêtement.
#[tokio::test]
async fn assist_says_done_only_when_the_device_obeyed() {
    let app = TestApp::new().await;
    let core = Arc::clone(&app.state.core);
    for (id, value, name) in [
        ("light.salon", "off", "Lampe salon"),
        ("cover.chambre", "open", "Volet chambre"),
        ("light.cuisine", "unavailable", "Lampe cuisine"),
    ] {
        let mut attributes = Attributes::new();
        attributes.insert("friendly_name".into(), name.into());
        core.states
            .set_by("mqtt", id.parse().unwrap(), value, attributes)
            .unwrap();
    }
    // Une fausse lampe : elle s'allume 200 ms après l'ordre. Le volet, lui,
    // ne répond jamais.
    let mut events = core.bus.subscribe();
    let lamp = Arc::clone(&core);
    tokio::spawn(async move {
        while let Ok(event) = events.recv().await {
            if let ha_core::Event::CallService {
                domain, service, ..
            } = event.as_ref()
                && (domain.as_str(), service.as_str()) == ("light", "turn_on")
            {
                tokio::time::sleep(Duration::from_millis(200)).await;
                let id: ha_core::EntityId = "light.salon".parse().unwrap();
                let attributes = lamp.states.get(&id).unwrap().attributes().clone();
                lamp.states.set(id, "on", attributes).unwrap();
            }
        }
    });
    let david = app.login().await;
    let say = |text: &str| {
        let text = text.to_owned();
        let app = &app;
        let david = &david;
        async move {
            let started = std::time::Instant::now();
            let (status, body) = app
                .request(
                    "POST",
                    "/api/conversation/process",
                    Some(david),
                    Some(json!({ "text": text })),
                )
                .await;
            assert_eq!(status, StatusCode::OK, "{body}");
            (
                body["response"]["response_type"]
                    .as_str()
                    .unwrap()
                    .to_owned(),
                body["response"]["speech"]["plain"]["speech"]
                    .as_str()
                    .unwrap()
                    .to_owned(),
                started.elapsed(),
            )
        }
    };

    // Confirmé après un moment : la réponse n'attend pas la fin du délai.
    let (kind, speech, took) = say("allume la lampe du salon").await;
    assert_eq!(
        (kind.as_str(), speech.as_str()),
        ("action_done", "C'est fait : allumer Lampe salon.")
    );
    assert!(took < Duration::from_secs(2), "{took:?}");

    // Déjà dans le bon état : pas d'attente.
    let (kind, speech, took) = say("allume la lampe du salon").await;
    assert_eq!(
        (kind.as_str(), speech.as_str()),
        ("action_done", "C'est fait : allumer Lampe salon.")
    );
    assert!(took < Duration::from_secs(1), "{took:?}");

    // Jamais confirmé : on le dit, au bout de 3 s au plus.
    let (kind, speech, took) = say("ferme le volet de la chambre").await;
    assert_eq!(kind, "action_done");
    assert_eq!(
        speech,
        "J'ai envoyé l'ordre, mais Volet chambre n'a pas encore répondu."
    );
    assert!(took >= Duration::from_secs(3), "{took:?}");
    assert!(took < Duration::from_secs(5), "{took:?}");

    // Indisponible : dit tout de suite.
    let (kind, speech, took) = say("allume la lampe de la cuisine").await;
    assert_eq!(
        (kind.as_str(), speech.as_str()),
        (
            "error",
            "J'ai envoyé l'ordre, mais Lampe cuisine est indisponible."
        )
    );
    assert!(took < Duration::from_secs(1), "{took:?}");
}

/// Un morceau envoyé au mot d'appel : (statut, réponse).
async fn wake(app: &TestApp, token: &str, wav: Vec<u8>) -> (StatusCode, Value) {
    let (status, bytes, _) = app
        .send_bytes("POST", "/api/conversation/wake", token, wav)
        .await;
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

/// Le mot d'appel (étape 97) : « Maison, allume la lumière du salon » obéit,
/// « Maison » seul ouvre le micro, une autre phrase ne fait rien.
#[tokio::test]
async fn the_wake_word_wakes_the_tablet() {
    let app = TestApp::new().await;
    two_rooms(&app);
    let david = app.login().await;

    // Sans écoute : le mot d'appel n'existe pas.
    let (status, _) = wake(&app, &david, spoken_wav()).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);

    let (url, heard) = fake_listener(vec![
        "Maison, allume la lumière du salon.",
        "Maison.",
        "Il fait beau aujourd'hui.",
    ])
    .await;
    app.state
        .set_voice(Some(ha_stt::Stt::from_settings(&url).unwrap()), None);
    let (_, body) = app
        .request("GET", "/api/conversation/voice", Some(&david), None)
        .await;
    assert_eq!(body["wake"], true);
    assert_eq!(body["wake_word"], "Maison");
    let mut events = app.state.core.bus.subscribe();

    // Le mot puis une commande : obéie, comme à la voix.
    let (status, body) = wake(&app, &david, spoken_wav()).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["wake"], true);
    assert_eq!(body["text"], "allume la lumière du salon.");
    assert_eq!(body["response"]["response_type"], "action_done");
    assert_eq!(body["continue_conversation"], true);
    assert_eq!(
        next_order(&mut events).await,
        ("turn_on".into(), "light.lampe_salon".into())
    );

    // Le mot seul : la tablette ouvrira le micro, aucun ordre.
    let (status, body) = wake(&app, &david, spoken_wav()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({"wake": true}));

    // Pas de mot : rien d'autre que « non », ni le texte entendu.
    let (status, body) = wake(&app, &david, spoken_wav()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({"wake": false}));
    assert_eq!(heard.load(std::sync::atomic::Ordering::SeqCst), 3);
    assert!(
        tokio::time::timeout(Duration::from_millis(200), events.recv())
            .await
            .is_err(),
        "aucun ordre"
    );

    // Un silence : pas même écouté.
    let (status, body) = wake(&app, &david, silent_wav(1000)).await;
    assert_eq!((status, body), (StatusCode::OK, json!({"wake": false})));
    assert_eq!(heard.load(std::sync::atomic::Ordering::SeqCst), 3);

    // Sans connexion : refusé.
    let request = Request::builder()
        .method("POST")
        .uri("/api/conversation/wake")
        .body(Body::from(spoken_wav()))
        .unwrap();
    let response = app.router().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

/// Les garde-fous du mot d'appel : morceaux courts, débit limité par
/// appareil, jeton d'assistant refusé.
#[tokio::test]
async fn the_wake_word_has_limits() {
    let app = TestApp::new().await;
    let david = app.login().await;
    let (url, heard) = fake_listener(vec!["Maison."]).await;
    app.state
        .set_voice(Some(ha_stt::Stt::from_settings(&url).unwrap()), None);

    // Trop long : refusé dès la lecture (7 s), ou après l'en-tête (6,6 s),
    // sans être écouté.
    let (status, _) = wake(&app, &david, tone_wav(7000)).await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    let (status, body) = wake(&app, &david, tone_wav(6600)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    // Pas un WAV : refusé.
    let (status, _) = wake(&app, &david, b"bonjour".to_vec()).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(heard.load(std::sync::atomic::Ordering::SeqCst), 0);

    // Le débit : 30 morceaux par minute et par appareil (les 2 refusés
    // après lecture comptent déjà).
    for _ in 2..30 {
        let (status, _) = wake(&app, &david, silent_wav(1000)).await;
        assert_eq!(status, StatusCode::OK);
    }
    let (status, body) = wake(&app, &david, silent_wav(1000)).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{body}");
    // Un autre appareil (un autre jeton) n'est pas gêné.
    let tablet = app.login_as("cuisine", ha_auth::Role::Family).await;
    let (status, body) = wake(&app, &tablet, silent_wav(1000)).await;
    assert_eq!((status, body), (StatusCode::OK, json!({"wake": false})));

    // Un jeton d'assistant IA n'est pas une personne.
    let issued = app
        .state
        .auth
        .issue_program_token("david", Duration::from_hours(24))
        .await
        .unwrap()
        .unwrap();
    let (status, _) = wake(&app, issued.token.as_str(), spoken_wav()).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

/// Une vraie phrase passe avant : pendant qu'elle est écoutée, un morceau
/// du mot d'appel est abandonné tout de suite.
#[tokio::test]
async fn a_spoken_sentence_goes_before_the_wake_word() {
    let app = TestApp::new().await;
    two_rooms(&app);
    let david = app.login().await;
    let (url, heard) = slow_listener(
        vec!["Allume la lumière du salon."],
        Duration::from_millis(800),
    )
    .await;
    app.state
        .set_voice(Some(ha_stt::Stt::from_settings(&url).unwrap()), None);

    let sentence = app.send_bytes("POST", "/api/conversation/audio", &david, spoken_wav());
    let chunk = async {
        tokio::time::sleep(Duration::from_millis(200)).await;
        let started = std::time::Instant::now();
        let answer = wake(&app, &david, spoken_wav()).await;
        (answer, started.elapsed())
    };
    let ((status, bytes, _), ((wake_status, body), waited)) = tokio::join!(sentence, chunk);
    assert_eq!(
        status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&bytes)
    );
    assert_eq!(wake_status, StatusCode::OK);
    assert_eq!(body, json!({"wake": false, "busy": true}));
    assert!(waited < Duration::from_millis(500), "{waited:?}");
    assert_eq!(heard.load(std::sync::atomic::Ordering::SeqCst), 1);
}

/// Un WAV d'une seconde (16 bits, mono, 16 000 Hz) : une note.
fn spoken_wav() -> Vec<u8> {
    tone_wav(1000)
}

/// Une note de `millis` millisecondes.
fn tone_wav(millis: u32) -> Vec<u8> {
    wav_of(
        (0..16 * millis)
            .map(|i| if i % 40 < 20 { 6000 } else { -6000 })
            .collect(),
    )
}

/// Un silence de `millis` millisecondes (un léger souffle).
fn silent_wav(millis: u32) -> Vec<u8> {
    wav_of(
        (0..16 * millis)
            .map(|i| if i % 2 == 0 { 30 } else { -30 })
            .collect(),
    )
}

/// Un WAV 16 bits, mono, 16 000 Hz.
fn wav_of(samples: Vec<i16>) -> Vec<u8> {
    let data = u32::try_from(samples.len() * 2).unwrap();
    let mut out = Vec::new();
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    for (value, size) in [
        (16u32, 4),
        (1, 2),
        (1, 2),
        (16_000, 4),
        (32_000, 4),
        (2, 2),
        (16, 2),
    ] {
        out.extend_from_slice(&value.to_le_bytes()[..size]);
    }
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data.to_le_bytes());
    for sample in samples {
        out.extend_from_slice(&sample.to_le_bytes());
    }
    out
}

/// Parler à la maison (étape 84) : la tablette envoie le son, le serveur
/// l'écoute (ici un faux Whisper), le comprend et obéit ; la réponse est dite
/// par la voix (un faux Piper).
#[cfg(unix)]
#[tokio::test]
async fn the_tablet_can_talk_to_the_house() {
    use std::os::unix::fs::PermissionsExt;

    let app = TestApp::new().await;
    furnish(&app);
    let david = app.login().await;

    // Rien d'installé : pas de micro.
    let (_, body) = app
        .request("GET", "/api/conversation/voice", Some(&david), None)
        .await;
    assert_eq!(
        body,
        json!({"listen": false, "speak": false, "wake": false, "wake_word": "Maison"})
    );
    let (status, _, _) = app
        .send_bytes("POST", "/api/conversation/audio", &david, spoken_wav())
        .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);

    let dir = std::env::temp_dir().join(format!("ha-api-voix-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for name in [
        ha_stt::ENCODER,
        ha_stt::DECODER,
        ha_stt::TOKENS,
        "voix.onnx",
    ] {
        std::fs::write(dir.join(name), b"").unwrap();
    }
    let scripts = [
        (
            ha_stt::PROGRAM,
            "#!/bin/sh\necho '{\"lang\": \"fr\", \"text\": \" Ferme le volet du salon.\"}'\n",
        ),
        (
            "piper",
            "#!/bin/sh\ncat > /dev/null\nprintf '\\001\\000\\002\\000'\n",
        ),
    ];
    for (name, script) in scripts {
        std::fs::write(dir.join(name), script).unwrap();
        std::fs::set_permissions(dir.join(name), std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    app.state.set_voice(
        Some(ha_stt::Stt::from_dir(&dir).unwrap()),
        Some(ha_tts::Tts::new(ha_tts::Engine::Program {
            program: dir.join("piper"),
            model: dir.join("voix.onnx"),
        })),
    );
    let (_, body) = app
        .request("GET", "/api/conversation/voice", Some(&david), None)
        .await;
    // Le mot d'appel demande le service d'écoute, pas le programme.
    assert_eq!(
        body,
        json!({"listen": true, "speak": true, "wake": false, "wake_word": "Maison"})
    );
    let (status, _, _) = app
        .send_bytes("POST", "/api/conversation/wake", &david, spoken_wav())
        .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);

    let mut events = app.state.core.bus.subscribe();
    let (status, bytes, _) = app
        .send_bytes("POST", "/api/conversation/audio", &david, spoken_wav())
        .await;
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["text"], "Ferme le volet du salon.");
    assert_eq!(body["response"]["response_type"], "action_done");
    let event = events.recv().await.unwrap();
    let ha_core::Event::CallService { service, .. } = event.as_ref() else {
        panic!("un ordre est attendu");
    };
    assert_eq!(service, "close_cover");

    // Un son qui n'est pas un WAV : refusé avant d'être écouté.
    let (status, _, _) = app
        .send_bytes(
            "POST",
            "/api/conversation/audio",
            &david,
            b"bonjour".to_vec(),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    // Trop gros : refusé dès la lecture.
    let (status, _, _) = app
        .send_bytes(
            "POST",
            "/api/conversation/audio",
            &david,
            vec![0; ha_stt::MAX_WAV_BYTES + 1],
        )
        .await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);

    // La réponse, dite par la voix.
    let (status, bytes, content_type) = app
        .send_bytes(
            "POST",
            "/api/conversation/speech",
            &david,
            json!({"text": "C'est fait."}).to_string().into_bytes(),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(content_type.as_deref(), Some("audio/wav"));
    assert_eq!(&bytes[..4], b"RIFF");

    // Sans connexion : refusé.
    let (status, _) = app
        .request(
            "POST",
            "/api/conversation/speech",
            None,
            Some(json!({"text": "a"})),
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    std::fs::remove_dir_all(&dir).unwrap();
}

/// Une requête MCP (JSON-RPC), avec des en-têtes en plus.
async fn mcp(
    app: &TestApp,
    token: Option<&str>,
    headers: &[(&str, &str)],
    body: Value,
) -> (StatusCode, Value) {
    let mut request = Request::builder()
        .method("POST")
        .uri("/api/mcp")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ACCEPT, "application/json, text/event-stream");
    if let Some(token) = token {
        request = request.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    let response = app
        .router()
        .oneshot(request.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

fn tool(name: &str, arguments: Value) -> Value {
    let mut call = json!({"jsonrpc": "2.0", "id": 7, "method": "tools/call",
                          "params": {"name": name}});
    call["params"]["arguments"] = arguments;
    call
}

fn tool_text(body: &Value) -> (&str, bool) {
    (
        body["result"]["content"][0]["text"].as_str().unwrap(),
        body["result"]["isError"].as_bool().unwrap(),
    )
}

#[tokio::test]
async fn an_ai_assistant_plugs_in_through_mcp_under_the_guard() {
    let app = TestApp::new().await;
    furnish(&app);
    let core = &app.state.core;
    core.guard.configure(
        ha_core::Policy {
            protected: [("cover.salon".parse().unwrap(), "Salon".to_owned())].into(),
            quiet_hours: None,
        },
        || 0,
    );
    // Même avec un compte administrateur, un assistant IA reste un assistant.
    let claude = app.login().await;

    // Sans jeton : refusé. Depuis la page d'un autre site : refusé.
    let hello = json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {"protocolVersion": "2025-03-26", "capabilities": {},
                   "clientInfo": {"name": "essai", "version": "1"}}});
    let (status, _) = mcp(&app, None, &[], hello.clone()).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = mcp(
        &app,
        Some(&claude),
        &[
            ("origin", "https://pirate.example"),
            ("host", "maison.local"),
        ],
        hello.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, body) = mcp(
        &app,
        Some(&claude),
        &[("origin", "https://maison.local"), ("host", "maison.local")],
        hello.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // La négociation de version.
    let (_, body) = mcp(&app, Some(&claude), &[], hello).await;
    assert_eq!(body["id"], 1);
    assert_eq!(body["result"]["protocolVersion"], "2025-03-26");
    assert_eq!(body["result"]["serverInfo"]["name"], "maison");
    let (_, body) = mcp(
        &app,
        Some(&claude),
        &[],
        json!({"jsonrpc": "2.0", "id": 2, "method": "initialize",
               "params": {"protocolVersion": "1999-01-01"}}),
    )
    .await;
    assert_eq!(body["result"]["protocolVersion"], "2025-06-18");
    // Une notification : rien à répondre.
    let (status, _) = mcp(
        &app,
        Some(&claude),
        &[],
        json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);

    let (_, body) = mcp(
        &app,
        Some(&claude),
        &[],
        json!({"jsonrpc": "2.0", "id": 3, "method": "tools/list"}),
    )
    .await;
    let names: Vec<&str> = body["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["get_house_state", "assist", "call_service"]);

    let (_, body) = mcp(&app, Some(&claude), &[], tool("get_house_state", json!({}))).await;
    let (text, error) = tool_text(&body);
    assert!(!error);
    assert!(text.contains("(cover.salon) : ouvert"), "{text}");

    // Un ordre précis : passe, au nom de l'assistant.
    let mut events = core.bus.subscribe();
    let (_, body) = mcp(
        &app,
        Some(&claude),
        &[],
        tool(
            "call_service",
            json!({"domain": "light", "service": "turn_on", "entity_id": "light.salon"}),
        ),
    )
    .await;
    assert_eq!(
        tool_text(&body),
        ("C'est fait : allumer light.salon.", false)
    );
    let event = events.recv().await.unwrap();
    let ha_core::Event::CallService { origin, .. } = event.as_ref() else {
        panic!("un ordre est attendu");
    };
    assert_eq!(*origin, ha_core::Origin::Agent("david".into()));

    // Pièce protégée : retenu, même pour un compte administrateur.
    let (_, body) = mcp(
        &app,
        Some(&claude),
        &[],
        tool("assist", json!({"text": "ferme le volet du salon"})),
    )
    .await;
    let (text, error) = tool_text(&body);
    assert!(!error, "{text}");
    assert!(text.starts_with("En attente de validation"), "{text}");
    assert_eq!(
        core.held_orders()[0].origin,
        ha_core::Origin::Agent("david".into())
    );
    // Jamais ouvrir la maison.
    let (_, body) = mcp(
        &app,
        Some(&claude),
        &[],
        tool(
            "call_service",
            json!({"domain": "lock", "service": "unlock", "entity_id": "lock.entree"}),
        ),
    )
    .await;
    let (text, error) = tool_text(&body);
    assert!(error);
    assert!(text.contains("ne commande ni serrure"), "{text}");
    // Ni la porte du garage, même hors pièce protégée (revue de la PR #60).
    let (_, body) = mcp(
        &app,
        Some(&claude),
        &[],
        tool(
            "call_service",
            json!({"domain": "cover", "service": "open_cover", "entity_id": "cover.garage"}),
        ),
    )
    .await;
    let (text, error) = tool_text(&body);
    assert!(error, "{text}");
    assert!(text.contains("porte de garage"), "{text}");
    // Une valeur hors plage.
    let (_, body) = mcp(
        &app,
        Some(&claude),
        &[],
        tool(
            "call_service",
            json!({"domain": "cover", "service": "set_cover_position",
                   "entity_id": "cover.garage", "data": {"position": 300}}),
        ),
    )
    .await;
    assert!(tool_text(&body).1);

    // Erreurs du protocole.
    let (_, body) = mcp(
        &app,
        Some(&claude),
        &[],
        json!({"jsonrpc": "2.0", "id": 9, "method": "resources/list"}),
    )
    .await;
    assert_eq!(body["error"]["code"], -32601);
    let (_, body) = mcp(&app, Some(&claude), &[], tool("rm_rf", json!({}))).await;
    assert_eq!(body["error"]["code"], -32602);
    let (_, body) = mcp(&app, Some(&claude), &[], json!([1, 2])).await;
    assert_eq!(body["error"]["code"], -32600);

    // 60 appels d'outils par minute au plus.
    let mut limited = false;
    for _ in 0..60 {
        let (_, body) = mcp(&app, Some(&claude), &[], tool("get_house_state", json!({}))).await;
        if tool_text(&body).0.starts_with("Trop de demandes") {
            limited = true;
            break;
        }
    }
    assert!(limited);
    let journal = app.state.store.audit_log(200).await.unwrap();
    assert!(
        journal
            .iter()
            .any(|e| e.action == "mcp" && e.detail.as_deref() == Some("call_service"))
    );
}

#[tokio::test]
async fn an_assistant_token_only_opens_mcp() {
    let app = TestApp::new().await;
    furnish(&app);
    let _ = app.login().await;
    let issued = app
        .state
        .auth
        .issue_program_token("david", std::time::Duration::from_hours(24))
        .await
        .unwrap()
        .unwrap();
    let agent = issued.token.as_str();
    // Partout ailleurs que MCP, un jeton d'assistant n'existe pas : il ne
    // peut pas se faire passer pour une personne (garde contournée).
    for (method, uri, body) in [
        ("GET", "/api/states", None),
        (
            "POST",
            "/api/services/cover/open_cover",
            Some(json!({"entity_id": "cover.salon"})),
        ),
        (
            "POST",
            "/api/conversation/process",
            Some(json!({"text": "ouvre le volet du salon"})),
        ),
    ] {
        let (status, _) = app.request(method, uri, Some(agent), body).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{uri}");
    }
    let (status, body) = mcp(
        &app,
        Some(agent),
        &[],
        json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

/// Un ticket de bouton, comme le service des notifications en émet un.
async fn button_ticket(
    app: &TestApp,
    username: &str,
    purpose: ha_push::Purpose,
    actions: &[&str],
) -> String {
    let target = app
        .state
        .store
        .push_targets(ha_push::Category::ALL, None)
        .await
        .unwrap()
        .into_iter()
        .find(|target| target.username == username)
        .expect("un appareil abonné");
    app.state
        .push_tickets
        .issue(ha_push::Ticket::new(
            target.id,
            target.user_id,
            target.username,
            purpose,
            actions.iter().map(|a| (*a).to_owned()).collect(),
            "Ordre à valider".into(),
            std::time::Duration::from_mins(10),
        ))
        .unwrap()
}

#[tokio::test]
async fn a_held_order_is_answered_from_the_notification() {
    let app = TestApp::new().await;
    furnish(&app);
    let core = &app.state.core;
    core.guard.configure(
        ha_core::Policy {
            protected: [("light.salon".parse().unwrap(), "Salon".to_owned())].into(),
            quiet_hours: None,
        },
        || 0,
    );
    ha_push::load_or_create_vapid(&app.state.store)
        .await
        .unwrap();
    let guest = app.login_as("visiteur", ha_auth::Role::Guest).await;
    let alice = app.login_as("alice", ha_auth::Role::Family).await;
    let david = app.login().await;
    for (token, endpoint) in [
        (&alice, "https://fcm.googleapis.com/fcm/send/alice"),
        (&david, "https://fcm.googleapis.com/fcm/send/david"),
    ] {
        let (status, _) = app
            .request(
                "POST",
                "/api/push/subscriptions",
                Some(token),
                Some(browser_subscription(endpoint)),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED);
    }
    let lamp = json!({"entity_id": "light.salon"});
    app.request(
        "POST",
        "/api/services/light/turn_on",
        Some(&guest),
        Some(lamp.clone()),
    )
    .await;
    let id = core.held_orders()[0].id;
    let approval = ha_push::Purpose::Approval(id);
    let buttons = [ha_push::APPROVE, ha_push::DENY];
    let for_alice = button_ticket(&app, "alice", approval.clone(), &buttons).await;
    let for_david = button_ticket(&app, "david", approval.clone(), &buttons).await;
    let press = |token: &str, action: &str| json!({"token": token, "action": action});

    // Sans connexion, mais pas sans ticket.
    let (status, body) = app
        .request(
            "POST",
            "/api/push/action",
            None,
            Some(press("inventé", "approve")),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    // Un bouton qui n'est pas sur la notification : refusé, ticket gardé.
    let (status, _) = app
        .request(
            "POST",
            "/api/push/action",
            None,
            Some(press(&for_alice, "LESSIVE")),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // Alice valide depuis son téléphone : l'ordre part, avec son origine.
    let mut events = core.bus.subscribe();
    let (status, body) = app
        .request(
            "POST",
            "/api/push/action",
            None,
            Some(press(&for_alice, "approve")),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["result"], "sent");
    assert_eq!(body["message"], "Validé : allumer light.salon.");
    loop {
        let event = events.recv().await.unwrap();
        if let ha_core::Event::CallService {
            service, origin, ..
        } = event.as_ref()
        {
            assert_eq!(service, "turn_on");
            assert_eq!(*origin, ha_core::Origin::User("visiteur".into()));
            break;
        }
    }
    assert_eq!(core.held_orders().len(), 0);
    let journal = app.state.store.audit_log(20).await.unwrap();
    assert!(
        journal
            .iter()
            .any(|e| e.action == "approve" && e.actor == "alice")
    );
    // Le même bouton une deuxième fois, ou celui de David : plus rien.
    for token in [&for_alice, &for_david] {
        let (status, _) = app
            .request("POST", "/api/push/action", None, Some(press(token, "deny")))
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    // Refusé depuis la notification : rien ne part.
    app.request(
        "POST",
        "/api/services/light/turn_off",
        Some(&guest),
        Some(lamp.clone()),
    )
    .await;
    let id = core.held_orders()[0].id;
    let ticket = button_ticket(&app, "david", ha_push::Purpose::Approval(id), &buttons).await;
    let mut events = core.bus.subscribe();
    let (_, body) = app
        .request(
            "POST",
            "/api/push/action",
            None,
            Some(press(&ticket, "deny")),
        )
        .await;
    assert_eq!(body["result"], "refused");
    // Les téléphones apprennent la réponse, et qui l'a donnée.
    let answered = std::iter::from_fn(|| events.try_recv().ok())
        .find_map(|event| match event.as_ref() {
            ha_core::Event::Fired { event_type, data }
                if event_type == ha_core::guard::APPROVAL_ANSWERED =>
            {
                Some(data.clone())
            }
            _ => None,
        })
        .expect("la réponse est annoncée");
    assert_eq!(answered["id"], id);
    assert_eq!(answered["approved"], false);
    assert_eq!(answered["by"], "david");
    assert_eq!(body["message"], "Refusé : éteindre light.salon.");
    assert_eq!(core.held_orders().len(), 0);

    // Traité depuis la cloche : le bouton du téléphone ne sert plus.
    app.request(
        "POST",
        "/api/services/light/turn_on",
        Some(&guest),
        Some(lamp.clone()),
    )
    .await;
    let id = core.held_orders()[0].id;
    let ticket = button_ticket(&app, "alice", ha_push::Purpose::Approval(id), &buttons).await;
    let (status, _) = app
        .request(
            "POST",
            &format!("/api/approvals/{id}"),
            Some(&david),
            Some(json!({"approve": false})),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = app
        .request(
            "POST",
            "/api/push/action",
            None,
            Some(press(&ticket, "approve")),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Alice devient invitée : ses boutons ne valent plus rien.
    app.request(
        "POST",
        "/api/services/light/turn_on",
        Some(&guest),
        Some(lamp),
    )
    .await;
    let id = core.held_orders()[0].id;
    let ticket = button_ticket(&app, "alice", ha_push::Purpose::Approval(id), &buttons).await;
    app.state
        .auth
        .set_role("alice", ha_auth::Role::Guest)
        .await
        .unwrap();
    let (status, body) = app
        .request(
            "POST",
            "/api/push/action",
            None,
            Some(press(&ticket, "approve")),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(core.held_orders().len(), 1, "toujours en attente");
}

#[tokio::test]
async fn an_automation_button_fires_an_event() {
    let app = TestApp::new().await;
    ha_push::load_or_create_vapid(&app.state.store)
        .await
        .unwrap();
    let david = app.login().await;
    let (_, body) = app
        .request(
            "POST",
            "/api/push/subscriptions",
            Some(&david),
            Some(browser_subscription(
                "https://fcm.googleapis.com/fcm/send/d",
            )),
        )
        .await;
    let device = body["id"].as_i64().unwrap();
    let ticket = button_ticket(
        &app,
        "david",
        ha_push::Purpose::Automation,
        &["LESSIVE_VUE", "PLUS_TARD"],
    )
    .await;
    let mut events = app.state.core.bus.subscribe();
    let (status, body) = app
        .request(
            "POST",
            "/api/push/action",
            None,
            Some(json!({"token": ticket, "action": "LESSIVE_VUE"})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["result"], "fired");
    let event = events.recv().await.unwrap();
    let ha_core::Event::Fired { event_type, data } = event.as_ref() else {
        panic!("un événement est attendu");
    };
    assert_eq!(event_type, "notification_action");
    assert_eq!(data["action"], "LESSIVE_VUE");
    assert_eq!(data["user"], "david");

    // Un appareil retiré (téléphone perdu) : ses boutons ne valent plus rien.
    let ticket = button_ticket(&app, "david", ha_push::Purpose::Automation, &["A"]).await;
    let (status, _) = app
        .request(
            "DELETE",
            &format!("/api/push/subscriptions/{device}"),
            Some(&david),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = app
        .request(
            "POST",
            "/api/push/action",
            None,
            Some(json!({"token": ticket, "action": "A"})),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    // Un champ de trop : refusé.
    let (status, _) = app
        .request(
            "POST",
            "/api/push/action",
            None,
            Some(json!({"token": "x", "action": "A", "user": "david"})),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
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

#[tokio::test]
async fn a_wall_tablet_logs_in_once_and_the_administrator_can_disconnect_it() {
    let app = TestApp::new().await;
    let admin = app.login().await;
    app.state
        .auth
        .create_user_as("tablette", PASSWORD, ha_auth::Role::Family)
        .await
        .unwrap();

    let wall_login = |username: &'static str, label: &'static str| json!({"username": username, "password": PASSWORD, "wall_label": label});
    // Un compte administrateur ne reste pas connecté au mur.
    let (status, body) = app
        .request(
            "POST",
            "/api/auth/login",
            None,
            Some(wall_login("david", "Couloir")),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(
        body["error"].as_str().unwrap().contains("famille"),
        "{body}"
    );
    // Un nom vide est refusé.
    let (status, _) = app
        .request(
            "POST",
            "/api/auth/login",
            None,
            Some(wall_login("tablette", " ")),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let (status, body) = app
        .request(
            "POST",
            "/api/auth/login",
            None,
            Some(wall_login("tablette", "Couloir")),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["wall"], true);
    let tablet = body["access_token"].as_str().unwrap().to_owned();

    // La tablette sait qu'elle est une tablette murale ; une session ordinaire non.
    let (_, root) = app.request("GET", "/api/", Some(&tablet), None).await;
    assert_eq!(root["wall_label"], "Couloir");
    assert_eq!(root["permissions"]["tablets"], false);
    let (_, root) = app.request("GET", "/api/", Some(&admin), None).await;
    assert_eq!(root["wall_label"], Value::Null);
    assert_eq!(root["permissions"]["tablets"], true);

    // Seul l'administrateur voit et déconnecte les tablettes.
    let (status, _) = app
        .request("GET", "/api/auth/tablets", Some(&tablet), None)
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, list) = app
        .request("GET", "/api/auth/tablets", Some(&admin), None)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list.as_array().unwrap().len(), 1);
    assert_eq!(list[0]["label"], "Couloir");
    assert_eq!(list[0]["user"], "tablette");
    let id = list[0]["id"].as_i64().unwrap();
    let (status, _) = app
        .request(
            "DELETE",
            &format!("/api/auth/tablets/{id}"),
            Some(&tablet),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = app
        .request(
            "DELETE",
            &format!("/api/auth/tablets/{id}"),
            Some(&admin),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = app.request("GET", "/api/", Some(&tablet), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = app
        .request(
            "DELETE",
            &format!("/api/auth/tablets/{id}"),
            Some(&admin),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Le journal d'audit garde la connexion et la déconnexion, avec le nom.
    let (_, audit) = app.request("GET", "/api/audit", Some(&admin), None).await;
    let actions: Vec<(String, String)> = audit["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| {
            (
                e["action"].as_str().unwrap_or_default().to_owned(),
                e["target"].as_str().unwrap_or_default().to_owned(),
            )
        })
        .collect();
    for expected in ["login_wall", "tablet_forget"] {
        assert!(
            actions.contains(&(expected.to_owned(), "Couloir".to_owned())),
            "{expected} : {actions:?}"
        );
    }
}

/// Envoie une requête HTTP/1.1 brute et renvoie toute la réponse.
async fn raw_request<S>(mut stream: S, request: &str) -> Vec<u8>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut response = Vec::new();
    let _ = stream.read_to_end(&mut response).await;
    response
}

#[tokio::test]
async fn the_dashboard_is_served_in_https_and_plain_http_is_redirected() {
    let app = TestApp::new().await;
    let token = app.login().await;
    let ca = ha_tls::local_ca::LocalCa::generate().unwrap();
    let names = [ha_tls::local_ca::LocalName::parse("127.0.0.1").unwrap()];
    let cert = ha_tls::local_ca::ServerCert::new(ca.issue_server(&names).unwrap());
    let https = ha_api::Https {
        config: ha_tls::local_ca::server_config(cert).unwrap(),
        ca_der: Arc::from(ca.der()),
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(ha_api::serve_https(listener, Arc::clone(&app.state), https));

    // Un client qui ne fait confiance qu'à l'autorité de la maison.
    let mut roots = rustls::RootCertStore::empty();
    roots
        .add(rustls::pki_types::CertificateDer::from(ca.der().to_vec()))
        .unwrap();
    let config = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_root_certificates(roots)
    .with_no_client_auth();
    let connector = tokio_rustls::TlsConnector::from(Arc::new(config));
    let tcp = tokio::net::TcpStream::connect(addr).await.unwrap();
    let server = rustls::pki_types::ServerName::IpAddress(addr.ip().into());
    let tls = connector.connect(server, tcp).await.unwrap();
    let response = raw_request(
        tls,
        &format!(
            "GET /api/ HTTP/1.1\r\nHost: {addr}\r\nAuthorization: Bearer {token}\r\n\
             Connection: close\r\n\r\n"
        ),
    )
    .await;
    let text = String::from_utf8_lossy(&response);
    assert!(text.starts_with("HTTP/1.1 200 "), "{text}");
    assert!(text.contains("API en marche"), "{text}");
    // Les en-têtes de sécurité sont là aussi en HTTPS.
    assert!(
        text.to_ascii_lowercase()
            .contains("content-security-policy")
    );

    // En clair : redirigé vers la même adresse en https://.
    let tcp = tokio::net::TcpStream::connect(addr).await.unwrap();
    let response = raw_request(
        tcp,
        &format!("GET /api/states HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n"),
    )
    .await;
    let text = String::from_utf8_lossy(&response);
    assert!(text.starts_with("HTTP/1.1 301 "), "{text}");
    assert!(
        text.contains(&format!("\r\nLocation: https://{addr}/api/states\r\n")),
        "{text}"
    );

    // Sauf le certificat de l'autorité, à installer sur les appareils.
    let tcp = tokio::net::TcpStream::connect(addr).await.unwrap();
    let response = raw_request(
        tcp,
        &format!("GET /maison-ca.crt HTTP/1.1\r\nHost: {addr}\r\n\r\n"),
    )
    .await;
    assert!(response.starts_with(b"HTTP/1.1 200 OK\r\n"));
    assert!(response.ends_with(ca.der()));

    // Une autre autorité (un appareil qui ne connaît pas la maison) : refusé.
    let stranger = ha_tls::local_ca::LocalCa::generate().unwrap();
    let mut roots = rustls::RootCertStore::empty();
    roots
        .add(rustls::pki_types::CertificateDer::from(
            stranger.der().to_vec(),
        ))
        .unwrap();
    let config = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_root_certificates(roots)
    .with_no_client_auth();
    let tcp = tokio::net::TcpStream::connect(addr).await.unwrap();
    let refused = tokio_rustls::TlsConnector::from(Arc::new(config))
        .connect(
            rustls::pki_types::ServerName::IpAddress(addr.ip().into()),
            tcp,
        )
        .await;
    assert!(refused.is_err());
}

/// La photo de la sonnette (étape 82) : servie à la famille, jamais gardée par
/// le navigateur ; un invité ne la voit pas.
#[tokio::test]
async fn the_doorbell_snapshot_is_served_to_the_family_only() {
    let app = TestApp::new().await;
    let camera: ha_core::EntityId = "camera.porte".parse().unwrap();
    app.state
        .core
        .states
        .set(camera.clone(), "idle", Attributes::new())
        .unwrap();
    let family = app.login_as("famille", ha_auth::Role::Family).await;
    // Pas encore de photo.
    let (status, _, _) = app
        .send_bytes("GET", "/api/camera_proxy/camera.porte", &family, Vec::new())
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let jpeg = vec![0xFF, 0xD8, 0xFF, 0xE0, 1, 2, 3];
    app.state.core.images.set(camera, &jpeg).unwrap();
    let request = Request::builder()
        .uri("/api/camera_proxy/camera.porte")
        .header(header::AUTHORIZATION, format!("Bearer {family}"))
        .body(Body::empty())
        .unwrap();
    let response = app.router().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CONTENT_TYPE], "image/jpeg");
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    let body = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .unwrap();
    assert_eq!(body.as_ref(), jpeg.as_slice());

    // Un invité : comme si elle n'existait pas.
    let guest = app.login_as("visiteur", ha_auth::Role::Guest).await;
    let (status, _, _) = app
        .send_bytes("GET", "/api/camera_proxy/camera.porte", &guest, Vec::new())
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (_, listed) = app.request("GET", "/api/states", Some(&guest), None).await;
    assert!(!listed.to_string().contains("camera.porte"));
    // Une autre entité que la caméra n'a pas d'image.
    let (status, _, _) = app
        .send_bytes("GET", "/api/camera_proxy/light.salon", &family, Vec::new())
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// Envoie une notification au relais (étape 103) et renvoie (statut, corps JSON).
async fn relay(
    app: &TestApp,
    key: Option<&str>,
    content_type: &str,
    body: &str,
) -> (StatusCode, Value) {
    let mut request = Request::builder()
        .method("POST")
        .uri("/api/relais")
        .header(header::CONTENT_TYPE, content_type);
    if let Some(key) = key {
        request = request.header(header::AUTHORIZATION, format!("Bearer {key}"));
    }
    let response = app
        .router()
        .oneshot(request.body(Body::from(body.to_owned())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

const FORM: &str = "application/x-www-form-urlencoded";

#[tokio::test]
async fn the_relay_is_closed_without_a_key() {
    let app = TestApp::new().await;
    let (status, _) = relay(&app, Some(&"k".repeat(40)), FORM, "texte=Bonjour").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn phone_notifications_update_the_mower() {
    let app = TestApp::new().await;
    let key = "cle-du-relais-pour-les-tests-0123456789";
    assert!(app.state.set_relay_key("trop-courte").is_err());
    app.state.set_relay_key(&format!("{key}\n")).unwrap();
    let mut events = app.state.core.bus.subscribe();

    // Sans clé, ou avec la mauvaise : refusé, et rien ne change.
    let (status, _) = relay(&app, None, FORM, "app=Mammotion&texte=Tonte").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let wrong = "x".repeat(40);
    let (status, _) = relay(&app, Some(&wrong), FORM, "app=Mammotion&texte=Tonte").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let mower: ha_core::EntityId = "sensor.tondeuse".parse().unwrap();
    assert!(app.state.core.states.get(&mower).is_none());

    // Le formulaire de MacroDroid.
    let (status, body) = relay(
        &app,
        Some(key),
        FORM,
        "app=Mammotion&titre=Luba&texte=Votre+tondeuse+est+bloqu%C3%A9e",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["etat"], "bloquée");
    let state = app.state.core.states.get(&mower).unwrap();
    assert_eq!(state.state(), "bloquée");
    assert_eq!(
        state.attributes()["dernier_message"],
        "Votre tondeuse est bloquée"
    );
    assert_eq!(state.attributes()["friendly_name"], "Tondeuse");

    // En JSON. Un message inconnu garde l'état, mais le dernier message change.
    let (status, _) = relay(
        &app,
        Some(key),
        "application/json",
        r#"{"app": "Mammotion", "titre": "Luba", "texte": "Nouvelle carte disponible"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let state = app.state.core.states.get(&mower).unwrap();
    assert_eq!(state.state(), "bloquée");
    assert_eq!(
        state.attributes()["dernier_message"],
        "Nouvelle carte disponible"
    );

    // Une autre appli : l'événement seulement, la tondeuse ne bouge pas.
    let (status, body) = relay(
        &app,
        Some(key),
        "application/json",
        r#"{"app": "Météo", "texte": "Tonte terminée chez le voisin"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["tondeuse"], Value::Null);
    assert_eq!(
        app.state.core.states.get(&mower).unwrap().state(),
        "bloquée"
    );

    // Une notification vide est refusée.
    let (status, _) = relay(&app, Some(key), FORM, "app=Mammotion").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let mut fired = Vec::new();
    while let Ok(event) = events.try_recv() {
        if let ha_core::Event::Fired { event_type, data } = &*event
            && event_type == "notification_telephone"
        {
            fired.push(data["app"].as_str().unwrap().to_owned());
        }
    }
    assert_eq!(fired, ["Mammotion", "Mammotion", "Météo"]);
}
