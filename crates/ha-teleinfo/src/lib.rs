//! Le compteur **Linky** (télé-information client, TIC) : consommation par
//! index (heures creuses, heures pleines, Tempo…), puissance, intensité,
//! production solaire, couleurs Tempo du jour et du lendemain.
//!
//! Deux façons de brancher le compteur :
//!
//! - un module USB sur la machine du serveur (µTeleinfo, Teleinfo USB…) :
//!   `HA_TELEINFO=/dev/ttyUSB0` ;
//! - un module réseau qui renvoie la liaison série telle quelle (ser2net,
//!   ESP en « pont série ») : `HA_TELEINFO=tcp://192.168.1.50:3333`.
//!
//! (Un module qui publie déjà sur MQTT, comme ESPHome, Tasmota ou un ZLinky,
//! n'a besoin de rien ici : ses capteurs arrivent par la découverte MQTT.)
//!
//! Le mode (historique à 1200 bauds, ou standard à 9600) est reconnu tout
//! seul ; `HA_TELEINFO_MODE=historique` ou `standard` l'impose.
//!
//! Le compteur ne fait qu'**envoyer** : on ne peut rien lui commander. Le
//! numéro du compteur (qui identifie le foyer) n'est jamais montré en entier.

pub mod frame;
pub mod meter;

use ha_core::Health;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use std::time::SystemTime;

use ha_core::{Attributes, Background, CancellationToken, Core, EntityId};
use tokio::sync::mpsc;
use tokio::time::{Instant, MissedTickBehavior};
use tracing::{info, warn};

pub use frame::{Decoder, Frame, Mode};
pub use meter::{Color, Reading, Tempo};

/// Sans trame valide pendant ce temps, les capteurs passent « indisponible ».
const SILENCE: Duration = Duration::from_mins(1);
/// En mode automatique, on essaie l'autre vitesse après ce temps sans trame.
const TRY_OTHER_MODE_AFTER: Duration = Duration::from_secs(15);
/// Délai avant de rouvrir une source perdue (câble débranché, module éteint).
const RETRY_AFTER: Duration = Duration::from_secs(10);
/// Les index ne sont publiés qu'une fois par minute (une trame arrive toutes
/// les 1 à 2 secondes : l'historique serait noyé).
const ENERGY_EVERY: Duration = Duration::from_mins(1);
/// La puissance et l'intensité : toutes les 10 secondes au plus.
const POWER_EVERY: Duration = Duration::from_secs(10);

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum TeleinfoError {
    #[error(
        "source TIC invalide : « /dev/ttyUSB0 » (module USB) ou « tcp://adresse:port » (module réseau)"
    )]
    Source,
    #[error("mode TIC invalide : « auto », « historique » ou « standard »")]
    Mode,
}

/// D'où viennent les octets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// Un port série local (`/dev/ttyUSB0`).
    Serial(PathBuf),
    /// Un module réseau : `adresse:port`.
    Tcp(String),
}

/// Le réglage du mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModeChoice {
    Auto,
    Fixed(Mode),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub source: Source,
    pub mode: ModeChoice,
}

impl Config {
    /// Lit la configuration (`HA_TELEINFO`, `HA_TELEINFO_MODE`).
    pub fn parse(source: &str, mode: Option<&str>) -> Result<Self, TeleinfoError> {
        let source = source.trim();
        let source = if let Some(address) = source.strip_prefix("tcp://") {
            let (host, port) = address.rsplit_once(':').ok_or(TeleinfoError::Source)?;
            let host_ok = !host.is_empty()
                && host.len() <= 253
                && host
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':' | '[' | ']'));
            if !host_ok || port.parse::<u16>().map_or(true, |port| port == 0) {
                return Err(TeleinfoError::Source);
            }
            Source::Tcp(address.to_owned())
        } else if source.starts_with("/dev/")
            && source.len() <= 64
            && !source.contains("..")
            && source
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '_' | '-' | '.'))
        {
            Source::Serial(PathBuf::from(source))
        } else {
            return Err(TeleinfoError::Source);
        };
        let mode = match mode.map(str::trim) {
            None | Some("" | "auto") => ModeChoice::Auto,
            Some("historique") => ModeChoice::Fixed(Mode::Historic),
            Some("standard") => ModeChoice::Fixed(Mode::Standard),
            Some(_) => return Err(TeleinfoError::Mode),
        };
        Ok(Self { source, mode })
    }
}

