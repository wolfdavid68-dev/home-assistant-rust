//! Le protocole Wyoming, celui des services vocaux de Home Assistant
//! (`wyoming-piper`, `wyoming-whisper`…).
//!
//! Une connexion TCP où chaque message est :
//! 1. une ligne JSON, l'en-tête : `{"type": "audio-chunk", "data_length": 52, "payload_length": 2048}` ;
//! 2. `data_length` octets de JSON, les détails (qui peuvent aussi être dans
//!    l'en-tête, sous `data`) ;
//! 3. `payload_length` octets de données brutes (le son).
//!
//! Pour parler : on envoie `synthesize` avec le texte, le service répond
//! `audio-start`, des `audio-chunk`, puis `audio-stop`.

use std::time::Duration;

use serde_json::{Map, Value, json};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

use crate::{MAX_AUDIO_BYTES, Pcm, TtsError};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// Taille maximale d'un en-tête ou de ses détails.
const MAX_HEADER: u64 = 64 * 1024;

/// Un message reçu.
#[derive(Debug, Clone, PartialEq)]
pub struct Message {
    pub kind: String,
    pub data: Map<String, Value>,
    pub payload: Vec<u8>,
}

pub(crate) async fn speak(address: &str, voice: Option<&str>, text: &str) -> Result<Pcm, TtsError> {
    let stream = tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(address))
        .await
        .map_err(|_| TtsError::Connect(std::io::ErrorKind::TimedOut.into()))?
        .map_err(TtsError::Connect)?;
    let (reader, mut writer) = stream.into_split();
    let mut data = json!({ "text": text });
    if let Some(voice) = voice {
        data["voice"] = json!({ "name": voice });
    }
    let header = json!({ "type": "synthesize", "data": data });
    let mut line = header.to_string();
    line.push('\n');
    writer
        .write_all(line.as_bytes())
        .await
        .map_err(TtsError::Connect)?;
    writer.flush().await.map_err(TtsError::Connect)?;
    collect_audio(&mut BufReader::new(reader)).await
}

/// Lit les messages jusqu'à `audio-stop`, et assemble le son.
pub async fn collect_audio<R: AsyncRead + Unpin>(
    reader: &mut BufReader<R>,
) -> Result<Pcm, TtsError> {
    let mut bytes = Vec::new();
    let mut format: Option<(u32, u16)> = None;
    loop {
        let message = read_message(reader).await?;
        match message.kind.as_str() {
            "audio-start" | "audio-chunk" => {
                let width = message
                    .data
                    .get("width")
                    .and_then(Value::as_u64)
                    .unwrap_or(2);
                if width != 2 {
                    return Err(TtsError::Protocol(format!(
                        "échantillons de {width} octets"
                    )));
                }
                let rate = message
                    .data
                    .get("rate")
                    .and_then(Value::as_u64)
                    .and_then(|r| u32::try_from(r).ok())
                    .filter(|r| (8_000..=96_000).contains(r));
                let channels = message
                    .data
                    .get("channels")
                    .and_then(Value::as_u64)
                    .and_then(|c| u16::try_from(c).ok())
                    .filter(|c| (1..=2).contains(c));
                if let (Some(rate), Some(channels)) = (rate, channels) {
                    format = Some((rate, channels));
                }
                if bytes.len() + message.payload.len() > MAX_AUDIO_BYTES {
                    return Err(TtsError::Protocol("son trop long".into()));
                }
                bytes.extend_from_slice(&message.payload);
            }
            "audio-stop" => break,
            "error" => {
                let text = message
                    .data
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or("erreur inconnue");
                return Err(TtsError::Protocol(text.chars().take(200).collect()));
            }
            // Autres messages (informations…) : ignorés.
            _ => {}
        }
    }
    let (rate, channels) =
        format.ok_or_else(|| TtsError::Protocol("format du son inconnu".into()))?;
    if bytes.len() < 2 {
        return Err(TtsError::Silent("le service n'a rien envoyé".into()));
    }
    Ok(Pcm::from_le_bytes(rate, channels, &bytes))
}

/// Lit un message (en-tête, détails, données).
pub async fn read_message<R: AsyncRead + Unpin>(
    reader: &mut BufReader<R>,
) -> Result<Message, TtsError> {
    let mut line = Vec::new();
    let read = (&mut *reader)
        .take(MAX_HEADER)
        .read_until(b'\n', &mut line)
        .await
        .map_err(TtsError::Connect)?;
    if read == 0 {
        return Err(TtsError::Protocol("connexion fermée avant la fin".into()));
    }
    if line.last() != Some(&b'\n') {
        return Err(TtsError::Protocol("en-tête trop long".into()));
    }
    let header: Value = serde_json::from_slice(&line)
        .map_err(|error| TtsError::Protocol(format!("en-tête : {error}")))?;
    let kind = header
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| TtsError::Protocol("message sans type".into()))?
        .to_owned();
    let mut data = header
        .get("data")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let data_length = length(&header, "data_length", MAX_HEADER)?;
    if data_length > 0 {
        let mut extra = vec![0; data_length];
        reader
            .read_exact(&mut extra)
            .await
            .map_err(TtsError::Connect)?;
        let extra: Value = serde_json::from_slice(&extra)
            .map_err(|error| TtsError::Protocol(format!("détails : {error}")))?;
        if let Value::Object(extra) = extra {
            data.extend(extra);
        }
    }
    let payload_length = length(
        &header,
        "payload_length",
        u64::try_from(MAX_AUDIO_BYTES).unwrap_or(u64::MAX),
    )?;
    let mut payload = vec![0; payload_length];
    if payload_length > 0 {
        reader
            .read_exact(&mut payload)
            .await
            .map_err(TtsError::Connect)?;
    }
    Ok(Message {
        kind,
        data,
        payload,
    })
}

