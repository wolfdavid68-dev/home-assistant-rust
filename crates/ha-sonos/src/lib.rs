//! Intégration Sonos (étape 73) : les enceintes comme lecteurs
//! (`media_player`), et les **annonces vocales** (`tts.speak`), en local.
//!
//! - [`api`] : les requêtes UPnP vers une enceinte ;
//! - [`topology`] : les enceintes et leurs groupes ;
//! - [`discovery`] : les trouver sans réglage ;
//! - [`player`] : une enceinte ⇄ une entité ;
//! - [`favorites`] : les favoris Sonos, lancés d'un toucher ;
//! - [`art`] : les pochettes, montrées par le lecteur du tableau de bord ;
//! - [`announce`] : annoncer sans perdre la musique ;
//! - [`clips`] : le petit serveur où les enceintes téléchargent l'annonce ;
//! - [`Sonos`] : la boucle principale.

pub mod announce;
pub mod api;
pub mod art;
pub mod clips;
pub mod discovery;
pub mod favorites;
pub mod player;
pub mod topology;
pub mod xml;

use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime};

use ha_core::{Attributes, Background, CancellationToken, Core, EntityId, Event, Health};
use ha_tts::Tts;
use serde_json::{Value, json};
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::{Notify, oneshot};
use tracing::{info, warn};

use crate::announce::{Target, Timing};
use crate::api::{Api, SonosError};
use crate::clips::Clips;
use crate::favorites::{Favorite, Launch};
use crate::player::Reading;
use crate::topology::Group;

/// L'identifiant de l'intégration, propriétaire de ses entités.
pub const ID: &str = "sonos";
/// Son nom affiché.
pub const NAME: &str = "Sonos";
/// L'entité des annonces : `tts.speak` sur elle fait parler les enceintes.
pub const TTS_ENTITY: &str = "tts.annonces";

/// Entre deux lectures de l'état des enceintes.
const POLL: Duration = Duration::from_secs(3);
/// Entre deux relectures des groupes (une enceinte ajoutée, un groupe fait
/// dans l'appli Sonos…).
const TOPOLOGY_EVERY: Duration = Duration::from_mins(1);
const DISCOVERY_WAIT: Duration = Duration::from_secs(3);
const MAX_BACKOFF: Duration = Duration::from_mins(1);
/// Pas de « monter le son » et « baisser le son ».
const VOLUME_STEP: f64 = 5.0;
/// Enceintes visées par une annonce, au plus.
const MAX_TARGETS: usize = 32;

/// Les réglages.
#[derive(Debug, Clone)]
pub struct Config {
    /// Les adresses données (`192.168.1.30:1400`) ; vide : on les cherche.
    pub hosts: Vec<String>,
    /// Le port du serveur des annonces.
    pub audio_port: u16,
    /// L'adresse de ce PC vue par les enceintes ; `None` : devinée.
    pub audio_address: Option<IpAddr>,
    /// La voix ; sans elle, seulement le carillon.
    pub tts: Option<Tts>,
    /// Entre deux lectures de l'état (raccourci par les tests).
    pub poll: Duration,
    pub timing: Timing,
}

impl Config {
    #[must_use]
    pub fn new(hosts: Vec<String>) -> Self {
        Self {
            hosts,
            audio_port: clips::DEFAULT_PORT,
            audio_address: None,
            tts: None,
            poll: POLL,
            timing: Timing::default(),
        }
    }
}

/// `HA_SONOS` : `auto` (les chercher), ou des adresses séparées par des
/// virgules (`192.168.1.30, 192.168.1.31`). Une seule suffit.
pub fn parse_hosts(value: &str) -> Result<Vec<String>, String> {
    let value = value.trim();
    if value.is_empty() || value.eq_ignore_ascii_case("auto") || value == "on" {
        return Ok(Vec::new());
    }
    value
        .split(',')
        .map(str::trim)
        .filter(|h| !h.is_empty())
        .map(|host| {
            let host = host.strip_suffix(":1400").unwrap_or(host);
            if host.contains(':') || !ha_tls::is_valid_host(host) {
                return Err(format!(
                    "HA_SONOS : adresse invalide « {host} » (exemple : 192.168.1.30)"
                ));
            }
            Ok(format!("{host}:{}", api::PORT))
        })
        .collect()
}

/// `HA_SONOS_AUDIO` : l'adresse de ce PC que les enceintes utiliseront pour
/// télécharger les annonces, avec le port si ce n'est pas 8124
/// (`192.168.1.20` ou `192.168.1.20:8124`).
pub fn parse_audio(value: &str) -> Result<(Ipv4Addr, u16), String> {
    let value = value.trim();
    let invalid =
        || format!("HA_SONOS_AUDIO : adresse invalide « {value} » (exemple : 192.168.1.20)");
    let (ip, port) = match value.split_once(':') {
        Some((ip, port)) => (
            ip,
            port.parse::<u16>()
                .ok()
                .filter(|p| *p > 0)
                .ok_or_else(invalid)?,
        ),
        None => (value, clips::DEFAULT_PORT),
    };
    let ip: Ipv4Addr = ip.parse().map_err(|_| invalid())?;
    if ip.is_unspecified() || ip.is_loopback() {
        return Err(format!(
            "HA_SONOS_AUDIO : {ip} n'est pas joignable par les enceintes ; mets l'adresse de ce PC \
             sur le réseau local"
        ));
    }
    Ok((ip, port))
}

/// Une enceinte connue.
#[derive(Debug, Clone)]
struct Speaker {
    entity_id: EntityId,
    name: String,
    host: String,
}

