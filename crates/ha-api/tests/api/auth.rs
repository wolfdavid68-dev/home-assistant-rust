//! La connexion, les jetons et les mots de passe.

use axum::http::StatusCode;
use serde_json::{Value, json};

use crate::common::{PASSWORD, TestApp, furnish};

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
