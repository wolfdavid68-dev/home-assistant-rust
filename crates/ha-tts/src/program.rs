//! Le programme `piper` : on lui donne la phrase sur son entrée, il rend le
//! son brut (`--output_raw` : 16 bits, mono) sur sa sortie.

use std::path::Path;
use std::process::Stdio;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;

use crate::{MAX_AUDIO_BYTES, Pcm, TtsError};

/// La fréquence de la voix quand son fichier `.onnx.json` ne la donne pas
/// (celle des voix « medium » et « low »).
const DEFAULT_RATE: u32 = 22_050;

pub(crate) async fn speak(program: &Path, model: &Path, text: &str) -> Result<Pcm, TtsError> {
    let rate = model_rate(model);
    let mut command = Command::new(program);
    command
        .arg("--model")
        .arg(model)
        .arg("--output_raw")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        // Si l'annonce est abandonnée (délai dépassé), Piper est arrêté.
        .kill_on_drop(true);
    // Sous Windows, pas de fenêtre noire qui s'ouvre à chaque annonce.
    #[cfg(windows)]
    command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
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
                "#!/bin/sh\ncat > '{}'\nprintf '\\001\\000\\002\\000\\003\\000\\004\\000'\n",
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

        // Un programme qui ne dit rien : une erreur claire.
        std::fs::write(&program, "#!/bin/sh\ncat > /dev/null\nexit 3\n").unwrap();
        let error = tts.speak("Bonjour").await.unwrap_err();
        assert!(error.to_string().contains("rien dit"), "{error}");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
