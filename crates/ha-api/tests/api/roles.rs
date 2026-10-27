//! Les rôles : ce qu'un invité ou la famille voit et commande.

use axum::http::StatusCode;
use ha_core::Attributes;
use serde_json::{Value, json};

use crate::common::{TestApp, furnish, spawn_server, ws_recv, ws_send};

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
