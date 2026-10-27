//! Le relais des notifications du téléphone (tondeuse).

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use serde_json::Value;
use tower::ServiceExt;

use crate::common::TestApp;

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
