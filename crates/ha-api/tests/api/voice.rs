//! La voix : la tablette qui écoute, le mot d'appel.

use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message;
use tower::ServiceExt;

use crate::common::{TestApp, next_order, two_rooms};

/// Un faux service d'écoute (comme sherpa-onnx, par WebSocket) : il répond
/// les phrases données, une par son reçu, et compte les sons.
async fn fake_listener(
    sentences: Vec<&'static str>,
) -> (String, Arc<std::sync::atomic::AtomicUsize>) {
    slow_listener(sentences, Duration::ZERO).await
}

/// Le même, qui prend `delay` pour écouter chaque son.
async fn slow_listener(
    sentences: Vec<&'static str>,
    delay: Duration,
) -> (String, Arc<std::sync::atomic::AtomicUsize>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let heard = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let count = Arc::clone(&heard);
    tokio::spawn(async move {
        for sentence in sentences {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            let Some(Ok(Message::Binary(_))) = socket.next().await else {
                panic!("un son est attendu");
            };
            count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            tokio::time::sleep(delay).await;
            socket
                .send(Message::Text(json!({"text": sentence}).to_string().into()))
                .await
                .unwrap();
            let _ = socket.next().await;
        }
    });
    (format!("ws://{address}"), heard)
}

/// Enchaîner à la voix (étape 96) : `?conversation_id=` sur
/// `/api/conversation/audio`.
#[tokio::test]
async fn the_tablet_follows_a_spoken_conversation() {
    let app = TestApp::new().await;
    two_rooms(&app);
    let david = app.login().await;
    let (url, _) = fake_listener(vec!["Allume la lumière du salon.", "Et la cuisine."]).await;
    app.state
        .set_voice(Some(ha_stt::Stt::from_settings(&url).unwrap()), None);
    let mut events = app.state.core.bus.subscribe();

    let (status, bytes, _) = app
        .send_bytes("POST", "/api/conversation/audio", &david, spoken_wav())
        .await;
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["continue_conversation"], true);
    let id = body["conversation_id"].as_str().unwrap().to_owned();
    assert_eq!(next_order(&mut events).await.1, "light.lampe_salon");

    let (status, bytes, _) = app
        .send_bytes(
            "POST",
            &format!("/api/conversation/audio?conversation_id={id}"),
            &david,
            spoken_wav(),
        )
        .await;
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["text"], "Et la cuisine.");
    assert_eq!(body["conversation_id"], id.as_str());
    assert_eq!(next_order(&mut events).await.1, "light.plafonnier_cuisine");

    // Un paramètre inconnu : refusé.
    let (status, _, _) = app
        .send_bytes(
            "POST",
            "/api/conversation/audio?autre=1",
            &david,
            spoken_wav(),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

/// Un morceau envoyé au mot d'appel : (statut, réponse).
async fn wake(app: &TestApp, token: &str, wav: Vec<u8>) -> (StatusCode, Value) {
    let (status, bytes, _) = app
        .send_bytes("POST", "/api/conversation/wake", token, wav)
        .await;
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

/// Le mot d'appel (étape 97) : « Maison, allume la lumière du salon » obéit,
/// « Maison » seul ouvre le micro, une autre phrase ne fait rien.
#[tokio::test]
async fn the_wake_word_wakes_the_tablet() {
    let app = TestApp::new().await;
    two_rooms(&app);
    let david = app.login().await;

    // Sans écoute : le mot d'appel n'existe pas.
    let (status, _) = wake(&app, &david, spoken_wav()).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);

    let (url, heard) = fake_listener(vec![
        "Maison, allume la lumière du salon.",
        "Maison.",
        "Il fait beau aujourd'hui.",
    ])
    .await;
    app.state
        .set_voice(Some(ha_stt::Stt::from_settings(&url).unwrap()), None);
    let (_, body) = app
        .request("GET", "/api/conversation/voice", Some(&david), None)
        .await;
    assert_eq!(body["wake"], true);
    assert_eq!(body["wake_word"], "Maison");
    let mut events = app.state.core.bus.subscribe();

    // Le mot puis une commande : obéie, comme à la voix.
    let (status, body) = wake(&app, &david, spoken_wav()).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["wake"], true);
    assert_eq!(body["text"], "allume la lumière du salon.");
    assert_eq!(body["response"]["response_type"], "action_done");
    assert_eq!(body["continue_conversation"], true);
    assert_eq!(
        next_order(&mut events).await,
        ("turn_on".into(), "light.lampe_salon".into())
    );

    // Le mot seul : la tablette ouvrira le micro, aucun ordre.
    let (status, body) = wake(&app, &david, spoken_wav()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({"wake": true}));

    // Pas de mot : rien d'autre que « non », ni le texte entendu.
    let (status, body) = wake(&app, &david, spoken_wav()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({"wake": false}));
    assert_eq!(heard.load(std::sync::atomic::Ordering::SeqCst), 3);
    assert!(
        tokio::time::timeout(Duration::from_millis(200), events.recv())
            .await
            .is_err(),
        "aucun ordre"
    );

    // Un silence : pas même écouté.
    let (status, body) = wake(&app, &david, silent_wav(1000)).await;
    assert_eq!((status, body), (StatusCode::OK, json!({"wake": false})));
    assert_eq!(heard.load(std::sync::atomic::Ordering::SeqCst), 3);

    // Sans connexion : refusé.
    let request = Request::builder()
        .method("POST")
        .uri("/api/conversation/wake")
        .body(Body::from(spoken_wav()))
        .unwrap();
    let response = app.router().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

/// Les garde-fous du mot d'appel : morceaux courts, débit limité par
/// appareil, jeton d'assistant refusé.
#[tokio::test]
async fn the_wake_word_has_limits() {
    let app = TestApp::new().await;
    let david = app.login().await;
    let (url, heard) = fake_listener(vec!["Maison."]).await;
    app.state
        .set_voice(Some(ha_stt::Stt::from_settings(&url).unwrap()), None);

    // Trop long : refusé dès la lecture (7 s), ou après l'en-tête (6,6 s),
    // sans être écouté.
    let (status, _) = wake(&app, &david, tone_wav(7000)).await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    let (status, body) = wake(&app, &david, tone_wav(6600)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    // Pas un WAV : refusé.
    let (status, _) = wake(&app, &david, b"bonjour".to_vec()).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(heard.load(std::sync::atomic::Ordering::SeqCst), 0);

    // Le débit : 30 morceaux par minute et par appareil (les 2 refusés
    // après lecture comptent déjà).
    for _ in 2..30 {
        let (status, _) = wake(&app, &david, silent_wav(1000)).await;
        assert_eq!(status, StatusCode::OK);
    }
    let (status, body) = wake(&app, &david, silent_wav(1000)).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{body}");
    // Un autre appareil (un autre jeton) n'est pas gêné.
    let tablet = app.login_as("cuisine", ha_auth::Role::Family).await;
    let (status, body) = wake(&app, &tablet, silent_wav(1000)).await;
    assert_eq!((status, body), (StatusCode::OK, json!({"wake": false})));

    // Un jeton d'assistant IA n'est pas une personne.
    let issued = app
        .state
        .auth
        .issue_program_token("david", Duration::from_hours(24))
        .await
        .unwrap()
        .unwrap();
    let (status, _) = wake(&app, issued.token.as_str(), spoken_wav()).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

/// Une vraie phrase passe avant : pendant qu'elle est écoutée, un morceau
/// du mot d'appel est abandonné tout de suite.
#[tokio::test]
async fn a_spoken_sentence_goes_before_the_wake_word() {
    let app = TestApp::new().await;
    two_rooms(&app);
    let david = app.login().await;
    let (url, heard) = slow_listener(
        vec!["Allume la lumière du salon."],
        Duration::from_millis(800),
    )
    .await;
    app.state
        .set_voice(Some(ha_stt::Stt::from_settings(&url).unwrap()), None);

    let sentence = app.send_bytes("POST", "/api/conversation/audio", &david, spoken_wav());
    let chunk = async {
        tokio::time::sleep(Duration::from_millis(200)).await;
        let started = std::time::Instant::now();
        let answer = wake(&app, &david, spoken_wav()).await;
        (answer, started.elapsed())
    };
    let ((status, bytes, _), ((wake_status, body), waited)) = tokio::join!(sentence, chunk);
    assert_eq!(
        status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&bytes)
    );
    assert_eq!(wake_status, StatusCode::OK);
    assert_eq!(body, json!({"wake": false, "busy": true}));
    assert!(waited < Duration::from_millis(500), "{waited:?}");
    assert_eq!(heard.load(std::sync::atomic::Ordering::SeqCst), 1);
}

/// Un WAV d'une seconde (16 bits, mono, 16 000 Hz) : une note.
fn spoken_wav() -> Vec<u8> {
    tone_wav(1000)
}

/// Une note de `millis` millisecondes.
fn tone_wav(millis: u32) -> Vec<u8> {
    wav_of(
        (0..16 * millis)
            .map(|i| if i % 40 < 20 { 6000 } else { -6000 })
            .collect(),
    )
}

/// Un silence de `millis` millisecondes (un léger souffle).
fn silent_wav(millis: u32) -> Vec<u8> {
    wav_of(
        (0..16 * millis)
            .map(|i| if i % 2 == 0 { 30 } else { -30 })
            .collect(),
    )
}

/// Un WAV 16 bits, mono, 16 000 Hz.
fn wav_of(samples: Vec<i16>) -> Vec<u8> {
    let data = u32::try_from(samples.len() * 2).unwrap();
    let mut out = Vec::new();
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    for (value, size) in [
        (16u32, 4),
        (1, 2),
        (1, 2),
        (16_000, 4),
        (32_000, 4),
        (2, 2),
        (16, 2),
    ] {
        out.extend_from_slice(&value.to_le_bytes()[..size]);
    }
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data.to_le_bytes());
    for sample in samples {
        out.extend_from_slice(&sample.to_le_bytes());
    }
    out
}

/// Parler à la maison (étape 84) : la tablette envoie le son, le serveur
/// l'écoute (ici un faux Whisper), le comprend et obéit ; la réponse est dite
/// par la voix (un faux Piper).
#[cfg(unix)]
#[tokio::test]
async fn the_tablet_can_talk_to_the_house() {
    use std::os::unix::fs::PermissionsExt;

    let app = TestApp::new().await;
    furnish(&app);
    let david = app.login().await;

    // Rien d'installé : pas de micro.
    let (_, body) = app
        .request("GET", "/api/conversation/voice", Some(&david), None)
        .await;
    assert_eq!(
        body,
        json!({"listen": false, "speak": false, "wake": false, "wake_word": "Maison"})
    );
    let (status, _, _) = app
        .send_bytes("POST", "/api/conversation/audio", &david, spoken_wav())
        .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);

    let dir = std::env::temp_dir().join(format!("ha-api-voix-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for name in [
        ha_stt::ENCODER,
        ha_stt::DECODER,
        ha_stt::TOKENS,
        "voix.onnx",
    ] {
        std::fs::write(dir.join(name), b"").unwrap();
    }
    let scripts = [
        (
            ha_stt::PROGRAM,
            "#!/bin/sh\necho '{\"lang\": \"fr\", \"text\": \" Ferme le volet du salon.\"}'\n",
        ),
        (
            "piper",
            "#!/bin/sh\ncat > /dev/null\nprintf '\\001\\000\\002\\000'\n",
        ),
    ];
    for (name, script) in scripts {
        std::fs::write(dir.join(name), script).unwrap();
        std::fs::set_permissions(dir.join(name), std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    app.state.set_voice(
        Some(ha_stt::Stt::from_dir(&dir).unwrap()),
        Some(ha_tts::Tts::new(ha_tts::Engine::Program {
            program: dir.join("piper"),
            model: dir.join("voix.onnx"),
        })),
    );
    let (_, body) = app
        .request("GET", "/api/conversation/voice", Some(&david), None)
        .await;
    // Le mot d'appel demande le service d'écoute, pas le programme.
    assert_eq!(
        body,
        json!({"listen": true, "speak": true, "wake": false, "wake_word": "Maison"})
    );
    let (status, _, _) = app
        .send_bytes("POST", "/api/conversation/wake", &david, spoken_wav())
        .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);

    let mut events = app.state.core.bus.subscribe();
    let (status, bytes, _) = app
        .send_bytes("POST", "/api/conversation/audio", &david, spoken_wav())
        .await;
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["text"], "Ferme le volet du salon.");
    assert_eq!(body["response"]["response_type"], "action_done");
    let event = events.recv().await.unwrap();
    let ha_core::Event::CallService { service, .. } = event.as_ref() else {
        panic!("un ordre est attendu");
    };
    assert_eq!(service, "close_cover");

    // Un son qui n'est pas un WAV : refusé avant d'être écouté.
    let (status, _, _) = app
        .send_bytes(
            "POST",
            "/api/conversation/audio",
            &david,
            b"bonjour".to_vec(),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    // Trop gros : refusé dès la lecture.
    let (status, _, _) = app
        .send_bytes(
            "POST",
            "/api/conversation/audio",
            &david,
            vec![0; ha_stt::MAX_WAV_BYTES + 1],
        )
        .await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);

    // La réponse, dite par la voix.
    let (status, bytes, content_type) = app
        .send_bytes(
            "POST",
            "/api/conversation/speech",
            &david,
            json!({"text": "C'est fait."}).to_string().into_bytes(),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(content_type.as_deref(), Some("audio/wav"));
    assert_eq!(&bytes[..4], b"RIFF");

    // Sans connexion : refusé.
    let (status, _) = app
        .request(
            "POST",
            "/api/conversation/speech",
            None,
            Some(json!({"text": "a"})),
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    std::fs::remove_dir_all(&dir).unwrap();
}
