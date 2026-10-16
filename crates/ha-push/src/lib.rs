//! Notifications sur le téléphone, **par le navigateur** (Web Push) : rien à
//! installer, il suffit d'ouvrir le tableau de bord et d'accepter les
//! notifications (sur iPhone : l'ajouter d'abord à l'écran d'accueil).
//!
//! Trois sortes de notifications, que chaque appareil choisit :
//!
//! | Catégorie | Quand |
//! |-----------|-------|
//! | [`Category::Approvals`] | un ordre attend une validation (la garde, étape 48) |
//! | [`Category::Alerts`] | une alerte grave du serveur (intégration en panne, automatisation en erreur…) |
//! | [`Category::Automations`] | une automatisation écrit à `notify.appli` |
//!
//! ```yaml
//! - action: notify.send_message
//!   target:
//!     entity_id: notify.appli
//!   data:
//!     title: Lessive
//!     message: La machine a fini.
//!     users: [david]        # facultatif : sinon, toute la famille
//! ```
//!
//! Seuls les comptes famille et administrateur reçoivent des notifications.
//!
//! Sécurité : le serveur n'envoie qu'aux **services de notifications des
//! navigateurs** (Google, Mozilla, Apple, Microsoft), en HTTPS, sans suivre
//! de redirection. Une page malveillante ne peut donc pas lui faire appeler
//! une adresse de ton réseau. Le contenu est chiffré pour le seul navigateur
//! abonné (voir [`crypto`]).

pub mod crypto;

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use ha_core::{Attributes, Background, CancellationToken, Core, EntityId, Event, Origin};
use ha_storage::{PushTarget, Store};
use serde_json::{Value, json};
use tokio::sync::broadcast::error::RecvError;
use tokio::task::JoinSet;
use tracing::{info, warn};
use zeroize::Zeroizing;

pub use crypto::{CryptoError, Vapid};

/// L'entité à qui les automatisations écrivent.
pub const ENTITY: &str = "notify.appli";
/// Abonnements au plus par compte, et en tout.
pub const MAX_PER_USER: u64 = 10;
pub const MAX_TOTAL: u64 = 50;
/// Messages des automatisations : pas plus de 10 en 10 minutes (une
/// automatisation qui boucle ne doit pas faire vibrer le téléphone sans fin).
const MAX_BURST: usize = 10;
/// Toutes catégories confondues.
const MAX_TOTAL_BURST: usize = 60;
const BURST_WINDOW: Duration = Duration::from_mins(10);
const MAX_TITLE_CHARS: usize = 100;
const MAX_BODY_CHARS: usize = 600;
const MAX_NAME_CHARS: usize = 60;
const MAX_ENDPOINT_LEN: usize = 1024;
/// Le contact donné par défaut aux services de notifications (ils l'utilisent
/// pour prévenir en cas d'abus). `HA_PUSH_CONTACT` le remplace.
pub const DEFAULT_CONTACT: &str = "https://github.com/wolfdavid68-dev/home-assistant-rust";
/// Les services de notifications des navigateurs : Chrome, Edge, Firefox,
/// Safari. Une adresse d'abonnement doit être sur l'un d'eux (ou un de
/// leurs sous-domaines).
pub const PUSH_SERVICES: [&str; 4] = [
    "fcm.googleapis.com",
    "push.services.mozilla.com",
    "push.apple.com",
    "notify.windows.com",
];

/// Ce qu'un appareil peut choisir de recevoir.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    Approvals = 1,
    Alerts = 2,
    Automations = 4,
}

impl Category {
    /// Toutes les catégories (valeur par défaut d'un nouvel appareil).
    pub const ALL: u8 = 7;
}

#[derive(Debug, thiserror::Error)]
pub enum PushError {
    #[error(
        "adresse d'abonnement refusée : seuls les services de notifications des navigateurs sont acceptés"
    )]
    Endpoint,
    #[error("clés de l'abonnement invalides")]
    Keys,
    #[error("catégories invalides")]
    Categories,
    #[error("contact invalide (HA_PUSH_CONTACT) : « mailto:… » ou « https://… »")]
    Contact,
    #[error("clé VAPID : {0}")]
    Vapid(String),
    #[error(transparent)]
    Tls(#[from] ha_tls::TlsError),
}

