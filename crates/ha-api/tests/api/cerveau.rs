//! Le cerveau : préférences de la maison, « Pourquoi ? » sur le journal,
//! suite de conversation et limite par jour.

use std::sync::{Arc, Mutex};

use axum::http::StatusCode;
use serde_json::{Value, json};

use crate::common::{TestApp, furnish};

const KEY: &str = "une-cle-de-cerveau-assez-longue-pour-essai";

/// Un faux cerveau qui garde le JSON de chaque demande et répond toujours
/// la même phrase.
async fn recording_cerveau() -> (String, Arc<Mutex<Vec<Value>>>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let kept = Arc::clone(&seen);
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let mut request = Vec::new();
            let mut buffer = [0u8; 4096];
            let body = loop {
                let n = socket.read(&mut buffer).await.unwrap();
                assert!(n > 0, "demande coupée");
                request.extend_from_slice(&buffer[..n]);
                let text = String::from_utf8_lossy(&request).into_owned();
                let Some(end) = text.find("\r\n\r\n") else {
                    continue;
                };
                let length: usize = text
                    .lines()
                    .find_map(|l| l.strip_prefix("Content-Length: "))
                    .unwrap()
                    .parse()
                    .unwrap();
                if request.len() >= end + 4 + length {
                    break request[end + 4..].to_vec();
                }
            };
            kept.lock()
                .unwrap()
                .push(serde_json::from_slice(&body).unwrap());
            let reply = r#"{"reponse":"Parce que le soleil se couchait."}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}",
                reply.len()
            );
            socket.write_all(response.as_bytes()).await.unwrap();
        }
    });
    (address, seen)
}

