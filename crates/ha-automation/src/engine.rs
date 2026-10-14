//! Le moteur : écoute le bus et l'horloge, et exécute les actions.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

use ha_core::{Attributes, Background, CancellationToken, Core, EntityId, Event};
use jiff::civil::DateTime;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio::time::{Instant, MissedTickBehavior};
use tracing::{debug, info, warn};

use crate::config::{Action, Automation};
use crate::evaluator::Evaluator;
use crate::scripts::{Scene, Script};
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
        Self::start_with(
            Setup {
                automations,
                location,
                ..Setup::default()
            },
            core,
        )
    }

    /// Démarre le moteur avec les automatisations, les scripts et les scènes.
    pub fn start_with(setup: Setup, core: Arc<Core>) -> Self {
        let events = core.bus.subscribe();
        let mut evaluator = Evaluator::new(setup.automations);
        if let Some(location) = setup.location {
            evaluator = evaluator.with_sun(location, jiff::tz::TimeZone::system());
        }
        let library = Library::publish(setup.scripts, setup.scenes, &core);
        let reloads = setup.reloads.map(|reloads| reloads.0);
        Self(Background::spawn("le moteur d'automatisations", |token| {
            run(evaluator, library, core, events, reloads, token)
        }))
    }

    /// Arrête le moteur ; les actions en cours (attentes `delay`…) sont abandonnées.
    pub async fn stop(self) {
        self.0.stop().await;
    }
}

/// Tout ce que le moteur fait tourner.
#[derive(Debug, Default)]
pub struct Setup {
    pub automations: Vec<Automation>,
    pub scripts: Vec<Script>,
    pub scenes: Vec<Scene>,
    /// Position de la maison, pour le soleil.
    pub location: Option<Location>,
    /// Pour recharger les automatisations sans redémarrer (voir
    /// [`reload_channel`]).
    pub reloads: Option<Reloads>,
}

/// Côté « envoi » du rechargement : on lui passe la nouvelle liste.
#[derive(Debug)]
pub struct Reloader(watch::Sender<Arc<Vec<Automation>>>);

/// Côté « réception », à donner au moteur dans [`Setup::reloads`].
#[derive(Debug)]
pub struct Reloads(watch::Receiver<Arc<Vec<Automation>>>);

/// Crée le canal qui permet de recharger les automatisations d'un moteur
/// qui tourne.
///
/// Concept Rust : un canal `watch` ne garde que la **dernière** valeur
/// envoyée. Si deux modifications arrivent très vite, le moteur ne charge
/// que la plus récente : c'est exactement ce qu'on veut.
#[must_use]
pub fn reload_channel() -> (Reloader, Reloads) {
    let (sender, receiver) = watch::channel(Arc::new(Vec::new()));
    (Reloader(sender), Reloads(receiver))
}

impl Reloader {
    /// Demande au moteur de remplacer ses automatisations par `automations`.
    /// Sans effet si le moteur est arrêté.
    pub fn reload(&self, automations: Vec<Automation>) {
        self.0.send_replace(Arc::new(automations));
    }
}

/// Attend un rechargement. Sans canal (ou si l'envoi a disparu), attend pour
/// toujours : cette branche du `select!` ne se produit alors jamais.
async fn next_reload(
    reloads: &mut Option<watch::Receiver<Arc<Vec<Automation>>>>,
) -> Arc<Vec<Automation>> {
    if let Some(receiver) = reloads {
        if receiver.changed().await.is_ok() {
            return Arc::clone(&receiver.borrow_and_update());
        }
        *reloads = None;
    }
    std::future::pending().await
}

/// Les scripts et les scènes, et les scripts en cours d'exécution.
struct Library {
    scripts: Vec<Script>,
    scenes: Vec<Scene>,
    running: Vec<Option<JoinHandle<()>>>,
}

impl Library {
    /// Crée les entités `script.*` (« off ») et `scene.*` (« unknown »,
    /// comme Home Assistant tant qu'une scène n'a jamais été activée).
    fn publish(scripts: Vec<Script>, scenes: Vec<Scene>, core: &Core) -> Self {
        for script in &scripts {
            set_entity(core, script.entity_id(), "off", &script.alias);
        }
        for scene in &scenes {
            set_entity(core, scene.entity_id(), "unknown", &scene.name);
        }
        let running = scripts.iter().map(|_| None).collect();
        Self {
            scripts,
            scenes,
            running,
        }
    }

    /// Réagit à `script.turn_on`, `script.turn_off` et `scene.turn_on` ;
    /// les autres appels ne nous concernent pas.
    fn on_call(&mut self, domain: &str, service: &str, entity_id: &EntityId, core: &Arc<Core>) {
        match (domain, service) {
            ("script", "turn_on" | "toggle") => self.start_script(entity_id, core),
            ("script", "turn_off") => self.stop_script(entity_id, core),
            ("scene", "turn_on") => self.activate_scene(entity_id, core),
            _ => {}
        }
    }

    fn find_script(&self, entity_id: &EntityId) -> Option<usize> {
        self.scripts
            .iter()
            .position(|script| script.entity_id().as_ref() == Some(entity_id))
    }