#[derive(Debug, Default)]
struct Speakers {
    by_uid: HashMap<String, Speaker>,
    by_entity: HashMap<EntityId, String>,
    groups: Vec<Group>,
}

impl Speakers {
    /// Les nouveaux groupes ; rend les enceintes nouvelles.
    fn update(&mut self, groups: Vec<Group>) -> Vec<Speaker> {
        let mut added = Vec::new();
        for member in groups.iter().flat_map(|g| &g.members) {
            if let Some(known) = self.by_uid.get_mut(&member.uid) {
                known.host.clone_from(&member.host);
                known.name.clone_from(&member.name);
                continue;
            }
            let Some(entity_id) = self.allocate(&member.name, &member.uid) else {
                continue;
            };
            let speaker = Speaker {
                entity_id: entity_id.clone(),
                name: member.name.clone(),
                host: member.host.clone(),
            };
            self.by_entity.insert(entity_id, member.uid.clone());
            self.by_uid.insert(member.uid.clone(), speaker.clone());
            added.push(speaker);
        }
        self.groups = groups;
        added
    }

    /// `media_player.salon`, sinon `_2`, `_3`…
    fn allocate(&self, name: &str, uid: &str) -> Option<EntityId> {
        let slug = [name, uid]
            .into_iter()
            .map(ha_core::slugify)
            .find(|slug| !slug.is_empty())?;
        (1..=99).find_map(|n| {
            let candidate = if n == 1 {
                format!("media_player.{slug}")
            } else {
                format!("media_player.{slug}_{n}")
            };
            EntityId::new(&candidate)
                .ok()
                .filter(|id| !self.by_entity.contains_key(id))
        })
    }

    fn group_of(&self, uid: &str) -> Option<&Group> {
        self.groups
            .iter()
            .find(|g| g.members.iter().any(|m| m.uid == uid))
    }

    /// L'identifiant (`RINCON_…`) du coordinateur du groupe de l'enceinte.
    fn coordinator_uid(&self, entity_id: &EntityId) -> Option<String> {
        let uid = self.by_entity.get(entity_id)?;
        Some(
            self.group_of(uid)
                .and_then(Group::coordinator)
                .map_or_else(|| uid.clone(), |m| m.uid.clone()),
        )
    }

    /// L'adresse de l'enceinte, et celle du coordinateur de son groupe.
    fn hosts_for(&self, entity_id: &EntityId) -> Option<(String, String)> {
        let uid = self.by_entity.get(entity_id)?;
        let own = self.by_uid.get(uid)?.host.clone();
        let coordinator = self
            .group_of(uid)
            .and_then(Group::coordinator)
            .map_or_else(|| own.clone(), |m| m.host.clone());
        Some((own, coordinator))
    }

    fn entities(&self) -> Vec<String> {
        let mut list: Vec<String> = self
            .by_uid
            .values()
            .map(|s| s.entity_id.to_string())
            .collect();
        list.sort();
        list
    }

    fn hosts(&self) -> Vec<String> {
        self.by_uid.values().map(|s| s.host.clone()).collect()
    }
}

/// Ce que partagent la boucle et les ordres en cours.
struct Shared {
    api: Api,
    config: Config,
    clips: Option<Clips>,
    speakers: Mutex<Speakers>,
    /// Une annonce à la fois par groupe.
    locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    /// Relire l'état tout de suite (après un ordre).
    refresh: Notify,
    /// Les favoris Sonos, communs à toute la maison (étape 91).
    favorites: Mutex<Vec<Favorite>>,
    /// Les pochettes en cours de téléchargement (étape 92).
    art: Mutex<art::Wanted>,
}

impl Shared {
    fn speakers(&self) -> std::sync::MutexGuard<'_, Speakers> {
        self.speakers.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn favorites(&self) -> Vec<Favorite> {
        self.favorites
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn lock_for(&self, coordinator: &str) -> Arc<tokio::sync::Mutex<()>> {
        let mut locks = self.locks.lock().unwrap_or_else(PoisonError::into_inner);
        Arc::clone(locks.entry(coordinator.to_owned()).or_default())
    }
}

/// Poignée vers l'intégration Sonos qui tourne en arrière-plan.
#[derive(Debug)]
pub struct Sonos {
    task: Background,
    audio: Option<Background>,
}

impl Sonos {
    pub fn start(config: Config, core: Arc<Core>) -> Result<Self, String> {
        let api =
            Api::new().map_err(|error| format!("connexion impossible à préparer ({error})"))?;
        let (clips, audio) = match bind_audio(config.audio_port) {
            Ok(listener) => {
                let clips = Clips::default();
                let served = clips.clone();
                info!(
                    "Sonos : les annonces sont servies aux enceintes sur le port {}",
                    config.audio_port
                );
                let task = Background::spawn("annonces Sonos", move |token| {
                    clips::serve(listener, served, token)
                });
                (Some(clips), Some(task))
            }
            Err(error) => {
                warn!(
                    "Sonos : port {} indisponible ({error}) : pas d'annonces vocales",
                    config.audio_port
                );
                (None, None)
            }
        };
        if let Some(tts) = &config.tts {
            info!("Sonos : voix des annonces : {}", tts.describe());
        } else {
            info!("Sonos : pas de voix (HA_PIPER) : les annonces se limitent au carillon");
        }
        let shared = Arc::new(Shared {
            api,
            config,
            clips,
            speakers: Mutex::new(Speakers::default()),
            locks: Mutex::new(HashMap::new()),
            refresh: Notify::new(),
            favorites: Mutex::new(Vec::new()),
            art: Mutex::new(art::Wanted::default()),
        });
        publish_tts(&shared, &core, None);
        core.answers_commands(ID);
        let supervisor = Arc::clone(&core);
        let task = ha_core::supervise(supervisor, ID, NAME, move |token| {
            run(Arc::clone(&shared), Arc::clone(&core), token)
        });
        Ok(Self { task, audio })
    }

