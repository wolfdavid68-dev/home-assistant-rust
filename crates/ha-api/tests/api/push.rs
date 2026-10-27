//! Les notifications des téléphones et leurs boutons.

use axum::http::StatusCode;
use serde_json::{Value, json};

use crate::common::{TestApp, furnish};

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
