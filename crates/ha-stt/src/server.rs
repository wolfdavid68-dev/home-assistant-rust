//! Le service d'écoute (`sherpa-onnx-offline-websocket-server`), qui garde
//! Whisper en mémoire : une phrase est écoutée en 3 à 4 s au lieu de 8 à 9.
//!
//! Son protocole, en WebSocket : un message binaire avec la fréquence (entier
//! de 32 bits), la taille du son en octets (32 bits), puis le son en nombres
//! à virgule de 32 bits (tout en petit-boutiste). Le service répond par un
//! texte JSON (`{"text": "…"}`), et on lui dit « Done » pour finir.

use std::net::SocketAddr;

use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;

use crate::{RATE, SttError};

/// La réponse du service fait au plus cette taille.
const MAX_REPLY: usize = 64 * 1024;

fn unreachable(error: impl std::fmt::Display) -> SttError {
    SttError::Failed(format!("service d'écoute injoignable : {error}"))
}

/// Les échantillons 16 bits du WAV, comme le service les attend.
pub(crate) fn message(samples: &[u8]) -> Vec<u8> {
    let count = samples.len() / 2;
    let mut out = Vec::with_capacity(8 + count * 4);
    out.extend_from_slice(&RATE.to_le_bytes());
    out.extend_from_slice(&u32::try_from(count * 4).unwrap_or(u32::MAX).to_le_bytes());
    for pair in samples.as_chunks::<2>().0 {
        let sample = f32::from(i16::from_le_bytes(*pair)) / 32_768.0;
        out.extend_from_slice(&sample.to_le_bytes());
    }
    out
}

pub(crate) async fn transcribe(address: SocketAddr, samples: &[u8]) -> Result<String, SttError> {
    let config = WebSocketConfig::default()
        .max_message_size(Some(MAX_REPLY))
        .max_frame_size(Some(MAX_REPLY));
    let (mut socket, _) =
        tokio_tungstenite::connect_async_with_config(format!("ws://{address}"), Some(config), true)
            .await
            .map_err(unreachable)?;
    socket
        .send(Message::Binary(message(samples).into()))
        .await
        .map_err(unreachable)?;
    while let Some(received) = socket.next().await {
        match received.map_err(unreachable)? {
            Message::Text(text) => {
                // « Done » : le service ferme la connexion de son côté.
                let _ = socket.send(Message::Text("Done".into())).await;
                let _ = socket.close(None).await;
                return Ok(text.to_string());
            }
            Message::Close(_) => break,
            _ => {}
        }
    }
    Err(SttError::Failed(
        "le service d'écoute n'a rien répondu".into(),
    ))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]

    use super::*;
    use crate::Stt;

    /// Un faux service : il vérifie le message, répond comme le vrai, et
    /// attend « Done ».
    #[tokio::test]
    async fn the_service_hears_the_sentence() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let service = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            let Some(Ok(Message::Binary(bytes))) = socket.next().await else {
                panic!("un message binaire est attendu");
            };
            let rate = u32::from_le_bytes(bytes[..4].try_into().unwrap());
            let size = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
            assert_eq!(rate, 16_000);
            assert_eq!(size as usize, bytes.len() - 8);
            let first = f32::from_le_bytes(bytes[8..12].try_into().unwrap());
            socket
                .send(Message::Text(
                    format!("{{\"lang\": \"fr\", \"text\": \" Ouvre le volet. \", \"premier\": {first}}}")
                        .into(),
                ))
                .await
                .unwrap();
            let Some(Ok(Message::Text(done))) = socket.next().await else {
                panic!("« Done » est attendu");
            };
            assert_eq!(done.as_str(), "Done");
            size
        });
        let stt = Stt::from_settings(&format!("ws://{address}")).unwrap();
        assert!(stt.describe().contains("service"), "{}", stt.describe());
        let wav = crate::wav::tests::tone(16_000, 1.0);
        assert_eq!(stt.transcribe(&wav).await.unwrap(), "Ouvre le volet.");
        assert_eq!(service.await.unwrap(), 64_000);

        // Plus de service : une erreur claire.
        let error = stt.transcribe(&wav).await.unwrap_err().to_string();
        assert!(error.contains("injoignable"), "{error}");
    }

    #[test]
    fn only_a_local_service_is_accepted() {
        assert!(Stt::from_settings("ws://127.0.0.1:10350").is_ok());
        assert!(Stt::from_settings("ws://localhost:10350/").is_ok());
        assert!(Stt::from_settings("ws://[::1]:10350").is_ok());
        let error = Stt::from_settings("ws://192.168.1.20:10350")
            .unwrap_err()
            .to_string();
        assert!(error.contains("cette machine"), "{error}");
        assert!(Stt::from_settings("ws://maison").is_err());
    }

    #[test]
    fn samples_become_floats() {
        let bytes = message(&[0x00, 0x40, 0x00, 0xc0]);
        assert_eq!(&bytes[..8], &[0x80, 0x3e, 0, 0, 8, 0, 0, 0]);
        assert!((f32::from_le_bytes(bytes[8..12].try_into().unwrap()) - 0.5).abs() < 1e-6);
        assert!((f32::from_le_bytes(bytes[12..16].try_into().unwrap()) + 0.5).abs() < 1e-6);
    }
}