    pub async fn stop(self) {
        self.task.stop().await;
        if let Some(audio) = self.audio {
            audio.stop().await;
        }
    }
}

fn bind_audio(port: u16) -> std::io::Result<tokio::net::TcpListener> {
    let listener = std::net::TcpListener::bind((Ipv4Addr::UNSPECIFIED, port))?;
    listener.set_nonblocking(true)?;
    tokio::net::TcpListener::from_std(listener)
}

async fn run(shared: Arc<Shared>, core: Arc<Core>, token: CancellationToken) {
    let mut events = core.bus.subscribe();
    let mut ticker = tokio::time::interval(shared.config.poll);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut next_topology = tokio::time::Instant::now();
    let mut backoff = Duration::from_secs(1);
    loop {
        tokio::select! {
            () = token.cancelled() => break,
            _ = ticker.tick() => {
                if tokio::time::Instant::now() >= next_topology {
                    match refresh_topology(&shared, &core).await {
                        Ok(count) => {
                            if backoff > Duration::from_secs(1) || count > 0 {
                                info!("Sonos : {count} enceinte(s)");
                            }
                            next_topology = tokio::time::Instant::now() + TOPOLOGY_EVERY;
                            backoff = Duration::from_secs(1);
                            refresh_favorites(&shared).await;
                        }
                        Err(error) => {
                            warn!("Sonos : {error}, nouvel essai dans {} s", backoff.as_secs());
                            core.report(ID, NAME, Health::Retrying {
                                error,
                                retry_at: SystemTime::now() + backoff,
                            });
                            next_topology = tokio::time::Instant::now() + backoff;
                            backoff = (backoff * 2).min(MAX_BACKOFF);
                            continue;
                        }
                    }
                }
                if poll_all(&shared, &core).await {
                    core.report(ID, NAME, Health::Connected);
                } else {
                    // Plus aucune enceinte ne répond : les adresses ont peut-être changé.
                    next_topology = tokio::time::Instant::now();
                }
            }
            () = shared.refresh.notified() => {
                poll_all(&shared, &core).await;
            }
            received = events.recv() => match received {
                Ok(event) => handle(&shared, &core, &event),
                Err(RecvError::Lagged(missed)) => warn!("Sonos : {missed} événements manqués"),
                Err(RecvError::Closed) => break,
            },
        }
    }
    info!("Sonos : arrêté");
}

/// Relit les groupes. Rend le nombre d'enceintes.
async fn refresh_topology(shared: &Arc<Shared>, core: &Core) -> Result<usize, String> {
    let mut seeds = shared.config.hosts.clone();
    for host in shared.speakers().hosts() {
        if !seeds.contains(&host) {
            seeds.push(host);
        }
    }
    let mut groups = read_topology(&shared.api, &seeds).await;
    if groups.is_none() && shared.config.hosts.is_empty() {
        let found = discovery::search(DISCOVERY_WAIT).await;
        groups = read_topology(&shared.api, &found).await;
    }
    let Some(groups) = groups else {
        return Err(if shared.config.hosts.is_empty() {
            "aucune enceinte trouvée sur le réseau (donne l'adresse d'une enceinte avec HA_SONOS)"
                .to_owned()
        } else {
            "enceintes injoignables".to_owned()
        });
    };
    let added = shared.speakers().update(groups);
    for speaker in &added {
        info!(
            "Sonos : enceinte trouvée : {} ({})",
            speaker.entity_id, speaker.name
        );
    }
    let (count, addresses) = {
        let speakers = shared.speakers();
        let addresses: HashSet<IpAddr> = speakers
            .by_uid
            .values()
            .filter_map(|s| s.host.parse::<SocketAddr>().ok())
            .map(|a| a.ip())
            .collect();
        (speakers.by_uid.len(), addresses)
    };
    if let Some(clips) = &shared.clips {
        clips.allow(addresses);
    }
    if !added.is_empty() {
        publish_tts(shared, core, None);
    }
    Ok(count)
}

/// Relit les favoris (sur la première enceinte qui répond). En cas
/// d'échec, on garde les précédents.
async fn refresh_favorites(shared: &Shared) {
    let hosts = shared.speakers().hosts();
    for host in &hosts {
        match shared.api.favorites(host).await {
            Ok(list) => {
                let mut favorites = shared
                    .favorites
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner);
                if *favorites != list {
                    info!("Sonos : {} favori(s) à lancer", list.len());
                    *favorites = list;
                }
                return;
            }
            Err(error) => tracing::debug!("Sonos : favoris de {host} : {error}"),
        }
    }
}

/// Pour `ha-server sonos` : les groupes et leurs enceintes, d'après les
/// adresses données ou, à défaut, en les cherchant sur le réseau.
pub async fn find(hosts: &[String]) -> Result<Vec<Group>, String> {
    let api = Api::new().map_err(|error| error.to_string())?;
    let seeds = if hosts.is_empty() {
        discovery::search(DISCOVERY_WAIT).await
    } else {
        hosts.to_vec()
    };
    read_topology(&api, &seeds).await.ok_or_else(|| {
        if hosts.is_empty() {
            "aucune enceinte Sonos n'a répondu sur le réseau local".to_owned()
        } else {
            format!("aucune réponse de {}", hosts.join(", "))
        }
    })
}

