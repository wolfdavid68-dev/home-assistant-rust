//! Les pages à lire : historique, pièces, santé, statistiques, énergie, dessin.

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use ha_core::Attributes;
use serde_json::{Value, json};
use tower::ServiceExt;

use crate::common::TestApp;

#[tokio::test]
async fn history_is_available_through_the_api() {
    let mut app = TestApp::new().await;
    let token = app.login().await;
    for value in ["on", "off"] {
        app.request(
            "POST",
            "/api/states/switch.prise",
            Some(&token),
            Some(json!({"state": value})),
        )
        .await;
    }
    app.recorder.take().unwrap().stop().await;

    let (status, body) = app
        .request(
            "GET",
            "/api/history/switch.prise?limit=10",
            Some(&token),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let values: Vec<&str> = body["history"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["state"].as_str().unwrap())
        .collect();
    assert_eq!(values, ["off", "on"]);
}

#[tokio::test]
async fn areas_are_served() {
    let app = TestApp::new().await;
    let token = app.login().await;
    let (status, body) = app.request("GET", "/api/areas", Some(&token), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body,
        json!({"areas": [{"name": "Cuisine",
                          "entities": [{"entity_id": "switch.prise_cafe", "name": "Cafetière"}]}]})
    );
}

#[tokio::test]
async fn integrations_report_their_health() {
    let app = TestApp::new().await;
    let token = app.login().await;
    let core = &app.state.core;
    core.report("hue", "Hue", ha_core::Health::Connected);
    core.states
        .set_by(
            "hue",
            "light.salon".parse().unwrap(),
            "on",
            ha_core::Attributes::new(),
        )
        .unwrap();
    core.report(
        "somfy",
        "Somfy",
        ha_core::Health::Retrying {
            error: "box injoignable".into(),
            retry_at: std::time::SystemTime::now(),
        },
    );
    let (status, body) = app
        .request("GET", "/api/integrations", Some(&token), None)
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let list = body["integrations"].as_array().unwrap();
    assert_eq!(list.len(), 2);
    assert_eq!(list[0]["id"], "hue");
    assert_eq!(list[0]["status"], "connected");
    assert_eq!(list[0]["entities"], 1);
    assert_eq!(list[1]["status"], "retrying");
    assert_eq!(list[1]["error"], "box injoignable");
    assert!(list[1]["retry_at"].is_string());

    // Un invité ne voit pas l'état des intégrations (les erreurs citent le réseau).
    let guest = app.login_as("invite", ha_auth::Role::Guest).await;
    let (status, _) = app
        .request("GET", "/api/integrations", Some(&guest), None)
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn the_system_page_measures_the_server() {
    let app = TestApp::new().await;
    let token = app.login().await;
    // Quelques requêtes à mesurer.
    for _ in 0..3 {
        app.request("GET", "/api/states", Some(&token), None).await;
    }
    let (status, body) = app.request("GET", "/api/system", Some(&token), None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body["api"]["samples"].as_u64().unwrap() >= 3, "{body}");
    assert!(body["api"]["p95_ms"].as_f64().unwrap() >= 0.0);
    assert!(body["database_bytes"].as_u64().unwrap() > 0);
    assert!(body["uptime_seconds"].is_u64());
    assert_eq!(body["budget"]["memory_mib"], 64);
    assert_eq!(body["devices"], json!([]), "aucun ordre encore");
    assert_eq!(body["slow_ms"], 3000);
    if cfg!(target_os = "linux") {
        assert!(body["memory"]["resident_bytes"].as_u64().unwrap() > 0);
        assert!(body["cpu_percent"].is_number());
    }

    // Un ordre à une lampe Hue, et sa réponse 150 ms plus tard : la page
    // donne la rapidité de Hue, sous son nom.
    let core = &app.state.core;
    core.report("hue", "Hue", ha_core::Health::Connected);
    let lamp: ha_core::EntityId = "light.salon".parse().unwrap();
    core.states
        .set_by("hue", lamp.clone(), "off", ha_core::Attributes::new())
        .unwrap();
    let sent = std::time::Instant::now();
    core.latencies.observe(
        &ha_core::Event::CallService {
            domain: "light".into(),
            service: "turn_on".into(),
            entity_id: lamp.clone(),
            data: ha_core::Attributes::new(),
            origin: ha_core::Origin::System,
            command: None,
            issued_at: sent,
        },
        &core.states,
        sent,
    );
    core.latencies.observe(
        &ha_core::Event::StateChanged {
            entity_id: lamp.clone(),
            old_state: None,
            new_state: core.states.get(&lamp),
        },
        &core.states,
        sent + std::time::Duration::from_millis(150),
    );
    let (_, body) = app.request("GET", "/api/system", Some(&token), None).await;
    assert_eq!(
        body["devices"],
        json!([{"integration": "hue", "name": "Hue", "p50_ms": 150.0, "p95_ms": 150.0, "samples": 1, "slow": 0}])
    );

    // Réservé à l'administrateur.
    let family = app.login_as("alice", ha_auth::Role::Family).await;
    let (status, _) = app.request("GET", "/api/system", Some(&family), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn history_series_for_a_chart() {
    let mut app = TestApp::new().await;
    let token = app.login().await;
    for value in ["20.5", "21.5", "unavailable", "22"] {
        let (status, _) = app
            .request(
                "POST",
                "/api/states/sensor.temperature",
                Some(&token),
                Some(json!({"state": value})),
            )
            .await;
        assert!(status.is_success());
    }
    // L'enregistreur écrit tout avant de s'arrêter.
    app.recorder.take().unwrap().stop().await;

    let (status, body) = app
        .request(
            "GET",
            "/api/history/sensor.temperature/series?hours=24",
            Some(&token),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    // Les mesures, très proches dans le temps, tombent dans la même tranche.
    let points = body["numeric"].as_array().unwrap();
    assert_eq!(points.len(), 1, "{body}");
    assert_eq!(points[0]["min"], 20.5);
    assert_eq!(points[0]["max"], 22.0);
    let changes: Vec<&str> = body["changes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["state"].as_str().unwrap())
        .collect();
    assert_eq!(changes, ["20.5", "21.5", "unavailable", "22"]);

    for bad in ["hours=0", "hours=10000", "hours=abc", "jours=2"] {
        let (status, _) = app
            .request(
                "GET",
                &format!("/api/history/sensor.temperature/series?{bad}"),
                Some(&token),
                None,
            )
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}");
    }
}

#[tokio::test]
async fn the_logbook_is_read_with_filters() {
    let app = TestApp::new().await;
    let token = app.login().await;
    let door: ha_core::EntityId = "binary_sensor.porte".parse().unwrap();
    for (entity_id, name) in [(Some(&door), "Porte"), (None, "Bonne nuit")] {
        app.state
            .store
            .add_logbook(ha_storage::NewLogbookEntry {
                entity_id,
                name,
                message: "message",
                cause: Some("par david"),
            })
            .await
            .unwrap();
    }

    let (status, body) = app.request("GET", "/api/logbook", Some(&token), None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let names: Vec<&str> = body["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["Bonne nuit", "Porte"]);
    assert_eq!(body["entries"][1]["entity_id"], "binary_sensor.porte");
    assert_eq!(body["entries"][1]["cause"], "par david");

    let (_, body) = app
        .request(
            "GET",
            "/api/logbook?hours=1&entity_id=binary_sensor.porte",
            Some(&token),
            None,
        )
        .await;
    assert_eq!(body["entries"].as_array().unwrap().len(), 1);

    for bad in [
        "/api/logbook?hours=0",
        "/api/logbook?hours=100000",
        "/api/logbook?entity_id=Porte",
        "/api/logbook?inconnu=1",
    ] {
        let (status, _) = app.request("GET", bad, Some(&token), None).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}");
    }
}

#[tokio::test]
async fn statistics_are_checked_and_protected() {
    let app = TestApp::new().await;
    let token = app.login().await;
    let (status, body) = app
        .request(
            "GET",
            "/api/statistics/sensor.temperature?period=day&days=365",
            Some(&token),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["points"], json!([]));
    for bad in [
        "/api/statistics/sensor.temperature?period=week",
        "/api/statistics/sensor.temperature?period=hour&days=60",
        "/api/statistics/sensor.temperature?days=0",
        "/api/statistics/sensor.temperature?days=99999",
        "/api/statistics/Sensor?days=1",
    ] {
        let (status, _) = app.request("GET", bad, Some(&token), None).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}");
    }
    let guest = app.login_as("visiteur", ha_auth::Role::Guest).await;
    let (status, _) = app
        .request(
            "GET",
            "/api/statistics/sensor.temperature",
            Some(&guest),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn the_energy_report_is_for_the_family() {
    let app = TestApp::new().await;
    let mut attributes = serde_json::Map::new();
    attributes.insert("friendly_name".into(), json!("Linky heures creuses"));
    attributes.insert("state_class".into(), json!("total_increasing"));
    app.state
        .core
        .states
        .set(
            "sensor.linky_heures_creuses".parse().unwrap(),
            "12.5",
            attributes,
        )
        .unwrap();

    let token = app.login().await;
    let (status, body) = app
        .request("GET", "/api/energy?days=7", Some(&token), None)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["configured"], false);
    assert_eq!(body["sources"][0]["name"], "Linky heures creuses");
    assert_eq!(body["sources"][0]["kind"], "consumption");
    assert_eq!(body["today"]["consumed_kwh"], 0.0);
    assert_eq!(body["days"], json!([]));

    let (status, _) = app
        .request("GET", "/api/energy?days=0", Some(&token), None)
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = app
        .request("GET", "/api/energy?jours=3", Some(&token), None)
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let guest = app.login_as("visiteur", ha_auth::Role::Guest).await;
    let (status, _) = app.request("GET", "/api/energy", Some(&guest), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

/// Une image PNG (seuls les premiers octets comptent pour le serveur).
fn png(content: &[u8]) -> Vec<u8> {
    let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
    bytes.extend_from_slice(content);
    bytes
}

#[tokio::test]
async fn the_house_drawing_is_assembled_from_checked_images() {
    let app = TestApp::new().await;
    let token = app.login().await;

    // Pas encore de dessin : un assemblage vide, que l'administrateur peut régler.
    let (status, body) = app
        .request("GET", "/api/home/drawing", Some(&token), None)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["config"],
        json!({"pages": [{"id": "maison", "name": "Maison", "areas": [], "layers": [], "labels": [], "actions": [], "charts": [], "zones": []}]})
    );
    assert_eq!(body["editable"], true);

    // Des images : PNG, JPEG ou WebP, reconnus à leur contenu.
    for name in ["jour", "garage_ouvert"] {
        let (status, _, _) = app
            .send_bytes(
                "PUT",
                &format!("/api/home/images/{name}"),
                &token,
                png(name.as_bytes()),
            )
            .await;
        assert_eq!(status, StatusCode::NO_CONTENT, "{name}");
    }
    let (status, bytes, content_type) = app
        .send_bytes("GET", "/api/home/images/jour", &token, Vec::new())
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(bytes, png(b"jour"));
    assert_eq!(content_type.as_deref(), Some("image/png"));

    // Un SVG (qui peut contenir du script), un HTML, un mauvais nom, une image trop grosse.
    for (uri, body) in [
        ("/api/home/images/piege", b"<svg onload=alert(1)>".to_vec()),
        (
            "/api/home/images/page",
            b"<html><script>alert(1)</script>".to_vec(),
        ),
        ("/api/home/images/..%2Fha.key", png(b"x")),
        ("/api/home/images/Majuscule", png(b"x")),
        ("/api/home/images/enorme", png(&vec![0; 3 * 1024 * 1024])),
    ] {
        let (status, _, _) = app.send_bytes("PUT", uri, &token, body).await;
        assert!(
            status == StatusCode::BAD_REQUEST || status == StatusCode::PAYLOAD_TOO_LARGE,
            "{uri} : {status}"
        );
    }

    // Deux onglets : la maison (un fond, des calques, des valeurs), et la
    // cuisine seule.
    let home = json!({
        "id": "maison",
        "name": "Maison",
        "background": "jour",
        "ambiances": {"jour-pluie": "garage_ouvert"},
        "forced": "jour-pluie",
        "effects": "neige",
        "effects_level": "discret",
        "layers": [
            {"image": "garage_ouvert", "entity_id": "cover.garage", "states": ["open", "opening"]},
            {"image": "jour", "entity_id": "person.alice", "states": ["home"], "when": "night"},
            {"image": "garage_ouvert", "area": "Cuisine", "when": "night"},
        ],
        "labels": [
            {"entity_id": "sensor.dehors", "x": 90.04, "y": 8, "name": "Dehors"},
            {"entity_id": "person.alice", "x": 10, "y": 10},
        ],
        "actions": ["light.porche", "person.alice"],
        "charts": [
            {"entity_id": "sensor.nas", "name": "NAS", "color": "violet", "hours": 24},
            {"entity_id": "person.alice", "color": "vert", "hours": 6},
        ],
        "zones": [
            {"entity_id": "light.porche", "x": 40.04, "y": 55, "w": 8, "h": 10, "name": "Porche"},
            {"entity_id": "person.alice", "x": 0, "y": 0, "w": 5, "h": 5},
            {"area": "Cuisine", "x": 50, "y": 50, "w": 10, "h": 10},
        ],
    });
    let kitchen = json!({"id": "cuisine", "name": "Cuisine", "areas": ["Cuisine"]});
    let (status, body) = app
        .request(
            "PUT",
            "/api/home/drawing",
            Some(&token),
            Some(json!({"pages": [home, kitchen]})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["config"]["pages"][0]["labels"][0]["x"], 90.0);
    assert_eq!(body["config"]["pages"][0]["layers"][0]["when"], "always");
    assert_eq!(body["config"]["pages"][1]["areas"], json!(["Cuisine"]));
    assert_eq!(
        body["config"]["pages"][0]["ambiances"],
        json!({"jour-pluie": "garage_ouvert"})
    );
    assert_eq!(body["config"]["pages"][0]["forced"], "jour-pluie");
    assert_eq!(body["config"]["pages"][0]["effects"], "neige");
    assert_eq!(body["config"]["pages"][0]["effects_level"], "discret");
    assert_eq!(body["config"]["pages"][0]["zones"][0]["x"], 40.0);
    let page = |extra: Value| {
        let mut page = json!({"id": "maison", "name": "Maison"});
        page.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        json!({"pages": [page]})
    };
    for bad in [
        page(json!({"background": "absente"})),
        page(json!({"layers": [{"image": "jour", "entity_id": "cover.garage", "states": []}]})),
        page(json!({"labels": [{"entity_id": "sensor.a", "x": -1, "y": 0}]})),
        page(json!({"areas": ["Grenier"]})),
        page(json!({"zones": [{"entity_id": "light.porche", "x": 95, "y": 0, "w": 10, "h": 10}]})),
        page(json!({"zones": [{"area": "Grenier", "x": 0, "y": 0, "w": 10, "h": 10}]})),
        page(json!({"layers": [{"image": "jour", "area": "Grenier"}]})),
        page(json!({"effects": "tornade"})),
        page(json!({"background": "jour", "ambiances": {"jour-tempete": "jour"}})),
        json!({"pages": []}),
        json!({"script": "alert(1)"}),
    ] {
        let (status, body) = app
            .request("PUT", "/api/home/drawing", Some(&token), Some(bad.clone()))
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad} : {body}");
    }

    // Une image qui sert encore ne se retire pas.
    let (status, _, _) = app
        .send_bytes(
            "DELETE",
            "/api/home/images/garage_ouvert",
            &token,
            Vec::new(),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // La famille voit le dessin, mais ne le change pas.
    let family = app.login_as("alice", ha_auth::Role::Family).await;
    let (status, body) = app
        .request("GET", "/api/home/drawing", Some(&family), None)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["editable"], false);
    assert_eq!(body["config"]["pages"][0]["background"], "jour");
    let (status, _) = app
        .request("PUT", "/api/home/drawing", Some(&family), Some(json!({})))
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _, _) = app
        .send_bytes("PUT", "/api/home/images/autre", &family, png(b"x"))
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Un invité ne voit ni les calques ni les valeurs des appareils qui lui
    // sont cachés (où est Alice).
    let guest = app.login_as("invite", ha_auth::Role::Guest).await;
    let (_, body) = app
        .request("GET", "/api/home/drawing", Some(&guest), None)
        .await;
    let home = &body["config"]["pages"][0];
    let layers = home["layers"].as_array().unwrap();
    assert_eq!(layers.len(), 2);
    assert_eq!(layers[0]["entity_id"], "cover.garage");
    assert_eq!(layers[1]["area"], "Cuisine");
    assert_eq!(home["labels"].as_array().unwrap().len(), 1);
    assert_eq!(home["labels"][0]["entity_id"], "sensor.dehors");
    assert_eq!(home["actions"], json!(["light.porche"]));
    assert_eq!(home["charts"].as_array().unwrap().len(), 1);
    assert_eq!(home["charts"][0]["entity_id"], "sensor.nas");
    assert_eq!(home["zones"].as_array().unwrap().len(), 2);
    assert_eq!(home["zones"][0]["entity_id"], "light.porche");
    assert_eq!(home["zones"][1]["area"], "Cuisine");

    // Chaque changement est dans le journal d'audit.
    let (_, body) = app.request("GET", "/api/audit", Some(&token), None).await;
    let actions: Vec<&str> = body["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|entry| entry["action"].as_str())
        .collect();
    assert!(actions.contains(&"drawing_set"), "{actions:?}");
    assert!(actions.contains(&"drawing_image"), "{actions:?}");
}

/// La photo de la sonnette (étape 82) : servie à la famille, jamais gardée par
/// le navigateur ; un invité ne la voit pas.
#[tokio::test]
async fn the_doorbell_snapshot_is_served_to_the_family_only() {
    let app = TestApp::new().await;
    let camera: ha_core::EntityId = "camera.porte".parse().unwrap();
    app.state
        .core
        .states
        .set(camera.clone(), "idle", Attributes::new())
        .unwrap();
    let family = app.login_as("famille", ha_auth::Role::Family).await;
    // Pas encore de photo.
    let (status, _, _) = app
        .send_bytes("GET", "/api/camera_proxy/camera.porte", &family, Vec::new())
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let jpeg = vec![0xFF, 0xD8, 0xFF, 0xE0, 1, 2, 3];
    app.state.core.images.set(camera, &jpeg).unwrap();
    let request = Request::builder()
        .uri("/api/camera_proxy/camera.porte")
        .header(header::AUTHORIZATION, format!("Bearer {family}"))
        .body(Body::empty())
        .unwrap();
    let response = app.router().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CONTENT_TYPE], "image/jpeg");
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    let body = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .unwrap();
    assert_eq!(body.as_ref(), jpeg.as_slice());

    // Un invité : comme si elle n'existait pas.
    let guest = app.login_as("visiteur", ha_auth::Role::Guest).await;
    let (status, _, _) = app
        .send_bytes("GET", "/api/camera_proxy/camera.porte", &guest, Vec::new())
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (_, listed) = app.request("GET", "/api/states", Some(&guest), None).await;
    assert!(!listed.to_string().contains("camera.porte"));
    // Une autre entité que la caméra n'a pas d'image.
    let (status, _, _) = app
        .send_bytes("GET", "/api/camera_proxy/light.salon", &family, Vec::new())
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