/// Vérifie une adresse d'abonnement : HTTPS, sur un service de
/// notifications connu, port standard, sans identifiants.
pub fn check_endpoint(endpoint: &str) -> Result<url::Url, PushError> {
    if endpoint.len() > MAX_ENDPOINT_LEN {
        return Err(PushError::Endpoint);
    }
    let url = url::Url::parse(endpoint).map_err(|_| PushError::Endpoint)?;
    let known = match url.host() {
        Some(url::Host::Domain(host)) => PUSH_SERVICES.iter().any(|service| {
            host == *service
                || host
                    .strip_suffix(service)
                    .is_some_and(|rest| rest.ends_with('.'))
        }),
        _ => false,
    };
    if url.scheme() != "https"
        || !known
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err(PushError::Endpoint);
    }
    Ok(url)
}

/// Un abonnement reçu d'un navigateur, vérifié.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Subscription {
    pub endpoint: String,
    pub p256dh: String,
    pub auth: Zeroizing<String>,
    pub name: String,
    pub categories: u8,
}

impl Subscription {
    pub fn new(
        endpoint: &str,
        p256dh: &str,
        auth: &str,
        name: &str,
        categories: u8,
    ) -> Result<Self, PushError> {
        check_endpoint(endpoint)?;
        let public = crypto::decode(p256dh).ok_or(PushError::Keys)?;
        let secret = Zeroizing::new(crypto::decode(auth).ok_or(PushError::Keys)?);
        if public.len() != crypto::PUBLIC_KEY_LEN
            || public.first() != Some(&4)
            || secret.len() != crypto::AUTH_LEN
        {
            return Err(PushError::Keys);
        }
        check_categories(categories)?;
        let name = clean(name, MAX_NAME_CHARS);
        Ok(Self {
            endpoint: endpoint.to_owned(),
            // Rangées sous une seule forme (sans « = »).
            p256dh: crypto::encode(&public),
            auth: Zeroizing::new(crypto::encode(&secret)),
            name: if name.is_empty() {
                "Appareil".into()
            } else {
                name
            },
            categories,
        })
    }
}

pub fn check_categories(categories: u8) -> Result<(), PushError> {
    if categories > Category::ALL {
        return Err(PushError::Categories);
    }
    Ok(())
}

/// Vérifie le contact donné aux services de notifications.
pub fn check_contact(contact: &str) -> Result<String, PushError> {
    let contact = contact.trim();
    let valid = (contact.starts_with("mailto:") && contact.len() > 7 && contact.contains('@'))
        || url::Url::parse(contact).is_ok_and(|url| url.scheme() == "https");
    if !valid || contact.len() > 200 || contact.chars().any(char::is_control) {
        return Err(PushError::Contact);
    }
    Ok(contact.to_owned())
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

/// Une notification à envoyer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub title: String,
    pub body: String,
    /// Une notification du même `tag` remplace la précédente sur le
    /// téléphone.
    pub tag: String,
    /// La page ouverte quand on touche la notification.
    pub url: &'static str,
    /// Combien de temps le service la garde si le téléphone est éteint.
    pub ttl: Duration,
    /// Réveille le téléphone même en économie d'énergie.
    pub urgent: bool,
}

impl Message {
    /// Le contenu chiffré : du JSON lu par le service worker (`sw.js`).
    fn payload(&self) -> Vec<u8> {
        json!({
            "title": self.title,
            "body": self.body,
            "tag": self.tag,
            "url": self.url,
        })
        .to_string()
        .into_bytes()
    }
}

/// Ce qu'il faut envoyer, et à qui.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outgoing {
    pub category: Category,
    pub message: Message,
    /// Seulement ces comptes (`None` : tous ceux qui le veulent).
    pub users: Option<Vec<String>>,
    /// Un essai : envoyé quelles que soient les catégories choisies.
    pub test: bool,
}

fn text<'a>(attributes: &'a Attributes, key: &str) -> Option<&'a str> {
    attributes.get(key).and_then(Value::as_str)
}

