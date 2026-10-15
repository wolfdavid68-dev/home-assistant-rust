//! Tests du faux pont Hue, en jouant le rôle d'un Echo : inscription,
//! liste des lampes, ordres, refus.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "dans un test, un échec doit simplement faire échouer le test"
)]

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::extract::connect_info::MockConnectInfo;
use axum::http::{Request, StatusCode};
use ha_alexa::{Alexa, Allowed, Bridge, Identity, Settings, numbers, parse, router};
use ha_core::{Core, Event};
use ha_storage::Store;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::broadcast::Receiver;
use tower::ServiceExt;

const DEVICES: &str = "
- entity: switch.prise_cafe
  name: Cafetière
- entity: cover.volet_salon
  name: Volet du salon
- light.salon
- cover.garage
";

struct Echo {
    core: Arc<Core>,
    store: Store,
    app: axum::Router,
}

impl Echo {
    async fn new(from: [u8; 4]) -> Self {
        let store = Store::open_in_memory().await.unwrap();
        let core = Arc::new(Core::new());
        let set = |id: &str, state: &str, attributes: Value| {
            let Value::Object(map) = attributes else {
                panic!()
            };
            core.states.set(id.parse().unwrap(), state, map).unwrap();
        };
        set(
            "switch.prise_cafe",
            "off",
            json!({"friendly_name": "prise cafe"}),
        );
        set(
            "cover.volet_salon",
            "open",
            json!({"current_position": 100}),
        );
        set(
            "light.salon",
            "on",
            json!({"brightness": 255, "friendly_name": "Lampe du salon"}),
        );
        set("cover.garage", "closed", json!({"device_class": "garage"}));
        let lights = numbers::assign(&store, &parse(DEVICES).unwrap())
            .await
            .unwrap();
        let bridge = Bridge::new(
            Arc::clone(&core),
            store.clone(),
            lights,
            Allowed::LocalNetwork,
            Identity::new(Ipv4Addr::new(192, 168, 1, 20), 80),
        );
        let app = router(Arc::new(bridge)).layer(MockConnectInfo(SocketAddr::from((from, 50_000))));
        Self { core, store, app }
    }

    async fn call(&self, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, String) {
        let body = body.map_or_else(Body::empty, |b| Body::from(b.to_string()));
        let request = Request::builder()
            .method(method)
            .uri(uri)
            .body(body)
            .unwrap();
        let response = self.app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
            .await
            .unwrap();
        (status, String::from_utf8(bytes.to_vec()).unwrap())
    }

    async fn json(&self, method: &str, uri: &str, body: Option<Value>) -> Value {
        let (status, text) = self.call(method, uri, body).await;
        assert_eq!(status, StatusCode::OK, "{text}");
        serde_json::from_str(&text).unwrap()
    }
}

async fn next_call(rx: &mut Receiver<Arc<Event>>) -> String {
    loop {
        let event = tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("aucun appel de service")
            .unwrap();
        if let Event::CallService {
            domain,
            service,
            entity_id,
            data,
            origin,
        } = event.as_ref()
        {
            assert_eq!(*origin, ha_core::Origin::Voice);
            return format!(
                "{domain}.{service} {entity_id} {}",
                Value::Object(data.clone())
            );
        }
    }
}