async fn read_topology(api: &Api, seeds: &[String]) -> Option<Vec<Group>> {
    for host in seeds {
        match api.zone_group_state(host).await {
            Ok(state) => {
                let groups = topology::parse(&state);
                if !groups.is_empty() {
                    return Some(groups);
                }
            }
            Err(error) => tracing::debug!("Sonos : {host} : {error}"),
        }
    }
    None
}

/// Lit l'état de toutes les enceintes. Rend `false` si aucune n'a répondu.
async fn poll_all(shared: &Arc<Shared>, core: &Core) -> bool {
    let (groups, names) = {
        let speakers = shared.speakers();
        let names: HashMap<String, Speaker> = speakers.by_uid.clone();
        (speakers.groups.clone(), names)
    };
    if groups.is_empty() {
        return false;
    }
    let polls = groups
        .iter()
        .map(|group| poll_group(&shared.api, group, &names));
    let results = futures_util::future::join_all(polls).await;
    let favorites = shared.favorites();
    let mut hosts: Vec<String> = names.values().map(|s| s.host.clone()).collect();
    hosts.sort();
    let now = jiff::Timestamp::now();
    // Les pochettes à aller chercher : celles qui jouent d'abord.
    let mut wanted: Vec<(String, String)> = Vec::new();
    let favorite_pictures = favorite_pictures(&favorites, &hosts, &core.covers, &mut wanted);
    let mut answered = false;
    for (uid, reading) in results.into_iter().flatten() {
        let Some(speaker) = names.get(&uid) else {
            continue;
        };
        let (state, attributes) = if let Some(reading) = reading {
            answered = true;
            let (state, mut attributes) = player::entity(&reading);
            favorites::describe(&favorites, &reading.media, &mut attributes);
            if !favorite_pictures.is_empty() {
                attributes.insert("source_pictures".into(), json!(favorite_pictures));
            }
            if state != "idle"
                && let Some((key, url)) = playing_art(&reading, &favorites, &hosts)
            {
                if core.covers.contains(&key) {
                    attributes.insert("entity_picture".into(), json!(art::picture(&key)));
                } else {
                    wanted.insert(0, (key, url));
                }
            }
            let previous = core.states.get(&speaker.entity_id);
            player::track_position(
                &mut attributes,
                state,
                &reading,
                previous.as_ref().map(|p| (p.state(), p.attributes())),
                now,
            );
            (state, attributes)
        } else {
            let mut attributes = Attributes::new();
            attributes.insert("friendly_name".into(), json!(speaker.name));
            attributes.insert("device_class".into(), json!("speaker"));
            (ha_core::integrations::UNAVAILABLE, attributes)
        };
        if let Err(error) = core
            .states
            .set_by(ID, speaker.entity_id.clone(), state, attributes)
        {
            warn!("Sonos : état refusé pour {} : {error}", speaker.entity_id);
        }
    }
    fetch_covers(shared, core, wanted);
    answered
}

/// Au plus autant de pochettes de favoris.
const MAX_FAVORITE_PICTURES: usize = 48;
/// Au plus autant de pochettes téléchargées à la suite.
const COVERS_PER_BATCH: usize = 8;

/// Les pochettes des favoris déjà là (nom → adresse) ; les autres sont
/// ajoutées à `wanted`.
fn favorite_pictures(
    favorites: &[Favorite],
    hosts: &[String],
    covers: &ha_core::covers::Covers,
    wanted: &mut Vec<(String, String)>,
) -> serde_json::Map<String, Value> {
    let mut pictures = serde_json::Map::new();
    let Some(first) = hosts.first() else {
        return pictures;
    };
    for favorite in favorites.iter().take(MAX_FAVORITE_PICTURES) {
        let Some(url) = favorite
            .art
            .as_deref()
            .and_then(|a| art::resolve(a, first, hosts))
        else {
            continue;
        };
        let key = art::key(&url);
        if covers.contains(&key) {
            pictures.insert(favorite.title.clone(), json!(art::picture(&key)));
        } else {
            wanted.push((key, url));
        }
    }
    pictures
}

/// La pochette de ce qui joue : celle du morceau, sinon celle de la radio,
/// sinon celle du favori lancé. Rend (clé, adresse).
fn playing_art(
    reading: &Reading,
    favorites: &[Favorite],
    hosts: &[String],
) -> Option<(String, String)> {
    let found = art::in_metadata(&reading.position.metadata)
        .or_else(|| art::in_metadata(&reading.media.metadata))
        .or_else(|| {
            favorites
                .iter()
                .find(|f| f.uri == reading.media.uri)
                .and_then(|f| f.art.clone())
        })?;
    let url = art::resolve(&found, &reading.coordinator, hosts)?;
    Some((art::key(&url), url))
}

/// Va chercher les pochettes manquantes, en arrière-plan, puis fait relire
/// l'état pour qu'elles s'affichent.
fn fetch_covers(shared: &Arc<Shared>, core: &Core, wanted: Vec<(String, String)>) {
    let now = tokio::time::Instant::now();
    let batch: Vec<(String, String)> = {
        let mut state = shared.art.lock().unwrap_or_else(PoisonError::into_inner);
        let mut batch = Vec::new();
        for (key, url) in wanted {
            if batch.len() >= COVERS_PER_BATCH {
                break;
            }
            if !batch.iter().any(|(k, _)| *k == key) && state.start(&key, now) {
                batch.push((key, url));
            }
        }
        batch
    };
    if batch.is_empty() {
        return;
    }
    let shared = Arc::clone(shared);
    let covers = Arc::clone(&core.covers);
    tokio::spawn(async move {
        let mut stored = false;
        for (key, url) in batch {
            let result = art::fetch(shared.api.http(), &url)
                .await
                .and_then(|bytes| covers.set(&key, &bytes).map(drop));
            if let Err(error) = &result {
                // Pas l'adresse dans le journal : elle peut contenir un jeton.
                tracing::debug!("Sonos : pochette {key} : {error}");
            }
            stored |= result.is_ok();
            shared
                .art
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .finish(&key, result.is_ok(), tokio::time::Instant::now());
        }
        if stored {
            shared.refresh.notify_one();
        }
    });
}

