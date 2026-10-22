//! Le programme `piper`, de deux façons :
//! - **gardé en marche** ([`Resident`]) : Piper charge sa voix une fois
//!   (≈ 0,6 s), puis dit chaque phrase en 0,1 à 0,3 s. On lui écrit la
//!   phrase sur son entrée ; il écrit un fichier WAV dans un dossier à lui
//!   et donne son chemin sur sa sortie ;
//! - **lancé pour une phrase** ([`speak`]), si le premier ne marche pas : il
//!   rend le son brut (`--output_raw` : 16 bits, mono) sur sa sortie.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

use crate::{MAX_AUDIO_BYTES, Pcm, TtsError};

/// La fréquence de la voix quand son fichier `.onnx.json` ne la donne pas
/// (celle des voix « medium » et « low »).
const DEFAULT_RATE: u32 = 22_050;

fn command(program: &Path, model: &Path) -> Command {
    let mut command = Command::new(program);
    command
        .arg("--model")
        .arg(model)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        // Si l'annonce est abandonnée (délai dépassé), Piper est arrêté.
        .kill_on_drop(true);
    // Sous Windows, pas de fenêtre noire qui s'ouvre à chaque annonce.
    #[cfg(windows)]
    command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    command
}

/// Piper gardé en marche, avec son dossier de sortie (vidé et retiré à
/// l'arrêt).
#[derive(Debug)]
pub(crate) struct Resident {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    dir: PathBuf,
}

/// Une phrase dite par Piper gardé en marche dure au plus ceci.
const RESIDENT_TIMEOUT: Duration = Duration::from_secs(20);

impl Resident {
    /// Démarre Piper (il charge sa voix pendant qu'on attend la première
    /// phrase).
    pub(crate) fn start(program: &Path, model: &Path) -> Result<Self, TtsError> {
        let dir = private_dir()?;
        let mut command = command(program, model);
        command.arg("--output_dir").arg(&dir);
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(error) => {
                let _ = std::fs::remove_dir_all(&dir);
                return Err(TtsError::Start(error));
            }
        };
        let (Some(input), Some(output)) = (child.stdin.take(), child.stdout.take()) else {
            let _ = std::fs::remove_dir_all(&dir);
            return Err(TtsError::Silent("pas d'entrée ni de sortie".into()));
        };
        Ok(Self {
            child,
            input,
            output: BufReader::new(output),
            dir,
        })
    }

    /// Dit une phrase (sur une seule ligne).
    pub(crate) async fn speak(&mut self, text: &str) -> Result<Pcm, TtsError> {
        tokio::time::timeout(RESIDENT_TIMEOUT, self.speak_inner(text))
            .await
            .map_err(|_| TtsError::Timeout)?
    }

    async fn speak_inner(&mut self, text: &str) -> Result<Pcm, TtsError> {
        self.input
            .write_all(format!("{text}\n").as_bytes())
            .await
            .map_err(TtsError::Start)?;
        self.input.flush().await.map_err(TtsError::Start)?;
        let mut line = String::new();
        let read = (&mut self.output)
            .take(4096)
            .read_line(&mut line)
            .await
            .map_err(TtsError::Start)?;
        if read == 0 {
            return Err(TtsError::Silent("Piper s'est arrêté".into()));
        }
        // On ne garde que le nom du fichier, lu dans notre dossier : jamais un
        // autre chemin.
        let name = line
            .trim()
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or_default()
            .to_owned();
        let wav = Path::new(&name)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("wav"));
        if name.is_empty() || name.starts_with('.') || !wav || name.len() > 64 {
            return Err(TtsError::Protocol("fichier inattendu".into()));
        }
        let path = self.dir.join(name);
        let bytes = tokio::fs::read(&path).await.map_err(TtsError::Start);
        let _ = tokio::fs::remove_file(&path).await;
        let bytes = bytes?;
        if bytes.len() > MAX_AUDIO_BYTES + 1024 {
            return Err(TtsError::Protocol("son trop long".into()));
        }
        read_wav(&bytes)
    }

    /// Piper tourne-t-il encore ?
    pub(crate) fn alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }
}

