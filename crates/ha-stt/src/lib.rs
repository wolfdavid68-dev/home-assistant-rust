//! **L'écoute** (étape 84) : une phrase dite à la tablette devient du
//! texte, **sur le mini-PC**, sans envoyer la voix à Google ni à Amazon.
//!
//! La reconnaissance est faite par **Parakeet** de NVIDIA (étape 88 :
//! 4 fois plus rapide que Whisper « small » de l'étape 84, et plus juste en
//! français), ou par [Whisper](https://github.com/openai/whisper), tous deux
//! réduits en entiers 8 bits, avec les programmes du projet
//! [sherpa-onnx](https://github.com/k2-fsa/sherpa-onnx), de deux façons
//! (réglage `HA_ECOUTE`) :
//!
//! - **le service** `sherpa-onnx-offline-websocket-server`, installé par
//!   `installer.sh --voix` : il garde le modèle en mémoire (≈ 900 Mo) et
//!   répond en moins d'une seconde avec Parakeet (3 à 4 s avec Whisper) ;
//! - **le programme** `sherpa-onnx-offline`, lancé pour chaque phrase :
//!   rien qui tourne en permanence, mais 4 à 5 s de plus pour relire le
//!   modèle (pour essayer sur un PC).
//!
//! Le son arrive de la tablette en WAV (16 bits, mono, 16 000 Hz,
//! [`MAX_SECONDS`] au plus), vérifié avant d'être transmis. Pour le
//! programme, il est écrit dans un fichier temporaire lisible par le
//! serveur seul, effacé aussitôt après. Une seule phrase est écoutée à la
//! fois : Whisper prend presque tous les processeurs du mini-PC pendant
//! quelques secondes.

pub(crate) mod server;
mod wav;

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::sync::Semaphore;

pub use wav::{RATE, check_wav};

/// Une phrase dure au plus ce nombre de secondes.
pub const MAX_SECONDS: u32 = 15;
/// La taille d'un WAV de [`MAX_SECONDS`], avec de la marge pour l'en-tête.
pub const MAX_WAV_BYTES: usize = (RATE * 2 * MAX_SECONDS) as usize + 4096;
/// Au-delà, on abandonne la phrase.
pub const TIMEOUT: Duration = Duration::from_secs(60);
/// Le temps d'attendre son tour quand une autre phrase est écoutée.
pub const WAIT_TURN: Duration = Duration::from_secs(20);
/// Le texte reconnu est coupé à cette longueur.
pub const MAX_TEXT: usize = 300;
/// La sortie du programme gardée au plus.
const MAX_OUTPUT: u64 = 64 * 1024;

/// Les fichiers attendus dans le dossier de l'écoute (`HA_ECOUTE`).
#[cfg(windows)]
pub const PROGRAM: &str = "sherpa-onnx-offline.exe";
#[cfg(not(windows))]
pub const PROGRAM: &str = "sherpa-onnx-offline";
pub const ENCODER: &str = "encoder.onnx";
pub const DECODER: &str = "decoder.onnx";
pub const TOKENS: &str = "tokens.txt";
/// Présent pour Parakeet (un « transducteur ») ; absent pour Whisper.
pub const JOINER: &str = "joiner.onnx";

