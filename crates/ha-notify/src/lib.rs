//! Notifications sur le téléphone, avec **ntfy** (<https://ntfy.sh>) : une
//! appli libre pour Android et iPhone, et un serveur qu'on peut héberger
//! soi-même.
//!
//! Comme dans Home Assistant, on envoie une notification avec l'action
//! `notify.send_message` sur l'entité `notify.telephone` :
//!
//! ```yaml
//! - action: notify.send_message
//!   target:
//!     entity_id: notify.telephone
//!   data:
//!     title: Fumée !
//!     message: Le détecteur du salon s'est déclenché.
//!     priority: 5
//! ```

use std::collections::VecDeque;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use ha_core::{Attributes, Background, CancellationToken, Core, EntityId, Event};
use serde_json::{Value, json};
use tokio::sync::broadcast::error::RecvError;
use tracing::{info, warn};
use zeroize::Zeroizing;

/// L'entité qui représente le téléphone.
pub const ENTITY: &str = "notify.telephone";
/// Pas plus de `MAX_BURST` notifications en `BURST_WINDOW` : une
/// automatisation qui boucle ne doit pas faire vibrer le téléphone sans fin.
const MAX_BURST: usize = 10;
const BURST_WINDOW: Duration = Duration::from_mins(10);
const MAX_TITLE_CHARS: usize = 100;
const MAX_MESSAGE_CHARS: usize = 1_000;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum NotifyError {
    #[error("adresse du serveur ntfy invalide : {0}")]
    Url(String),
    #[error("sujet (topic) ntfy invalide : lettres, chiffres, « - » et « _ », 1 à 64 caractères")]
    Topic,
    #[error("jeton ntfy invalide")]
    Token,
}

/// Où envoyer les notifications.
#[derive(Clone)]
pub struct Config {
    /// `https://ntfy.sh`, ou ton serveur : `http://192.168.1.10:8080`.
    pub server: reqwest::Url,
    pub topic: String,
    /// Jeton d'accès ntfy (`tk_…`), si le sujet est protégé.
    pub token: Option<Zeroizing<String>>,
}

/// Le jeton ne doit jamais apparaître dans un affichage de débogage.
impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config")
            .field("server", &self.server.as_str())
            .field("topic", &"<masqué>")
            .field("token", &self.token.as_ref().map(|_| "<masqué>"))
            .finish()
    }
}

impl Config {
    /// Vérifie la configuration. Le HTTP non chiffré n'est accepté que vers
    /// une adresse du réseau local.
    pub fn new(
        server: &str,
        topic: &str,
        token: Option<Zeroizing<String>>,
    ) -> Result<Self, NotifyError> {
        let server = reqwest::Url::parse(server.trim())
            .map_err(|error| NotifyError::Url(error.to_string()))?;
        match server.scheme() {
            "https" => {}
            "http" if is_local(&server) => {}
            "http" => {
                return Err(NotifyError::Url(
                    "HTTP non chiffré seulement vers le réseau local ; utilise https://".into(),
                ));
            }
            other => return Err(NotifyError::Url(format!("protocole {other:?} refusé"))),
        }
        if !server.username().is_empty()
            || server.password().is_some()
            || server.query().is_some()
            || server.fragment().is_some()
        {
            return Err(NotifyError::Url(
                "pas d'identifiants, de « ? » ni de « # » dans l'adresse".into(),
            ));
        }
        let valid_topic = (1..=64).contains(&topic.len())
            && topic
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
        if !valid_topic {
            return Err(NotifyError::Topic);
        }
        if let Some(token) = &token {
            let valid =
                (1..=200).contains(&token.len()) && token.bytes().all(|b| b.is_ascii_graphic());
            if !valid {
                return Err(NotifyError::Token);
            }
        }
        Ok(Self {
            server,
            topic: topic.to_owned(),
            token,
        })
    }

    /// Conseils de sécurité à afficher au démarrage.
    #[must_use]
    pub fn warnings(&self) -> Vec<&'static str> {
        let mut warnings = Vec::new();
        let public = self.server.host_str() == Some("ntfy.sh");
        if public && self.token.is_none() && self.topic.len() < 20 {
            warnings.push(
                "sujet ntfy court sur le serveur public, sans jeton : n'importe qui pourrait \
                 le deviner et lire tes notifications. Choisis un sujet long et aléatoire.",
            );
        }
        if self.server.scheme() == "http" {
            warnings.push("notifications envoyées en HTTP non chiffré (réseau local seulement)");
        }
        warnings
    }
}