async fn poll_group(
    api: &Api,
    group: &Group,
    names: &HashMap<String, Speaker>,
) -> Vec<(String, Option<Reading>)> {
    let Some(coordinator) = group.coordinator() else {
        return Vec::new();
    };
    let host = coordinator.host.as_str();
    let (transport, position, media, play_mode) = tokio::join!(
        api.transport_state(host),
        api.position(host),
        api.media(host),
        api.play_mode(host)
    );
    let play_mode = play_mode.ok();
    let shared = match (transport, position, media) {
        (Ok(t), Ok(p), Ok(m)) => Some((t, p, m)),
        _ => None,
    };
    let mut out = Vec::new();
    for member in &group.members {
        let reading = match &shared {
            Some((transport, position, media)) => {
                match tokio::join!(api.volume(&member.host), api.muted(&member.host)) {
                    (Ok(volume), Ok(muted)) => Some(Reading {
                        name: member.name.clone(),
                        transport: transport.clone(),
                        position: position.clone(),
                        media: media.clone(),
                        volume,
                        muted,
                        group: group
                            .members
                            .iter()
                            .filter(|m| m.uid != member.uid)
                            .filter_map(|m| names.get(&m.uid))
                            .map(|s| s.entity_id.to_string())
                            .collect(),
                        coordinator: host.to_owned(),
                        play_mode: play_mode.clone(),
                    }),
                    _ => None,
                }
            }
            None => None,
        };
        out.push((member.uid.clone(), reading));
    }
    out
}

/// Un ordre pour une enceinte ou pour les annonces.
fn handle(shared: &Arc<Shared>, core: &Arc<Core>, event: &Event) {
    let Event::CallService {
        domain,
        service,
        entity_id,
        data,
        command,
        issued_at,
        ..
    } = event
    else {
        return;
    };
    let ours = match domain.as_str() {
        "media_player" => shared.speakers().by_entity.contains_key(entity_id),
        "tts" => entity_id.as_str() == TTS_ENTITY,
        _ => false,
    };
    if !ours {
        return;
    }
    if ha_core::commands::is_stale(*issued_at) {
        warn!("Sonos : ordre périmé ignoré pour {entity_id}");
        core.command_done(*command, Err("ordre périmé".into()));
        return;
    }
    let shared = Arc::clone(shared);
    let core = Arc::clone(core);
    let (domain, service, entity_id, data, command) = (
        domain.clone(),
        service.clone(),
        entity_id.clone(),
        data.clone(),
        *command,
    );
    tokio::spawn(async move {
        let result = if domain == "tts" {
            if service == "speak" {
                speak(&shared, &core, &data, command).await;
                return;
            }
            Err(format!("tts.{service} n'existe pas (seulement tts.speak)"))
        } else {
            media_command(&shared, &core, &service, &entity_id, &data).await
        };
        if let Err(error) = &result {
            warn!("Sonos : {domain}.{service} sur {entity_id} : {error}");
        }
        core.command_done(command, result);
        shared.refresh.notify_one();
    });
}

async fn media_command(
    shared: &Shared,
    core: &Core,
    service: &str,
    entity_id: &EntityId,
    data: &Attributes,
) -> Result<(), String> {
    let (own, coordinator) = shared
        .speakers()
        .hosts_for(entity_id)
        .ok_or_else(|| "enceinte inconnue".to_owned())?;
    let api = &shared.api;
    let state = core.states.get(entity_id);
    let current_volume = state
        .as_ref()
        .and_then(|s| s.attributes().get("volume_level")?.as_f64())
        .unwrap_or(0.0)
        * 100.0;
    let result = match service {
        "media_play" => api.play(&coordinator).await,
        "media_pause" => pause(api, &coordinator).await,
        "media_play_pause" => {
            if state.as_ref().is_some_and(|s| s.state() == "playing") {
                pause(api, &coordinator).await
            } else {
                api.play(&coordinator).await
            }
        }
        "media_stop" => api.stop(&coordinator).await,
        "media_next_track" => api.next(&coordinator).await,
        "media_previous_track" => api.previous(&coordinator).await,
        "volume_set" => {
            let level = data
                .get("volume_level")
                .and_then(Value::as_f64)
                .filter(|l| (0.0..=1.0).contains(l))
                .ok_or_else(|| "volume_level entre 0 et 1".to_owned())?;
            api.set_volume(&own, percent(level * 100.0)).await
        }
        "volume_up" => {
            api.set_volume(&own, percent(current_volume + VOLUME_STEP))
                .await
        }
        "volume_down" => {
            api.set_volume(&own, percent(current_volume - VOLUME_STEP))
                .await
        }
        "volume_mute" => {
            let muted = data
                .get("is_volume_muted")
                .and_then(Value::as_bool)
                .ok_or_else(|| "is_volume_muted : true ou false".to_owned())?;
            api.set_mute(&own, muted).await
        }
        "select_source" => return select_source(shared, entity_id, &coordinator, data).await,
        "media_seek" => {
            let seconds = data
                .get("seek_position")
                .and_then(Value::as_f64)
                .filter(|p| p.is_finite() && (0.0..86_400.0).contains(p))
                .ok_or_else(|| "seek_position : un nombre de secondes".to_owned())?;
            api.seek(
                &coordinator,
                "REL_TIME",
                &player::hms(whole_seconds(seconds)),
            )
            .await
        }
        "shuffle_set" | "repeat_set" => {
            let current = api
                .play_mode(&coordinator)
                .await
                .map_err(|error| error.to_string())?;
            let (mut shuffle, mut repeat) = player::play_mode(&current).unwrap_or((false, "off"));
            if service == "shuffle_set" {
                shuffle = data
                    .get("shuffle")
                    .and_then(Value::as_bool)
                    .ok_or_else(|| "shuffle : true ou false".to_owned())?;
            } else {
                let wanted = data.get("repeat").and_then(Value::as_str);
                repeat = ["off", "all", "one"]
                    .into_iter()
                    .find(|r| Some(*r) == wanted)
                    .ok_or_else(|| "repeat : off, all ou one".to_owned())?;
            }
            let mode = player::mode_name(shuffle, repeat)
                .ok_or_else(|| "mode de lecture inconnu".to_owned())?;
            api.set_play_mode(&coordinator, mode).await
        }
        other => return Err(format!("media_player.{other} non pris en charge par Sonos")),
    };
    result.map_err(|error| error.to_string())
}