/// Ce qui peut empêcher d'écouter.
#[derive(Debug, thiserror::Error)]
pub enum SttError {
    #[error("{0}")]
    Config(String),
    #[error("son refusé : {0}")]
    BadAudio(&'static str),
    #[error("je n'ai rien entendu")]
    Silence,
    #[error("une autre phrase est en cours d'écoute")]
    Busy,
    #[error("l'écoute n'a pas pu démarrer : {0}")]
    Start(std::io::Error),
    #[error("l'écoute a échoué ({0})")]
    Failed(String),
    #[error("l'écoute a mis plus de {} s", TIMEOUT.as_secs())]
    Timeout,
}

/// Où tourne Whisper.
#[derive(Debug)]
enum Engine {
    /// Le programme `sherpa-onnx-offline`, lancé pour chaque phrase : le
    /// modèle est relu à chaque fois (4 à 5 s de plus).
    Program(Program),
    /// Le service `sherpa-onnx-offline-websocket-server`, qui garde le
    /// modèle en mémoire, sur cette machine (installé par `--voix`).
    Server(SocketAddr),
}

#[derive(Debug)]
struct Program {
    path: PathBuf,
    encoder: PathBuf,
    decoder: PathBuf,
    tokens: PathBuf,
    /// Parakeet : son troisième fichier. `None` : Whisper.
    joiner: Option<PathBuf>,
    threads: usize,
}

/// L'écoute, prête.
#[derive(Debug)]
pub struct Stt {
    engine: Engine,
    turn: Semaphore,
}

impl Stt {
    /// D'après le réglage `HA_ECOUTE` :
    /// - `ws://127.0.0.1:port` : le service d'écoute, sur cette machine
    ///   seulement (il n'a pas de mot de passe) ;
    /// - sinon, le dossier du programme (voir [`Stt::from_dir`]).
    pub fn from_settings(setting: &str) -> Result<Self, SttError> {
        let setting = setting.trim();
        let Some(address) = setting.strip_prefix("ws://") else {
            return Self::from_dir(Path::new(setting));
        };
        let address = address.trim_end_matches('/');
        let address: SocketAddr = address
            .replacen("localhost:", "127.0.0.1:", 1)
            .parse()
            .map_err(|_| {
                SttError::Config(format!(
                    "HA_ECOUTE : adresse invalide « {address} » (attendu : ws://127.0.0.1:port)"
                ))
            })?;
        if !address.ip().is_loopback() {
            return Err(SttError::Config(
                "HA_ECOUTE : le service d'écoute doit tourner sur cette machine (ws://127.0.0.1:port)"
                    .into(),
            ));
        }
        Ok(Self {
            engine: Engine::Server(address),
            turn: Semaphore::new(1),
        })
    }

    /// D'après un dossier qui contient le programme et les fichiers du
    /// modèle ([`PROGRAM`], [`ENCODER`], [`DECODER`], [`TOKENS`], et
    /// [`JOINER`] pour Parakeet).
    pub fn from_dir(dir: &Path) -> Result<Self, SttError> {
        let file = |name: &str| {
            let path = dir.join(name);
            if path.is_file() {
                Ok(path)
            } else {
                Err(SttError::Config(format!(
                    "HA_ECOUTE : fichier introuvable : {}",
                    path.display()
                )))
            }
        };
        // Au-delà de quatre processeurs, Whisper ne va guère plus vite.
        let threads = std::thread::available_parallelism()
            .map_or(2, std::num::NonZero::get)
            .clamp(1, 4);
        Ok(Self {
            engine: Engine::Program(Program {
                path: file(PROGRAM)?,
                encoder: file(ENCODER)?,
                decoder: file(DECODER)?,
                tokens: file(TOKENS)?,
                joiner: Some(dir.join(JOINER)).filter(|path| path.is_file()),
                threads,
            }),
            turn: Semaphore::new(1),
        })
    }

    /// Pour le journal : « Whisper, service sherpa-onnx (ws://127.0.0.1:10350) ».
    #[must_use]
    pub fn describe(&self) -> String {
        match &self.engine {
            Engine::Program(program) => format!(
                "{}, programme sherpa-onnx ({} processeurs)",
                if program.joiner.is_some() {
                    "Parakeet"
                } else {
                    "Whisper"
                },
                program.threads
            ),
            Engine::Server(address) => format!("service d'écoute sherpa-onnx (ws://{address})"),
        }
    }