/// Adresse du réseau local : `localhost`, IP privée, nom en `.local`/`.lan`.
fn is_local(url: &reqwest::Url) -> bool {
    match url.host() {
        Some(url::Host::Ipv4(ip)) => ip.is_private() || ip.is_loopback() || ip.is_link_local(),
        Some(url::Host::Ipv6(ip)) => IpAddr::V6(ip).is_loopback(),
        // `url` a déjà mis le nom en minuscules.
        Some(url::Host::Domain(name)) => {
            name == "localhost" || matches!(name.rsplit_once('.'), Some((_, "local" | "lan")))
        }
        None => false,
    }
}

/// Une notification à envoyer, nettoyée et bornée.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub title: Option<String>,
    pub message: String,
    /// 1 (discrète) à 5 (urgente) ; 3 par défaut.
    pub priority: u8,
}

impl Message {
    /// Lit les données de `notify.send_message`. `None` s'il n'y a pas de
    /// message.
    #[must_use]
    pub fn from_data(data: &Attributes) -> Option<Self> {
        let message = clean(data.get("message")?.as_str()?, MAX_MESSAGE_CHARS);
        if message.is_empty() {
            return None;
        }
        let title = data
            .get("title")
            .and_then(Value::as_str)
            .map(|title| clean(title, MAX_TITLE_CHARS))
            .filter(|title| !title.is_empty());
        let priority = data
            .get("priority")
            .and_then(Value::as_u64)
            .and_then(|p| u8::try_from(p).ok())
            .map_or(3, |p| p.clamp(1, 5));
        Some(Self {
            title,
            message,
            priority,
        })
    }
}

/// Garde les retours à la ligne, retire les autres caractères de contrôle,
/// et coupe à `max` caractères.
fn clean(text: &str, max: usize) -> String {
    text.trim()
        .chars()
        .filter(|c| *c == '\n' || !c.is_control())
        .take(max)
        .collect()
}

/// Envoie une notification à ntfy (format JSON : pas d'en-tête à composer,
/// donc pas d'injection d'en-tête possible).
pub async fn send(
    http: &reqwest::Client,
    config: &Config,
    message: &Message,
) -> Result<(), reqwest::Error> {
    let mut body = json!({
        "topic": config.topic,
        "message": message.message,
        "priority": message.priority,
    });
    if let (Some(title), Value::Object(map)) = (&message.title, &mut body) {
        map.insert("title".into(), title.clone().into());
    }
    let mut request = http
        .post(config.server.clone())
        .header("Content-Type", "application/json")
        .body(body.to_string());
    if let Some(token) = &config.token {
        request = request.bearer_auth(token.as_str());
    }
    request.send().await?.error_for_status()?;
    Ok(())
}

/// Le service de notifications : écoute `notify.send_message` sur le bus.
#[derive(Debug)]
pub struct Notifier(Background);

impl Notifier {
    pub fn start(config: Config, core: Arc<Core>) -> Result<Self, ha_tls::TlsError> {
        let http = ha_tls::web_client(config.server.scheme() == "https")?;
        let events = core.bus.subscribe();
        if let Ok(entity_id) = ENTITY.parse::<EntityId>() {
            let mut attributes = Attributes::new();
            attributes.insert("friendly_name".into(), "Téléphone".into());
            if let Err(error) = core.states.set(entity_id, "unknown", attributes) {
                warn!("notifications : état refusé ({error})");
            }
        }
        Ok(Self(Background::spawn("les notifications", move |token| {
            run(config, http, core, events, token)
        })))
    }

    pub async fn stop(self) {
        self.0.stop().await;
    }
}