/// Lance un favori sur le groupe de l'enceinte, comme Home Assistant : une
/// radio est chargée directement ; une playlist ou un album remplace la
/// file d'attente, qui est ensuite lue.
async fn select_source(
    shared: &Shared,
    entity_id: &EntityId,
    coordinator: &str,
    data: &Attributes,
) -> Result<(), String> {
    let name = data
        .get("source")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "indique le favori (source)".to_owned())?;
    let favorite = shared
        .favorites()
        .into_iter()
        .find(|f| f.title == name)
        .ok_or_else(|| format!("favori « {name} » inconnu"))?;
    let api = &shared.api;
    let result = match favorite.launch() {
        Launch::Direct => {
            api.set_uri(coordinator, &favorite.uri, &favorite.metadata)
                .await
        }
        Launch::Queue => {
            let uid = shared
                .speakers()
                .coordinator_uid(entity_id)
                .ok_or_else(|| "enceinte inconnue".to_owned())?;
            async {
                api.clear_queue(coordinator).await?;
                api.add_to_queue(coordinator, &favorite.uri, &favorite.metadata)
                    .await?;
                api.set_uri(coordinator, &format!("x-rincon-queue:{uid}#0"), "")
                    .await
            }
            .await
        }
    };
    result.map_err(|error| error.to_string())?;
    api.play(coordinator)
        .await
        .map_err(|error| error.to_string())?;
    info!(
        "Sonos : favori « {} » lancé sur {entity_id}",
        favorite.title
    );
    Ok(())
}

/// Une radio ne se met pas en pause (erreur 701) : on l'arrête.
async fn pause(api: &Api, host: &str) -> Result<(), SonosError> {
    match api.pause(host).await {
        Err(SonosError::Refused(701)) => api.stop(host).await,
        other => other,
    }
}

/// Des secondes (déjà bornées entre 0 et un jour) en nombre entier.
fn whole_seconds(value: f64) -> u64 {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "borné"
    )]
    let value = value.round().clamp(0.0, 86_399.0) as u64;
    value
}

fn percent(value: f64) -> u8 {
    // Borné entre 0 et 100 juste avant.
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "borné"
    )]
    let value = value.round().clamp(0.0, 100.0) as u8;
    value
}

/// Ce que demande `tts.speak`.
#[derive(Debug, Clone, PartialEq)]
pub struct SpeakRequest {
    pub targets: Vec<EntityId>,
    pub message: Option<String>,
    pub chime: bool,
    pub volume: Option<u8>,
}