    fn start_script(&mut self, entity_id: &EntityId, core: &Arc<Core>) {
        let Some(index) = self.find_script(entity_id) else {
            warn!("script inconnu : {entity_id}");
            return;
        };
        let Some(slot) = self.running.get_mut(index) else {
            return;
        };
        // Mode « une à la fois » : un script déjà en cours n'est pas relancé.
        // C'est aussi ce qui empêche un script de se relancer lui-même à l'infini.
        if slot.as_ref().is_some_and(|handle| !handle.is_finished()) {
            warn!("{entity_id} est déjà en cours : nouveau lancement ignoré");
            return;
        }
        let Some(script) = self.scripts.get(index) else {
            return;
        };
        info!("script « {} » lancé", script.alias);
        let alias = script.alias.clone();
        let actions = script.actions.clone();
        let entity = script.entity_id();
        let core = Arc::clone(core);
        *slot = Some(tokio::spawn(async move {
            set_entity(&core, entity.clone(), "on", &alias);
            run_actions(alias.clone(), actions, Arc::clone(&core)).await;
            set_entity(&core, entity, "off", &alias);
        }));
    }

    fn stop_script(&mut self, entity_id: &EntityId, core: &Core) {
        let Some(index) = self.find_script(entity_id) else {
            return;
        };
        if let Some(Some(handle)) = self.running.get_mut(index) {
            handle.abort();
        }
        if let Some(script) = self.scripts.get(index) {
            info!("script « {} » arrêté", script.alias);
            set_entity(core, script.entity_id(), "off", &script.alias);
        }
    }

    fn activate_scene(&self, entity_id: &EntityId, core: &Arc<Core>) {
        let Some(scene) = self
            .scenes
            .iter()
            .find(|scene| scene.entity_id().as_ref() == Some(entity_id))
        else {
            warn!("scène inconnue : {entity_id}");
            return;
        };
        info!("scène « {} » activée", scene.name);
        // Comme Home Assistant : l'état d'une scène est la date de sa
        // dernière activation.
        set_entity(
            core,
            scene.entity_id(),
            &jiff::Timestamp::now().to_string(),
            &scene.name,
        );
        tokio::spawn(run_actions(
            scene.name.clone(),
            scene.actions.clone(),
            Arc::clone(core),
        ));
    }

    fn abort_all(self) {
        for handle in self.running.into_iter().flatten() {
            handle.abort();
        }
    }
}

fn set_entity(core: &Core, entity_id: Option<EntityId>, state: &str, name: &str) {
    let Some(entity_id) = entity_id else { return };
    let mut attributes = Attributes::new();
    attributes.insert("friendly_name".into(), name.into());
    if let Err(error) = core.states.set(entity_id, state, attributes) {
        warn!("état refusé : {error}");
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
    mut library: Library,
    core: Arc<Core>,
    mut events: tokio::sync::broadcast::Receiver<Arc<Event>>,
    mut reloads: Option<watch::Receiver<Arc<Vec<Automation>>>>,
    token: CancellationToken,
) {
    let count = evaluator.automations().len();
    info!(
        "{count} automatisation(s), {} script(s), {} scène(s) chargé(e)s",
        library.scripts.len(),
        library.scenes.len()
    );
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
                Ok(event) => match event.as_ref() {
                    Event::StateChanged { entity_id, old_state, new_state } => {
                        let fired = runner.evaluator.on_state_changed(
                            entity_id,
                            old_state.as_ref(),
                            new_state.as_ref(),
                            Instant::now(),
                        );
                        runner.launch_all(fired);
                    }
                    Event::CallService { domain, service, entity_id, .. } => {
                        library.on_call(domain, service, entity_id, &runner.core);
                    }
                },
                Err(RecvError::Lagged(missed)) => warn!("automatisations : {missed} événements manqués"),
                Err(RecvError::Closed) => break,
            },

            automations = next_reload(&mut reloads) => runner.reload(automations.as_ref().clone()),

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
    library.abort_all();
    info!("automatisations arrêtées");
}

impl Runner {
    /// Remplace les automatisations. Celles qui n'ont pas changé continuent
    /// comme si de rien n'était (actions en cours, attentes `for`, protection
    /// contre les boucles) ; les actions en cours de celles qui ont été
    /// modifiées ou supprimées sont arrêtées.
    fn reload(&mut self, automations: Vec<Automation>) {
        let count = automations.len();
        let previous = self.evaluator.replace(automations);
        let mut old_running = std::mem::take(&mut self.running);
        let mut old_runs = std::mem::take(&mut self.recent_runs);
        for old in &previous {
            let kept = old.and_then(|old| old_running.get_mut(old)?.take());
            self.running.push(kept);
            let runs = old
                .and_then(|old| old_runs.get_mut(old).map(std::mem::take))
                .unwrap_or_default();
            self.recent_runs.push(runs);
        }
        for handle in old_running.into_iter().flatten() {
            handle.abort();
        }
        let unchanged = previous.iter().filter(|old| old.is_some()).count();
        info!(
            "automatisations rechargées : {count} au total, {} nouvelle(s) ou modifiée(s)",
            count - unchanged
        );
    }

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
