//! Les ordres envoyés aux appareils et le journal des actions.

use std::sync::Arc;

use axum::http::StatusCode;
use serde_json::json;

use crate::common::{TestApp, furnish};

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
