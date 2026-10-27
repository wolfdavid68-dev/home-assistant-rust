//! Outils ponctuels : le relais Alexa (`alexa relay`) et les essais
//! (`vigilance test`, `sonos`, `voix`, `ecoute`).

use std::error::Error;

/// `ha-server alexa relay` : le relais, sur la machine, devant un serveur
/// lancé dans Docker Desktop (voir `ha_alexa::relay`). Tourne jusqu'à Ctrl+C.
pub async fn run_alexa_relay(
    settings: ha_alexa::relay::RelaySettings,
    stop: impl std::future::Future<Output = std::io::Result<()>>,
) -> Result<(), Box<dyn Error>> {
    let target = settings.target;
    let relay = ha_alexa::relay::Relay::start(settings).await?;
    println!(
        "Relais Alexa : l'Echo trouvera le pont sur http://{}/, relayé vers {target}. Ctrl+C pour arrêter.",
        relay.address
    );
    let stopped = stop.await;
    relay.stop().await;
    Ok(stopped?)
}

/// `ha-server vigilance test` : interroge Météo-France une fois et affiche
/// la vigilance du département.
pub async fn test_vigilance(
    config: &ha_weather::vigilance::VigilanceConfig,
) -> Result<(), Box<dyn Error>> {
    let department = config.department().as_str();
    let map = ha_weather::vigilance::fetch_once(config).await?;
    let zone = jiff::tz::TimeZone::system();
    let local = |time: jiff::Timestamp| time.to_zoned(zone.clone()).strftime("%d/%m à %H:%M");
    println!("Clé acceptée. Carte publiée le {}.", local(map.updated));
    let now = jiff::Timestamp::now();
    let Some(today) = map.at(now) else {
        println!("La carte ne couvre pas l'heure actuelle (elle sera bientôt republiée).");
        return Ok(());
    };
    let mut periods = vec![("Aujourd'hui", today)];
    if let Some(tomorrow) = map.after(today) {
        periods.push(("Demain", tomorrow));
    }
    for (label, period) in periods {
        println!(
            "{label} ({department}) : vigilance {}",
            period.level.name().to_lowercase()
        );
        for hazard in &period.hazards {
            if hazard.level > ha_weather::carte::Level::Green {
                let coast = if hazard.coast { " (littoral)" } else { "" };
                println!(
                    "  - {}{coast} : {}",
                    hazard.phenomenon.name(),
                    hazard.level.name().to_lowercase()
                );
            }
        }
    }
    Ok(())
}

/// `sonos` : les enceintes trouvées, par groupe, et le nom de leur entité.
pub(crate) async fn list_sonos(hosts: &[String]) -> Result<(), Box<dyn Error>> {
    if hosts.is_empty() {
        println!("recherche des enceintes Sonos sur le réseau local (3 s)…");
    }
    let groups = ha_sonos::find(hosts).await?;
    for group in &groups {
        let names: Vec<String> = group
            .members
            .iter()
            .map(|m| {
                let slug = ha_core::slugify(&m.name);
                format!("{} (media_player.{slug}, {})", m.name, m.host)
            })
            .collect();
        if names.len() == 1 {
            println!("- {}", names[0]);
        } else {
            println!("- groupe : {}", names.join(" + "));
        }
    }
    println!(
        "{} enceinte(s). Pour le serveur : HA_SONOS={}",
        groups.iter().map(|g| g.members.len()).sum::<usize>(),
        groups
            .first()
            .and_then(|g| g.coordinator())
            .map_or("auto", |m| m.host.trim_end_matches(":1400"))
    );
    Ok(())
}

/// `voix "<phrase>"` : fait dire la phrase à Piper, et l'écrit dans
/// `essai-voix.wav` (avec le carillon), à écouter sur le PC.
pub(crate) async fn test_voice(tts: &ha_tts::Tts, phrase: &str) -> Result<(), Box<dyn Error>> {
    println!("voix : {}", tts.describe());
    let started = std::time::Instant::now();
    let voice = tts.speak(phrase).await?;
    let sound = ha_tts::announcement(true, Some(&voice));
    std::fs::write("essai-voix.wav", &sound.wav)?;
    println!(
        "essai-voix.wav écrit : {:.1} s de son, préparé en {:.1} s",
        sound.duration.as_secs_f64(),
        started.elapsed().as_secs_f64()
    );
    Ok(())
}

/// `ecoute <fichier.wav>` : fait écouter un enregistrement (16 bits, mono,
/// 16 000 Hz) à Whisper, comme une phrase dite à la tablette.
pub(crate) async fn test_listening(stt: &ha_stt::Stt, file: &str) -> Result<(), Box<dyn Error>> {
    println!("écoute : {}", stt.describe());
    let wav = std::fs::read(file)?;
    let started = std::time::Instant::now();
    let text = stt.transcribe(&wav).await?;
    println!(
        "entendu en {:.1} s : « {text} »",
        started.elapsed().as_secs_f64()
    );
    Ok(())
}
