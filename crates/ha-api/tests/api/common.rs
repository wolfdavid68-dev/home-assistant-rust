//! Ce que les tests partagent : l'application de test (`TestApp`) et les petites aides.

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

pub(crate) const PASSWORD: &str = "cafe chaud sur la terrasse";

pub(crate) struct TestApp {
    pub(crate) state: Arc<ApiState>,
    pub(crate) recorder: Option<Recorder>,
}

impl TestApp {
    pub(crate) async fn new() -> Self {
        Self::with_editor(true).await
    }

    /// `editable` : l'éditeur d'automatisations accepte-t-il les modifications ?
    pub(crate) async fn with_editor(editable: bool) -> Self {
        Self::build(
            editable,
            "- {area: Cuisine, entities: [{entity: switch.prise_cafe, name: Cafetière}]}",
        )
        .await
    }

    /// Avec ces pièces (le contenu d'un `areas.yaml`).
    pub(crate) async fn with_areas(areas: &str) -> Self {
        Self::build(true, areas).await
    }

    pub(crate) async fn build(editable: bool, areas: &str) -> Self {
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

    pub(crate) fn router(&self) -> Router {
        ha_api::router(Arc::clone(&self.state))
            .layer(MockConnectInfo(SocketAddr::from(([127, 0, 0, 1], 50_000))))
    }

    /// Envoie une requête et renvoie (statut, corps JSON).
    pub(crate) async fn request(
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
    pub(crate) async fn send_bytes(
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
    pub(crate) async fn login_as(&self, username: &str, role: ha_auth::Role) -> String {
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

    pub(crate) async fn login(&self) -> String {
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

/// Démarre un vrai serveur sur un port libre de 127.0.0.1.
pub(crate) async fn spawn_server(app: &TestApp) -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(ha_api::serve(listener, Arc::clone(&app.state)));
    addr
}

pub(crate) type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

pub(crate) async fn ws_send(socket: &mut Socket, value: Value) {
    socket
        .send(Message::Text(value.to_string().into()))
        .await
        .unwrap();
}

pub(crate) async fn ws_recv(socket: &mut Socket) -> Value {
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

/// La maison des tests de rôles : une lumière, une porte de garage, un volet,
/// une personne, l'alarme.
pub(crate) fn furnish(app: &TestApp) {
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

/// Deux lampes rangées dans deux pièces.
pub(crate) fn two_rooms(app: &TestApp) {
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
pub(crate) async fn next_order(
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