/// La lecture du compteur, en tâche de fond.
#[derive(Debug)]
pub struct Teleinfo(Background);

impl Teleinfo {
    pub fn start(config: Config, core: Arc<Core>) -> Self {
        // Sous surveillance : si la lecture plante, elle est relancée.
        let supervisor = Arc::clone(&core);
        Self(ha_core::supervise(supervisor, ID, NAME, move |token| {
            run(config.clone(), Arc::clone(&core), token)
        }))
    }

    pub async fn stop(self) {
        self.0.stop().await;
    }
}

/// Ouvre la source et renvoie le canal qui reçoit ses octets. Le canal se
/// ferme quand la source est perdue.
fn open(source: &Source, mode: Mode) -> Result<mpsc::Receiver<Vec<u8>>, String> {
    let (sender, receiver) = mpsc::channel(64);
    match source {
        Source::Tcp(address) => {
            let address = address.clone();
            tokio::spawn(async move {
                use tokio::io::AsyncReadExt;
                let mut stream = match tokio::time::timeout(
                    Duration::from_secs(10),
                    tokio::net::TcpStream::connect(&address),
                )
                .await
                {
                    Ok(Ok(stream)) => stream,
                    Ok(Err(error)) => {
                        warn!("Linky : connexion à {address} impossible ({error})");
                        return;
                    }
                    Err(_) => {
                        warn!("Linky : {address} ne répond pas");
                        return;
                    }
                };
                info!("Linky : connecté à {address}");
                let mut buffer = [0_u8; 512];
                loop {
                    match stream.read(&mut buffer).await {
                        Ok(0) | Err(_) => break,
                        Ok(count) => {
                            let bytes = buffer.get(..count).unwrap_or_default().to_vec();
                            if sender.send(bytes).await.is_err() {
                                break;
                            }
                        }
                    }
                }
                warn!("Linky : liaison avec {address} perdue");
            });
        }
        Source::Serial(path) => serial::spawn(path, mode, sender)?,
    }
    Ok(receiver)
}

async fn run(config: Config, core: Arc<Core>, token: CancellationToken) {
    let mut mode = match config.mode {
        ModeChoice::Fixed(mode) => mode,
        // La plupart des Linky sont livrés en mode historique.
        ModeChoice::Auto => Mode::Historic,
    };
    let mut publisher = Publisher::new(core);
    let mut ticker = tokio::time::interval(Duration::from_secs(1));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
    'sources: loop {
        let mut bytes = match open(&config.source, mode) {
            Ok(receiver) => receiver,
            Err(error) => {
                warn!("Linky : {error}");
                core_report_retry(&publisher, &error);
                tokio::select! {
                    () = token.cancelled() => break 'sources,
                    () = tokio::time::sleep(RETRY_AFTER) => continue 'sources,
                }
            }
        };
        let opened = Instant::now();
        let mut decoder = Decoder::new();
        let mut frames_here = 0_u64;
        loop {
            tokio::select! {
                () = token.cancelled() => break 'sources,
                received = bytes.recv() => {
                    let Some(chunk) = received else {
                        core_report_retry(&publisher, "liaison perdue");
                        // Source perdue : on réessaie un peu plus tard.
                        tokio::select! {
                            () = token.cancelled() => break 'sources,
                            () = tokio::time::sleep(RETRY_AFTER) => continue 'sources,
                        }
                    };
                    for frame in decoder.feed(&chunk) {
                        if frames_here == 0 {
                            info!("Linky : trames reçues (mode {})", frame.mode.name());
                            publisher.core.report(ID, NAME, Health::Connected);
                        }
                        frames_here += 1;
                        publisher.publish(&Reading::from_frame(&frame), Instant::now());
                    }
                }
                _ = ticker.tick() => {
                    publisher.check_silence(Instant::now());
                    // Mode automatique sur une liaison série : sans trame, on
                    // essaie l'autre vitesse.
                    if frames_here == 0
                        && config.mode == ModeChoice::Auto
                        && matches!(config.source, Source::Serial(_))
                        && opened.elapsed() >= TRY_OTHER_MODE_AFTER
                    {
                        mode = mode.other();
                        info!("Linky : aucune trame, essai du mode {} ({} bauds)", mode.name(), mode.baud_rate());
                        continue 'sources;
                    }
                }
            }
        }
    }
    info!("Linky : lecture arrêtée");
}

