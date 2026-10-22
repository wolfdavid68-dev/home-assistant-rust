//! SmartThings de bout en bout, contre un faux SmartThings sur cette
//! machine : l'association, le renouvellement des jetons, les appareils,
//! leur état et les ordres.

#![allow(clippy::unwrap_used, reason = "tests")]

use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use ha_core::{Attributes, Core};
use ha_smartthings::api::{App, Endpoint};
use ha_smartthings::{SmartThings, fresh_tokens};
use ha_storage::{SmartThingsAccount, SmartThingsTokens, Store};
use serde_json::{Value, json};
use zeroize::Zeroizing;

const LAMP: &str = "11111111-1111-1111-1111-111111111111";
const DOOR: &str = "22222222-2222-2222-2222-222222222222";
const LOCATION: &str = "33333333-3333-3333-3333-333333333333";
const ROOM: &str = "44444444-4444-4444-4444-444444444444";
/// `client-1:secret-1` en Base64.
const BASIC: &str = "Basic Y2xpZW50LTE6c2VjcmV0LTE=";

#[derive(Default)]
struct Fake {
    /// Le jeton de renouvellement valable en ce moment.
    refresh: String,
    /// Combien de jetons d'accès ont été donnés.
    issued: u32,
    lamp_on: bool,
    commands: Vec<Value>,
}

type Shared = Arc<Mutex<Fake>>;

fn authorized(fake: &Shared, headers: &HeaderMap) -> bool {
    let issued = fake.lock().unwrap().issued;
    headers.get("authorization").and_then(|v| v.to_str().ok())
        == Some(&format!("Bearer acces-{issued}"))
}

async fn token(State(fake): State<Shared>, headers: HeaderMap, body: String) -> impl IntoResponse {
    if headers.get("authorization").and_then(|v| v.to_str().ok()) != Some(BASIC) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "invalid_client"})),
        );
    }
    let form: Vec<(String, String)> = url::form_urlencoded::parse(body.as_bytes())
        .into_owned()
        .collect();
    let get = |key: &str| form.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone());
    let mut fake = fake.lock().unwrap();
    let ok = match get("grant_type").as_deref() {
        Some("authorization_code") => {
            get("code").as_deref() == Some("le-code")
                && get("redirect_uri").as_deref() == Some("https://192.168.1.10:8123/smartthings")
        }
        // À usage unique : seul le dernier donné marche.
        Some("refresh_token") => get("refresh_token") == Some(fake.refresh.clone()),
        _ => false,
    };
    if !ok {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "invalid_grant"})),
        );
    }
    fake.issued += 1;
    fake.refresh = format!("renouvellement-{}", fake.issued);
    (
        StatusCode::OK,
        Json(json!({
            "access_token": format!("acces-{}", fake.issued),
            "refresh_token": fake.refresh,
            "token_type": "bearer",
            "expires_in": 86_399,
        })),
    )
}

async fn devices(State(fake): State<Shared>, headers: HeaderMap) -> impl IntoResponse {
    if !authorized(&fake, &headers) {
        return (StatusCode::UNAUTHORIZED, Json(Value::Null));
    }
    (
        StatusCode::OK,
        Json(json!({"items": [
            {"deviceId": LAMP, "label": "Lampe salon", "locationId": LOCATION, "roomId": ROOM,
             "components": [{"id": "main", "capabilities": [{"id": "switch"}, {"id": "switchLevel"}],
                             "categories": [{"name": "Light"}]}]},
            {"deviceId": DOOR, "label": "Porte entrée", "locationId": LOCATION,
             "components": [{"id": "main", "capabilities": [{"id": "contactSensor"}, {"id": "battery"}],
                             "categories": [{"name": "ContactSensor"}]}]},
        ]})),
    )
}