/// Ce qu'un événement du bus doit envoyer, s'il doit envoyer quelque chose.
#[must_use]
pub fn outgoing(event: &Event) -> Option<Outgoing> {
    match event {
        Event::CallService {
            domain,
            service,
            entity_id,
            data,
            origin,
            ..
        } if domain == "notify" && service == "send_message" && entity_id.as_str() == ENTITY => {
            from_automation(data, origin)
        }
        Event::StateChanged {
            entity_id,
            old_state,
            new_state: Some(new_state),
        } if entity_id.domain() == ha_notices::DOMAIN
            && new_state.state() == ha_notices::NOTIFYING
            && old_state
                .as_ref()
                .is_none_or(|old| old.state() != ha_notices::NOTIFYING) =>
        {
            let attributes = new_state.attributes();
            let title = clean(text(attributes, "title")?, MAX_TITLE_CHARS);
            let body = clean(text(attributes, "message")?, MAX_BODY_CHARS);
            let tag = entity_id.as_str().to_owned();
            match text(attributes, "kind") {
                Some("approval") => Some(Outgoing {
                    category: Category::Approvals,
                    message: Message {
                        title,
                        body,
                        tag,
                        url: "/#notifications",
                        ttl: ha_core::guard::HELD_TTL,
                        urgent: true,
                    },
                    users: None,
                    test: false,
                }),
                Some("repair") if text(attributes, "severity") == Some("error") => Some(Outgoing {
                    category: Category::Alerts,
                    message: Message {
                        title,
                        body,
                        tag,
                        url: "/#notifications",
                        ttl: Duration::from_hours(24),
                        urgent: true,
                    },
                    users: None,
                    test: false,
                }),
                _ => None,
            }
        }
        _ => None,
    }
}

/// `notify.send_message` sur `notify.appli`.
fn from_automation(data: &Attributes, origin: &Origin) -> Option<Outgoing> {
    let body = clean(text(data, "message")?, MAX_BODY_CHARS);
    if body.is_empty() {
        return None;
    }
    let title = text(data, "title")
        .map(|title| clean(title, MAX_TITLE_CHARS))
        .filter(|title| !title.is_empty())
        .unwrap_or_else(|| "Maison".into());
    let mut users: Option<Vec<String>> = data.get("users").map(|users| {
        users
            .as_array()
            .into_iter()
            .flatten()
            .chain(users.as_str().map(|_| users))
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .take(20)
            .collect()
    });
    // Un essai depuis « Mon compte » : seulement vers soi-même.
    let test = data.get("test") == Some(&Value::Bool(true));
    if test {
        let Origin::User(me) = origin else {
            return None;
        };
        users = Some(vec![me.clone()]);
    }
    let priority = data.get("priority").and_then(Value::as_u64).unwrap_or(3);
    Some(Outgoing {
        category: Category::Automations,
        message: Message {
            title,
            body,
            tag: if test {
                "essai".into()
            } else {
                "automatisation".into()
            },
            url: "/",
            ttl: Duration::from_hours(24),
            urgent: priority >= 4,
        },
        users,
        test,
    })
}

/// Le résultat d'un envoi à un appareil.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delivery {
    Delivered,
    /// L'abonnement n'existe plus (navigateur désinstallé, notifications
    /// refusées) : on l'oublie.
    Gone,
    Failed,
}

/// Ce qu'il faut pour envoyer.
#[derive(Debug)]
pub struct Sender {
    http: reqwest::Client,
    vapid: Vapid,
    contact: String,
}

impl Sender {
    #[must_use]
    pub fn new(http: reqwest::Client, vapid: Vapid, contact: String) -> Self {
        Self {
            http,
            vapid,
            contact,
        }
    }

