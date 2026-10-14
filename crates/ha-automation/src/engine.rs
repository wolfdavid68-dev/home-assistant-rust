//! Le moteur : écoute le bus et l'horloge, et exécute les actions.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

use ha_core::{Background, CancellationToken, Core, Event};
use jiff::civil::DateTime;
use tokio::sync::broadcast::error::RecvError;
use tokio::task::JoinHandle;
use tokio::time::{Instant, MissedTickBehavior};
use tracing::{debug, info, warn};

use crate::config::{Action, Automation};
use crate::evaluator::Evaluator;
use ha_sun::Location;

/// Protection contre les boucles : une automatisation qui se déclenche plus
/// de `MAX_RUNS` fois en `RUN_WINDOW` est mise en pause jusqu'à ce que ça se calme.
/// (Exemple de boucle : A allume la lampe quand elle s'éteint, B l'éteint
/// quand elle s'allume…)
const MAX_RUNS: usize = 10;
const RUN_WINDOW: Duration = Duration::from_mins(1);

/// Poignée vers le moteur d'automatisations.
#[derive(Debug)]
pub struct Engine(Background);

impl Engine {
    /// Démarre le moteur. L'abonnement au bus est fait avant de rendre la main.
    pub fn start(automations: Vec<Automation>, core: Arc<Core>) -> Self {
        Self::start_with_location(automations, core, None)
    }

    /// Comme [`Engine::start`], avec la position de la maison pour les
    /// déclencheurs et conditions `sun`.
    pub fn start_with_location(
        automations: Vec<Automation>,
        core: Arc<Core>,
        location: Option<Location>,
    ) -> Self {
        let events = core.bus.subscribe();
        let mut evaluator = Evaluator::new(automations);
        if let Some(location) = location {
            evaluator = evaluator.with_sun(location, jiff::tz::TimeZone::system());
        }
        Self(Background::spawn("le moteur d'automatisations", |token| {
            run(evaluator, core, events, token)
        }))
    }

    /// Arrête le moteur ; les actions en cours (attentes `delay`…) sont abandonnées.
    pub async fn stop(self) {
        self.0.stop().await;
    }
}

/// L'heure locale, selon le fuseau horaire de la machine.
fn local_now() -> DateTime {
    jiff::Zoned::now().datetime()
}

/// Concept Rust : une structure qui regroupe l'état « vivant » du moteur,
/// pour ne pas passer cinq paramètres à chaque fonction.
struct Runner {
    evaluator: Evaluator,
    core: Arc<Core>,
    /// Exécution en cours de chaque automatisation (mode « une à la fois »).
    running: Vec<Option<JoinHandle<()>>>,
    /// Dates des derniers déclenchements, pour la protection contre les boucles.
    recent_runs: Vec<VecDeque<Instant>>,
}

async fn run(
    evaluator: Evaluator,
    core: Arc<Core>,
    mut events: tokio::sync::broadcast::Receiver<Arc<Event>>,
    token: CancellationToken,
) {
    let count = evaluator.automations().len();
    info!("{count} automatisation(s) chargée(s)");
    let mut runner = Runner {
        evaluator,
        core,
        running: (0..count).map(|_| None).collect(),
        recent_runs: vec![VecDeque::new(); count],
    };
    let mut ticker = tokio::time::interval(Duration::from_secs(1));
    // Si la machine était en veille, on ne rattrape pas toutes les secondes perdues.
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut previous = local_now();

    loop {
        tokio::select! {
            () = token.cancelled() => break,

            received = events.recv() => match received {
                Ok(event) => {
                    if let Event::StateChanged { entity_id, old_state, new_state } = event.as_ref() {
                        let fired = runner.evaluator.on_state_changed(
                            entity_id,
                            old_state.as_ref(),
                            new_state.as_ref(),
                            Instant::now(),
                        );
                        runner.launch_all(fired);
                    }
                }
                Err(RecvError::Lagged(missed)) => warn!("automatisations : {missed} événements manqués"),
                Err(RecvError::Closed) => break,
            },

            _ = ticker.tick() => {
                let now = local_now();
                let fired = runner.evaluator.on_tick(previous, now, Instant::now(), &runner.core.states);
                previous = now;
                runner.launch_all(fired);
            }
        }
    }

    for handle in runner.running.into_iter().flatten() {
        handle.abort();
    }
    info!("automatisations arrêtées");
}

impl Runner {
    fn launch_all(&mut self, fired: Vec<usize>) {
        for index in fired {
            self.launch(index);
        }
    }

    fn launch(&mut self, index: usize) {
        let Some(automation) = self.evaluator.automations().get(index) else {
            return;
        };
        let alias = automation.alias.clone();

        if !self
            .evaluator
            .conditions_pass(index, &self.core.states, local_now())
        {
            debug!("« {alias} » : conditions non remplies");
            return;
        }
        if let Some(Some(handle)) = self.running.get(index)
            && !handle.is_finished()
        {
            warn!("« {alias} » est déjà en cours : nouveau déclenchement ignoré");
            return;
        }
        if let Some(runs) = self.recent_runs.get_mut(index) {
            let now = Instant::now();
            while runs
                .front()
                .is_some_and(|t| now.duration_since(*t) > RUN_WINDOW)
            {
                runs.pop_front();
            }
            if runs.len() >= MAX_RUNS {
                warn!(
                    "« {alias} » s'est déclenchée {MAX_RUNS} fois en une minute : \
                     boucle probable, déclenchement ignoré"
                );
                return;
            }
            runs.push_back(now);
        }

        info!("automatisation « {alias} » déclenchée");
        let actions = automation.actions.clone();
        let core = Arc::clone(&self.core);
        let handle = tokio::spawn(run_actions(alias, actions, core));
        if let Some(slot) = self.running.get_mut(index) {
            *slot = Some(handle);
        }
    }
}

/// Exécute les actions l'une après l'autre.
async fn run_actions(alias: String, actions: Vec<Action>, core: Arc<Core>) {
    for action in actions {
        match action {
            Action::Delay(duration) => tokio::time::sleep(duration).await,
            Action::Service {
                domain,
                service,
                entity_ids,
                data,
            } => {
                for entity_id in entity_ids {
                    debug!("« {alias} » : {domain}.{service} sur {entity_id}");
                    if let Err(error) =
                        core.call_service(&domain, &service, entity_id, data.clone())
                    {
                        warn!("« {alias} » : action refusée : {error}");
                    }
                }
            }
        }
    }
}