#[tokio::test]
async fn an_echo_discovers_and_controls_the_devices() {
    let echo = Echo::new([192, 168, 1, 40]).await;
    let mut rx = echo.core.bus.subscribe();

    // La fiche UPnP.
    let (status, xml) = echo.call("GET", "/description.xml", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        xml.contains("<URLBase>http://192.168.1.20:80/</URLBase>"),
        "{xml}"
    );
    assert!(xml.contains("<modelName>Philips hue bridge 2015</modelName>"));

    // Inscription, puis liste des lampes.
    let user = echo
        .json("POST", "/api", Some(json!({"devicetype": "Echo"})))
        .await;
    let user = user[0]["success"]["username"].as_str().unwrap().to_owned();
    let lights = echo.json("GET", &format!("/api/{user}/lights"), None).await;
    let lights = lights.as_object().unwrap();
    let names: Vec<&str> = lights
        .values()
        .map(|l| l["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        ["Cafetière", "Volet du salon", "Lampe du salon"],
        "jamais le garage"
    );
    assert_eq!(lights["2"]["state"]["bri"], 254);
    assert_eq!(lights["1"]["type"], "On/Off light");

    // « Alexa, allume la cafetière. »
    let answer = echo
        .json(
            "PUT",
            &format!("/api/{user}/lights/1/state"),
            Some(json!({"on": true})),
        )
        .await;
    assert_eq!(answer, json!([{"success": {"/lights/1/state/on": true}}]));
    assert_eq!(
        next_call(&mut rx).await,
        "switch.turn_on switch.prise_cafe {}"
    );
    // La prise n'a pas encore confirmé, mais Alexa voit l'état promis.
    let light = echo
        .json("GET", &format!("/api/{user}/lights/1"), None)
        .await;
    assert_eq!(light["state"]["on"], true);

    // « Alexa, mets le volet du salon à 30 %. »
    echo.json(
        "PUT",
        &format!("/api/{user}/lights/2/state"),
        Some(json!({"on": true, "bri": 76})),
    )
    .await;
    assert_eq!(
        next_call(&mut rx).await,
        r#"cover.set_cover_position cover.volet_salon {"position":30}"#
    );
    // « Alexa, éteins la lampe du salon. »
    echo.json(
        "PUT",
        &format!("/api/{user}/lights/3/state"),
        Some(json!({"on": false})),
    )
    .await;
    assert_eq!(next_call(&mut rx).await, "light.turn_off light.salon {}");

    // Le garage n'est ni visible ni pilotable, même en connaissant son numéro.
    let refused = echo
        .json(
            "PUT",
            &format!("/api/{user}/lights/4/state"),
            Some(json!({"on": true})),
        )
        .await;
    assert_eq!(refused[0]["error"]["type"], 1);
    let missing = echo
        .json("GET", &format!("/api/{user}/lights/4"), None)
        .await;
    assert_eq!(missing[0]["error"]["type"], 3);
    // Lampe inconnue, ordre illisible : erreurs au format Hue.
    let unknown = echo
        .json(
            "PUT",
            &format!("/api/{user}/lights/99/state"),
            Some(json!({"on": true})),
        )
        .await;
    assert_eq!(unknown[0]["error"]["type"], 3);
    let bad = echo
        .json(
            "PUT",
            &format!("/api/{user}/lights/1/state"),
            Some(json!({"on": "oui"})),
        )
        .await;
    assert_eq!(bad[0]["error"]["type"], 2);
    assert!(rx.try_recv().is_err(), "aucun ordre envoyé pour les refus");

    // Chaque ordre est dans le journal d'audit.
    let journal = echo.store.audit_log(10).await.unwrap();
    let targets: Vec<&str> = journal.iter().filter_map(|e| e.target.as_deref()).collect();
    assert_eq!(
        targets,
        ["light.salon", "cover.volet_salon", "switch.prise_cafe"]
    );
    assert!(
        journal
            .iter()
            .all(|e| e.actor == "alexa" && e.source == "192.168.1.40")
    );
}

#[tokio::test]
async fn addresses_outside_the_local_network_are_refused() {
    let echo = Echo::new([8, 8, 8, 8]).await;
    for (method, uri) in [
        ("GET", "/description.xml"),
        ("POST", "/api"),
        ("GET", "/api/x/lights"),
        ("PUT", "/api/x/lights/1/state"),
    ] {
        let (status, _) = echo.call(method, uri, Some(json!({"on": true}))).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{method} {uri}");
    }
}

#[tokio::test]
async fn the_bridge_runs_on_a_real_socket() {
    let store = Store::open_in_memory().await.unwrap();
    let core = Arc::new(Core::new());
    let settings = Settings {
        ip: Ipv4Addr::LOCALHOST,
        port: 0,
        discovery_port: 0,
        allowed: Allowed::LocalNetwork,
        devices: parse("- light.salon").unwrap(),
    };
    let alexa = Alexa::start(settings, store, core).await.unwrap();
    let mut stream = tokio::net::TcpStream::connect(alexa.address).await.unwrap();
    stream
        .write_all(b"GET /api/x/lights HTTP/1.1\r\nHost: test\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).await.unwrap();
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    assert!(response.contains("\"uniqueid\""), "{response}");
    alexa.stop().await;
}

#[tokio::test]
async fn huge_bodies_are_refused() {
    let echo = Echo::new([192, 168, 1, 40]).await;
    let big = json!({"on": true, "pad": "x".repeat(10_000)});
    let (status, _) = echo.call("PUT", "/api/x/lights/1/state", Some(big)).await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
}
