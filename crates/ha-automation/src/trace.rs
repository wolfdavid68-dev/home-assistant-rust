//! L'historique des exécutions (les « traces » de Home Assistant) : pour
//! chaque automatisation, ses dernières exécutions, avec la raison du
//! déclenchement, le résultat de chaque condition et chaque action.
//!
//! Il répond à la question qu'on se pose toujours : « pourquoi la lumière
//! ne s'est-elle pas allumée ? ».
//!
//! Les traces sont gardées **en mémoire** (comme Home Assistant par défaut) :
//! elles disparaissent au redémarrage. Les données envoyées par les actions
//! (`data`) ne sont jamais enregistrées : elles peuvent contenir un code.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use jiff::Timestamp;

/// Exécutions gardées par automatisation.
pub const STORED_TRACES: usize = 10;
/// Étapes enregistrées par exécution, au plus : une répétition de 100 tours
/// ne doit pas remplir la mémoire.
pub const MAX_STEPS: usize = 200;

/// Une exécution d'une automatisation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trace {
    /// Numéro de l'exécution (croissant, propre à ce démarrage du serveur).
    pub run: u64,
    pub alias: String,
    pub started: Timestamp,
    /// `None` tant que l'exécution n'est pas finie.
    pub finished: Option<Timestamp>,
    /// La raison du déclenchement : « Porte d'entrée passe à « on » ».
    pub trigger: Option<String>,
    /// Les conditions vérifiées, dans l'ordre (on s'arrête à la première fausse).
    pub conditions: Vec<Check>,
    pub steps: Vec<Step>,
    /// Des étapes n'ont pas été enregistrées (plus de [`MAX_STEPS`]).
    pub truncated: bool,
    pub outcome: Outcome,
}

/// Une condition et son résultat.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    pub description: String,
    pub passed: bool,
}

/// Une action exécutée.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    /// Sa place : `"2"`, ou `"3 › alors › 1"` dans un `if`.
    pub path: String,
    pub description: String,
    pub at: Timestamp,
    pub status: StepStatus,
    /// Un complément : la branche choisie, le nombre de tours, l'erreur…
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepStatus {
    /// L'action a été faite.
    Done,
    /// L'action a été refusée (appel de service invalide…).
    Failed,
    /// L'action a arrêté l'exécution (condition fausse, attente dépassée).
    Stopped,
}

/// Comment l'exécution s'est terminée.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// En cours (ou en attente de son tour, en mode `queued`).
    Running,
    /// Toutes les actions ont été faites.
    Finished,
    /// Une condition de l'automatisation était fausse : rien n'a été fait.
    ConditionsFailed,
    /// Une action `condition` (ou une attente dépassée) a arrêté l'exécution.
    Stopped,
    /// Pas lancée : déjà en cours (mode `single`).
    AlreadyRunning,
    /// Pas lancée : trop d'exécutions en cours ou en attente.
    TooMany,
    /// Pas lancée : protection contre les boucles.
    Loop,
    /// Interrompue : relancée (mode `restart`), modifiée, ou serveur arrêté.
    Aborted,
}

impl Outcome {
    /// Le nom utilisé par l'API.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Finished => "finished",
            Self::ConditionsFailed => "conditions_failed",
            Self::Stopped => "stopped",
            Self::AlreadyRunning => "already_running",
            Self::TooMany => "too_many",
            Self::Loop => "loop",
            Self::Aborted => "aborted",
        }
    }
}

impl StepStatus {
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Stopped => "stopped",
        }
    }
}

/// Toutes les traces, partagées entre le moteur et l'API.
///
/// Concept Rust : `Arc<Mutex<…>>` est le duo classique pour partager une
/// donnée modifiable entre plusieurs tâches. `Arc` compte les propriétaires,
/// `Mutex` garantit qu'une seule tâche la modifie à la fois. On utilise le
/// `Mutex` de la bibliothèque standard (et non celui de tokio) : on ne garde
/// jamais le verrou pendant un `.await`.
#[derive(Debug, Clone, Default)]
pub struct Traces {
    inner: Arc<Inner>,
}

#[derive(Debug, Default)]
struct Inner {
    /// Identifiant de l'automatisation → ses dernières exécutions (la plus
    /// récente à la fin).
    traces: Mutex<HashMap<String, VecDeque<Trace>>>,
    next_run: AtomicU64,
}

impl Traces {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Le verrou, même si une tâche a paniqué en le tenant : les traces ne
    /// sont qu'un historique, mieux vaut les garder que tout bloquer.
    fn lock(&self) -> MutexGuard<'_, HashMap<String, VecDeque<Trace>>> {
        self.inner
            .traces
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Les exécutions d'une automatisation, la plus récente en premier.
    #[must_use]
    pub fn of(&self, automation: &str) -> Vec<Trace> {
        self.lock()
            .get(automation)
            .map(|list| list.iter().rev().cloned().collect())
            .unwrap_or_default()
    }