async fn run(
    config: Config,
    http: reqwest::Client,
    core: Arc<Core>,
    mut events: tokio::sync::broadcast::Receiver<Arc<Event>>,
    token: CancellationToken,
) {
    info!("notifications : prêtes ({})", config.server);
    let mut recent: VecDeque<Instant> = VecDeque::new();
    loop {
        let event = tokio::select! {
            () = token.cancelled() => break,
            received = events.recv() => match received {
                Ok(event) => event,
                Err(RecvError::Lagged(missed)) => {
                    warn!("notifications : {missed} événements manqués");
                    continue;
                }
                Err(RecvError::Closed) => break,
            },
        };
        let Event::CallService {
            domain,
            service,
            entity_id,
            data,
        } = event.as_ref()
        else {
            continue;
        };
        if domain != "notify" || service != "send_message" || entity_id.as_str() != ENTITY {
            continue;
        }
        let Some(message) = Message::from_data(data) else {
            warn!("notifications : « message » manquant, rien envoyé");
            continue;
        };
        let now = Instant::now();
        while recent
            .front()
            .is_some_and(|t| now.duration_since(*t) > BURST_WINDOW)
        {
            recent.pop_front();
        }
        if recent.len() >= MAX_BURST {
            warn!("notifications : plus de {MAX_BURST} en 10 minutes, celle-ci est ignorée");
            continue;
        }
        recent.push_back(now);
        // Le contenu n'est pas écrit dans les logs : il peut être personnel.
        match send(&http, &config, &message).await {
            Ok(()) => {
                info!("notification envoyée");
                if let Ok(entity_id) = ENTITY.parse::<EntityId>() {
                    let mut attributes = Attributes::new();
                    attributes.insert("friendly_name".into(), "Téléphone".into());
                    let _ =
                        core.states
                            .set(entity_id, jiff::Timestamp::now().to_string(), attributes);
                }
            }
            Err(error) => warn!("notifications : envoi impossible ({error})"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configurations() {
        assert!(Config::new("https://ntfy.sh", "maison-a8f3k2", None).is_ok());
        assert!(Config::new("http://192.168.1.10:8080", "maison", None).is_ok());
        assert!(Config::new("http://ntfy.local", "maison", None).is_ok());
        // HTTP vers Internet, protocoles exotiques, identifiants dans l'adresse.
        for bad in [
            "http://ntfy.sh",
            "ftp://ntfy.sh",
            "https://user:pass@ntfy.sh",
            "https://ntfy.sh/?x=1",
            "pas une adresse",
        ] {
            assert!(Config::new(bad, "maison", None).is_err(), "{bad}");
        }
        for topic in ["", "a/b", "../x", "é", &"a".repeat(65)] {
            assert_eq!(
                Config::new("https://ntfy.sh", topic, None).unwrap_err(),
                NotifyError::Topic
            );
        }
        let token = Zeroizing::new("tk_abc\r\nX-Injection: 1".to_owned());
        assert_eq!(
            Config::new("https://ntfy.sh", "maison", Some(token)).unwrap_err(),
            NotifyError::Token
        );
    }

    #[test]
    fn security_warnings() {
        let weak = Config::new("https://ntfy.sh", "maison", None).unwrap();
        assert_eq!(weak.warnings().len(), 1);
        let strong = Config::new("https://ntfy.sh", "maison-7f3a9c1e2b5d8a4f6c0e", None).unwrap();
        assert_eq!(strong.warnings().len(), 0);
        let token = Zeroizing::new("tk_secret".to_owned());
        let with_token = Config::new("https://ntfy.sh", "maison", Some(token)).unwrap();
        assert!(!format!("{with_token:?}").contains("tk_secret"));
    }

    #[test]
    fn messages_are_cleaned_and_bounded() {
        let data = |value: Value| match value {
            Value::Object(map) => map,
            _ => Attributes::new(),
        };
        let message = Message::from_data(&data(json!({
            "title": "Fumée\u{1b}[2J !",
            "message": "Salon\nvérifie vite",
            "priority": 9,
        })))
        .unwrap();
        assert_eq!(message.title.as_deref(), Some("Fumée[2J !"));
        assert_eq!(message.message, "Salon\nvérifie vite");
        assert_eq!(message.priority, 5);
        assert_eq!(Message::from_data(&data(json!({"title": "x"}))), None);
        assert_eq!(Message::from_data(&data(json!({"message": "  "}))), None);
        let long = Message::from_data(&data(json!({"message": "x".repeat(5000)}))).unwrap();
        assert_eq!(long.message.len(), MAX_MESSAGE_CHARS);
    }
}