/// L'identifiant et le nom de l'intégration (pour sa santé).
const ID: &str = "teleinfo";
const NAME: &str = "Linky";

/// Le compteur ne répond plus : la lecture réessaie toute seule.
fn core_report_retry(publisher: &Publisher, error: &str) {
    publisher.core.report(
        ID,
        NAME,
        Health::Retrying {
            error: error.to_owned(),
            retry_at: SystemTime::now() + RETRY_AFTER,
        },
    );
}

/// Publie les capteurs, sans noyer l'historique.
struct Publisher {
    core: Arc<Core>,
    /// Clé → (dernière publication, dernière valeur).
    last: HashMap<String, (Instant, String)>,
    last_frame: Option<Instant>,
    silent: bool,
}

/// Les réglages d'un capteur.
struct Sensor<'a> {
    key: &'a str,
    name: String,
    unit: Option<&'a str>,
    device_class: Option<&'a str>,
    state_class: Option<&'a str>,
    every: Duration,
}

/// `12345678` Wh → `"12345.678"` kWh, sans erreur d'arrondi.
fn kwh(wh: u64) -> String {
    format!("{}.{:03}", wh / 1000, wh % 1000)
}

impl Publisher {
    fn new(core: Arc<Core>) -> Self {
        Self {
            core,
            last: HashMap::new(),
            last_frame: None,
            silent: false,
        }
    }