/// Lit les détails de `tts.speak` : `media_player_entity_id` (une enceinte
/// ou une liste), `message`, `carillon` (oui par défaut), `volume` (0 à 100).
pub fn parse_speak(data: &Attributes) -> Result<SpeakRequest, String> {
    let targets = match data.get("media_player_entity_id") {
        Some(Value::String(one)) => vec![one.clone()],
        Some(Value::Array(many)) => many
            .iter()
            .map(|v| v.as_str().map(str::to_owned).ok_or("enceinte invalide"))
            .collect::<Result<_, _>>()?,
        _ => return Err("indique les enceintes (media_player_entity_id)".into()),
    };
    if targets.is_empty() || targets.len() > MAX_TARGETS {
        return Err(format!("de 1 à {MAX_TARGETS} enceintes"));
    }
    let targets = targets
        .iter()
        .map(|t| {
            t.parse::<EntityId>()
                .ok()
                .filter(|id| id.domain() == "media_player")
                .ok_or_else(|| format!("« {t} » n'est pas une enceinte (media_player.…)"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let message = match data.get("message") {
        None | Some(Value::Null) => None,
        Some(Value::String(text)) => ha_tts::clean_text(text),
        Some(_) => return Err("le message doit être un texte".into()),
    };
    if let Some(Value::String(text)) = data.get("message")
        && text.chars().count() > ha_tts::MAX_TEXT
    {
        return Err(format!(
            "message de {} caractères au plus",
            ha_tts::MAX_TEXT
        ));
    }
    let chime = match data.get("carillon") {
        None | Some(Value::Null) => true,
        Some(Value::Bool(chime)) => *chime,
        Some(_) => return Err("carillon : true ou false".into()),
    };
    let volume = match data.get("volume") {
        None | Some(Value::Null) => None,
        Some(value) => Some(
            value
                .as_f64()
                .filter(|v| (0.0..=100.0).contains(v))
                .map(percent)
                .ok_or("volume entre 0 et 100")?,
        ),
    };
    if message.is_none() && !chime {
        return Err("rien à annoncer : un message, ou le carillon".into());
    }
    Ok(SpeakRequest {
        targets,
        message,
        chime,
        volume,
    })
}

/// `tts.speak` : prépare le son, puis le joue sur chaque groupe visé.
async fn speak(shared: &Arc<Shared>, core: &Arc<Core>, data: &Attributes, command: Option<u64>) {
    let result = prepare_and_play(shared, core, data, command).await;
    if let Err(error) = result {
        warn!("Sonos : annonce impossible : {error}");
        core.command_done(command, Err(error));
    }
}

async fn prepare_and_play(
    shared: &Arc<Shared>,
    core: &Arc<Core>,
    data: &Attributes,
    command: Option<u64>,
) -> Result<(), String> {
    let request = parse_speak(data)?;
    let clips = shared.clips.clone().ok_or_else(|| {
        format!(
            "le serveur des annonces n'a pas pu démarrer (port {} déjà pris ?)",
            shared.config.audio_port
        )
    })?;
    // Les groupes visés, chacun une seule fois.
    let mut targets: Vec<Target> = Vec::new();
    {
        let speakers = shared.speakers();
        for entity_id in &request.targets {
            let uid = speakers
                .by_entity
                .get(entity_id)
                .ok_or_else(|| format!("enceinte inconnue : {entity_id}"))?;
            let group = speakers
                .group_of(uid)
                .ok_or_else(|| format!("enceinte absente : {entity_id}"))?;
            let coordinator = group
                .coordinator()
                .ok_or_else(|| format!("groupe sans coordinateur : {entity_id}"))?;
            if !targets.iter().any(|t| t.coordinator == coordinator.host) {
                targets.push(Target {
                    coordinator: coordinator.host.clone(),
                    members: group.members.iter().map(|m| m.host.clone()).collect(),
                });
            }
        }
    }
    let voice = match (&request.message, &shared.config.tts) {
        (Some(message), Some(tts)) => Some(tts.speak(message).await.map_err(|e| e.to_string())?),
        (Some(_), None) => {
            return Err(
                "pas de voix installée (HA_PIPER, voir docs/sonos.md) : seul le carillon est possible"
                    .into(),
            );
        }
        (None, _) => None,
    };
    let sound = ha_tts::announcement(request.chime, voice.as_ref());
    let length = sound.duration;
    let file = clips
        .add(sound.wav)
        .ok_or_else(|| "aléatoire indisponible".to_owned())?;

    let mut started = Vec::new();
    let mut tasks = Vec::new();
    for target in targets {
        let address = match shared.config.audio_address {
            Some(address) => address,
            None => local_address_toward(&target.coordinator)
                .ok_or_else(|| format!("pas de chemin réseau vers {}", target.coordinator))?,
        };
        let url = format!(
            "http://{address}:{}/annonce/{file}",
            shared.config.audio_port
        );
        let (sender, receiver) = oneshot::channel();
        started.push(receiver);
        let shared = Arc::clone(shared);
        let volume = request.volume;
        tasks.push(tokio::spawn(async move {
            let lock = shared.lock_for(&target.coordinator);
            let _turn = lock.lock().await;
            match announce::start(&shared.api, &target, &url, volume).await {
                Ok(snapshot) => {
                    let _ = sender.send(Ok(()));
                    shared.refresh.notify_one();
                    announce::finish(
                        &shared.api,
                        &target,
                        &url,
                        length,
                        &snapshot,
                        shared.config.timing,
                    )
                    .await;
                }
                Err(error) => {
                    let _ = sender.send(Err(error.to_string()));
                }
            }
        }));
    }
    let mut outcome = Ok(());
    for receiver in started {
        if let Ok(Err(error)) = receiver.await
            && outcome.is_ok()
        {
            outcome = Err(error);
        }
    }
    core.command_done(command, outcome.clone());
    if outcome.is_ok() {
        info!(
            "Sonos : annonce « {} »",
            request.message.as_deref().unwrap_or("carillon")
        );
        publish_tts(
            shared,
            core,
            Some(request.message.as_deref().unwrap_or("(carillon)")),
        );
    }
    for task in tasks {
        let _ = task.await;
    }
    clips.remove(&file);
    shared.refresh.notify_one();
    outcome
}

/// L'adresse de ce PC sur le chemin vers l'enceinte : on « connecte » une
/// socket UDP (rien n'est envoyé) et on regarde l'adresse locale choisie.
fn local_address_toward(host: &str) -> Option<IpAddr> {
    let target: SocketAddr = host.parse().ok()?;
    let socket = std::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    socket.connect(target).ok()?;
    let ip = socket.local_addr().ok()?.ip();
    (!ip.is_unspecified()).then_some(ip)
}

/// L'entité `tts.annonces` : la voix, les enceintes, la dernière annonce.
fn publish_tts(shared: &Shared, core: &Core, message: Option<&str>) {
    let Ok(entity_id) = EntityId::new(TTS_ENTITY) else {
        return;
    };
    let previous = core.states.get(&entity_id);
    let state = if message.is_some() {
        jiff::Timestamp::now().to_string()
    } else {
        previous
            .as_ref()
            .map_or_else(|| "unknown".to_owned(), |s| s.state().to_owned())
    };
    let last = message.map(str::to_owned).or_else(|| {
        previous.as_ref().and_then(|s| {
            s.attributes()
                .get("dernier_message")?
                .as_str()
                .map(str::to_owned)
        })
    });
    let mut attributes = Attributes::new();
    attributes.insert("friendly_name".into(), json!("Annonces"));
    attributes.insert(
        "voix".into(),
        json!(
            shared
                .config
                .tts
                .as_ref()
                .map_or_else(|| "aucune : carillon seulement".to_owned(), Tts::describe)
        ),
    );
    attributes.insert("enceintes".into(), json!(shared.speakers().entities()));
    if let Some(last) = last {
        attributes.insert("dernier_message".into(), json!(last));
    }
    if shared.clips.is_none() {
        attributes.insert(
            "indisponible".into(),
            json!("serveur des annonces non démarré"),
        );
    }
    if let Err(error) = core.states.set_by(ID, entity_id, state, attributes) {
        warn!("Sonos : état des annonces refusé : {error}");
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]

    use super::*;

    fn data(value: Value) -> Attributes {
        match value {
            Value::Object(map) => map,
            _ => Attributes::new(),
        }
    }

    #[test]
    fn hosts_and_audio_settings() {
        assert_eq!(parse_hosts("auto").unwrap(), Vec::<String>::new());
        assert_eq!(
            parse_hosts("192.168.1.30, 192.168.1.31:1400").unwrap(),
            ["192.168.1.30:1400", "192.168.1.31:1400"]
        );
        assert!(parse_hosts("192.168.1.30:80").is_err());
        assert!(parse_hosts("a/b").is_err());
        assert_eq!(
            parse_audio("192.168.1.20").unwrap(),
            ("192.168.1.20".parse().unwrap(), 8124)
        );
        assert_eq!(parse_audio("192.168.1.20:9000").unwrap().1, 9000);
        assert!(parse_audio("127.0.0.1").is_err());
        assert!(parse_audio("0.0.0.0:8124").is_err());
        assert!(parse_audio("maison").is_err());
        assert!(parse_audio("192.168.1.20:0").is_err());
    }

    #[test]
    fn speak_requests() {
        let request = parse_speak(&data(json!({
            "media_player_entity_id": "media_player.salon",
            "message": "  Le linge\nest fini ",
            "volume": 40,
        })))
        .unwrap();
        assert_eq!(
            request.targets,
            ["media_player.salon".parse::<EntityId>().unwrap()]
        );
        assert_eq!(request.message.as_deref(), Some("Le linge est fini"));
        assert!(request.chime);
        assert_eq!(request.volume, Some(40));
        let chime_only = parse_speak(&data(json!({
            "media_player_entity_id": ["media_player.salon", "media_player.cuisine"],
        })))
        .unwrap();
        assert_eq!(chime_only.targets.len(), 2);
        assert_eq!(chime_only.message, None);
        for bad in [
            json!({"message": "x"}),
            json!({"media_player_entity_id": "light.salon", "message": "x"}),
            json!({"media_player_entity_id": [], "message": "x"}),
            json!({"media_player_entity_id": "media_player.salon", "carillon": false}),
            json!({"media_player_entity_id": "media_player.salon", "message": "x", "volume": 140}),
            json!({"media_player_entity_id": "media_player.salon", "message": 3}),
            json!({"media_player_entity_id": "media_player.salon", "message": "x".repeat(600)}),
        ] {
            assert!(parse_speak(&data(bad.clone())).is_err(), "{bad}");
        }
    }

    #[test]
    fn speakers_get_stable_names() {
        let group = |coordinator: &str, members: &[(&str, &str, &str)]| Group {
            coordinator: coordinator.into(),
            members: members
                .iter()
                .map(|(uid, name, host)| topology::Member {
                    uid: (*uid).into(),
                    name: (*name).into(),
                    host: (*host).into(),
                })
                .collect(),
        };
        let mut speakers = Speakers::default();
        let added = speakers.update(vec![
            group(
                "RINCON_A",
                &[
                    ("RINCON_A", "Salon", "192.168.1.30:1400"),
                    ("RINCON_B", "Cuisine", "192.168.1.31:1400"),
                ],
            ),
            group("RINCON_C", &[("RINCON_C", "Salon", "192.168.1.32:1400")]),
        ]);
        let ids: Vec<String> = added.iter().map(|s| s.entity_id.to_string()).collect();
        assert_eq!(
            ids,
            [
                "media_player.salon",
                "media_player.cuisine",
                "media_player.salon_2"
            ]
        );
        let cuisine: EntityId = "media_player.cuisine".parse().unwrap();
        assert_eq!(
            speakers.hosts_for(&cuisine),
            Some(("192.168.1.31:1400".into(), "192.168.1.30:1400".into()))
        );
        // La cuisine quitte le groupe, et change d'adresse : même entité.
        let added = speakers.update(vec![
            group("RINCON_A", &[("RINCON_A", "Salon", "192.168.1.30:1400")]),
            group("RINCON_B", &[("RINCON_B", "Cuisine", "192.168.1.40:1400")]),
        ]);
        assert!(added.is_empty());
        assert_eq!(
            speakers.hosts_for(&cuisine),
            Some(("192.168.1.40:1400".into(), "192.168.1.40:1400".into()))
        );
        assert_eq!(speakers.entities().len(), 3);
    }

    #[test]
    fn volume_percent_is_bounded() {
        assert_eq!(percent(-3.0), 0);
        assert_eq!(percent(42.4), 42);
        assert_eq!(percent(250.0), 100);
    }
}