impl Drop for Resident {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// La place de Piper gardé en marche, partagée par toutes les copies de la
/// voix.
#[derive(Debug, Default)]
pub(crate) struct Slot {
    resident: Option<Resident>,
    /// Après un démarrage raté, on n'essaie pas avant ce moment.
    retry_after: Option<Instant>,
    /// Une phrase a été abandonnée en route (délai dépassé) : Piper a
    /// peut-être encore une réponse en attente, on le relance.
    dirty: bool,
}

/// Après un démarrage raté, on attend ceci avant de réessayer.
const RETRY: Duration = Duration::from_mins(1);

impl Slot {
    /// Démarre Piper s'il ne tourne pas (il charge sa voix tout de suite).
    pub(crate) fn ensure(
        &mut self,
        program: &Path,
        model: &Path,
    ) -> Result<&mut Resident, TtsError> {
        if self.dirty {
            self.resident = None;
            self.dirty = false;
        }
        if self
            .resident
            .as_mut()
            .is_some_and(|resident| !resident.alive())
        {
            self.resident = None;
        }
        if self.resident.is_none() {
            if self.retry_after.is_some_and(|at| Instant::now() < at) {
                return Err(TtsError::Silent("Piper n'a pas pu rester en marche".into()));
            }
            match Resident::start(program, model) {
                Ok(resident) => {
                    self.retry_after = None;
                    self.resident = Some(resident);
                }
                Err(error) => {
                    self.retry_after = Some(Instant::now() + RETRY);
                    return Err(error);
                }
            }
        }
        self.resident
            .as_mut()
            .ok_or_else(|| TtsError::Silent("Piper absent".into()))
    }

