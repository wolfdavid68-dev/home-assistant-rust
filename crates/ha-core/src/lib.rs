//! Noyau domotique : bus d'événements, machine à états et entités.
//!
//! Concept Rust : chaque `mod` correspond à un fichier dans `src/`. Les `pub use`
//! remettent les types principaux à la racine, pour écrire `ha_core::EntityId`
//! au lieu de `ha_core::entity_id::EntityId`.

pub mod checks;
pub mod commands;
mod entity_id;
mod error;
mod event;
pub mod guard;
pub mod images;
pub mod integrations;
pub mod latency;
mod origin;
mod state;
mod state_machine;
pub mod task;

pub use commands::{COMMAND_TTL, CommandResult, Commands};
pub use entity_id::{EntityId, MAX_ENTITY_ID_LEN, slugify};
pub use error::CoreError;
pub use event::{DEFAULT_BUS_CAPACITY, Event, EventBus};
pub use guard::{Guard, HeldOrder, Policy, QuietHours};
pub use integrations::{Health, IntegrationStatus, Integrations, supervise};
pub use origin::Origin;
pub use state::{Attributes, MAX_STATE_LEN, State};
pub use state_machine::StateMachine;
pub use task::Background;
pub use tokio_util::sync::CancellationToken;

/// Un nom simple (domaine, service, identifiant d'automatisation…) :
/// 1 à 64 caractères parmi `a-z`, `0-9` et `_`.
///
/// Ces noms viennent souvent de l'extérieur (API, fichiers) et finissent
/// dans des logs : on n'accepte rien d'autre.
#[must_use]
pub fn is_valid_name(name: &str) -> bool {
    (1..=64).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

/// Un nom d'événement, comme `timer.finished` : 1 à 64 caractères parmi
/// `a-z`, `0-9`, `_` et `.`, sans point au début ni à la fin.
#[must_use]
pub fn is_valid_event_type(name: &str) -> bool {
    (1..=64).contains(&name.len())
        && !name.starts_with('.')
        && !name.ends_with('.')
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'.')
}

/// Version du noyau, reprise de `Cargo.toml`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Le noyau complet : un bus d'événements et la machine à états qui publie dessus.
///
/// Pour le partager entre plusieurs tâches, on l'enveloppe dans un `Arc<Core>`.
#[derive(Debug)]
pub struct Core {
    pub bus: EventBus,
    pub states: StateMachine,
    /// La santé des intégrations (voir [`integrations`]).
    pub integrations: Integrations,
    /// Les ordres en attente de réponse (voir [`commands`]).
    pub commands: Commands,
    /// Les pièces protégées et les heures calmes (voir [`guard`]).
    pub guard: Guard,
    /// La rapidité des appareils (voir [`latency`]).
    pub latencies: latency::Latencies,
    /// La dernière image de chaque caméra (voir [`images`]).
    pub images: std::sync::Arc<images::Images>,
}

impl Core {
    #[must_use]
    pub fn new() -> Self {
        let bus = EventBus::default();
        let states = StateMachine::new(bus.clone());
        Self {
            bus,
            states,
            integrations: Integrations::default(),
            commands: Commands::default(),
            guard: Guard::default(),
            latencies: latency::Latencies::default(),
            images: std::sync::Arc::default(),
        }
    }
}

impl Core {
    /// Publie un appel de service sur le bus (voir [`Event::CallService`]).
    ///
    /// Le domaine et le service sont validés comme les identifiants : ils
    /// viennent souvent de l'extérieur (API) et finissent dans des logs.
    pub fn call_service(
        &self,
        domain: &str,
        service: &str,
        entity_id: EntityId,
        data: Attributes,
    ) -> Result<(), CoreError> {
        self.call_service_from(Origin::System, domain, service, entity_id, data)
    }

    /// Comme [`Core::call_service`], en disant qui le demande.
    pub fn call_service_from(
        &self,
        origin: Origin,
        domain: &str,
        service: &str,
        entity_id: EntityId,
        data: Attributes,
    ) -> Result<(), CoreError> {
        self.publish_call(origin, domain, service, entity_id, data, None)
    }

    /// Comme [`Core::call_service_from`], en attendant la réponse : le
    /// numéro de l'ordre, et de quoi recevoir ce que l'intégration en dit
    /// (voir [`commands`]). Il faut appeler [`Core::forget_command`] si on
    /// renonce à attendre.
    pub fn call_service_tracked(
        &self,
        origin: Origin,
        domain: &str,
        service: &str,
        entity_id: EntityId,
        data: Attributes,
    ) -> Result<(u64, tokio::sync::oneshot::Receiver<CommandResult>), CoreError> {
        check_service_names(domain, service)?;
        // Vérifié avant d'ouvrir l'attente : un ordre refusé n'en laisse pas.
        self.check_call(domain, service, &entity_id, &data)?;
        let (id, reply) = self.commands.open();
        self.publish_call(origin, domain, service, entity_id, data, Some(id))?;
        Ok((id, reply))
    }

    fn publish_call(
        &self,
        origin: Origin,
        domain: &str,
        service: &str,
        entity_id: EntityId,
        data: Attributes,
        command: Option<u64>,
    ) -> Result<(), CoreError> {
        check_service_names(domain, service)?;
        self.check_call(domain, service, &entity_id, &data)?;
        self.bus.publish(Event::CallService {
            domain: domain.to_owned(),
            service: service.to_owned(),
            entity_id,
            data,
            origin,
            command,
            issued_at: std::time::Instant::now(),
        });
        Ok(())
    }

    /// Vérifie qu'un ordre convient à l'appareil (voir [`checks`]) : fait
    /// avant chaque envoi, et avant qu'un ordre soit retenu par la garde.
    pub fn check_call(
        &self,
        domain: &str,
        service: &str,
        entity_id: &EntityId,
        data: &Attributes,
    ) -> Result<(), CoreError> {
        let Some(state) = self.states.get(entity_id) else {
            return Ok(());
        };
        checks::check(domain, service, data, &state).map_err(|reason| {
            let device = state
                .attributes()
                .get("friendly_name")
                .and_then(serde_json::Value::as_str)
                .map_or_else(|| entity_id.to_string(), str::to_owned);
            CoreError::Refused { device, reason }
        })
    }

    /// Publie un événement nommé (voir [`Event::Fired`]).
    pub fn fire(&self, event_type: &str, data: Attributes) -> Result<(), CoreError> {
        if !is_valid_event_type(event_type) {
            return Err(CoreError::InvalidEventType(event_type.to_owned()));
        }
        self.bus.publish(Event::Fired {
            event_type: event_type.to_owned(),
            data,
        });
        Ok(())
    }
}

fn check_service_names(domain: &str, service: &str) -> Result<(), CoreError> {
    for name in [domain, service] {
        if !is_valid_name(name) {
            return Err(CoreError::InvalidServiceName(name.to_owned()));
        }
    }
    Ok(())
}

impl Default for Core {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_events_are_checked_and_published() {
        let core = Core::new();
        let mut events = core.bus.subscribe();
        for good in ["timer.finished", "mon_evenement", "a.b.c"] {
            assert!(core.fire(good, Attributes::new()).is_ok(), "{good}");
        }
        for bad in ["", "Timer.finished", ".x", "x.", "a b", &"a".repeat(65)] {
            assert!(core.fire(bad, Attributes::new()).is_err(), "{bad}");
        }
        let Ok(event) = events.try_recv() else {
            panic!("un événement est attendu");
        };
        assert_eq!(
            *event,
            Event::Fired {
                event_type: "timer.finished".into(),
                data: Attributes::new()
            }
        );
    }
}