    /// La dernière exécution de chaque automatisation.
    #[must_use]
    pub fn latest(&self) -> HashMap<String, Trace> {
        self.lock()
            .iter()
            .filter_map(|(id, list)| list.back().map(|trace| (id.clone(), trace.clone())))
            .collect()
    }

    /// Oublie les automatisations qui n'existent plus.
    pub fn retain(&self, keep: impl Fn(&str) -> bool) {
        self.lock().retain(|id, _| keep(id));
    }

    /// Commence l'enregistrement d'une exécution.
    #[must_use]
    pub fn begin(
        &self,
        automation: &str,
        alias: &str,
        trigger: Option<String>,
        conditions: Vec<Check>,
    ) -> Recorder {
        let run = self.inner.next_run.fetch_add(1, Ordering::Relaxed) + 1;
        let trace = Trace {
            run,
            alias: alias.to_owned(),
            started: Timestamp::now(),
            finished: None,
            trigger,
            conditions,
            steps: Vec::new(),
            truncated: false,
            outcome: Outcome::Running,
        };
        let mut traces = self.lock();
        let list = traces.entry(automation.to_owned()).or_default();
        list.push_back(trace);
        while list.len() > STORED_TRACES {
            list.pop_front();
        }
        drop(traces);
        Recorder {
            traces: self.clone(),
            automation: automation.to_owned(),
            run,
            finished: false,
        }
    }

    /// Modifie la trace `run` de `automation`, si elle est encore gardée.
    fn update(&self, automation: &str, run: u64, change: impl FnOnce(&mut Trace)) {
        if let Some(trace) = self
            .lock()
            .get_mut(automation)
            .and_then(|list| list.iter_mut().find(|trace| trace.run == run))
        {
            change(trace);
        }
    }
}

/// Enregistre une exécution en cours.
///
/// Concept Rust : `Drop`. Quand l'exécution est interrompue (`abort()` en
/// mode `restart`, automatisation modifiée, serveur arrêté), sa tâche est
/// détruite **sans** atteindre la fin de son code. Mais tout ce qu'elle
/// possédait est détruit proprement, et `drop` est appelé : c'est là qu'on
/// note « interrompue ». Aucune exécution ne reste « en cours » pour toujours.
#[derive(Debug)]
pub struct Recorder {
    traces: Traces,
    automation: String,
    run: u64,
    finished: bool,
}

impl Recorder {
    /// Ajoute une étape.
    pub fn step(
        &self,
        path: &str,
        at: Timestamp,
        description: String,
        status: StepStatus,
        detail: Option<String>,
    ) {
        self.traces.update(&self.automation, self.run, |trace| {
            if trace.steps.len() >= MAX_STEPS {
                trace.truncated = true;
                return;
            }
            trace.steps.push(Step {
                path: path.to_owned(),
                description,
                at,
                status,
                detail,
            });
        });
    }

    /// Termine l'enregistrement.
    pub fn finish(mut self, outcome: Outcome) {
        self.close(outcome);
    }

    fn close(&mut self, outcome: Outcome) {
        self.finished = true;
        self.traces.update(&self.automation, self.run, |trace| {
            trace.outcome = outcome;
            trace.finished = Some(Timestamp::now());
        });
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        if !self.finished {
            self.close(Outcome::Aborted);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_last_runs_are_kept() {
        let traces = Traces::new();
        for _ in 0..STORED_TRACES + 3 {
            traces
                .begin("a", "A", None, Vec::new())
                .finish(Outcome::Finished);
        }
        let list = traces.of("a");
        assert_eq!(list.len(), STORED_TRACES);
        // La plus récente d'abord.
        assert_eq!(list.first().map(|t| t.run), Some(STORED_TRACES as u64 + 3));
        assert_eq!(traces.of("b"), Vec::<Trace>::new());
    }

    #[test]
    fn an_interrupted_run_is_marked_aborted() {
        let traces = Traces::new();
        let recorder = traces.begin("a", "A", Some("il est 07:30".into()), Vec::new());
        recorder.step(
            "1",
            Timestamp::now(),
            "attendre 5 min".into(),
            StepStatus::Done,
            None,
        );
        assert_eq!(traces.of("a")[0].outcome, Outcome::Running);
        drop(recorder);
        let trace = &traces.of("a")[0];
        assert_eq!(trace.outcome, Outcome::Aborted);
        assert!(trace.finished.is_some());
        assert_eq!(trace.steps.len(), 1);
    }

    #[test]
    fn steps_are_limited() {
        let traces = Traces::new();
        let recorder = traces.begin("a", "A", None, Vec::new());
        for _ in 0..MAX_STEPS + 5 {
            recorder.step("1", Timestamp::now(), "x".into(), StepStatus::Done, None);
        }
        recorder.finish(Outcome::Finished);
        let trace = &traces.of("a")[0];
        assert_eq!(trace.steps.len(), MAX_STEPS);
        assert!(trace.truncated);
        assert_eq!(trace.outcome, Outcome::Finished);
    }
}