    /// Envoie `message` à un appareil. `endpoint` doit déjà être vérifié
    /// (voir [`check_endpoint`]).
    pub async fn deliver(
        &self,
        endpoint: &url::Url,
        p256dh: &str,
        auth: &str,
        message: &Message,
    ) -> Delivery {
        let (Some(public), Some(secret)) = (crypto::decode(p256dh), crypto::decode(auth)) else {
            return Delivery::Failed;
        };
        let secret = Zeroizing::new(secret);
        let body = match crypto::encrypt(&public, &secret, &message.payload()) {
            Ok(body) => body,
            Err(error) => {
                warn!("notification : chiffrement impossible ({error})");
                return Delivery::Failed;
            }
        };
        // Le jeton VAPID est valable 12 heures (24 au plus).
        let expires = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs())
            + 12 * 3600;
        let audience = endpoint.origin().ascii_serialization();
        let authorization = match self.vapid.authorization(&audience, expires, &self.contact) {
            Ok(value) => value,
            Err(error) => {
                warn!("notification : signature impossible ({error})");
                return Delivery::Failed;
            }
        };
        let response = self
            .http
            .post(endpoint.clone())
            .header("Content-Encoding", "aes128gcm")
            .header("Content-Type", "application/octet-stream")
            .header("TTL", message.ttl.as_secs().to_string())
            .header("Urgency", if message.urgent { "high" } else { "normal" })
            .header("Authorization", authorization)
            .body(body)
            .send()
            .await;
        let host = endpoint.host_str().unwrap_or_default();
        match response {
            Ok(response) if response.status().is_success() => Delivery::Delivered,
            Ok(response) if matches!(response.status().as_u16(), 404 | 410) => Delivery::Gone,
            Ok(response) => {
                warn!("notification : {host} a répondu {}", response.status());
                Delivery::Failed
            }
            Err(error) => {
                warn!("notification : {host} injoignable ({error})");
                Delivery::Failed
            }
        }
    }
}

/// Lit la clé VAPID dans la base, ou la crée au premier démarrage.
pub async fn load_or_create_vapid(store: &Store) -> Result<Vapid, PushError> {
    let vapid_error = |error: &dyn std::fmt::Display| PushError::Vapid(error.to_string());
    if let Some(keys) = store.push_vapid().await.map_err(|e| vapid_error(&e))? {
        let pkcs8 = Zeroizing::new(
            decode_hex(&keys.private_pkcs8_hex)
                .ok_or_else(|| PushError::Vapid("illisible".into()))?,
        );
        return Vapid::from_pkcs8(&pkcs8).map_err(|e| vapid_error(&e));
    }
    let (pkcs8, vapid) = Vapid::generate().map_err(|e| vapid_error(&e))?;
    let mut hex = Zeroizing::new(String::with_capacity(pkcs8.len() * 2));
    for byte in pkcs8.iter() {
        hex.push(char::from(b"0123456789abcdef"[usize::from(byte >> 4)]));
        hex.push(char::from(b"0123456789abcdef"[usize::from(byte & 0xf)]));
    }
    if store
        .save_push_vapid(&hex, vapid.public_key())
        .await
        .map_err(|e| vapid_error(&e))?
    {
        info!("notifications du navigateur : clé du serveur créée");
        Ok(vapid)
    } else {
        // Créée entre-temps par un autre démarrage : on prend celle-là.
        Box::pin(load_or_create_vapid(store)).await
    }
}

fn decode_hex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(text.get(i..i + 2)?, 16).ok())
        .collect()
}

/// Le service : écoute le bus et envoie les notifications.
#[derive(Debug)]
pub struct Push(Background);

impl Push {
    /// Démarre le service. L'abonnement au bus est fait avant de rendre la
    /// main : aucun ordre retenu n'est manqué.
    pub async fn start(store: Store, core: Arc<Core>, contact: String) -> Result<Self, PushError> {
        let contact = check_contact(&contact)?;
        let vapid = load_or_create_vapid(&store).await?;
        let http = ha_tls::web_client(true)?;
        let sender = Arc::new(Sender::new(http, vapid, contact));
        let events = core.bus.subscribe();
        if let Ok(entity_id) = ENTITY.parse::<EntityId>() {
            let mut attributes = Attributes::new();
            attributes.insert("friendly_name".into(), "Appli Maison".into());
            if let Err(error) = core.states.set(entity_id, "unknown", attributes) {
                warn!("notifications du navigateur : état refusé ({error})");
            }
        }
        Ok(Self(Background::spawn(
            "les notifications du navigateur",
            move |token| run(store, core, sender, events, token),
        )))
    }

    pub async fn stop(self) {
        self.0.stop().await;
    }
}

