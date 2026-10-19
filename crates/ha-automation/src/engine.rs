//! Le moteur : écoute le bus et l'horloge, et exécute les actions.

use std::collections::VecDeque;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use ha_core::{Attributes, Background, CancellationToken, Core, EntityId, Event, Origin};
use jiff::civil::DateTime;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::{Mutex, watch};
use tokio::task::JoinHandle;
use tokio::time::{Instant, MissedTickBehavior};
use tracing::{debug, info, warn};

use crate::config::{Action, Automation, Condition, MAX_REPEAT, RepeatUntil, RunMode, Trigger};
use crate::evaluator::{Checker, Evaluator, Fired};
use crate::scripts::{Scene, Script};
use crate::template::{Scope, TriggerData};
use crate::trace::{Check, Outcome, Recorder, StepStatus, Traces};
use ha_sun::Location;

/// Protection contre les boucles : une automatisation qui se déclenche plus
/// de `MAX_RUNS` fois en `RUN_WINDOW` est mise en pause jusqu'à ce que ça se calme.
/// (Exemple de boucle : A allume la lampe quand elle s'éteint, B l'éteint
/// quand elle s'allume…)
/// L'événement publié à chaque lancement d'une automatisation (comme dans
/// Home Assistant), avec son nom et la raison : `{"name": …, "trigger": …}`.
pub const AUTOMATION_TRIGGERED: &str = "automation_triggered";
const MAX_RUNS: usize = 10;
const RUN_WINDOW: Duration = Duration::from_mins(1);
/// Les entités citées par les automatisations sont vérifiées après ce délai
/// (le temps que les appareils se connectent), puis régulièrement.
const FIRST_ENTITY_CHECK: Duration = Duration::from_mins(2);
const ENTITY_CHECK_EVERY: Duration = Duration::from_mins(5);
/// Le début de l'identifiant des alertes « entité inconnue ».
const MISSING_PREFIX: &str = "repair_automation_";

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
        let library = Library::publish(setup.scripts, setup.scenes, &core, evaluator.checker());
        let reloads = setup.reloads.map(|reloads| reloads.0);
        let traces = setup.traces;
        Self(Background::spawn("le moteur d'automatisations", |token| {
            run(evaluator, library, core, events, reloads, traces, token)
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
    /// Où enregistrer l'historique des exécutions (partagé avec l'API).
    pub traces: Traces,
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
    /// Pour les conditions des scripts (`if`, `choose`, `condition`).
    checker: Checker,
}

impl Library {
    /// Crée les entités `script.*` (« off ») et `scene.*` (« unknown »,
    /// comme Home Assistant tant qu'une scène n'a jamais été activée).
    fn publish(scripts: Vec<Script>, scenes: Vec<Scene>, core: &Core, checker: Checker) -> Self {
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
            checker,
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
        let context = Context {
            alias: alias.clone().into(),
            core: Arc::clone(core),
            checker: self.checker.clone(),
            trigger_data: None,
            origin: Origin::Script(alias.clone()),
            trace: None,
        };
        *slot = Some(tokio::spawn(async move {
            set_entity(&context.core, entity.clone(), "on", &alias);
            run_actions(&context, &actions, "").await;
            set_entity(&context.core, entity, "off", &alias);
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
        let context = Context {
            alias: scene.name.clone().into(),
            core: Arc::clone(core),
            checker: self.checker.clone(),
            trigger_data: None,
            origin: Origin::Scene(scene.name.clone()),
            trace: None,
        };
        let actions = scene.actions.clone();
        tokio::spawn(async move {
            run_actions(&context, &actions, "").await;
        });
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
    /// Exécutions en cours (ou en attente) de chaque automatisation.
    running: Vec<Vec<JoinHandle<()>>>,
    /// Pour le mode `queued` : une seule exécution à la fois, dans l'ordre.
    queues: Vec<Arc<Mutex<()>>>,
    /// Dates des derniers déclenchements, pour la protection contre les boucles.
    recent_runs: Vec<VecDeque<Instant>>,
    traces: Traces,
    started: Instant,
    /// Prochaine vérification des entités citées.
    next_check: Instant,
}

async fn run(
    evaluator: Evaluator,
    mut library: Library,
    core: Arc<Core>,
    mut events: tokio::sync::broadcast::Receiver<Arc<Event>>,
    mut reloads: Option<watch::Receiver<Arc<Vec<Automation>>>>,
    traces: Traces,
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
        running: (0..count).map(|_| Vec::new()).collect(),
        queues: (0..count).map(|_| Arc::default()).collect(),
        recent_runs: vec![VecDeque::new(); count],
        traces,
        started: Instant::now(),
        next_check: Instant::now() + FIRST_ENTITY_CHECK,
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
                        let fired = runner.evaluator.fire_on_state_changed(
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
                    Event::Fired { event_type, data } => {
                        let fired = runner.evaluator.fire_on_event(event_type, data);
                        runner.launch_all(fired);
                    }
                },
                Err(RecvError::Lagged(missed)) => warn!("automatisations : {missed} événements manqués"),
                Err(RecvError::Closed) => break,
            },

            automations = next_reload(&mut reloads) => runner.reload(automations.as_ref().clone()),

            _ = ticker.tick() => {
                let now = local_now();
                let fired = runner.evaluator.fire_on_tick(previous, now, Instant::now(), &runner.core.states);
                previous = now;
                runner.launch_all(fired);
                if Instant::now() >= runner.next_check {
                    runner.check_entities();
                }
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
        let old_queues = std::mem::take(&mut self.queues);
        let mut old_runs = std::mem::take(&mut self.recent_runs);
        for old in &previous {
            let kept = old
                .and_then(|old| old_running.get_mut(old).map(std::mem::take))
                .unwrap_or_default();
            self.running.push(kept);
            let queue = old
                .and_then(|old| old_queues.get(old).cloned())
                .unwrap_or_default();
            self.queues.push(queue);
            let runs = old
                .and_then(|old| old_runs.get_mut(old).map(std::mem::take))
                .unwrap_or_default();
            self.recent_runs.push(runs);
        }
        for handle in old_running.into_iter().flatten() {
            handle.abort();
        }
        // L'historique des automatisations supprimées ne sert plus à rien.
        let ids: Vec<&str> = self
            .evaluator
            .automations()
            .iter()
            .map(|a| a.id.as_str())
            .collect();
        self.traces.retain(|id| ids.contains(&id));
        // Les entités des automatisations modifiées sont vérifiées bientôt
        // (mais pas avant que les appareils aient eu le temps de se connecter).
        self.next_check = (Instant::now() + Duration::from_secs(5))
            .max(self.started + FIRST_ENTITY_CHECK)
            .min(self.next_check);
        let unchanged = previous.iter().filter(|old| old.is_some()).count();
        info!(
            "automatisations rechargées : {count} au total, {} nouvelle(s) ou modifiée(s)",
            count - unchanged
        );
    }

    /// Une alerte de réparation pour chaque automatisation qui cite des
    /// entités inconnues (souvent un appareil renommé ou retiré).
    fn check_entities(&mut self) {
        self.next_check = Instant::now() + ENTITY_CHECK_EVERY;
        let issues: Vec<ha_notices::Issue> = self
            .evaluator
            .automations()
            .iter()
            .filter_map(|automation| {
                let missing: Vec<&str> = automation
                    .entity_ids()
                    .into_iter()
                    .filter(|id| self.core.states.get(id).is_none())
                    .map(EntityId::as_str)
                    .collect();
                (!missing.is_empty()).then(|| ha_notices::Issue {
                    key: automation.id.clone(),
                    title: format!("« {} » vise des entités inconnues", automation.alias),
                    message: format!(
                        "Ces entités n'existent pas : {}. Vérifie que l'appareil est \
                         connecté, ou corrige l'automatisation.",
                        missing.join(", ")
                    ),
                    severity: ha_notices::Severity::Error,
                })
            })
            .collect();
        ha_notices::reconcile(&self.core.states, MISSING_PREFIX, &issues);
    }

    fn launch_all(&mut self, fired: Vec<Fired>) {
        for fired in fired {
            self.launch(fired);
        }
    }

    fn launch(&mut self, fired: Fired) {
        let index = fired.automation;
        let Some(automation) = self.evaluator.automations().get(index) else {
            return;
        };
        let alias = automation.alias.clone();
        let trigger = automation.trigger_ids.get(fired.trigger).cloned();
        // Pour le journal de bord et l'historique : pourquoi.
        let because = automation
            .triggers
            .get(fired.trigger)
            .map(|t| t.describe(&self.core.states));

        // Les conditions, une par une, pour l'historique : on s'arrête à la
        // première fausse (comme `all`).
        let checker = self.evaluator.checker();
        let now = local_now();
        // Pour les modèles : `trigger.id`, `trigger.to_state`…
        let trigger_data = TriggerData {
            id: trigger.clone(),
            ..fired.data.unwrap_or_default()
        };
        let mut checks = Vec::new();
        let mut conditions_pass = true;
        for condition in &automation.conditions {
            conditions_pass =
                checker.passes(condition, &self.core.states, now, Some(&trigger_data));
            checks.push(Check {
                description: condition.describe(&self.core.states),
                passed: conditions_pass,
            });
            if !conditions_pass {
                break;
            }
        }
        let recorder = self.traces.begin(
            &automation.id,
            &alias,
            because.clone(),
            Some(fired.trigger),
            checks,
        );
        if !conditions_pass {
            debug!("« {alias} » : conditions non remplies");
            recorder.finish(Outcome::ConditionsFailed);
            return;
        }
        let Some(running) = self.running.get_mut(index) else {
            return;
        };
        running.retain(|handle| !handle.is_finished());
        match automation.mode {
            RunMode::Single if !running.is_empty() => {
                warn!("« {alias} » est déjà en cours : nouveau déclenchement ignoré");
                recorder.finish(Outcome::AlreadyRunning);
                return;
            }
            RunMode::Restart if !running.is_empty() => {
                info!("« {alias} » : on recommence depuis le début");
                for handle in running.drain(..) {
                    handle.abort();
                }
            }
            RunMode::Queued | RunMode::Parallel if running.len() >= automation.max => {
                warn!(
                    "« {alias} » : déjà {} exécution(s) en cours ou en attente, \
                     nouveau déclenchement ignoré",
                    automation.max
                );
                recorder.finish(Outcome::TooMany);
                return;
            }
            _ => {}
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
                recorder.finish(Outcome::Loop);
                warn_loop(&self.core, &automation.id, &alias);
                return;
            }
            runs.push_back(now);
        }

        info!("automatisation « {alias} » déclenchée");
        let mut data = Attributes::new();
        data.insert("id".into(), automation.id.clone().into());
        data.insert("name".into(), alias.clone().into());
        if let Some(because) = because {
            data.insert("trigger".into(), because.into());
        }
        if let Err(error) = self.core.fire(AUTOMATION_TRIGGERED, data) {
            warn!("« {alias} » : événement refusé ({error})");
        }
        let mut context = Context {
            origin: Origin::Automation(alias.clone()),
            alias: alias.into(),
            core: Arc::clone(&self.core),
            checker,
            trigger_data: Some(Arc::new(trigger_data)),
            trace: Some(recorder),
        };
        let actions = automation.actions.clone();
        let queue = (automation.mode == RunMode::Queued)
            .then(|| self.queues.get(index).cloned())
            .flatten();
        running.push(tokio::spawn(async move {
            // Mode `queued` : on attend son tour. Le verrou de tokio est
            // équitable : les exécutions passent dans l'ordre d'arrivée.
            let _turn = match &queue {
                Some(queue) => Some(queue.lock().await),
                None => None,
            };
            let flow = run_actions(&context, &actions, "").await;
            if let Some(recorder) = context.trace.take() {
                recorder.finish(match flow {
                    Flow::Continue => Outcome::Finished,
                    Flow::Stop => Outcome::Stopped,
                });
            }
        }));
    }
}

/// Prévient sur le tableau de bord qu'une automatisation boucle.
fn warn_loop(core: &Core, id: &str, alias: &str) {
    let Ok(entity_id) = ha_notices::notice_id(&format!("automation_loop_{id}")) else {
        return;
    };
    let message = format!(
        "« {alias} » s'est déclenchée {MAX_RUNS} fois en une minute : ses nouveaux \
         déclenchements sont ignorés. Regarde son historique pour trouver la boucle."
    );
    if let Err(error) = ha_notices::notify(
        &core.states,
        &entity_id,
        Some("Automatisation en boucle"),
        &message,
    ) {
        warn!("notification refusée : {error}");
    }
}

/// Ce dont une exécution a besoin.
pub(crate) struct Context {
    pub(crate) alias: Arc<str>,
    pub(crate) core: Arc<Core>,
    pub(crate) checker: Checker,
    /// Ce qui a déclenché l'exécution : son identifiant (condition
    /// `trigger`), et ce qu'il faut aux modèles (`trigger.…`).
    pub(crate) trigger_data: Option<Arc<TriggerData>>,
    /// Qui envoie les commandes (pour le journal de bord).
    pub(crate) origin: Origin,
    /// L'historique de cette exécution (automatisations seulement).
    pub(crate) trace: Option<Recorder>,
}

impl Context {
    /// Note une étape dans l'historique. La description n'est calculée que
    /// s'il y a un historique (elle lit les noms des appareils).
    fn record(
        &self,
        path: &str,
        at: jiff::Timestamp,
        action: &Action,
        status: StepStatus,
        detail: Option<String>,
    ) {
        if let Some(recorder) = &self.trace {
            recorder.step(path, at, action.describe(&self.core.states), status, detail);
        }
    }
}

/// La place d'une action : `"2"`, ou `"3 › alors › 1"`.
fn step_path(prefix: &str, index: usize) -> String {
    if prefix.is_empty() {
        (index + 1).to_string()
    } else {
        format!("{prefix} › {}", index + 1)
    }
}

/// Faut-il continuer après une action ?
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Flow {
    Continue,
    /// Une action `condition` était fausse : on arrête tout.
    Stop,
}

/// Attend qu'un des déclencheurs se produise. Renvoie `false` si l'attente
/// a dépassé `timeout`.
///
/// On réutilise l'évaluateur des automatisations, avec une automatisation
/// « fantôme » qui n'a que ces déclencheurs : `for:`, `numeric_state`,
/// `time`… marchent exactement comme dans les déclencheurs habituels.
async fn wait_for_trigger(
    context: &Context,
    triggers: &[Trigger],
    timeout: Option<Duration>,
) -> bool {
    let mut events = context.core.bus.subscribe();
    let mut evaluator = Evaluator::new(vec![Automation {
        id: "attente".into(),
        alias: context.alias.to_string(),
        triggers: triggers.to_vec(),
        trigger_ids: Vec::new(),
        conditions: Vec::new(),
        actions: Vec::new(),
        mode: RunMode::Single,
        max: 1,
    }]);
    let deadline = timeout.map(|timeout| Instant::now() + timeout);
    let mut ticker = tokio::time::interval(Duration::from_secs(1));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut previous = local_now();
    loop {
        let fired = tokio::select! {
            () = async {
                match deadline {
                    Some(deadline) => tokio::time::sleep_until(deadline).await,
                    None => std::future::pending().await,
                }
            } => return false,
            received = events.recv() => match received {
                Ok(event) => match event.as_ref() {
                    Event::StateChanged { entity_id, old_state, new_state } => evaluator
                        .fire_on_state_changed(entity_id, old_state.as_ref(), new_state.as_ref(), Instant::now()),
                    Event::Fired { event_type, data } => evaluator.fire_on_event(event_type, data),
                    Event::CallService { .. } => Vec::new(),
                },
                Err(RecvError::Lagged(_)) => Vec::new(),
                // Le noyau s'arrête : l'attente ne finira jamais.
                Err(RecvError::Closed) => return false,
            },
            _ = ticker.tick() => {
                let now = local_now();
                let fired = evaluator.fire_on_tick(previous, now, Instant::now(), &context.core.states);
                previous = now;
                fired
            }
        };
        if !fired.is_empty() {
            return true;
        }
    }
}

/// Exécute les actions l'une après l'autre. `prefix` : la place de ces
/// actions dans l'automatisation (vide au premier niveau), pour l'historique.
///
/// Concept Rust : une fonction `async` qui s'appelle elle-même (pour `if` et
/// `choose`) doit renvoyer son `Future` dans une `Box` (`Box::pin`) : sinon
/// le compilateur ne pourrait pas calculer sa taille, qui serait infinie.
pub(crate) fn run_actions<'a>(
    context: &'a Context,
    actions: &'a [Action],
    prefix: &'a str,
) -> Pin<Box<dyn Future<Output = Flow> + Send + 'a>> {
    Box::pin(async move {
        let alias = &context.alias;
        let passes = |conditions: &[Condition]| {
            context.checker.all(
                conditions,
                &context.core.states,
                local_now(),
                context.trigger_data.as_deref(),
            )
        };
        for (index, action) in actions.iter().enumerate() {
            let path = step_path(prefix, index);
            let at = jiff::Timestamp::now();
            let flow = match action {
                Action::Delay(duration) => {
                    tokio::time::sleep(*duration).await;
                    context.record(&path, at, action, StepStatus::Done, None);
                    Flow::Continue
                }
                Action::Service {
                    domain,
                    service,
                    entity_ids,
                    data,
                    templates,
                } => {
                    // Les modèles sont calculés au moment de l'action, avec
                    // les états de cet instant.
                    let data = if templates.is_empty() {
                        data.clone()
                    } else {
                        templates.render(
                            data,
                            &Scope {
                                states: &context.core.states,
                                trigger: context.trigger_data.as_deref(),
                            },
                        )
                    };
                    let mut errors = Vec::new();
                    for entity_id in entity_ids {
                        debug!("« {alias} » : {domain}.{service} sur {entity_id}");
                        if let Err(error) = context.core.call_service_from(
                            context.origin.clone(),
                            domain,
                            service,
                            entity_id.clone(),
                            data.clone(),
                        ) {
                            warn!("« {alias} » : action refusée : {error}");
                            errors.push(error.to_string());
                        }
                    }
                    if errors.is_empty() {
                        context.record(&path, at, action, StepStatus::Done, None);
                    } else {
                        let detail = Some(errors.join(" ; "));
                        context.record(&path, at, action, StepStatus::Failed, detail);
                    }
                    Flow::Continue
                }
                Action::Condition(condition) => {
                    if passes(std::slice::from_ref(condition)) {
                        context.record(&path, at, action, StepStatus::Done, Some("vraie".into()));
                        Flow::Continue
                    } else {
                        debug!("« {alias} » : condition fausse, on s'arrête là");
                        let detail = Some("fausse : on s'arrête là".into());
                        context.record(&path, at, action, StepStatus::Stopped, detail);
                        Flow::Stop
                    }
                }
                Action::If {
                    conditions,
                    then,
                    otherwise,
                } => {
                    let (branch, name) = if passes(conditions) {
                        (then, "alors")
                    } else {
                        (otherwise, "sinon")
                    };
                    context.record(&path, at, action, StepStatus::Done, Some(name.into()));
                    run_actions(context, branch, &format!("{path} › {name}")).await
                }
                Action::Choose { options, default } => {
                    let chosen = options
                        .iter()
                        .position(|(conditions, _)| passes(conditions));
                    let (branch, name) = match chosen.and_then(|i| options.get(i).map(|o| (i, o))) {
                        Some((i, (_, actions))) => {
                            (actions.as_slice(), format!("option {}", i + 1))
                        }
                        None if default.is_empty() => (default.as_slice(), "aucune option".into()),
                        None => (default.as_slice(), "par défaut".into()),
                    };
                    context.record(&path, at, action, StepStatus::Done, Some(name.clone()));
                    run_actions(context, branch, &format!("{path} › {name}")).await
                }
                Action::Repeat { until, actions } => {
                    context.record(&path, at, action, StepStatus::Done, None);
                    let mut flow = Flow::Continue;
                    for round in 1..=MAX_REPEAT {
                        let go_on = match until {
                            RepeatUntil::Count(count) => round <= *count,
                            RepeatUntil::While(conditions) => passes(conditions),
                            RepeatUntil::Until(_) => true,
                        };
                        if !go_on {
                            break;
                        }
                        flow =
                            run_actions(context, actions, &format!("{path} › tour {round}")).await;
                        if flow == Flow::Stop {
                            break;
                        }
                        if let RepeatUntil::Until(conditions) = until
                            && passes(conditions)
                        {
                            break;
                        }
                        if round == MAX_REPEAT {
                            warn!("« {alias} » : répétition arrêtée après {MAX_REPEAT} tours");
                        }
                        // Laisse les autres tâches avancer entre deux tours.
                        tokio::task::yield_now().await;
                    }
                    flow
                }
                Action::WaitForTrigger {
                    triggers,
                    timeout,
                    continue_on_timeout,
                } => {
                    if wait_for_trigger(context, triggers, *timeout).await {
                        let detail = Some("déclencheur arrivé".into());
                        context.record(&path, at, action, StepStatus::Done, detail);
                        Flow::Continue
                    } else if *continue_on_timeout {
                        let detail = Some("délai dépassé, on continue".into());
                        context.record(&path, at, action, StepStatus::Done, detail);
                        Flow::Continue
                    } else {
                        debug!("« {alias} » : attente dépassée, on s'arrête là");
                        let detail = Some("délai dépassé : on s'arrête là".into());
                        context.record(&path, at, action, StepStatus::Stopped, detail);
                        Flow::Stop
                    }
                }
            };
            if flow == Flow::Stop {
                return Flow::Stop;
            }
        }
        Flow::Continue
    })
}
