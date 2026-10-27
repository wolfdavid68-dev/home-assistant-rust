//! Assist : les phrases en français et les conversations.

use std::sync::Arc;
use std::time::Duration;

use axum::http::StatusCode;
use ha_core::Attributes;
use serde_json::{Value, json};

use crate::common::{TestApp, furnish, next_order, spawn_server, two_rooms, ws_recv, ws_send};

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
