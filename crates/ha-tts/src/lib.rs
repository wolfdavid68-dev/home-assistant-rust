//! La voix des annonces (étape 73) : un texte devient un son, **sur place**,
//! sans envoyer la phrase à un service sur Internet.
//!
//! On s'appuie sur [Piper](https://github.com/rhasspy/piper), la voix locale
//! de Home Assistant, de deux façons :
//! - **le programme** `piper` (ou `piper.exe` sous Windows), lancé pour
//!   chaque annonce : le plus simple sur un PC ;
//! - **le service Wyoming** (`wyoming-piper`), qui tourne à côté, dans son
//!   propre conteneur Docker : le serveur lui parle en TCP.
//!
//! - [`audio`] : le carillon, la conversion au format du CD, le fichier WAV ;
//! - [`wyoming`] : le protocole du service ;
//! - [`Tts`] : le choix entre les deux, d'après les réglages.

pub mod audio;
mod program;
pub mod wyoming;

use std::path::PathBuf;
use std::time::Duration;

pub use audio::{Announcement, Pcm, announcement};

/// Longueur maximale d'une annonce, en caractères : au-delà, ce n'est plus
/// une annonce mais une lecture.
pub const MAX_TEXT: usize = 500;
/// Au-delà, on abandonne : la voix ne doit pas bloquer une automatisation.
pub const TIMEOUT: Duration = Duration::from_secs(30);
/// Son brut gardé au plus (≈ 4 minutes à 22 050 Hz) : un service qui
/// enverrait sans fin ne remplit pas la mémoire.
pub const MAX_AUDIO_BYTES: usize = 10 * 1024 * 1024;
/// La voix proposée par défaut (française, claire, de qualité moyenne).
pub const DEFAULT_VOICE: &str = "fr_FR-siwis-medium";

/// Ce qui peut empêcher la voix de parler.
#[derive(Debug, thiserror::Error)]
pub enum TtsError {
    #[error("{0}")]
    Config(String),
    #[error("le texte de l'annonce est vide")]
    Empty,
    #[error("Piper n'a pas pu démarrer : {0}")]
    Start(std::io::Error),
    #[error("Piper n'a rien dit ({0})")]
    Silent(String),
    #[error("la voix a mis plus de {} s", TIMEOUT.as_secs())]
    Timeout,
    #[error("service de voix injoignable : {0}")]
    Connect(std::io::Error),
    #[error("réponse du service de voix illisible : {0}")]
    Protocol(String),
}

/// Où trouver la voix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Engine {
    /// Le programme `piper`, avec le fichier de la voix (`.onnx`).
    Program { program: PathBuf, model: PathBuf },
    /// Un service Wyoming (`hôte:port`), et la voix à lui demander.
    Wyoming {
        address: String,
        voice: Option<String>,
    },
}

/// La voix, prête à parler.
#[derive(Debug, Clone)]
pub struct Tts {
    engine: Engine,
}

impl Tts {
    /// D'après les réglages `HA_PIPER` et `HA_PIPER_VOIX` :
    /// - `tcp://hôte:port` : un service Wyoming ; la voix est un nom
    ///   (`fr_FR-siwis-medium`), facultatif ;
    /// - sinon, le chemin du programme `piper` ; la voix est le chemin du
    ///   fichier `.onnx` (par défaut, `fr_FR-siwis-medium.onnx` à côté du
    ///   programme).
    pub fn from_settings(piper: &str, voice: Option<&str>) -> Result<Self, TtsError> {
        let piper = piper.trim();
        let voice = voice.map(str::trim).filter(|v| !v.is_empty());
        if let Some(address) = piper.strip_prefix("tcp://") {
            if !is_valid_address(address) {
                return Err(TtsError::Config(format!(
                    "HA_PIPER : adresse invalide « {address} » (attendu : tcp://hôte:port)"
                )));
            }
            if let Some(voice) = voice
                && !is_valid_voice_name(voice)
            {
                return Err(TtsError::Config(format!(
                    "HA_PIPER_VOIX : nom de voix invalide « {voice} » (exemple : {DEFAULT_VOICE})"
                )));
            }
            return Ok(Self {
                engine: Engine::Wyoming {
                    address: address.to_owned(),
                    voice: voice.map(str::to_owned),
                },
            });
        }
        if piper.is_empty() {
            return Err(TtsError::Config("HA_PIPER est vide".into()));
        }
        let program = PathBuf::from(piper);
        if !program.is_file() {
            return Err(TtsError::Config(format!(
                "HA_PIPER : programme introuvable : {}",
                program.display()
            )));
        }
        let model = voice.map_or_else(
            || {
                program
                    .parent()
                    .unwrap_or_else(|| std::path::Path::new("."))
                    .join(format!("{DEFAULT_VOICE}.onnx"))
            },
            PathBuf::from,
        );
        if !model.is_file() {
            return Err(TtsError::Config(format!(
                "voix introuvable : {} (indique-la avec HA_PIPER_VOIX)",
                model.display()
            )));
        }
        Ok(Self {
            engine: Engine::Program { program, model },
        })
    }