    /// Le texte dit dans `wav`. Une seule phrase à la fois : les suivantes
    /// attendent leur tour ([`WAIT_TURN`] au plus).
    pub async fn transcribe(&self, wav: &[u8]) -> Result<String, SttError> {
        let samples = check_wav(wav)?;
        if wav::is_silent(samples) {
            return Err(SttError::Silence);
        }
        let _turn = tokio::time::timeout(WAIT_TURN, self.turn.acquire())
            .await
            .map_err(|_| SttError::Busy)?
            .map_err(|_| SttError::Busy)?;
        let output = match &self.engine {
            Engine::Program(program) => {
                let file = TempWav::write(wav).await?;
                tokio::time::timeout(TIMEOUT, program.run(&file.path))
                    .await
                    .map_err(|_| SttError::Timeout)??
            }
            Engine::Server(address) => {
                tokio::time::timeout(TIMEOUT, server::transcribe(*address, samples))
                    .await
                    .map_err(|_| SttError::Timeout)??
            }
        };
        let text =
            parse_output(&output).ok_or_else(|| SttError::Failed("réponse illisible".into()))?;
        clean(&text).ok_or(SttError::Silence)
    }
}

impl Program {
    async fn run(&self, wav: &Path) -> Result<String, SttError> {
        let mut command = Command::new(&self.path);
        if let Some(joiner) = &self.joiner {
            command
                .arg(arg("--encoder=", &self.encoder))
                .arg(arg("--decoder=", &self.decoder))
                .arg(arg("--joiner=", joiner))
                .arg("--model-type=nemo_transducer");
        } else {
            command
                .arg(arg("--whisper-encoder=", &self.encoder))
                .arg(arg("--whisper-decoder=", &self.decoder))
                .arg("--whisper-language=fr")
                .arg("--whisper-task=transcribe");
        }
        command
            .arg(arg("--tokens=", &self.tokens))
            .arg(format!("--num-threads={}", self.threads))
            .arg(wav)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            // Une phrase abandonnée (délai dépassé) arrête le programme.
            .kill_on_drop(true);
        #[cfg(windows)]
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        let mut child = command.spawn().map_err(SttError::Start)?;
        let mut bytes = Vec::new();
        if let Some(stdout) = child.stdout.take() {
            stdout
                .take(MAX_OUTPUT)
                .read_to_end(&mut bytes)
                .await
                .map_err(SttError::Start)?;
        }
        let status = child.wait().await.map_err(SttError::Start)?;
        if !status.success() {
            return Err(SttError::Failed(format!("code de sortie {status}")));
        }
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }
}

/// `--tokens=chemin`.
fn arg(name: &str, path: &Path) -> std::ffi::OsString {
    let mut out = std::ffi::OsString::from(name);
    out.push(path.as_os_str());
    out
}

/// Le programme écrit, en dernière ligne, `{"lang": "fr", …, "text": "…"}`.
fn parse_output(output: &str) -> Option<String> {
    output.lines().rev().find_map(|line| {
        let line = line.trim();
        if !line.starts_with('{') {
            return None;
        }
        let json: serde_json::Value = serde_json::from_str(line).ok()?;
        json.get("text")?.as_str().map(str::to_owned)
    })
}

/// Ce que Whisper « entend » parfois dans un bruit de fond : des
/// génériques de sous-titres appris dans ses vidéos d'entraînement.
const HALLUCINATIONS: &[&str] = &["amara", "sous-titr", "sous titr", "merci d'avoir regard"];

/// Le texte sur une ligne, sans caractère de contrôle, [`MAX_TEXT`]
/// caractères au plus. `None` : rien de dit (ou une hallucination).
#[must_use]
pub fn clean(text: &str) -> Option<String> {
    let text: String = text
        .split(|c: char| c.is_whitespace() || c.is_control())
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(MAX_TEXT)
        .collect();
    let lower = text.to_lowercase();
    if !text.chars().any(char::is_alphanumeric) || HALLUCINATIONS.iter().any(|h| lower.contains(h))
    {
        return None;
    }
    Some(text)
}

/// Le son, le temps de l'écoute : dans le dossier temporaire (privé au
/// service sous systemd, `PrivateTmp`), lisible par le serveur seul, effacé
/// à la fin, même en cas d'erreur.
struct TempWav {
    path: PathBuf,
}

impl TempWav {
    async fn write(wav: &[u8]) -> Result<Self, SttError> {
        let mut random = [0u8; 12];
        getrandom::fill(&mut random)
            .map_err(|error| SttError::Failed(format!("aléatoire indisponible : {error}")))?;
        let name = random.iter().fold(String::new(), |mut name, b| {
            let _ = std::fmt::Write::write_fmt(&mut name, format_args!("{b:02x}"));
            name
        });
        let path = std::env::temp_dir().join(format!("ha-ecoute-{name}.wav"));
        let mut options = tokio::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);
        let mut file = options.open(&path).await.map_err(SttError::Start)?;
        let me = Self { path };
        tokio::io::AsyncWriteExt::write_all(&mut file, wav)
            .await
            .map_err(SttError::Start)?;
        Ok(me)
    }
}

