//! L'éditeur d'automatisations et les automatisations toutes prêtes.

use axum::http::StatusCode;
use ha_core::Attributes;
use serde_json::{Value, json};

use crate::common::{PASSWORD, TestApp, furnish};

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
