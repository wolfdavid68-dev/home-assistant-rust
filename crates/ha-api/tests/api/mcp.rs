//! MCP : un assistant IA branché sous le garde-fou.

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use serde_json::{Value, json};
use tower::ServiceExt;

use crate::common::{TestApp, furnish};

/// Une requête MCP (JSON-RPC), avec des en-têtes en plus.
async fn mcp(
    app: &TestApp,
    token: Option<&str>,
    headers: &[(&str, &str)],
    body: Value,
) -> (StatusCode, Value) {
    let mut request = Request::builder()
        .method("POST")
        .uri("/api/mcp")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ACCEPT, "application/json, text/event-stream");
    if let Some(token) = token {
        request = request.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    let response = app
        .router()
        .oneshot(request.body(Body::from(body.to_string())).unwrap())
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

fn tool(name: &str, arguments: Value) -> Value {
    let mut call = json!({"jsonrpc": "2.0", "id": 7, "method": "tools/call",
                          "params": {"name": name}});
    call["params"]["arguments"] = arguments;
    call
}

fn tool_text(body: &Value) -> (&str, bool) {
    (
        body["result"]["content"][0]["text"].as_str().unwrap(),
        body["result"]["isError"].as_bool().unwrap(),
    )
}

#[tokio::test]
async fn an_ai_assistant_plugs_in_through_mcp_under_the_guard() {
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
    // Même avec un compte administrateur, un assistant IA reste un assistant.
    let claude = app.login().await;

    // Sans jeton : refusé. Depuis la page d'un autre site : refusé.
    let hello = json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {"protocolVersion": "2025-03-26", "capabilities": {},
                   "clientInfo": {"name": "essai", "version": "1"}}});
    let (status, _) = mcp(&app, None, &[], hello.clone()).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = mcp(
        &app,
        Some(&claude),
        &[
            ("origin", "https://pirate.example"),
            ("host", "maison.local"),
        ],
        hello.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, body) = mcp(
        &app,
        Some(&claude),
        &[("origin", "https://maison.local"), ("host", "maison.local")],
        hello.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // La négociation de version.
    let (_, body) = mcp(&app, Some(&claude), &[], hello).await;
    assert_eq!(body["id"], 1);
    assert_eq!(body["result"]["protocolVersion"], "2025-03-26");
    assert_eq!(body["result"]["serverInfo"]["name"], "maison");
    let (_, body) = mcp(
        &app,
        Some(&claude),
        &[],
        json!({"jsonrpc": "2.0", "id": 2, "method": "initialize",
               "params": {"protocolVersion": "1999-01-01"}}),
    )
    .await;
    assert_eq!(body["result"]["protocolVersion"], "2025-06-18");
    // Une notification : rien à répondre.
    let (status, _) = mcp(
        &app,
        Some(&claude),
        &[],
        json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);

    let (_, body) = mcp(
        &app,
        Some(&claude),
        &[],
        json!({"jsonrpc": "2.0", "id": 3, "method": "tools/list"}),
    )
    .await;
    let names: Vec<&str> = body["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["get_house_state", "assist", "call_service"]);

    let (_, body) = mcp(&app, Some(&claude), &[], tool("get_house_state", json!({}))).await;
    let (text, error) = tool_text(&body);
    assert!(!error);
    assert!(text.contains("(cover.salon) : ouvert"), "{text}");

    // Un ordre précis : passe, au nom de l'assistant.
    let mut events = core.bus.subscribe();
    let (_, body) = mcp(
        &app,
        Some(&claude),
        &[],
        tool(
            "call_service",
            json!({"domain": "light", "service": "turn_on", "entity_id": "light.salon"}),
        ),
    )
    .await;
    assert_eq!(
        tool_text(&body),
        ("C'est fait : allumer light.salon.", false)
    );
    let event = events.recv().await.unwrap();
    let ha_core::Event::CallService { origin, .. } = event.as_ref() else {
        panic!("un ordre est attendu");
    };
    assert_eq!(*origin, ha_core::Origin::Agent("david".into()));

    // Pièce protégée : retenu, même pour un compte administrateur.
    let (_, body) = mcp(
        &app,
        Some(&claude),
        &[],
        tool("assist", json!({"text": "ferme le volet du salon"})),
    )
    .await;
    let (text, error) = tool_text(&body);
    assert!(!error, "{text}");
    assert!(text.starts_with("En attente de validation"), "{text}");
    assert_eq!(
        core.held_orders()[0].origin,
        ha_core::Origin::Agent("david".into())
    );
    // Jamais ouvrir la maison.
    let (_, body) = mcp(
        &app,
        Some(&claude),
        &[],
        tool(
            "call_service",
            json!({"domain": "lock", "service": "unlock", "entity_id": "lock.entree"}),
        ),
    )
    .await;
    let (text, error) = tool_text(&body);
    assert!(error);
    assert!(text.contains("ne commande ni serrure"), "{text}");
    // Ni la porte du garage, même hors pièce protégée (revue de la PR #60).
    let (_, body) = mcp(
        &app,
        Some(&claude),
        &[],
        tool(
            "call_service",
            json!({"domain": "cover", "service": "open_cover", "entity_id": "cover.garage"}),
        ),
    )
    .await;
    let (text, error) = tool_text(&body);
    assert!(error, "{text}");
    assert!(text.contains("porte de garage"), "{text}");
    // Une valeur hors plage.
    let (_, body) = mcp(
        &app,
        Some(&claude),
        &[],
        tool(
            "call_service",
            json!({"domain": "cover", "service": "set_cover_position",
                   "entity_id": "cover.garage", "data": {"position": 300}}),
        ),
    )
    .await;
    assert!(tool_text(&body).1);

    // Erreurs du protocole.
    let (_, body) = mcp(
        &app,
        Some(&claude),
        &[],
        json!({"jsonrpc": "2.0", "id": 9, "method": "resources/list"}),
    )
    .await;
    assert_eq!(body["error"]["code"], -32601);
    let (_, body) = mcp(&app, Some(&claude), &[], tool("rm_rf", json!({}))).await;
    assert_eq!(body["error"]["code"], -32602);
    let (_, body) = mcp(&app, Some(&claude), &[], json!([1, 2])).await;
    assert_eq!(body["error"]["code"], -32600);

    // 60 appels d'outils par minute au plus.
    let mut limited = false;
    for _ in 0..60 {
        let (_, body) = mcp(&app, Some(&claude), &[], tool("get_house_state", json!({}))).await;
        if tool_text(&body).0.starts_with("Trop de demandes") {
            limited = true;
            break;
        }
    }
    assert!(limited);
    let journal = app.state.store.audit_log(200).await.unwrap();
    assert!(
        journal
            .iter()
            .any(|e| e.action == "mcp" && e.detail.as_deref() == Some("call_service"))
    );
}

#[tokio::test]
async fn an_assistant_token_only_opens_mcp() {
    let app = TestApp::new().await;
    furnish(&app);
    let _ = app.login().await;
    let issued = app
        .state
        .auth
        .issue_program_token("david", std::time::Duration::from_hours(24))
        .await
        .unwrap()
        .unwrap();
    let agent = issued.token.as_str();
    // Partout ailleurs que MCP, un jeton d'assistant n'existe pas : il ne
    // peut pas se faire passer pour une personne (garde contournée).
    for (method, uri, body) in [
        ("GET", "/api/states", None),
        (
            "POST",
            "/api/services/cover/open_cover",
            Some(json!({"entity_id": "cover.salon"})),
        ),
        (
            "POST",
            "/api/conversation/process",
            Some(json!({"text": "ouvre le volet du salon"})),
        ),
    ] {
        let (status, _) = app.request(method, uri, Some(agent), body).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{uri}");
    }
    let (status, body) = mcp(
        &app,
        Some(agent),
        &[],
        json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
}