impl Drop for TempWav {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]

    use super::*;

    #[test]
    fn the_output_gives_the_text() {
        let output = "/chemin/phrase.wav\n{\"lang\": \"fr\", \"emotion\": \"\", \"event\": \"\", \
                      \"text\": \" Ferme le volet du salon.\", \"timestamps\": [], \"tokens\": []}\n";
        assert_eq!(
            parse_output(output).as_deref(),
            Some(" Ferme le volet du salon.")
        );
        assert_eq!(parse_output("rien\n"), None);
        assert_eq!(
            clean(" Ferme le\nvolet. ").as_deref(),
            Some("Ferme le volet.")
        );
        assert_eq!(clean(" … "), None);
        assert_eq!(
            clean("Sous-titres réalisés par la communauté d'Amara.org"),
            None
        );
        assert_eq!(clean(&"a".repeat(400)).unwrap().len(), MAX_TEXT);
    }

    #[test]
    fn a_missing_file_is_named() {
        let dir = std::env::temp_dir().join(format!("ha-stt-vide-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let error = Stt::from_dir(&dir).unwrap_err().to_string();
        assert!(error.contains(PROGRAM), "{error}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn the_program_hears_the_sentence() {
        use std::os::unix::fs::PermissionsExt;

        let dir = std::env::temp_dir().join(format!("ha-stt-prog-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for name in [ENCODER, DECODER, TOKENS] {
            std::fs::write(dir.join(name), b"").unwrap();
        }
        let seen = dir.join("vu.txt");
        // Un faux sherpa-onnx : il note ses arguments et la taille du son,
        // puis répond comme le vrai.
        std::fs::write(
            dir.join(PROGRAM),
            format!(
                "#!/bin/sh\nfor a in \"$@\"; do last=\"$a\"; echo \"$a\"; done > '{seen}'\n\
                 wc -c < \"$last\" >> '{seen}'\n\
                 echo \"$last\"\n\
                 echo '{{\"lang\": \"fr\", \"text\": \" Allume la lampe.\"}}'\n",
                seen = seen.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(dir.join(PROGRAM), std::fs::Permissions::from_mode(0o755))
            .unwrap();
        let stt = Stt::from_dir(&dir).unwrap();
        let wav = wav::tests::tone(16_000, 1.0);
        assert_eq!(stt.transcribe(&wav).await.unwrap(), "Allume la lampe.");
        let seen = std::fs::read_to_string(&seen).unwrap();
        assert!(seen.contains("--whisper-language=fr"), "{seen}");
        assert!(
            seen.contains(&format!("--tokens={}", dir.join(TOKENS).display())),
            "{seen}"
        );
        assert!(seen.trim_end().ends_with(&wav.len().to_string()), "{seen}");
        // Le fichier temporaire est effacé.
        let temp = seen.lines().find(|l| l.contains("ha-ecoute-")).unwrap();
        assert!(!Path::new(temp).exists());

        // Un silence n'est pas écouté.
        let quiet = wav::tests::tone(16_000, 0.0);
        assert!(matches!(
            stt.transcribe(&quiet).await,
            Err(SttError::Silence)
        ));
        // Avec le fichier de Parakeet : les réglages de Parakeet.
        std::fs::write(dir.join(JOINER), b"").unwrap();
        let parakeet = Stt::from_dir(&dir).unwrap();
        assert!(parakeet.describe().starts_with("Parakeet"));
        assert_eq!(parakeet.transcribe(&wav).await.unwrap(), "Allume la lampe.");
        let seen = std::fs::read_to_string(dir.join("vu.txt")).unwrap();
        assert!(seen.contains("--model-type=nemo_transducer"), "{seen}");
        assert!(
            seen.contains(&format!("--joiner={}", dir.join(JOINER).display())),
            "{seen}"
        );
        assert!(!seen.contains("--whisper"), "{seen}");
        // Un programme en erreur : une erreur claire.
        std::fs::write(dir.join(PROGRAM), "#!/bin/sh\nexit 3\n").unwrap();
        let error = stt.transcribe(&wav).await.unwrap_err().to_string();
        assert!(error.contains("code de sortie"), "{error}");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