/// Garde les instants des derniers envois, pour limiter leur nombre.
#[derive(Debug, Default)]
struct Burst(VecDeque<Instant>);

impl Burst {
    fn allow(&mut self, max: usize, now: Instant) -> bool {
        while self
            .0
            .front()
            .is_some_and(|t| now.duration_since(*t) > BURST_WINDOW)
        {
            self.0.pop_front();
        }
        if self.0.len() >= max {
            return false;
        }
        self.0.push_back(now);
        true
    }
}

async fn run(
    store: Store,
    core: Arc<Core>,
    sender: Arc<Sender>,
    mut events: tokio::sync::broadcast::Receiver<Arc<Event>>,
    token: CancellationToken,
) {
    info!("notifications du navigateur : prêtes");
    let mut automations = Burst::default();
    let mut total = Burst::default();
    let mut sending = JoinSet::new();
    loop {
        let event = tokio::select! {
            () = token.cancelled() => break,
            Some(_) = sending.join_next(), if !sending.is_empty() => continue,
            received = events.recv() => match received {
                Ok(event) => event,
                Err(RecvError::Lagged(missed)) => {
                    warn!("notifications du navigateur : {missed} événements manqués");
                    continue;
                }
                Err(RecvError::Closed) => break,
            },
        };
        let Some(outgoing) = outgoing(&event) else {
            continue;
        };
        let now = Instant::now();
        if outgoing.category == Category::Automations
            && !outgoing.test
            && !automations.allow(MAX_BURST, now)
        {
            warn!(
                "notifications du navigateur : plus de {MAX_BURST} messages en 10 minutes, celui-ci est ignoré"
            );
            continue;
        }
        if !total.allow(MAX_TOTAL_BURST, now) {
            warn!("notifications du navigateur : trop d'envois en 10 minutes, celui-ci est ignoré");
            continue;
        }
        sending.spawn(send_all(
            store.clone(),
            Arc::clone(&core),
            Arc::clone(&sender),
            outgoing,
        ));
    }
}