    fn energy(key: &str, name: String) -> Sensor<'_> {
        Sensor {
            key,
            name,
            unit: Some("kWh"),
            device_class: Some("energy"),
            state_class: Some("total_increasing"),
            every: ENERGY_EVERY,
        }
    }

    fn publish(&mut self, reading: &Reading, now: Instant) {
        self.last_frame = Some(now);
        // Après un silence, tout est republié tout de suite.
        if std::mem::take(&mut self.silent) {
            self.last.clear();
        }
        let mut extra = Attributes::new();
        if let Some(mode) = reading.mode {
            extra.insert("mode".into(), mode.name().into());
        }
        for (key, value) in [
            ("option_tarifaire", &reading.tariff),
            ("puissance_souscrite", &reading.subscribed),
            ("compteur", &reading.meter),
        ] {
            if let Some(value) = value {
                extra.insert(key.into(), value.clone().into());
            }
        }
        if let Some(total) = reading.total_wh {
            let sensor = Self::energy("energie", "Linky consommation".into());
            self.set(&sensor, kwh(total), extra, now);
        }
        for index in &reading.indexes {
            let sensor = Self::energy(&index.key, format!("Linky {}", index.name.to_lowercase()));
            self.set(&sensor, kwh(index.wh), Attributes::new(), now);
        }
        if let Some(injected) = reading.injected_wh.filter(|wh| *wh > 0) {
            let sensor = Self::energy("production", "Linky production injectée".into());
            self.set(&sensor, kwh(injected), Attributes::new(), now);
            if let Some(va) = reading.injected_va {
                let sensor = Sensor {
                    key: "puissance_injectee",
                    name: "Linky puissance injectée".into(),
                    unit: Some("VA"),
                    device_class: Some("apparent_power"),
                    state_class: Some("measurement"),
                    every: POWER_EVERY,
                };
                self.set(&sensor, va.to_string(), Attributes::new(), now);
            }
        }
        let measures = [
            (
                "puissance",
                "Linky puissance",
                "VA",
                "apparent_power",
                reading.power_va,
            ),
            (
                "intensite",
                "Linky intensité",
                "A",
                "current",
                reading.current_a,
            ),
            (
                "tension",
                "Linky tension",
                "V",
                "voltage",
                reading.voltage_v,
            ),
        ];
        for (key, name, unit, class, value) in measures {
            if let Some(value) = value {
                let sensor = Sensor {
                    key,
                    name: name.into(),
                    unit: Some(unit),
                    device_class: Some(class),
                    state_class: Some("measurement"),
                    every: POWER_EVERY,
                };
                self.set(&sensor, value.to_string(), Attributes::new(), now);
            }
        }
        if let Some(period) = &reading.period {
            let sensor = Sensor {
                key: "periode",
                name: "Linky période tarifaire".into(),
                unit: None,
                device_class: None,
                state_class: None,
                every: Duration::ZERO,
            };
            self.set(&sensor, period.clone(), Attributes::new(), now);
        }
        if let Some(tempo) = reading.tempo {
            for (key, name, color) in [
                ("tempo_aujourdhui", "Tempo aujourd'hui", tempo.today),
                ("tempo_demain", "Tempo demain", tempo.tomorrow),
            ] {
                let sensor = Sensor {
                    key,
                    name: name.into(),
                    unit: None,
                    device_class: Some("enum"),
                    state_class: None,
                    every: Duration::ZERO,
                };
                let mut attributes = Attributes::new();
                attributes.insert(
                    "options".into(),
                    [Color::Blue, Color::White, Color::Red]
                        .map(Color::name)
                        .to_vec()
                        .into(),
                );
                // Pas encore annoncée : « unknown », comme dans Home Assistant.
                let state = color.map_or("unknown", Color::name);
                self.set(&sensor, state.into(), attributes, now);
            }
        }
    }

    fn set(
        &mut self,
        sensor: &Sensor<'_>,
        value: String,
        mut attributes: Attributes,
        now: Instant,
    ) {
        if let Some((at, previous)) = self.last.get(sensor.key)
            && (*previous == value || now.duration_since(*at) < sensor.every)
        {
            return;
        }
        let Ok(entity_id) = format!("sensor.linky_{}", sensor.key).parse::<EntityId>() else {
            return;
        };
        attributes.insert("friendly_name".into(), sensor.name.clone().into());
        if let Some(unit) = sensor.unit {
            attributes.insert("unit_of_measurement".into(), unit.into());
        }
        if let Some(class) = sensor.device_class {
            attributes.insert("device_class".into(), class.into());
        }
        if let Some(class) = sensor.state_class {
            attributes.insert("state_class".into(), class.into());
        }
        if let Err(error) = self
            .core
            .states
            .set_by(ID, entity_id, value.clone(), attributes)
        {
            warn!("Linky : état refusé ({error})");
            return;
        }
        self.last.insert(sensor.key.to_owned(), (now, value));
    }

    /// Plus de trame depuis une minute : les capteurs passent « indisponible »
    /// (et une alerte de réparation apparaîtra au bout d'une heure).
    fn check_silence(&mut self, now: Instant) {
        if self.silent
            || self
                .last_frame
                .is_none_or(|last| now.duration_since(last) < SILENCE)
        {
            return;
        }
        self.silent = true;
        warn!("Linky : plus aucune trame depuis une minute");
        core_report_retry(self, "plus aucune trame depuis une minute");
        for key in self.last.keys() {
            let Ok(entity_id) = format!("sensor.linky_{key}").parse::<EntityId>() else {
                continue;
            };
            let attributes = self
                .core
                .states
                .get(&entity_id)
                .map(|state| state.attributes().clone())
                .unwrap_or_default();
            if let Err(error) = self.core.states.set(entity_id, "unavailable", attributes) {
                warn!("Linky : état refusé ({error})");
            }
        }
    }
}

/// La liaison série. Concept Rust : `#[cfg(unix)]` compile ce module
/// seulement sous Linux et macOS ; sous Windows, c'est l'autre version.
#[cfg(unix)]
mod serial {
    use std::fs::{File, OpenOptions};
    use std::io::Read;
    use std::os::unix::fs::OpenOptionsExt;
    use std::path::Path;

    use nix::sys::termios::{self, BaudRate, ControlFlags, SetArg, SpecialCharacterIndices};
    use tokio::sync::mpsc;
    use tracing::warn;

