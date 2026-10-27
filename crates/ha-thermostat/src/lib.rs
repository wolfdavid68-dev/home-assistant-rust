//! **Thermostat**, comme le `generic_thermostat` de Home Assistant : il
//! allume et éteint un radiateur (une prise connectée ou un relais) d'après
//! un capteur de température.
//!
//! Chaque thermostat de `thermostats.yaml` devient une entité
//! `climate.<id>`, pilotable depuis le tableau de bord ou une automatisation :
//!
//! | Action | Données |
//! |--------|---------|
//! | `climate.set_temperature` | `temperature: 20` |
//! | `climate.set_preset_mode` | `preset_mode: eco` |
//! | `climate.set_hvac_mode` | `hvac_mode: heat` ou `off` |
//! | `climate.turn_on` / `climate.turn_off` | — |
//!
//! Sécurité : température inconnue, fenêtre ouverte ou température maximale
//! atteinte → le radiateur est **coupé**, immédiatement.

pub mod config;
pub mod logic;

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use ha_core::{Attributes, Background, CancellationToken, Core, EntityId, Event, State};
use serde_json::{Value, json};
use tokio::sync::broadcast::error::RecvError;
use tracing::{info, warn};

pub use config::{Thermostat, load, parse};
use logic::{Inputs, Mode, Reason};

/// Toutes les 30 secondes, on revérifie (pour un changement que la durée
/// minimale avait fait attendre).
const TICK: Duration = Duration::from_secs(30);
/// Un ordre non suivi d'effet (radiateur injoignable) est renvoyé au plus
/// une fois par minute.
const RESEND_AFTER: Duration = Duration::from_mins(1);

/// Les thermostats, en marche.
#[derive(Debug)]
pub struct Thermostats(Background);

impl Thermostats {
    pub fn start(thermostats: Vec<Thermostat>, core: Arc<Core>) -> Self {
        Self::start_with_tick(thermostats, core, TICK)
    }

    /// Comme [`Thermostats::start`], avec une autre période (pour les tests).
    pub fn start_with_tick(thermostats: Vec<Thermostat>, core: Arc<Core>, tick: Duration) -> Self {
        let events = core.bus.subscribe();
        let live: Vec<Live> = thermostats
            .into_iter()
            .filter_map(|config| Live::restore(config, &core))
            .collect();
        Self(Background::spawn("les thermostats", move |token| {
            run(live, core, events, tick, token)
        }))
    }

    pub async fn stop(self) {
        self.0.stop().await;
    }
}

/// Un thermostat et ce qui change pendant qu'il tourne.
struct Live {
    config: Thermostat,
    entity: EntityId,
    mode: Mode,
    target: f64,
    preset: Option<String>,
    /// Dernier ordre envoyé au radiateur, et quand.
    last_command: Option<(bool, tokio::time::Instant)>,
}

impl Live {
    /// Reprend la consigne, le mode et le préréglage d'avant l'arrêt (l'état
    /// est rechargé depuis la base au démarrage). Au tout premier
    /// démarrage : arrêté, à la consigne du fichier.
    fn restore(config: Thermostat, core: &Core) -> Option<Self> {
        let entity = config.entity_id()?;
        let previous = core.states.get(&entity);
        let attributes = previous.as_deref().map(State::attributes);
        let mode = previous
            .as_ref()
            .and_then(|state| Mode::parse(state.state()))
            .unwrap_or(Mode::Off);
        let target = attributes
            .and_then(|a| a.get("temperature"))
            .and_then(Value::as_f64)
            .map_or(config.target, |t| config.clamp(t));
        let preset = attributes
            .and_then(|a| a.get("preset_mode"))
            .and_then(Value::as_str)
            .filter(|p| config.presets.contains_key(*p))
            .map(str::to_owned);
        Some(Self {
            config,
            entity,
            mode,
            target,
            preset,
            last_command: None,
        })
    }

    fn watches(&self, entity_id: &EntityId) -> bool {
        entity_id == &self.config.sensor
            || entity_id == &self.config.heater
            || self.config.window.as_ref() == Some(entity_id)
    }

    /// Un ordre venu du tableau de bord ou d'une automatisation.
    fn on_call(&mut self, service: &str, data: &Attributes) {
        let name = &self.config.name;
        match service {
            "set_temperature" => {
                let Some(temperature) = data.get("temperature").and_then(Value::as_f64) else {
                    warn!("{name} : « temperature » attendue");
                    return;
                };
                self.target = self.config.clamp(temperature);
                // Une consigne manuelle n'est plus un préréglage.
                self.preset = None;
                if let Some(mode) = data
                    .get("hvac_mode")
                    .and_then(Value::as_str)
                    .and_then(Mode::parse)
                {
                    self.mode = mode;
                }
                info!("{name} : consigne {} °C", self.target);
            }
            "set_hvac_mode" => {
                if let Some(mode) = data
                    .get("hvac_mode")
                    .and_then(Value::as_str)
                    .and_then(Mode::parse)
                {
                    self.mode = mode;
                } else {
                    warn!("{name} : « hvac_mode » doit valoir heat ou off");
                }
            }
            "set_preset_mode" => {
                let preset = data
                    .get("preset_mode")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                if preset == "none" {
                    self.preset = None;
                } else if let Some(value) = self.config.presets.get(preset) {
                    self.target = *value;
                    self.preset = Some(preset.to_owned());
                    info!("{name} : préréglage {preset} ({value} °C)");
                } else {
                    warn!("{name} : préréglage inconnu {preset:?}");
                }
            }
            "turn_on" => self.mode = Mode::Heat,
            "turn_off" => self.mode = Mode::Off,
            _ => {}
        }
    }