    /// Dit une phrase avec Piper gardé en marche.
    pub(crate) async fn speak(
        &mut self,
        program: &Path,
        model: &Path,
        text: &str,
    ) -> Result<Pcm, TtsError> {
        self.ensure(program, model)?;
        self.dirty = true;
        let result = match self.resident.as_mut() {
            Some(resident) => resident.speak(text).await,
            None => Err(TtsError::Silent("Piper absent".into())),
        };
        self.dirty = false;
        if result.is_err() {
            // Un Piper qui ne sait pas rester en marche (trop ancien) : une
            // phrase à la fois pendant un moment, sans le relancer à chaque fois.
            self.resident = None;
            self.retry_after = Some(Instant::now() + RETRY);
        }
        result
    }
}

/// Les dossiers laissés par un serveur arrêté brutalement (son numéro de
/// processus n'existe plus) : retirés. Sous Linux seulement (`/proc`).
fn remove_stale_dirs() {
    if !Path::new("/proc/self").exists() {
        return;
    }
    let Ok(entries) = std::fs::read_dir(std::env::temp_dir()) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(pid) = name
            .to_str()
            .and_then(|name| name.strip_prefix("ha-piper-"))
            .and_then(|rest| rest.split('-').next())
            .and_then(|pid| pid.parse::<u32>().ok())
        else {
            continue;
        };
        if pid != std::process::id() && !Path::new(&format!("/proc/{pid}")).exists() {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

/// Un dossier rien qu'à nous, pour les fichiers de Piper.
fn private_dir() -> Result<PathBuf, TtsError> {
    remove_stale_dirs();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    let dir = std::env::temp_dir().join(format!("ha-piper-{}-{nanos:08x}", std::process::id()));
    #[cfg(unix)]
    let builder = {
        let mut builder = std::fs::DirBuilder::new();
        std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
        builder
    };
    // Sous Windows, le dossier temporaire est déjà propre au compte.
    #[cfg(not(unix))]
    let builder = std::fs::DirBuilder::new();
    // Échoue si le dossier existe déjà : on ne réutilise jamais celui d'un autre.
    builder.create(&dir).map_err(TtsError::Start)?;
    Ok(dir)
}

/// Le son d'un fichier WAV de Piper (16 bits).
pub(crate) fn read_wav(bytes: &[u8]) -> Result<Pcm, TtsError> {
    let bad = |why: &str| TtsError::Protocol(format!("WAV illisible : {why}"));
    if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(bad("en-tête"));
    }
    let mut at = 12;
    let mut format = None;
    while at + 8 <= bytes.len() {
        let id = &bytes[at..at + 4];
        let size = usize::try_from(u32::from_le_bytes([
            bytes[at + 4],
            bytes[at + 5],
            bytes[at + 6],
            bytes[at + 7],
        ]))
        .map_err(|_| bad("taille"))?;
        let body = at + 8;
        let end = body.saturating_add(size).min(bytes.len());
        if id == b"fmt " && end >= body + 16 {
            let channels = u16::from_le_bytes([bytes[body + 2], bytes[body + 3]]);
            let rate = u32::from_le_bytes([
                bytes[body + 4],
                bytes[body + 5],
                bytes[body + 6],
                bytes[body + 7],
            ]);
            let bits = u16::from_le_bytes([bytes[body + 14], bytes[body + 15]]);
            if bits != 16 || !(1..=2).contains(&channels) || !(8_000..=96_000).contains(&rate) {
                return Err(bad("format"));
            }
            format = Some((rate, channels));
        } else if id == b"data" {
            let (rate, channels) = format.ok_or_else(|| bad("format absent"))?;
            if end - body < 2 {
                return Err(TtsError::Silent("son vide".into()));
            }
            return Ok(Pcm::from_le_bytes(rate, channels, &bytes[body..end]));
        }
        at = body.saturating_add(size + size % 2);
    }
    Err(bad("pas de son"))
}

pub(crate) async fn speak(program: &Path, model: &Path, text: &str) -> Result<Pcm, TtsError> {
    let rate = model_rate(model);
    let mut command = command(program, model);
    command.arg("--output_raw");
    let mut child = command.spawn().map_err(TtsError::Start)?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(format!("{text}\n").as_bytes())
            .await
            .map_err(TtsError::Start)?;
        // Fermer l'entrée dit à Piper qu'il n'y a pas d'autre phrase.
        drop(stdin);
    }
    let mut bytes = Vec::new();
    if let Some(stdout) = child.stdout.take() {
        let limit = u64::try_from(MAX_AUDIO_BYTES).unwrap_or(u64::MAX);
        stdout
            .take(limit)
            .read_to_end(&mut bytes)
            .await
            .map_err(TtsError::Start)?;
    }
    let status = child.wait().await.map_err(TtsError::Start)?;
    if bytes.len() < 2 {
        return Err(TtsError::Silent(format!("code de sortie {status}")));
    }
    Ok(Pcm::from_le_bytes(rate, 1, &bytes))
}

/// Piper range la fréquence de chaque voix dans `voix.onnx.json`
/// (`{"audio": {"sample_rate": 22050}}`).
fn model_rate(model: &Path) -> u32 {
    let mut name = model.as_os_str().to_owned();
    name.push(".json");
    std::fs::read(Path::new(&name))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .and_then(|json| json.pointer("/audio/sample_rate")?.as_u64())
        .and_then(|rate| u32::try_from(rate).ok())
        .filter(|rate| (8_000..=96_000).contains(rate))
        .unwrap_or(DEFAULT_RATE)
}

#[cfg(all(test, unix))]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]

    use std::os::unix::fs::PermissionsExt;

    use crate::{Engine, Tts};