#[tokio::test]
async fn the_family_writes_the_house_preferences() {
    let app = TestApp::new().await;
    let david = app.login().await;
    let guest = app.login_as("visiteur", ha_auth::Role::Guest).await;
    let url = "/api/cerveau/preferences";

    let (status, body) = app.request("GET", url, Some(&david), None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body, json!({"texte": "", "max": 2000}));

    let (status, body) = app
        .request(
            "PUT",
            url,
            Some(&david),
            Some(json!({"texte": "  Cosy = lampes à 30 %.\r\nOn se couche à 22 h.\n"})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["texte"], "Cosy = lampes à 30 %.\nOn se couche à 22 h.");

    let (_, body) = app.request("GET", url, Some(&david), None).await;
    assert_eq!(body["texte"], "Cosy = lampes à 30 %.\nOn se couche à 22 h.");
    assert_eq!(body["modifie_par"], "david");
    assert!(body["modifie_le"].is_string());

    for bad in [
        json!({"texte": "x".repeat(2001)}),
        json!({"texte": "un\u{7}bip"}),
        json!({"texte": "ok", "autre": 1}),
    ] {
        let (status, _) = app.request("PUT", url, Some(&david), Some(bad)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    // Un invité ne lit ni n'écrit les préférences.
    let (status, _) = app.request("GET", url, Some(&guest), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = app
        .request("PUT", url, Some(&guest), Some(json!({"texte": "rien"})))
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn the_cerveau_explains_a_logbook_line() {
    let app = TestApp::new().await;
    let david = app.login().await;
    let guest = app.login_as("visiteur", ha_auth::Role::Guest).await;
    let door: ha_core::EntityId = "cover.salon".parse().unwrap();
    for (entity_id, name, message) in [
        (None, "Coucher du soleil", "le soleil se couche"),
        (Some(&door), "Volet salon", "s'est fermé"),
    ] {
        app.state
            .store
            .add_logbook(ha_storage::NewLogbookEntry {
                entity_id,
                name,
                message,
                cause: Some("par l'automatisation Soir"),
            })
            .await
            .unwrap();
    }
    let (_, body) = app.request("GET", "/api/logbook", Some(&david), None).await;
    let shutter = body["entries"][0]["id"].as_i64().unwrap();
    assert_eq!(body["entries"][0]["name"], "Volet salon");

    // Sans cerveau : 503.
    let explain = |token: &String, body: Value| {
        let token = token.clone();
        let app = &app;
        async move {
            app.request("POST", "/api/cerveau/explique", Some(&token), Some(body))
                .await
        }
    };
    let (status, _) = explain(&david, json!({"id": shutter})).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    let (_, body) = app.request("GET", "/api/cerveau", Some(&david), None).await;
    assert_eq!(body, json!({"installe": false, "usage": null}));

    let (address, seen) = recording_cerveau().await;
    app.state
        .set_cerveau(&address, KEY, ha_api::CERVEAU_PER_DAY)
        .unwrap();
    let (status, body) = explain(&david, json!({"id": shutter})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["reponse"], "Parce que le soleil se couchait.");
    {
        let seen = seen.lock().unwrap();
        let sent = &seen[0];
        let texte = sent["texte"].as_str().unwrap();
        assert!(texte.starts_with("Pourquoi « Volet salon : s'est fermé » à "));
        let contexte = sent["contexte"].as_str().unwrap();
        let lines: Vec<&str> = contexte.lines().skip(1).collect();
        assert_eq!(lines.len(), 2, "{contexte}");
        // Dans l'ordre, la ligne à expliquer marquée.
        assert!(!lines[0].starts_with("→ ") && lines[0].contains("Coucher du soleil"));
        assert!(lines[1].starts_with("→ ") && lines[1].contains("Volet salon"));
        assert!(lines[1].contains("par l'automatisation Soir"));
        assert!(sent.get("avant").is_none());
    }

    // Une précision passe telle quelle.
    let (status, _) = explain(
        &david,
        json!({"id": shutter, "question": "pourquoi si tôt ?"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        seen.lock().unwrap()[1]["texte"]
            .as_str()
            .unwrap()
            .ends_with(": pourquoi si tôt ?")
    );

    let (status, _) = explain(&david, json!({"id": 999_999})).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = explain(&david, json!({"id": shutter, "question": "x".repeat(201)})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = explain(&guest, json!({"id": shutter})).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(seen.lock().unwrap().len(), 2);

    // Le compte des questions, pour la famille seulement.
    let (_, body) = app.request("GET", "/api/cerveau", Some(&david), None).await;
    assert_eq!(body["installe"], true);
    assert_eq!(body["usage"]["last_day"], 2);
    assert_eq!(body["usage"]["per_day"], ha_api::CERVEAU_PER_DAY);
    let (_, body) = app.request("GET", "/api/cerveau", Some(&guest), None).await;
    assert_eq!(body, json!({"installe": true, "usage": null}));
}

#[tokio::test]
async fn the_cerveau_follows_the_conversation_and_the_preferences() {
    let app = TestApp::new().await;
    furnish(&app);
    let (address, seen) = recording_cerveau().await;
    app.state.set_cerveau(&address, KEY, 2).unwrap();
    let david = app.login().await;
    let (status, _) = app
        .request(
            "PUT",
            "/api/cerveau/preferences",
            Some(&david),
            Some(json!({"texte": "Cosy = lampes à 30 %."})),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let say = |text: &str| {
        let token = david.clone();
        let text = text.to_owned();
        let app = &app;
        async move {
            let (status, body) = app
                .request(
                    "POST",
                    "/api/conversation/process",
                    Some(&token),
                    Some(json!({"text": text})),
                )
                .await;
            assert_eq!(status, StatusCode::OK, "{body}");
            body["response"]["response_type"]
                .as_str()
                .unwrap()
                .to_owned()
        }
    };

    assert_eq!(
        say("Raconte-moi une blague sur le jardin").await,
        "query_answer"
    );
    assert_eq!(say("Et une autre sur la cuisine").await, "query_answer");
    {
        let seen = seen.lock().unwrap();
        assert!(seen[0].get("avant").is_none());
        assert_eq!(seen[0]["preferences"], "Cosy = lampes à 30 %.");
        assert_eq!(
            seen[1]["avant"],
            json!([{
                "phrase": "Raconte-moi une blague sur le jardin",
                "reponse": "Parce que le soleil se couchait."
            }])
        );
    }

    // La limite du jour (2) est atteinte : Assist répond seul.
    assert_eq!(say("Et une troisième sur le garage").await, "error");
    assert_eq!(seen.lock().unwrap().len(), 2);
}