    /// Décide, commande le radiateur si besoin, et publie l'état.
    ///
    /// `forced` : quelqu'un vient de donner un ordre (mise en marche, nouvelle
    /// consigne). On obéit tout de suite, sans attendre la durée minimale
    /// entre deux changements, comme dans Home Assistant.
    fn evaluate(&mut self, core: &Core, forced: bool) {
        let sensor = core.states.get(&self.config.sensor);
        let heater = core.states.get(&self.config.heater);
        let current = sensor
            .as_ref()
            .and_then(|s| s.state().parse::<f64>().ok())
            .filter(|t| (-50.0..=60.0).contains(t));
        let window_open = self
            .config
            .window
            .as_ref()
            .and_then(|w| core.states.get(w))
            .is_some_and(|w| w.state() == "on");
        let heater_on = heater.as_ref().is_some_and(|h| h.state() == "on");
        let (want, reason) = logic::decide(Inputs {
            mode: self.mode,
            target: self.target,
            tolerance: self.config.tolerance,
            max_temp: self.config.max_temp,
            current,
            window_open,
            heater_on,
        });

        if want != heater_on {
            // Depuis quand le radiateur est-il dans son état actuel ?
            let since = heater
                .as_ref()
                .and_then(|h| SystemTime::now().duration_since(h.last_changed()).ok());
            let already_sent = self
                .last_command
                .is_some_and(|(sent, at)| sent == want && at.elapsed() < RESEND_AFTER);
            let allowed = forced || logic::may_switch(reason, since, self.config.min_cycle);
            if allowed && !already_sent {
                self.command(core, want, reason, current);
            }
        } else if self.last_command.is_some_and(|(sent, _)| sent == heater_on) {
            // Le radiateur a obéi : l'ordre est terminé. (S'il n'a pas obéi,
            // on garde l'ordre en mémoire pour ne pas le renvoyer en rafale.)
            self.last_command = None;
        }
        self.publish(core, current, heater_on, reason);
    }

    fn command(&mut self, core: &Core, on: bool, reason: Reason, current: Option<f64>) {
        let service = if on { "turn_on" } else { "turn_off" };
        let temperature = current.map_or_else(|| "inconnue".to_owned(), |t| format!("{t} °C"));
        info!(
            "{} : {} {} (température {temperature}, consigne {} °C, {reason:?})",
            self.config.name,
            if on { "allume" } else { "éteint" },
            self.config.heater,
            self.target
        );
        if let Err(error) = core.call_service(
            "switch",
            service,
            self.config.heater.clone(),
            Attributes::new(),
        ) {
            warn!("{} : ordre refusé ({error})", self.config.name);
        }
        self.last_command = Some((on, tokio::time::Instant::now()));
    }

    fn publish(&self, core: &Core, current: Option<f64>, heater_on: bool, reason: Reason) {
        let action = match (self.mode, heater_on) {
            (Mode::Off, _) => "off",
            (Mode::Heat, true) => "heating",
            (Mode::Heat, false) => "idle",
        };
        let config = &self.config;
        let mut attributes = match json!({
            "friendly_name": config.name,
            "current_temperature": current,
            "temperature": self.target,
            "min_temp": config.min_temp,
            "max_temp": config.max_temp,
            "target_temp_step": 0.5,
            "hvac_modes": ["heat", "off"],
            "hvac_action": action,
            "preset_mode": self.preset.as_deref().unwrap_or("none"),
            "preset_modes": config.presets.keys().collect::<Vec<_>>(),
            "info": reason.info(),
        }) {
            Value::Object(map) => map,
            _ => Attributes::new(),
        };
        attributes.retain(|_, value| !value.is_null());
        if let Err(error) = core
            .states
            .set(self.entity.clone(), self.mode.as_str(), attributes)
        {
            warn!("{} : état refusé ({error})", config.name);
        }
    }
}

async fn run(
    mut thermostats: Vec<Live>,
    core: Arc<Core>,
    mut events: tokio::sync::broadcast::Receiver<Arc<Event>>,
    tick: Duration,
    token: CancellationToken,
) {
    info!("{} thermostat(s)", thermostats.len());
    for thermostat in &mut thermostats {
        thermostat.evaluate(&core, false);
    }
    let mut ticker = tokio::time::interval(tick);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            () = token.cancelled() => break,
            received = events.recv() => match received {
                Ok(event) => match event.as_ref() {
                    Event::StateChanged { entity_id, .. } => {
                        for thermostat in thermostats.iter_mut().filter(|t| t.watches(entity_id)) {
                            thermostat.evaluate(&core, false);
                        }
                    }
                    Event::CallService { domain, service, entity_id, data, .. } if domain == "climate" => {
                        if let Some(thermostat) = thermostats.iter_mut().find(|t| &t.entity == entity_id) {
                            thermostat.on_call(service, data);
                            thermostat.evaluate(&core, true);
                        }
                    }
                    Event::CallService { .. } | Event::Fired { .. } => {}
                },
                Err(RecvError::Lagged(missed)) => {
                    warn!("thermostats : {missed} événements manqués, on revérifie tout");
                    for thermostat in &mut thermostats {
                        thermostat.evaluate(&core, false);
                    }
                }
                Err(RecvError::Closed) => break,
            },
            _ = ticker.tick() => {
                for thermostat in &mut thermostats {
                    thermostat.evaluate(&core, false);
                }
            }
        }
    }
    info!("thermostats arrêtés");
}
