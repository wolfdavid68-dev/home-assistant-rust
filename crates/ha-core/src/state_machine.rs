//! Machine à états : connaît l'état actuel de chaque entité et publie un
//! événement `StateChanged` à chaque changement.

use std::collections::HashMap;
use std::sync::{Arc, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::time::SystemTime;

use crate::entity_id::EntityId;
use crate::error::CoreError;
use crate::event::{Event, EventBus};
use crate::state::{Attributes, MAX_STATE_LEN, State};

/// Concept Rust : plusieurs tâches vont lire et modifier les états en même
/// temps. Rust refuse de compiler un accès concurrent non protégé. Le `RwLock`
/// (verrou lecteurs/rédacteur) autorise soit plusieurs lecteurs, soit un seul
/// rédacteur à la fois.
///
/// Chaque état est rangé dans un `Arc` : le lire ne le copie pas (attributs
/// compris), on reçoit une deuxième « poignée » sur le même état. Un état
/// n'est jamais modifié sur place : un changement en range un nouveau, et
/// ceux qui tiennent l'ancien le gardent tel quel.
#[derive(Debug)]
pub struct StateMachine {
    states: RwLock<HashMap<EntityId, Arc<State>>>,
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

    /// Renvoie l'état actuel d'une entité, ou `None` si elle n'existe pas.
    ///
    /// Concept Rust : `Option<T>` remplace `null`. Le compilateur oblige
    /// l'appelant à traiter le cas `None`. L'état n'est pas copié : cloner un
    /// `Arc` ne fait qu'incrémenter un compteur.
    pub fn get(&self, entity_id: &EntityId) -> Option<Arc<State>> {
        self.read().get(entity_id).cloned()
    }

    /// Tous les états, triés par identifiant.
    pub fn all(&self) -> Vec<Arc<State>> {
        let mut states: Vec<Arc<State>> = self.read().values().cloned().collect();
        states.sort_by(|a, b| a.entity_id().cmp(b.entity_id()));
        states
    }

    /// Nombre d'entités connues.
    #[must_use]
    pub fn len(&self) -> usize {
        self.read().len()
    }

    /// Aucune entité connue ?
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.read().is_empty()
    }

    /// Cette entité existe-t-elle ?
    #[must_use]
    pub fn contains(&self, entity_id: &EntityId) -> bool {
        self.read().contains_key(entity_id)
    }

    /// L'entité vaut-elle `value` ? `false` si elle n'existe pas.
    #[must_use]
    pub fn is_state(&self, entity_id: &EntityId, value: &str) -> bool {
        self.read()
            .get(entity_id)
            .is_some_and(|state| state.state() == value)
    }

    /// Lit l'état d'une entité sur place, sans même toucher au compteur de
    /// l'`Arc` : `f` reçoit l'état, et son résultat est renvoyé (`None` si
    /// l'entité n'existe pas).
    ///
    /// Le verrou est tenu pendant `f` : elle doit être courte et ne jamais
    /// rappeler la machine à états (ce serait un interblocage).
    pub fn with<R>(&self, entity_id: &EntityId, f: impl FnOnce(&State) -> R) -> Option<R> {
        self.read().get(entity_id).map(|state| f(state))
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
        self.update(entity_id, new_state.into(), attributes, None)
    }

    /// `owner` : le nouveau propriétaire, ou `None` pour garder l'actuel.
    fn update(
        &self,
        entity_id: EntityId,
        new_state: String,
        attributes: Attributes,
        owner: Option<&str>,
    ) -> Result<bool, CoreError> {
        check_state_len(&entity_id, &new_state)?;

        let now = SystemTime::now();
        let mut states = self.write();
        let old = states.get(&entity_id);
        let owner: Option<Arc<str>> = match owner {
            Some(owner) => Some(Arc::from(owner)),
            None => old.and_then(|old| old.owner.clone()),
        };

        let last_changed = match old {
            Some(old) if old.state() == new_state && old.attributes() == &attributes => {
                // Rien ne change, sauf peut-être le propriétaire : on le note
                // sans publier d'événement.
                if old.owner != owner {
                    let mut kept = State::clone(old);
                    kept.owner = owner;
                    states.insert(entity_id, Arc::new(kept));
                }
                return Ok(false);
            }
            // Seuls les attributs changent : la date de changement de valeur est conservée.
            Some(old) if old.state() == new_state => old.last_changed(),
            _ => now,
        };

        let state = Arc::new(State::new(
            entity_id.clone(),
            new_state,
            attributes,
            last_changed,
            now,
            owner,
        ));
        // L'événement et la table partagent le même état : aucune copie.
        let old_state = states.insert(entity_id.clone(), Arc::clone(&state));

        // On publie pendant que le verrou est tenu : ainsi les événements sortent
        // toujours dans l'ordre réel des changements. `publish` ne bloque jamais.
        self.bus.publish(Event::StateChanged {
            entity_id,
            old_state,
            new_state: Some(state),
        });
        Ok(true)
    }

    /// Comme [`StateMachine::set`], pour une entité publiée par l'intégration
    /// `owner` (`hue`, `mqtt`…), qui en devient propriétaire : si elle tombe
    /// en panne, ses entités passent « indisponibles » (voir
    /// [`crate::integrations`]).
    pub fn set_by(
        &self,
        owner: &str,
        entity_id: EntityId,
        new_state: impl Into<String>,
        attributes: Attributes,
    ) -> Result<bool, CoreError> {
        self.update(entity_id, new_state.into(), attributes, Some(owner))
    }

    /// Les entités publiées par l'intégration `owner`, triées.
    #[must_use]
    pub fn owned_by(&self, owner: &str) -> Vec<EntityId> {
        let mut ids: Vec<EntityId> = self
            .read()
            .values()
            .filter(|state| state.owner() == Some(owner))
            .map(|state| state.entity_id().clone())
            .collect();
        ids.sort();
        ids
    }

    /// L'intégration qui publie cette entité, si on la connaît.
    #[must_use]
    pub fn owner_of(&self, entity_id: &EntityId) -> Option<String> {
        self.read()
            .get(entity_id)
            .and_then(|state| state.owner().map(str::to_owned))
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
        self.restore_owned(
            entity_id,
            state,
            attributes,
            last_changed,
            last_updated,
            None,
        )
    }

    /// Comme [`StateMachine::restore`], avec l'intégration propriétaire.
    pub fn restore_owned(
        &self,
        entity_id: EntityId,
        state: impl Into<String>,
        attributes: Attributes,
        last_changed: SystemTime,
        last_updated: SystemTime,
        owner: Option<&str>,
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
            owner.map(Arc::from),
        );
        states.insert(entity_id, Arc::new(restored));
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
    fn read(&self) -> RwLockReadGuard<'_, HashMap<EntityId, Arc<State>>> {
        self.states.read().unwrap_or_else(PoisonError::into_inner)
    }

    fn write(&self) -> RwLockWriteGuard<'_, HashMap<EntityId, Arc<State>>> {
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
