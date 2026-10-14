//! Machine à états : connaît l'état actuel de chaque entité et publie un
//! événement `StateChanged` à chaque changement.

use std::collections::HashMap;
use std::sync::{PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::time::SystemTime;

use crate::entity_id::EntityId;
use crate::error::CoreError;
use crate::event::{Event, EventBus};
use crate::state::{Attributes, MAX_STATE_LEN, State};

/// Concept Rust : plusieurs tâches vont lire et modifier les états en même
/// temps. Rust refuse de compiler un accès concurrent non protégé. Le `RwLock`
/// (verrou lecteurs/rédacteur) autorise soit plusieurs lecteurs, soit un seul
/// rédacteur à la fois.
#[derive(Debug)]
pub struct StateMachine {
    states: RwLock<HashMap<EntityId, State>>,
    bus: EventBus,
}

impl StateMachine {
    #[must_use]
    pub fn new(bus: EventBus) -> Self {
        Self {
            states: RwLock::new(HashMap::new()),
            bus,
        }
    }

    /// Renvoie une copie de l'état actuel d'une entité, ou `None` si elle n'existe pas.
    ///
    /// Concept Rust : `Option<T>` remplace `null`. Le compilateur oblige
    /// l'appelant à traiter le cas `None`.
    pub fn get(&self, entity_id: &EntityId) -> Option<State> {
        self.read().get(entity_id).cloned()
    }

    /// Tous les états, triés par identifiant.
    pub fn all(&self) -> Vec<State> {
        let mut states: Vec<State> = self.read().values().cloned().collect();
        states.sort_by(|a, b| a.entity_id().cmp(b.entity_id()));
        states
    }

    /// Crée ou met à jour l'état d'une entité.
    ///
    /// Renvoie `Ok(true)` si quelque chose a changé (un événement est alors
    /// publié), `Ok(false)` si l'état et les attributs étaient déjà identiques.
    pub fn set(
        &self,
        entity_id: EntityId,
        new_state: impl Into<String>,
        attributes: Attributes,
    ) -> Result<bool, CoreError> {
        let new_state: String = new_state.into();
        check_state_len(&entity_id, &new_state)?;

        let now = SystemTime::now();
        let mut states = self.write();
        let old = states.get(&entity_id);

        let last_changed = match old {
            Some(old) if old.state() == new_state && old.attributes() == &attributes => {
                return Ok(false);
            }
            // Seuls les attributs changent : la date de changement de valeur est conservée.
            Some(old) if old.state() == new_state => old.last_changed(),
            _ => now,
        };

        let state = State::new(entity_id.clone(), new_state, attributes, last_changed, now);
        let old_state = states.insert(entity_id.clone(), state.clone());

        // On publie pendant que le verrou est tenu : ainsi les événements sortent
        // toujours dans l'ordre réel des changements. `publish` ne bloque jamais.
        self.bus.publish(Event::StateChanged {
            entity_id,
            old_state,
            new_state: Some(state),
        });
        Ok(true)
    }

    /// Remet en place un état sauvegardé (au démarrage), avec ses dates d'origine.
    ///
    /// Contrairement à [`StateMachine::set`], aucun événement n'est publié : rien
    /// n'a réellement changé, on retrouve simplement ce qu'on savait avant l'arrêt.
    /// Une entité déjà présente n'est pas écrasée : un état « vivant » est
    /// toujours plus récent qu'un état sauvegardé. Renvoie `Ok(true)` si l'état
    /// a été restauré.
    pub fn restore(
        &self,
        entity_id: EntityId,
        state: impl Into<String>,
        attributes: Attributes,
        last_changed: SystemTime,
        last_updated: SystemTime,
    ) -> Result<bool, CoreError> {
        let state: String = state.into();
        check_state_len(&entity_id, &state)?;

        let mut states = self.write();
        if states.contains_key(&entity_id) {
            return Ok(false);
        }
        let restored = State::new(
            entity_id.clone(),
            state,
            attributes,
            last_changed,
            last_updated,
        );
        states.insert(entity_id, restored);
        Ok(true)
    }

    /// Supprime une entité. Renvoie `true` si elle existait.
    pub fn remove(&self, entity_id: &EntityId) -> bool {
        let mut states = self.write();
        let Some(old_state) = states.remove(entity_id) else {
            return false;
        };
        self.bus.publish(Event::StateChanged {
            entity_id: entity_id.clone(),
            old_state: Some(old_state),
            new_state: None,
        });
        true
    }

    // Un verrou est « empoisonné » si une tâche a paniqué en le tenant. Nos
    // modifications sont faites en une seule opération (`insert` / `remove`),
    // donc les données restent cohérentes : on peut continuer sans planter.
    fn read(&self) -> RwLockReadGuard<'_, HashMap<EntityId, State>> {
        self.states.read().unwrap_or_else(PoisonError::into_inner)
    }

    fn write(&self) -> RwLockWriteGuard<'_, HashMap<EntityId, State>> {
        self.states.write().unwrap_or_else(PoisonError::into_inner)
    }
}

fn check_state_len(entity_id: &EntityId, state: &str) -> Result<(), CoreError> {
    let len = state.chars().count();
    if len > MAX_STATE_LEN {
        return Err(CoreError::StateTooLong {
            entity_id: entity_id.to_string(),
            len,
            max: MAX_STATE_LEN,
        });
    }
    Ok(())
}