    /// Une voix déjà décrite (pour les tests et les réglages faits ailleurs).
    #[must_use]
    pub fn new(engine: Engine) -> Self {
        Self { engine }
    }

    #[must_use]
    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    /// Pour le journal et la tuile : « Piper (fr_FR-siwis-medium) ».
    #[must_use]
    pub fn describe(&self) -> String {
        match &self.engine {
            Engine::Program { model, .. } => format!(
                "Piper ({})",
                model.file_stem().map_or_else(
                    || model.display().to_string(),
                    |s| s.to_string_lossy().into_owned()
                )
            ),
            Engine::Wyoming { voice, .. } => format!(
                "Piper, service Wyoming ({})",
                voice.as_deref().unwrap_or("voix par défaut")
            ),
        }
    }

    /// Dit `text` : le son brut, tel que Piper le donne.
    pub async fn speak(&self, text: &str) -> Result<Pcm, TtsError> {
        let text = clean_text(text).ok_or(TtsError::Empty)?;
        let speaking = async {
            match &self.engine {
                Engine::Program { program, model } => program::speak(program, model, &text).await,
                Engine::Wyoming { address, voice } => {
                    wyoming::speak(address, voice.as_deref(), &text).await
                }
            }
        };
        tokio::time::timeout(TIMEOUT, speaking)
            .await
            .map_err(|_| TtsError::Timeout)?
    }
}

/// Le texte tel que la voix le lira : sur une seule ligne (Piper lit une
/// phrase par ligne), sans caractère de contrôle, [`MAX_TEXT`] caractères au
/// plus. `None` s'il ne reste rien.
#[must_use]
pub fn clean_text(text: &str) -> Option<String> {
    let mut out = String::new();
    for word in text
        .split(|c: char| c.is_whitespace() || c.is_control())
        .filter(|w| !w.is_empty())
    {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(word);
    }
    let out: String = out.chars().take(MAX_TEXT).collect();
    (!out.is_empty()).then_some(out)
}

/// `hôte:port`, l'hôte en lettres, chiffres, `.` et `-`.
fn is_valid_address(address: &str) -> bool {
    let Some((host, port)) = address.rsplit_once(':') else {
        return false;
    };
    !host.is_empty()
        && host.len() <= 253
        && host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
        && port.parse::<u16>().is_ok_and(|p| p > 0)
}

fn is_valid_voice_name(voice: &str) -> bool {
    (1..=100).contains(&voice.len())
        && voice
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-' || b == b'.')
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]

    use super::*;

    #[test]
    fn the_text_is_put_on_one_line() {
        assert_eq!(
            clean_text("  Le linge\nest  fini.\t\u{7}Merci ").as_deref(),
            Some("Le linge est fini. Merci")
        );
        assert_eq!(clean_text(" \n "), None);
        assert_eq!(clean_text(&"a".repeat(800)).unwrap().len(), MAX_TEXT);
    }

    #[test]
    fn settings_for_the_wyoming_service() {
        let tts = Tts::from_settings("tcp://piper:10200", None).unwrap();
        assert_eq!(
            tts.engine(),
            &Engine::Wyoming {
                address: "piper:10200".into(),
                voice: None
            }
        );
        assert_eq!(tts.describe(), "Piper, service Wyoming (voix par défaut)");
        let tts = Tts::from_settings("tcp://127.0.0.1:10200", Some("fr_FR-siwis-medium")).unwrap();
        assert!(tts.describe().contains("fr_FR-siwis-medium"));
        for bad in [
            "tcp://",
            "tcp://piper",
            "tcp://pi/per:1",
            "tcp://piper:0",
            "tcp://a@b:1",
        ] {
            assert!(Tts::from_settings(bad, None).is_err(), "{bad}");
        }
        assert!(Tts::from_settings("tcp://piper:10200", Some("../voix")).is_err());
    }

    #[test]
    fn settings_for_the_program() {
        let dir = std::env::temp_dir().join(format!("ha-tts-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let program = dir.join("piper");
        std::fs::write(&program, b"").unwrap();
        // Sans la voix à côté : refusé, avec le chemin attendu.
        let error = Tts::from_settings(program.to_str().unwrap(), None).unwrap_err();
        assert!(
            error.to_string().contains("fr_FR-siwis-medium.onnx"),
            "{error}"
        );
        std::fs::write(dir.join("fr_FR-siwis-medium.onnx"), b"").unwrap();
        let tts = Tts::from_settings(program.to_str().unwrap(), None).unwrap();
        assert_eq!(tts.describe(), "Piper (fr_FR-siwis-medium)");
        assert!(Tts::from_settings(dir.join("absent").to_str().unwrap(), None).is_err());
        assert!(Tts::from_settings("", None).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