async fn status(
    State(fake): State<Shared>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> impl IntoResponse {
    if !authorized(&fake, &headers) {
        return (StatusCode::UNAUTHORIZED, Json(Value::Null));
    }
    let on = fake.lock().unwrap().lamp_on;
    let body = if id == LAMP {
        json!({"switch": {"switch": {"value": if on { "on" } else { "off" }}},
               "switchLevel": {"level": {"value": 100}}})
    } else {
        json!({"contactSensor": {"contact": {"value": "closed"}}, "battery": {"battery": {"value": 91}}})
    };
    (StatusCode::OK, Json(body))
}

async fn rooms() -> Json<Value> {
    Json(json!({"items": [{"roomId": ROOM, "name": "Salon"}]}))
}

async fn command(
    State(fake): State<Shared>,
    headers: HeaderMap,
    Path(id): Path<String>,
    body: String,
) -> impl IntoResponse {
    if !authorized(&fake, &headers) {
        return StatusCode::UNAUTHORIZED;
    }
    let body: Value = serde_json::from_str(&body).unwrap();
    let mut fake = fake.lock().unwrap();
    if id == LAMP && body["commands"][0]["command"] == "on" {
        fake.lamp_on = true;
    }
    fake.commands.push(json!({"device": id, "body": body}));
    StatusCode::OK
}

async fn fake_smartthings() -> (Endpoint, Shared) {
    let fake = Shared::default();
    let app = Router::new()
        .route("/v1/oauth/token", post(token))
        .route("/v1/devices", get(devices))
        .route("/v1/devices/{id}/components/main/status", get(status))
        .route("/v1/devices/{id}/commands", post(command))
        .route("/v1/locations/{id}/rooms", get(rooms))
        .with_state(Arc::clone(&fake));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (Endpoint::local(&format!("http://{address}")).unwrap(), fake)
}

fn app() -> App {
    App {
        client_id: "client-1".into(),
        client_secret: Zeroizing::new("secret-1".into()),
        redirect_uri: "https://192.168.1.10:8123/smartthings".into(),
    }
}

#[tokio::test]
async fn pairing_then_refresh_tokens_are_single_use() {
    let (endpoint, _fake) = fake_smartthings().await;
    let http = endpoint.http().unwrap();
    let first = app().exchange(&http, &endpoint, "le-code").await.unwrap();
    assert_eq!(first.access_token.as_str(), "acces-1");
    assert!(first.expires_at > SystemTime::now() + Duration::from_hours(23));
    assert!(app().exchange(&http, &endpoint, "mauvais").await.is_err());

    let second = app()
        .refresh(&http, &endpoint, &first.refresh_token)
        .await
        .unwrap();
    assert_eq!(second.access_token.as_str(), "acces-2");
    // L'ancien jeton de renouvellement ne marche plus.
    let error = app()
        .refresh(&http, &endpoint, &first.refresh_token)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        ha_smartthings::SmartThingsError::Unauthorized
    ));
    // Un mauvais secret non plus.
    let mut wrong = app();
    wrong.client_secret = Zeroizing::new("autre".into());
    assert!(
        wrong
            .refresh(&http, &endpoint, &second.refresh_token)
            .await
            .is_err()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn devices_states_and_orders() {
    let (endpoint, fake) = fake_smartthings().await;
    let http = endpoint.http().unwrap();
    let first = app().exchange(&http, &endpoint, "le-code").await.unwrap();
    let store = Store::open_in_memory().await.unwrap();
    // Un jeton qui expire dans une heure : il est renouvelé d'abord.
    store
        .save_smartthings_account(&SmartThingsAccount {
            client_id: "client-1".into(),
            client_secret: Zeroizing::new("secret-1".into()),
            redirect_uri: "https://192.168.1.10:8123/smartthings".into(),
            tokens: SmartThingsTokens {
                expires_at: SystemTime::now() + Duration::from_hours(1),
                ..first
            },
        })
        .await
        .unwrap();
    let tokens = fresh_tokens(&store, &endpoint, &http).await.unwrap();
    assert_eq!(tokens.access_token.as_str(), "acces-2");
    let saved = store.smartthings_account().await.unwrap().unwrap();
    assert_eq!(
        saved.tokens.refresh_token.as_str(),
        "renouvellement-2",
        "enregistré aussitôt"
    );
    // Encore valable longtemps : pas de renouvellement.
    let again = fresh_tokens(&store, &endpoint, &http).await.unwrap();
    assert_eq!(again.access_token.as_str(), "acces-2");

    let core = Arc::new(Core::new());
    let smartthings = SmartThings::start(store.clone(), endpoint, Arc::clone(&core));
    let wait_for = |entity: &'static str, wanted: &'static str| {
        let core = Arc::clone(&core);
        async move {
            for _ in 0..100 {
                if core
                    .states
                    .get(&entity.parse().unwrap())
                    .is_some_and(|s| s.state() == wanted)
                {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            panic!("{entity} n'est jamais passé à {wanted}");
        }
    };
    wait_for("light.lampe_salon", "off").await;
    wait_for("binary_sensor.porte_entree", "off").await;
    wait_for("sensor.porte_entree_batterie", "91").await;
    let lamp = core
        .states
        .get(&"light.lampe_salon".parse().unwrap())
        .unwrap();
    assert_eq!(lamp.attributes()["piece"], "Salon");

    // Un ordre : la commande part, puis l'état est relu.
    let mut data = Attributes::new();
    data.insert("brightness_pct".into(), json!(60));
    core.call_service(
        "light",
        "turn_on",
        "light.lampe_salon".parse().unwrap(),
        data,
    )
    .unwrap();
    wait_for("light.lampe_salon", "on").await;
    let commands = fake.lock().unwrap().commands.clone();
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0]["device"], LAMP);
    assert_eq!(
        commands[0]["body"]["commands"],
        json!([
            {"component": "main", "capability": "switch", "command": "on", "arguments": []},
            {"component": "main", "capability": "switchLevel", "command": "setLevel", "arguments": [60.0]}
        ])
    );
    // Un capteur ne se commande pas : rien ne part.
    core.call_service(
        "binary_sensor",
        "turn_on",
        "binary_sensor.porte_entree".parse().unwrap(),
        Attributes::new(),
    )
    .unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(fake.lock().unwrap().commands.len(), 1);
    smartthings.stop().await;
}