/// Envoie à tous les appareils concernés.
async fn send_all(store: Store, core: Arc<Core>, sender: Arc<Sender>, outgoing: Outgoing) {
    let category = if outgoing.test {
        Category::ALL
    } else {
        outgoing.category as u8
    };
    let targets = match store.push_targets(category, None).await {
        Ok(targets) => targets,
        Err(error) => {
            warn!("notifications du navigateur : abonnements illisibles ({error})");
            return;
        }
    };
    let targets: Vec<PushTarget> = targets
        .into_iter()
        .filter(|target| {
            outgoing
                .users
                .as_ref()
                .is_none_or(|users| users.contains(&target.username))
        })
        .collect();
    let mut delivered = 0;
    for target in &targets {
        // Vérifiée à l'abonnement, revérifiée ici : la base pourrait avoir
        // été modifiée à la main.
        let Ok(endpoint) = check_endpoint(&target.endpoint) else {
            warn!(
                "notifications du navigateur : adresse refusée, abonnement {} oublié",
                target.id
            );
            let _ = store.forget_push_subscription(target.id).await;
            continue;
        };
        // Le contenu n'est pas écrit dans les logs : il peut être personnel.
        let result = sender
            .deliver(&endpoint, &target.p256dh, &target.auth, &outgoing.message)
            .await;
        let stored = match result {
            Delivery::Delivered => {
                delivered += 1;
                store.push_delivered(target.id, true).await
            }
            Delivery::Failed => store.push_delivered(target.id, false).await,
            Delivery::Gone => {
                info!(
                    "notifications du navigateur : un appareil de {} s'est désabonné",
                    target.username
                );
                store.forget_push_subscription(target.id).await
            }
        };
        if let Err(error) = stored {
            warn!("notifications du navigateur : résultat non enregistré ({error})");
        }
    }
    if !targets.is_empty() {
        info!(
            "notification envoyée à {delivered} appareil(s) sur {}",
            targets.len()
        );
    }
    if delivered > 0
        && outgoing.category == Category::Automations
        && let Ok(entity_id) = ENTITY.parse::<EntityId>()
    {
        let mut attributes = Attributes::new();
        attributes.insert("friendly_name".into(), "Appli Maison".into());
        let _ = core
            .states
            .set(entity_id, jiff::Timestamp::now().to_string(), attributes);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ha_core::State;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn attributes(value: Value) -> Attributes {
        match value {
            Value::Object(map) => map,
            _ => Attributes::new(),
        }
    }

    #[test]
    fn only_browser_push_services_are_accepted() {
        for good in [
            "https://fcm.googleapis.com/fcm/send/abc:def",
            "https://updates.push.services.mozilla.com/wpush/v2/gAAA",
            "https://web.push.apple.com/QGm2",
            "https://wns2-par02p.notify.windows.com/w/?token=BQYAAA",
        ] {
            assert!(check_endpoint(good).is_ok(), "{good}");
        }
        for bad in [
            "http://fcm.googleapis.com/fcm/send/abc",
            "https://192.168.1.1/fcm/send",
            "https://localhost/x",
            "https://evilfcm.googleapis.com.example.org/x",
            "https://notfcm.googleapis.com/x",
            "https://fcm.googleapis.com:8443/x",
            "https://user:pw@fcm.googleapis.com/x",
            "https://fcm.googleapis.com/x#frag",
            "https://xn--fcm.googleapis.com.evil/x",
            "file:///etc/passwd",
            "pas une adresse",
        ] {
            assert!(check_endpoint(bad).is_err(), "{bad}");
        }
        assert!(
            check_endpoint(&format!("https://fcm.googleapis.com/{}", "a".repeat(1100))).is_err()
        );
    }

    #[test]
    fn subscriptions_are_checked() {
        let (_, public, auth) = crypto::tests::browser();
        let p256dh = crypto::encode(&public);
        let secret = crypto::encode(&auth);
        let endpoint = "https://fcm.googleapis.com/fcm/send/x";
        let ok = Subscription::new(endpoint, &format!("{p256dh}="), &secret, " Pixel\u{7}  ", 5)
            .unwrap();
        assert_eq!(ok.p256dh, p256dh);
        assert_eq!(ok.name, "Pixel");
        assert_eq!(
            Subscription::new(endpoint, &p256dh, &secret, "", 7)
                .unwrap()
                .name,
            "Appareil"
        );
        assert!(matches!(
            Subscription::new(endpoint, &p256dh, "AAAA", "x", 7),
            Err(PushError::Keys)
        ));
        assert!(matches!(
            Subscription::new(endpoint, &secret, &secret, "x", 7),
            Err(PushError::Keys)
        ));
        assert!(matches!(
            Subscription::new(endpoint, &p256dh, &secret, "x", 8),
            Err(PushError::Categories)
        ));
        assert!(matches!(
            Subscription::new("https://exemple.fr/", &p256dh, &secret, "x", 7),
            Err(PushError::Endpoint)
        ));
    }

    #[test]
    fn contacts() {
        assert!(check_contact("mailto:david@exemple.fr").is_ok());
        assert!(check_contact(DEFAULT_CONTACT).is_ok());
        for bad in [
            "",
            "mailto:",
            "david@exemple.fr",
            "http://exemple.fr",
            "mailto:a@b\r\nX: y",
        ] {
            assert!(check_contact(bad).is_err(), "{bad:?}");
        }
    }

    fn notice(id: &str, state: &str, value: Value) -> State {
        let core = Core::new();
        let id: EntityId = id.parse().unwrap();
        core.states
            .set(id.clone(), state, attributes(value))
            .unwrap();
        core.states.get(&id).unwrap()
    }

    fn changed(new: State, old: Option<State>) -> Event {
        Event::StateChanged {
            entity_id: new.entity_id().clone(),
            old_state: old,
            new_state: Some(new),
        }
    }

    #[test]
    fn approvals_and_serious_alerts_are_pushed_once() {
        let approval = notice(
            "persistent_notification.approval_3",
            "notifying",
            json!({"title": "Ordre à valider", "message": "Un ordre…", "kind": "approval", "approval_id": 3}),
        );
        let out = outgoing(&changed(approval.clone(), None)).unwrap();
        assert_eq!(out.category, Category::Approvals);
        assert_eq!(out.message.tag, "persistent_notification.approval_3");
        assert_eq!(out.message.url, "/#notifications");
        assert!(out.message.urgent);
        assert_eq!(out.message.ttl, Duration::from_mins(10));
        // Mise à jour du texte : pas de nouvelle notification.
        assert_eq!(outgoing(&changed(approval.clone(), Some(approval))), None);

        let error = notice(
            "persistent_notification.repair_integration_mqtt",
            "notifying",
            json!({"title": "MQTT en panne", "message": "…", "kind": "repair", "severity": "error"}),
        );
        assert_eq!(
            outgoing(&changed(error, None)).unwrap().category,
            Category::Alerts
        );
        let warning = notice(
            "persistent_notification.repair_lampe",
            "notifying",
            json!({"title": "Lampe injoignable", "message": "…", "kind": "repair", "severity": "warning"}),
        );
        assert_eq!(
            outgoing(&changed(warning, None)),
            None,
            "un avertissement reste dans la cloche"
        );
        let ignored = notice(
            "persistent_notification.repair_x",
            "ignored",
            json!({"title": "x", "message": "x", "kind": "repair", "severity": "error"}),
        );
        assert_eq!(outgoing(&changed(ignored, None)), None);
        let plain = notice(
            "persistent_notification.lessive",
            "notifying",
            json!({"title": "Lessive", "message": "finie", "kind": "notification"}),
        );
        assert_eq!(outgoing(&changed(plain, None)), None);
    }

    fn call(data: Value, origin: Origin, entity: &str) -> Event {
        Event::CallService {
            domain: "notify".into(),
            service: "send_message".into(),
            entity_id: entity.parse().unwrap(),
            data: attributes(data),
            origin,
            command: None,
            issued_at: Instant::now(),
        }
    }

    #[test]
    fn automations_write_to_notify_appli() {
        let out = outgoing(&call(
            json!({"title": "Lessive", "message": "La machine a fini.\u{1b}", "users": ["david"], "priority": 5}),
            Origin::Automation("lessive".into()),
            ENTITY,
        ))
        .unwrap();
        assert_eq!(out.category, Category::Automations);
        assert_eq!(out.message.body, "La machine a fini.");
        assert_eq!(out.users, Some(vec!["david".to_owned()]));
        assert!(out.message.urgent);
        assert!(!out.test);
        let plain = outgoing(&call(
            json!({"message": "x".repeat(5000)}),
            Origin::System,
            ENTITY,
        ))
        .unwrap();
        assert_eq!(plain.message.title, "Maison");
        assert_eq!(plain.message.body.chars().count(), MAX_BODY_CHARS);
        assert_eq!(plain.users, None);
        assert!(plain.message.payload().len() < crypto::MAX_PLAINTEXT);
        let one = outgoing(&call(
            json!({"message": "x", "users": "alice"}),
            Origin::System,
            ENTITY,
        ))
        .unwrap();
        assert_eq!(one.users, Some(vec!["alice".to_owned()]));

        // Pas de message, autre entité : rien.
        assert_eq!(
            outgoing(&call(json!({"title": "x"}), Origin::System, ENTITY)),
            None
        );
        assert_eq!(
            outgoing(&call(
                json!({"message": "x"}),
                Origin::System,
                "notify.telephone"
            )),
            None
        );

        // Un essai ne va qu'à celui qui l'a demandé.
        let test = outgoing(&call(
            json!({"message": "Essai", "test": true, "users": ["alice"]}),
            Origin::User("david".into()),
            ENTITY,
        ))
        .unwrap();
        assert!(test.test);
        assert_eq!(test.users, Some(vec!["david".to_owned()]));
        assert_eq!(
            outgoing(&call(
                json!({"message": "x", "test": true}),
                Origin::Automation("a".into()),
                ENTITY
            )),
            None
        );
    }

    #[test]
    fn bursts_are_limited() {
        let mut burst = Burst::default();
        let start = Instant::now();
        for _ in 0..3 {
            assert!(burst.allow(3, start));
        }
        assert!(!burst.allow(3, start));
        assert!(burst.allow(3, start + BURST_WINDOW + Duration::from_secs(1)));
    }

    /// Un faux service de notifications : renvoie la requête reçue.
    async fn fake_service(status: &'static str) -> (url::Url, tokio::task::JoinHandle<Vec<u8>>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut buffer = [0u8; 8192];
            loop {
                let n = socket.read(&mut buffer).await.unwrap();
                request.extend_from_slice(&buffer[..n]);
                let text = String::from_utf8_lossy(&request);
                if let Some(end) = text.find("\r\n\r\n") {
                    let length: usize = text
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase()
                                .strip_prefix("content-length: ")
                                .map(|v| v.trim().parse().unwrap())
                        })
                        .unwrap_or(0);
                    if request.len() >= end + 4 + length {
                        break;
                    }
                }
            }
            let response =
                format!("HTTP/1.1 {status}\r\ncontent-length: 0\r\nconnection: close\r\n\r\n");
            socket.write_all(response.as_bytes()).await.unwrap();
            request
        });
        (
            format!("http://{address}/push/abc").parse().unwrap(),
            handle,
        )
    }

    fn message() -> Message {
        Message {
            title: "Ordre à valider".into(),
            body: "Fermer le volet".into(),
            tag: "t".into(),
            url: "/#notifications",
            ttl: Duration::from_mins(10),
            urgent: true,
        }
    }

    #[tokio::test]
    async fn a_notification_reaches_the_push_service_encrypted_and_signed() {
        let (private, public, auth) = crypto::tests::browser();
        let (_, vapid) = Vapid::generate().unwrap();
        let vapid_public = vapid.public_key().to_owned();
        let sender = Sender::new(
            ha_tls::web_client(false).unwrap(),
            vapid,
            "mailto:a@b.fr".into(),
        );
        let (endpoint, service) = fake_service("201 Created").await;
        let result = sender
            .deliver(
                &endpoint,
                &crypto::encode(&public),
                &crypto::encode(&auth),
                &message(),
            )
            .await;
        assert_eq!(result, Delivery::Delivered);

        let request = service.await.unwrap();
        let split = request.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
        let head = String::from_utf8_lossy(&request[..split]).to_ascii_lowercase();
        assert!(head.starts_with("post /push/abc "), "{head}");
        assert!(head.contains("content-encoding: aes128gcm"));
        assert!(head.contains("ttl: 600"));
        assert!(head.contains("urgency: high"));
        assert!(head.contains(&format!(", k={}", vapid_public.to_ascii_lowercase())));
        let body = &request[split + 4..];
        assert!(!String::from_utf8_lossy(body).contains("volet"), "chiffré");
        let plain = crypto::tests::decrypt(private, &public, &auth, body);
        let payload: Value = serde_json::from_slice(&plain).unwrap();
        assert_eq!(payload["title"], "Ordre à valider");
        assert_eq!(payload["url"], "/#notifications");
    }

    #[tokio::test]
    async fn expired_subscriptions_are_reported() {
        let (_, public, auth) = crypto::tests::browser();
        let (_, vapid) = Vapid::generate().unwrap();
        let sender = Sender::new(
            ha_tls::web_client(false).unwrap(),
            vapid,
            DEFAULT_CONTACT.into(),
        );
        for (status, expected) in [
            ("410 Gone", Delivery::Gone),
            ("404 Not Found", Delivery::Gone),
            ("403 Forbidden", Delivery::Failed),
        ] {
            let (endpoint, _) = fake_service(status).await;
            let result = sender
                .deliver(
                    &endpoint,
                    &crypto::encode(&public),
                    &crypto::encode(&auth),
                    &message(),
                )
                .await;
            assert_eq!(result, expected, "{status}");
        }
        let (endpoint, _) = fake_service("201 Created").await;
        assert_eq!(
            sender
                .deliver(&endpoint, "AAAA", &crypto::encode(&auth), &message())
                .await,
            Delivery::Failed
        );
    }

    #[tokio::test]
    async fn the_server_key_is_created_once() {
        let store = Store::open_in_memory().await.unwrap();
        let first = load_or_create_vapid(&store).await.unwrap();
        let second = load_or_create_vapid(&store).await.unwrap();
        assert_eq!(first.public_key(), second.public_key());
        assert_eq!(
            store.push_public_key().await.unwrap().as_deref(),
            Some(first.public_key())
        );
    }
}