    use crate::frame::Mode;

    /// Ouvre le port et le règle comme la TIC l'exige : 7 bits de données,
    /// parité paire, 1 bit d'arrêt, à 1200 ou 9600 bauds.
    ///
    /// La bibliothèque `nix` donne une interface **sûre** (sans `unsafe`)
    /// aux réglages du terminal du système (`termios`).
    fn configure(path: &Path, mode: Mode) -> std::io::Result<File> {
        let file = OpenOptions::new()
            .read(true)
            // Le port ne doit pas devenir le « terminal » du serveur.
            .custom_flags(nix::fcntl::OFlag::O_NOCTTY.bits())
            .open(path)?;
        let mut settings = termios::tcgetattr(&file)?;
        termios::cfmakeraw(&mut settings);
        let speed = match mode {
            Mode::Historic => BaudRate::B1200,
            Mode::Standard => BaudRate::B9600,
        };
        termios::cfsetspeed(&mut settings, speed)?;
        settings.control_flags &=
            !(ControlFlags::CSIZE | ControlFlags::PARODD | ControlFlags::CSTOPB);
        settings.control_flags |=
            ControlFlags::CS7 | ControlFlags::PARENB | ControlFlags::CREAD | ControlFlags::CLOCAL;
        // `read` rend la main au bout d'une seconde, même sans octet : la
        // lecture peut ainsi s'arrêter quand on le lui demande.
        settings.control_chars[SpecialCharacterIndices::VMIN as usize] = 0;
        settings.control_chars[SpecialCharacterIndices::VTIME as usize] = 10;
        termios::tcsetattr(&file, SetArg::TCSANOW, &settings)?;
        Ok(file)
    }

    /// Lit le port dans un fil d'exécution à part (la lecture bloque).
    pub(crate) fn spawn(
        path: &Path,
        mode: Mode,
        sender: mpsc::Sender<Vec<u8>>,
    ) -> Result<(), String> {
        let mut file = configure(path, mode)
            .map_err(|error| format!("ouverture de {} impossible ({error})", path.display()))?;
        let name = path.display().to_string();
        std::thread::Builder::new()
            .name("linky".into())
            .spawn(move || {
                let mut buffer = [0_u8; 256];
                // Le canal fermé (serveur arrêté, mode changé) : on s'arrête.
                while !sender.is_closed() {
                    match file.read(&mut buffer) {
                        Ok(0) => {}
                        Ok(count) => {
                            let bytes = buffer.get(..count).unwrap_or_default().to_vec();
                            if sender.blocking_send(bytes).is_err() {
                                break;
                            }
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                        Err(error) => {
                            warn!("Linky : lecture de {name} impossible ({error})");
                            break;
                        }
                    }
                }
            })
            .map_err(|error| format!("lecture impossible ({error})"))?;
        Ok(())
    }
}

#[cfg(not(unix))]
mod serial {
    use std::path::Path;

    use tokio::sync::mpsc;

    use crate::frame::Mode;

    pub(crate) fn spawn(
        path: &Path,
        _mode: Mode,
        _sender: mpsc::Sender<Vec<u8>>,
    ) -> Result<(), String> {
        Err(format!(
            "{} : la lecture série n'est pas prise en charge sur ce système ; \
             utilise un module réseau (tcp://…) ou MQTT",
            path.display()
        ))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]

    use super::*;

    #[test]
    fn the_configuration_is_checked() {
        assert_eq!(
            Config::parse("/dev/ttyUSB0", None),
            Ok(Config {
                source: Source::Serial("/dev/ttyUSB0".into()),
                mode: ModeChoice::Auto
            })
        );
        assert_eq!(
            Config::parse("tcp://192.168.1.50:3333", Some("standard")).map(|c| (c.source, c.mode)),
            Ok((
                Source::Tcp("192.168.1.50:3333".into()),
                ModeChoice::Fixed(Mode::Standard)
            ))
        );
        for bad in [
            "",
            "ttyUSB0",
            "/etc/passwd",
            "/dev/../etc/passwd",
            "tcp://192.168.1.50",
            "tcp://:3333",
            "tcp://hôte:3333",
            "tcp://192.168.1.50:0",
            "http://192.168.1.50:80",
        ] {
            assert_eq!(
                Config::parse(bad, None),
                Err(TeleinfoError::Source),
                "{bad}"
            );
        }
        assert_eq!(
            Config::parse("/dev/ttyUSB0", Some("rapide")),
            Err(TeleinfoError::Mode)
        );
    }