    /// Un faux Piper : un script qui lit la phrase et rend quatre
    /// échantillons, à la fréquence lue dans le `.onnx.json`.
    #[tokio::test]
    async fn the_program_reads_the_text_and_returns_sound() {
        let dir = std::env::temp_dir().join(format!("ha-tts-prog-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let program = dir.join("piper");
        let heard = dir.join("entendu.txt");
        std::fs::write(
            &program,
            format!(
                "#!/bin/sh\ncase \"$*\" in *--output_dir*) exit 1;; esac\ncat > '{}'\nprintf '\\001\\000\\002\\000\\003\\000\\004\\000'\n",
                heard.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        let model = dir.join("voix.onnx");
        std::fs::write(&model, b"").unwrap();
        std::fs::write(
            dir.join("voix.onnx.json"),
            br#"{"audio":{"sample_rate":16000}}"#,
        )
        .unwrap();

        let tts = Tts::new(Engine::Program {
            program: program.clone(),
            model,
        });
        let pcm = tts.speak("Le linge\nest fini").await.unwrap();
        assert_eq!(pcm.rate, 16_000);
        assert_eq!(pcm.channels, 1);
        assert_eq!(pcm.samples, vec![1, 2, 3, 4]);
        assert_eq!(
            std::fs::read_to_string(&heard).unwrap(),
            "Le linge est fini\n"
        );

        // Gardé en marche : le faux Piper n'a pas `--output_dir`, la voix
        // retombe sur un Piper par phrase (ci-dessus). Un programme qui ne
        // dit rien : une erreur claire.
        std::fs::write(&program, "#!/bin/sh\ncat > /dev/null\nexit 3\n").unwrap();
        let error = tts.speak("Bonjour").await.unwrap_err();
        assert!(error.to_string().contains("rien dit"), "{error}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Un faux Piper gardé en marche : pour chaque ligne, un fichier WAV
    /// dans `--output_dir`, et son chemin sur la sortie. Il compte ses
    /// démarrages : il ne doit démarrer qu'une fois pour trois phrases.
    #[tokio::test]
    async fn a_resident_piper_says_each_line() {
        let dir = std::env::temp_dir().join(format!("ha-tts-res-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let program = dir.join("piper");
        let starts = dir.join("demarrages");
        std::fs::write(
            &program,
            format!(
                r#"#!/bin/sh
echo x >> '{starts}'
out=""
raw=0
while [ $# -gt 0 ]; do
  case "$1" in
    --output_dir) out="$2"; shift ;;
    --output_raw) raw=1 ;;
  esac
  shift
done
if [ "$raw" = 1 ]; then cat > /dev/null; printf '\001\000'; exit 0; fi
n=0
while read -r line; do
  n=$((n+1))
  f="$out/$n.wav"
  printf 'RIFF\044\000\000\000WAVEfmt \020\000\000\000\001\000\001\000\200\076\000\000\000\175\000\000\002\000\020\000data\004\000\000\000\007\000\010\000' > "$f"
  echo "$f"
done
"#,
                starts = starts.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        let model = dir.join("voix.onnx");
        std::fs::write(&model, b"").unwrap();
        let tts = Tts::new(Engine::Program {
            program: program.clone(),
            model,
        });
        tts.warm_up().await;
        let copy = tts.clone();
        for text in ["Bonjour", "D'accord", "C'est fait"] {
            let pcm = copy.speak(text).await.unwrap();
            assert_eq!(pcm.rate, 16_000);
            assert_eq!(pcm.samples, vec![7, 8]);
        }
        assert_eq!(std::fs::read_to_string(&starts).unwrap().lines().count(), 1);
        drop(tts);
        drop(copy);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn wav_files_are_read() {
        let mut wav = b"RIFF\0\0\0\0WAVEfmt ".to_vec();
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&[1, 0, 1, 0]);
        wav.extend_from_slice(&22_050u32.to_le_bytes());
        wav.extend_from_slice(&44_100u32.to_le_bytes());
        wav.extend_from_slice(&[2, 0, 16, 0]);
        wav.extend_from_slice(b"LIST");
        wav.extend_from_slice(&3u32.to_le_bytes());
        wav.extend_from_slice(b"abc\0");
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&4u32.to_le_bytes());
        wav.extend_from_slice(&[1, 0, 2, 0]);
        let pcm = super::read_wav(&wav).unwrap();
        assert_eq!(
            (pcm.rate, pcm.channels, pcm.samples.clone()),
            (22_050, 1, vec![1, 2])
        );
        assert!(super::read_wav(b"pas un wav").is_err());
        assert!(super::read_wav(&wav[..30]).is_err());
    }
}