fn length(header: &Value, key: &str, max: u64) -> Result<usize, TtsError> {
    match header.get(key) {
        None | Some(Value::Null) => Ok(0),
        Some(value) => value
            .as_u64()
            .filter(|n| *n <= max)
            .and_then(|n| usize::try_from(n).ok())
            .ok_or_else(|| TtsError::Protocol(format!("{key} invalide"))),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]

    use tokio::net::TcpListener;

    use super::*;
    use crate::{Engine, Tts};

    fn event(kind: &str, data: &Value, payload: &[u8]) -> Vec<u8> {
        // Les détails à part (`data_length`), comme les versions récentes.
        let data = data.to_string();
        let mut out = json!({
            "type": kind,
            "data_length": data.len(),
            "payload_length": payload.len(),
        })
        .to_string()
        .into_bytes();
        out.push(b'\n');
        out.extend_from_slice(data.as_bytes());
        out.extend_from_slice(payload);
        out
    }

    /// Un faux `wyoming-piper` : il vérifie la demande, puis envoie deux
    /// morceaux de son.
    #[tokio::test]
    async fn the_service_speaks() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap().to_string();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let (reader, mut writer) = stream.into_split();
            let mut reader = BufReader::new(reader);
            let request = read_message(&mut reader).await.unwrap();
            assert_eq!(request.kind, "synthesize");
            assert_eq!(request.data["text"], "Bonjour la maison");
            assert_eq!(request.data["voice"]["name"], "fr_FR-siwis-medium");
            let format = json!({"rate": 22050, "width": 2, "channels": 1});
            let mut reply = event("audio-start", &format, &[]);
            reply.extend(event("audio-chunk", &format, &[1, 0, 2, 0]));
            reply.extend(event("audio-chunk", &format, &[3, 0]));
            // Un en-tête avec les détails dedans, comme les anciennes versions.
            reply.extend(b"{\"type\": \"audio-stop\", \"data\": {}}\n");
            writer.write_all(&reply).await.unwrap();
        });
        let tts = Tts::new(Engine::Wyoming {
            address,
            voice: Some("fr_FR-siwis-medium".into()),
        });
        let pcm = tts.speak("Bonjour  la\nmaison").await.unwrap();
        server.await.unwrap();
        assert_eq!(pcm.rate, 22_050);
        assert_eq!(pcm.samples, vec![1, 2, 3]);
    }

    #[tokio::test]
    async fn errors_from_the_service_are_reported() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap().to_string();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut line = vec![0u8; 256];
            let _ = stream.read(&mut line).await;
            stream
                .write_all(&event("error", &json!({"text": "voix inconnue"}), &[]))
                .await
                .unwrap();
        });
        let tts = Tts::new(Engine::Wyoming {
            address,
            voice: None,
        });
        let error = tts.speak("Bonjour").await.unwrap_err();
        assert!(error.to_string().contains("voix inconnue"), "{error}");

        // Personne n'écoute : injoignable.
        let closed = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = closed.local_addr().unwrap().to_string();
        drop(closed);
        let tts = Tts::new(Engine::Wyoming {
            address,
            voice: None,
        });
        assert!(matches!(
            tts.speak("Bonjour").await,
            Err(TtsError::Connect(_))
        ));
        assert!(matches!(tts.speak(" ").await, Err(TtsError::Empty)));
    }

    #[tokio::test]
    async fn oversized_or_broken_messages_are_refused() {
        let broken: &[&[u8]] = &[
            b"pas du json\n",
            b"{\"data\": {}}\n",
            b"{\"type\": \"audio-chunk\", \"payload_length\": 99999999999}\n",
            b"{\"type\": \"audio-chunk\", \"payload_length\": 10}\nabc",
        ];
        for bytes in broken {
            let mut reader = BufReader::new(*bytes);
            assert!(read_message(&mut reader).await.is_err(), "{bytes:?}");
        }
        let long = vec![b' '; 70 * 1024];
        assert!(
            read_message(&mut BufReader::new(long.as_slice()))
                .await
                .is_err()
        );
        // Un son sans format annoncé.
        let mut bytes = event("audio-chunk", &json!({}), &[1, 0]);
        bytes.extend(event("audio-stop", &json!({}), &[]));
        assert!(
            collect_audio(&mut BufReader::new(bytes.as_slice()))
                .await
                .is_err()
        );
    }
}
