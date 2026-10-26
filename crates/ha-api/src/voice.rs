//! Parler à la maison (étape 84) : la tablette enregistre une phrase, le
//! serveur l'écoute (Whisper, sur place), la comprend avec Assist, et la
//! réponse est lue par la voix de la maison (Piper, sur place).
//!
//! - `GET /api/conversation/voice` : `{"listen": true, "speak": true,
//!   "wake": true, "wake_word": "Maison"}`, ce qui est installé (`wake` :
//!   le mot d'appel de l'étape 97, possible avec le service d'écoute) ;
//! - `POST /api/conversation/audio` : le son en WAV (16 bits, mono,
//!   16 000 Hz, 15 s au plus). La réponse : `{"text": "ferme le volet",
//!   "response": {…}, "conversation_id": "…", "continue_conversation":
//!   true}`, comme `/api/conversation/process`. Avec
//!   `?conversation_id=…`, la phrase continue la conversation (étape 96 :
//!   « et la cuisine aussi ») ;
//! - `POST /api/conversation/speech` avec `{"text": "…"}` : la phrase dite
//!   par la voix, en WAV.
//!
//! Comme pour Assist, ni le son ni le texte entendu ne sont écrits dans les
//! journaux, et chaque ordre passe par les mêmes vérifications qu'un toucher.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::Json;
use axum::body::Bytes;
use axum::extract::{ConnectInfo, Query, State};
use axum::http::header;
use axum::response::{IntoResponse, Response};
use ha_stt::SttError;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::ApiState;
use crate::assist::{Turn, converse_in};
use crate::auth::Authenticated;
use crate::error::ApiError;
use crate::rest::parse_json;

/// Le temps d'attendre son tour pour parler.
const SPEECH_TURN: Duration = Duration::from_secs(10);

/// `GET /api/conversation/voice`
pub(crate) async fn overview(
    State(state): State<Arc<ApiState>>,
    _auth: Authenticated,
) -> Json<Value> {
    Json(json!({
        "listen": state.listening.get().is_some(),
        "speak": state.speaking.get().is_some(),
        "wake": state.listening.get().is_some_and(ha_stt::Stt::keeps_model),
        "wake_word": state.wake.word().shown(),
    }))
}

/// Les paramètres de `POST /api/conversation/audio`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AudioQuery {
    /// La conversation à continuer (étape 96).
    conversation_id: Option<String>,
}

/// `POST /api/conversation/audio`
pub(crate) async fn listen(
    State(state): State<Arc<ApiState>>,
    ConnectInfo(client): ConnectInfo<SocketAddr>,
    auth: Authenticated,
    Query(query): Query<AudioQuery>,
    body: Bytes,
) -> Result<Json<Value>, ApiError> {
    let conversation_id = query.conversation_id.as_deref();
    let stt = state.listening.get().ok_or_else(|| {
        ApiError::Unavailable("l'écoute n'est pas installée (voir docs/voix.md)".into())
    })?;
    let text = match stt.transcribe(&body).await {
        Ok(text) => text,
        Err(SttError::Silence) => {
            // Rien entendu : la tablette n'écoute pas encore une fois.
            let turn = Turn {
                kind: "error",
                speech: "Je n'ai rien entendu. Réessaie, un peu plus près de la tablette.".into(),
                conversation_id: conversation_id
                    .filter(|id| crate::conversation::valid(id))
                    .map(str::to_owned),
                more: false,
            };
            return Ok(heard("", &turn));
        }
        Err(SttError::BadAudio(why)) => {
            return Err(ApiError::BadRequest(format!("son refusé : {why}")));
        }
        Err(SttError::Busy) => {
            return Err(ApiError::Unavailable(
                "une autre phrase est en cours d'écoute, réessaie".into(),
            ));
        }
        Err(error) => {
            tracing::warn!("écoute : {error}");
            return Err(ApiError::Unavailable("l'écoute a échoué".into()));
        }
    };
    let turn = converse_in(&state, &auth, client, &text, conversation_id).await;
    Ok(heard(&text, &turn))
}

fn heard(text: &str, turn: &Turn) -> Json<Value> {
    let mut reply = turn.json();
    reply["text"] = json!(text);
    Json(reply)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SpeechBody {
    text: String,
}

/// `POST /api/conversation/speech`
pub(crate) async fn speak(
    State(state): State<Arc<ApiState>>,
    _auth: Authenticated,
    body: Bytes,
) -> Result<Response, ApiError> {
    let tts = state.speaking.get().ok_or_else(|| {
        ApiError::Unavailable("la voix n'est pas installée (voir docs/sonos.md)".into())
    })?;
    let SpeechBody { text } = parse_json(&body)?;
    if text.chars().count() > ha_tts::MAX_TEXT {
        return Err(ApiError::BadRequest(format!(
            "texte trop long ({} caractères au plus)",
            ha_tts::MAX_TEXT
        )));
    }
    let _turn = tokio::time::timeout(SPEECH_TURN, state.speech_turn.acquire())
        .await
        .ok()
        .and_then(Result::ok)
        .ok_or_else(|| ApiError::Unavailable("la voix est occupée, réessaie".into()))?;
    let pcm = match tts.speak(&text).await {
        Ok(pcm) => pcm,
        Err(ha_tts::TtsError::Empty) => {
            return Err(ApiError::BadRequest("texte vide".into()));
        }
        Err(error) => {
            tracing::warn!("voix de la tablette : {error}");
            return Err(ApiError::Unavailable("la voix n'a pas pu parler".into()));
        }
    };
    Ok((
        [
            (header::CONTENT_TYPE, "audio/wav"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        ha_tts::audio::wav(&pcm),
    )
        .into_response())
}