    #[test]
    fn values_are_throttled_and_marked_unavailable_on_silence() {
        let core = Arc::new(Core::new());
        let mut publisher = Publisher::new(Arc::clone(&core));
        let reading = |wh: u64, va: u64| Reading {
            total_wh: Some(wh),
            power_va: Some(va),
            ..Reading::default()
        };
        let state = |id: &str| {
            core.states
                .get(&id.parse().unwrap())
                .map(|s| s.state().to_owned())
        };
        let start = Instant::now();
        publisher.publish(&reading(1_000, 500), start);
        publisher.publish(&reading(1_001, 600), start + Duration::from_secs(2));
        // Trop tôt : rien ne change.
        assert_eq!(state("sensor.linky_energie").as_deref(), Some("1.000"));
        assert_eq!(state("sensor.linky_puissance").as_deref(), Some("500"));
        publisher.publish(&reading(1_002, 700), start + Duration::from_secs(11));
        assert_eq!(state("sensor.linky_energie").as_deref(), Some("1.000"));
        assert_eq!(state("sensor.linky_puissance").as_deref(), Some("700"));
        publisher.publish(&reading(1_003, 700), start + Duration::from_secs(61));
        assert_eq!(state("sensor.linky_energie").as_deref(), Some("1.003"));

        // Une minute sans trame : indisponible…
        publisher.check_silence(start + Duration::from_secs(100));
        assert_eq!(state("sensor.linky_energie").as_deref(), Some("1.003"));
        publisher.check_silence(start + Duration::from_secs(122));
        assert_eq!(
            state("sensor.linky_energie").as_deref(),
            Some("unavailable")
        );
        assert_eq!(
            state("sensor.linky_puissance").as_deref(),
            Some("unavailable")
        );
        // … et tout revient dès la trame suivante.
        publisher.publish(&reading(1_004, 800), start + Duration::from_secs(123));
        assert_eq!(state("sensor.linky_energie").as_deref(), Some("1.004"));
        assert_eq!(state("sensor.linky_puissance").as_deref(), Some("800"));
    }

    #[test]
    fn tempo_colors_become_two_sensors() {
        let core = Arc::new(Core::new());
        let mut publisher = Publisher::new(Arc::clone(&core));
        let reading = |tomorrow| Reading {
            tempo: Some(Tempo {
                today: Some(Color::Blue),
                tomorrow,
            }),
            ..Reading::default()
        };
        let state = |id: &str| {
            core.states
                .get(&id.parse().unwrap())
                .map(|s| (s.state().to_owned(), s.attributes().clone()))
        };
        let start = Instant::now();
        publisher.publish(&reading(None), start);
        let (today, attributes) = state("sensor.linky_tempo_aujourdhui").unwrap();
        assert_eq!(today, "Bleu");
        assert_eq!(attributes["device_class"], "enum");
        assert_eq!(
            attributes["options"].to_string(),
            r#"["Bleu","Blanc","Rouge"]"#
        );
        assert_eq!(state("sensor.linky_tempo_demain").unwrap().0, "unknown");
        // La couleur de demain arrive : publiée tout de suite.
        publisher.publish(&reading(Some(Color::Red)), start + Duration::from_secs(1));
        assert_eq!(state("sensor.linky_tempo_demain").unwrap().0, "Rouge");
        // Sans contrat Tempo, pas de capteur.
        let other = Arc::new(Core::new());
        Publisher::new(Arc::clone(&other)).publish(&Reading::default(), start);
        assert!(
            other
                .states
                .get(&"sensor.linky_tempo_demain".parse().unwrap())
                .is_none()
        );
    }

    #[test]
    fn energy_is_written_in_kwh_exactly() {
        assert_eq!(kwh(12_345_678), "12345.678");
        assert_eq!(kwh(5), "0.005");
    }
}
