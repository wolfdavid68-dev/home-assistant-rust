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
//!     actions:              # facultatif : deux boutons au plus
//!       - action: LESSIVE_VUE
//!         title: Je m'en occupe
//! ```
//!
//! Un ordre à valider arrive avec « Valider » et « Refuser » ; un bouton
//! d'automatisation annonce l'événement `notification_action` (voir
//! [`tickets`]).
//!
//! Les notifications se rangent toutes seules (étape 52) : un ordre validé,
//! refusé ou oublié remplace « Ordre à valider » sur tous les téléphones, et
//! une alerte grave qui disparaît devient « Résolu », sans faire sonner.
//!
//! Seuls les comptes famille et administrateur reçoivent des notifications.
//!
//! Sécurité : le serveur n'envoie qu'aux **services de notifications des
//! navigateurs** (Google, Mozilla, Apple, Microsoft), en HTTPS, sans suivre
//! de redirection. Une page malveillante ne peut donc pas lui faire appeler
//! une adresse de ton réseau. Le contenu est chiffré pour le seul navigateur
//! abonné (voir [`crypto`]).

pub mod crypto;
pub mod tickets;

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use ha_core::guard::{APPROVAL_ANSWERED, APPROVAL_EXPIRED};
use ha_core::{
    Attributes, Background, CancellationToken, Core, EntityId, Event, Origin, State, StateMachine,
};
use ha_storage::{PushTarget, Store};
use serde_json::{Value, json};
use tokio::sync::broadcast::error::RecvError;
use tokio::task::JoinSet;
use tracing::{info, warn};
use zeroize::Zeroizing;

pub use crypto::{CryptoError, Vapid};
pub use tickets::{Purpose, Refused, Ticket, Tickets};

/// L'entité à qui les automatisations écrivent.
pub const ENTITY: &str = "notify.appli";
/// L'événement annoncé quand on touche un bouton d'automatisation.
pub const ACTION_EVENT: &str = "notification_action";
/// Les boutons d'un ordre à valider.
pub const APPROVE: &str = "approve";
pub const DENY: &str = "deny";
/// Les navigateurs n'en montrent souvent que deux.
pub const MAX_BUTTONS: usize = 2;
const MAX_BUTTON_TITLE_CHARS: usize = 24;
const MAX_BUTTON_ID_LEN: usize = 40;
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

/// Un bouton d'une notification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Button {
    /// L'identifiant renvoyé au serveur (`approve`, `LESSIVE_VUE`…).
    pub action: String,
    /// Ce qui est écrit dessus.
    pub title: String,
}

/// Un identifiant de bouton : lettres, chiffres, « _ » et « - ».
#[must_use]
pub fn is_valid_button_id(id: &str) -> bool {
    (1..=MAX_BUTTON_ID_LEN).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
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
    /// Les boutons (montrés seulement avec un ticket, voir [`tickets`]).
    pub buttons: Vec<Button>,
    /// Remplace la notification du même `tag` sans faire sonner : une
    /// réponse, pas une nouvelle.
    pub silent: bool,
}

impl Message {
    /// Le contenu chiffré : du JSON lu par le service worker (`sw.js`).
    /// Le ticket est propre à chaque appareil.
    fn payload(&self, ticket: Option<&str>) -> Vec<u8> {
        let mut payload = json!({
            "title": self.title,
            "body": self.body,
            "tag": self.tag,
            "url": self.url,
        });
        if self.silent {
            payload["silent"] = true.into();
        }
        if let Some(ticket) = ticket
            && !self.buttons.is_empty()
        {
            payload["token"] = ticket.into();
            payload["actions"] = self
                .buttons
                .iter()
                .map(|button| json!({ "action": button.action, "title": button.title }))
                .collect();
        }
        payload.to_string().into_bytes()
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
    /// Ce que permettent les boutons (`None` : pas de boutons).
    pub purpose: Option<Purpose>,
}

fn text<'a>(attributes: &'a Attributes, key: &str) -> Option<&'a str> {
    attributes.get(key).and_then(Value::as_str)
}

/// Une alerte de réparation affichée qui devient grave (une pile faible
/// presque vide) : elle sonne comme une nouvelle.
fn worsened(old: &State, new: &State) -> bool {
    text(new.attributes(), "kind") == Some("repair")
        && text(new.attributes(), "severity") == Some("error")
        && text(old.attributes(), "severity") != Some("error")
}

/// Ce qu'un événement du bus doit envoyer, s'il doit envoyer quelque chose.
/// `states` donne le nom des appareils (« Volet chambre »).
#[must_use]
pub fn outgoing(event: &Event, states: &StateMachine) -> Option<Outgoing> {
    match event {
        Event::Fired { event_type, data }
            if event_type == APPROVAL_ANSWERED || event_type == APPROVAL_EXPIRED =>
        {
            approval_closed(event_type, data, states)
        }
        // Une alerte grave qui disparaît : le problème est réglé.
        Event::StateChanged {
            entity_id,
            old_state: Some(old_state),
            new_state: None,
        } if entity_id.domain() == ha_notices::DOMAIN
            && old_state.state() == ha_notices::NOTIFYING
            && text(old_state.attributes(), "kind") == Some("repair")
            && text(old_state.attributes(), "severity") == Some("error") =>
        {
            let title = clean(text(old_state.attributes(), "title")?, MAX_TITLE_CHARS);
            Some(Outgoing {
                category: Category::Alerts,
                message: Message {
                    title: "Résolu".into(),
                    body: format!("{title} : c'est réglé."),
                    tag: entity_id.as_str().to_owned(),
                    url: "/",
                    ttl: Duration::from_hours(24),
                    urgent: false,
                    buttons: Vec::new(),
                    silent: true,
                },
                users: None,
                test: false,
                purpose: None,
            })
        }
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
            && old_state.as_ref().is_none_or(|old| {
                old.state() != ha_notices::NOTIFYING || worsened(old, new_state)
            }) =>
        {
            let attributes = new_state.attributes();
            let title = clean(text(attributes, "title")?, MAX_TITLE_CHARS);
            let body = clean(text(attributes, "message")?, MAX_BODY_CHARS);
            let tag = entity_id.as_str().to_owned();
            match text(attributes, "kind") {
                Some("approval") => {
                    let approval = attributes.get("approval_id").and_then(Value::as_u64);
                    Some(Outgoing {
                        category: Category::Approvals,
                        message: Message {
                            title,
                            body,
                            tag,
                            url: "/#notifications",
                            ttl: ha_core::guard::HELD_TTL,
                            urgent: true,
                            buttons: if approval.is_some() {
                                vec![
                                    Button {
                                        action: APPROVE.into(),
                                        title: "Valider".into(),
                                    },
                                    Button {
                                        action: DENY.into(),
                                        title: "Refuser".into(),
                                    },
                                ]
                            } else {
                                Vec::new()
                            },
                            silent: false,
                        },
                        users: None,
                        test: false,
                        purpose: approval.map(Purpose::Approval),
                    })
                }
                Some("repair") if text(attributes, "severity") == Some("error") => Some(Outgoing {
                    category: Category::Alerts,
                    message: Message {
                        title,
                        body,
                        tag,
                        url: "/#notifications",
                        ttl: Duration::from_hours(24),
                        urgent: true,
                        buttons: Vec::new(),
                        silent: false,
                    },
                    users: None,
                    test: false,
                    purpose: None,
                }),
                _ => None,
            }
        }
        _ => None,
    }
}

/// Un ordre retenu validé, refusé ou oublié : la réponse remplace « Ordre à
/// valider » (même `tag`) sur tous les téléphones, sans sonner.
fn approval_closed(event_type: &str, data: &Attributes, states: &StateMachine) -> Option<Outgoing> {
    let id = data.get("id").and_then(Value::as_u64)?;
    let entity_id: EntityId = text(data, "entity_id")?.parse().ok()?;
    let summary = ha_notices::describe_order(
        states,
        text(data, "domain")?,
        text(data, "service")?,
        &entity_id,
    );
    let (title, body) = if event_type == APPROVAL_EXPIRED {
        (
            "Ordre oublié",
            format!("Personne n'a répondu à temps : rien n'a été fait ({summary})."),
        )
    } else {
        let by = clean(text(data, "by")?, MAX_NAME_CHARS);
        if data.get("approved") == Some(&Value::Bool(true)) {
            ("Ordre validé", format!("{by} a validé : {summary}."))
        } else {
            ("Ordre refusé", format!("{by} a refusé : {summary}."))
        }
    };
    Some(Outgoing {
        category: Category::Approvals,
        message: Message {
            title: title.into(),
            body: clean(&body, MAX_BODY_CHARS),
            tag: format!("{}.{}{id}", ha_notices::DOMAIN, ha_notices::APPROVAL_PREFIX),
            url: "/",
            ttl: ha_core::guard::HELD_TTL,
            urgent: false,
            buttons: Vec::new(),
            silent: true,
        },
        users: None,
        test: false,
        purpose: None,
    })
}

/// Le sujet d'un message d'automatisation : celui qu'elle choisit (`tag` :
/// le message suivant du même sujet remplace le précédent, comme dans Home
/// Assistant), sinon un sujet propre à ce message (il n'efface pas les
/// autres).
fn automation_tag(data: &Attributes) -> String {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let chosen = text(data, "tag").filter(|tag| {
        (1..=40).contains(&tag.chars().count()) && !tag.chars().any(char::is_control)
    });
    match chosen {
        Some(tag) => format!("automatisation.{tag}"),
        None => format!(
            "automatisation.{}",
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ),
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
    // Les boutons : identifiant et titre, deux au plus ; un bouton mal
    // formé est ignoré.
    let buttons: Vec<Button> = if test {
        Vec::new()
    } else {
        data.get("actions")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|button| {
                let action = button.get("action")?.as_str()?;
                let title = clean(button.get("title")?.as_str()?, MAX_BUTTON_TITLE_CHARS)
                    .replace('\n', " ");
                (is_valid_button_id(action) && !title.is_empty()).then(|| Button {
                    action: action.to_owned(),
                    title,
                })
            })
            .take(MAX_BUTTONS)
            .collect()
    };
    let purpose = (!buttons.is_empty()).then_some(Purpose::Automation);
    Some(Outgoing {
        category: Category::Automations,
        message: Message {
            title,
            body,
            tag: if test {
                "essai".into()
            } else {
                automation_tag(data)
            },
            url: "/",
            ttl: Duration::from_hours(24),
            urgent: priority >= 4,
            buttons,
            silent: false,
        },
        users,
        test,
        purpose,
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
        ticket: Option<&str>,
    ) -> Delivery {
        let (Some(public), Some(secret)) = (crypto::decode(p256dh), crypto::decode(auth)) else {
            return Delivery::Failed;
        };
        let secret = Zeroizing::new(secret);
        let body = match crypto::encrypt(&public, &secret, &message.payload(ticket)) {
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
    /// `tickets` est partagé avec l'API, qui sert les boutons touchés.
    pub async fn start(
        store: Store,
        core: Arc<Core>,
        contact: String,
        tickets: Tickets,
    ) -> Result<Self, PushError> {
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
            move |token| run(store, core, sender, tickets, events, token),
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
    tickets: Tickets,
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
        let Some(outgoing) = outgoing(&event, &core.states) else {
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
            tickets.clone(),
            outgoing,
        ));
    }
}

/// Un ticket par appareil : il ne vaut que pour ce compte et cet appareil,
/// le temps que la notification reste utile. `None` sans boutons.
fn issue_ticket(tickets: &Tickets, outgoing: &Outgoing, target: &PushTarget) -> Option<String> {
    let purpose = outgoing.purpose.clone()?;
    if outgoing.message.buttons.is_empty() {
        return None;
    }
    tickets.issue(Ticket::new(
        target.id,
        target.user_id,
        target.username.clone(),
        purpose,
        outgoing
            .message
            .buttons
            .iter()
            .map(|button| button.action.clone())
            .collect(),
        outgoing.message.title.clone(),
        outgoing.message.ttl,
    ))
}

/// Envoie à tous les appareils concernés.
async fn send_all(
    store: Store,
    core: Arc<Core>,
    sender: Arc<Sender>,
    tickets: Tickets,
    outgoing: Outgoing,
) {
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
        let ticket = issue_ticket(&tickets, &outgoing, target);
        // Le contenu n'est pas écrit dans les logs : il peut être personnel.
        let result = sender
            .deliver(
                &endpoint,
                &target.p256dh,
                &target.auth,
                &outgoing.message,
                ticket.as_deref(),
            )
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

    /// Sans appareils nommés : l'identifiant sert de nom.
    fn outgoing(event: &Event) -> Option<Outgoing> {
        super::outgoing(event, &Core::new().states)
    }

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
        assert_eq!(out.purpose, Some(Purpose::Approval(3)));
        let buttons: Vec<&str> = out
            .message
            .buttons
            .iter()
            .map(|b| b.action.as_str())
            .collect();
        assert_eq!(buttons, [APPROVE, DENY]);
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

    #[test]
    fn an_alert_that_becomes_serious_rings() {
        // Une pile à 18 % (avertissement, dans la cloche) tombe à 4 % : la
        // même alerte devient grave, et le téléphone sonne.
        let alert = |severity: &str, message: &str| {
            notice(
                "persistent_notification.repair_battery_sensor_cave",
                "notifying",
                json!({"title": "Pile", "message": message, "kind": "repair", "severity": severity}),
            )
        };
        let out = outgoing(&changed(
            alert("error", "4 %"),
            Some(alert("warning", "18 %")),
        ))
        .unwrap();
        assert_eq!(out.category, Category::Alerts);
        assert!(out.message.urgent);
        // Elle reste grave (3 %) : pas de nouvelle sonnerie.
        assert_eq!(
            outgoing(&changed(alert("error", "3 %"), Some(alert("error", "4 %")))),
            None
        );
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
        assert!(plain.message.payload(None).len() < crypto::MAX_PLAINTEXT);
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
            buttons: Vec::new(),
            silent: false,
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
                None,
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
                    None,
                )
                .await;
            assert_eq!(result, expected, "{status}");
        }
        let (endpoint, _) = fake_service("201 Created").await;
        assert_eq!(
            sender
                .deliver(&endpoint, "AAAA", &crypto::encode(&auth), &message(), None)
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

    #[test]
    fn each_automation_message_keeps_its_own_notification() {
        // Revue de la PR #60 : tous les messages portaient le même sujet et
        // chacun effaçait le précédent sur le téléphone.
        let message = |data: Value| {
            outgoing(&call(data, Origin::Automation("a".into()), ENTITY))
                .unwrap()
                .message
                .tag
        };
        let first = message(json!({"message": "La machine a fini."}));
        let second = message(json!({"message": "Porte du garage ouverte."}));
        assert_ne!(first, second);
        // Un sujet choisi (« tag ») remplace le message précédent du même sujet.
        assert_eq!(
            message(json!({"message": "x", "tag": "lessive"})),
            message(json!({"message": "y", "tag": "lessive"}))
        );
        assert_eq!(
            message(json!({"message": "x", "tag": "lessive"})),
            "automatisation.lessive"
        );
        // Un sujet bizarre est ignoré (pas de caractères de contrôle, 40 au plus).
        assert_ne!(
            message(json!({"message": "x", "tag": "a\u{7}b"})),
            message(json!({"message": "y", "tag": "a\u{7}b"}))
        );
    }

    #[test]
    fn automations_can_add_two_buttons() {
        let out = outgoing(&call(
            json!({"message": "La machine a fini.", "actions": [
                {"action": "LESSIVE_VUE", "title": "Je m'en occupe"},
                {"action": "pas d'espace", "title": "ignoré"},
                {"action": "SANS_TITRE"},
                {"action": "PLUS_TARD", "title": "Plus tard\nvraiment, c'est beaucoup trop long"},
                {"action": "TROISIEME", "title": "de trop"},
            ]}),
            Origin::Automation("lessive".into()),
            ENTITY,
        ))
        .unwrap();
        assert_eq!(out.purpose, Some(Purpose::Automation));
        assert_eq!(
            out.message.buttons,
            [
                Button {
                    action: "LESSIVE_VUE".into(),
                    title: "Je m'en occupe".into()
                },
                Button {
                    action: "PLUS_TARD".into(),
                    title: "Plus tard vraiment, c'es".into()
                },
            ]
        );
        // Sans bouton valable : pas de ticket.
        let plain = outgoing(&call(
            json!({"message": "x", "actions": [{"action": "", "title": "x"}]}),
            Origin::System,
            ENTITY,
        ))
        .unwrap();
        assert_eq!(plain.purpose, None);
        // Un essai n'a jamais de boutons.
        let test = outgoing(&call(
            json!({"message": "x", "test": true, "actions": [{"action": "A", "title": "a"}]}),
            Origin::User("david".into()),
            ENTITY,
        ))
        .unwrap();
        assert_eq!(test.purpose, None);
        assert_eq!(test.message.buttons, []);
    }

    #[test]
    fn buttons_travel_only_with_a_ticket() {
        let mut message = message();
        message.buttons = vec![Button {
            action: APPROVE.into(),
            title: "Valider".into(),
        }];
        let without: Value = serde_json::from_slice(&message.payload(None)).unwrap();
        assert!(without.get("actions").is_none());
        assert!(without.get("token").is_none());
        let with: Value = serde_json::from_slice(&message.payload(Some("secret"))).unwrap();
        assert_eq!(with["token"], "secret");
        assert_eq!(with["actions"][0]["action"], APPROVE);
        assert_eq!(with["actions"][0]["title"], "Valider");
        // Le pire cas tient dans un seul envoi chiffré.
        message.title = "é".repeat(MAX_TITLE_CHARS);
        message.body = "\u{1F600}".repeat(MAX_BODY_CHARS);
        message.buttons = vec![
            Button {
                action: "A".repeat(MAX_BUTTON_ID_LEN),
                title: "\u{1F600}".repeat(MAX_BUTTON_TITLE_CHARS),
            };
            MAX_BUTTONS
        ];
        assert!(message.payload(Some(&"t".repeat(43))).len() <= crypto::MAX_PLAINTEXT);
        assert!(is_valid_button_id("LESSIVE_VUE-2"));
        assert!(!is_valid_button_id("é"));
        assert!(!is_valid_button_id(&"a".repeat(41)));
    }

    #[test]
    fn each_device_gets_its_own_ticket() {
        let tickets = Tickets::default();
        let out = outgoing(&changed(
            notice(
                "persistent_notification.approval_4",
                "notifying",
                json!({"title": "Ordre à valider", "message": "…", "kind": "approval", "approval_id": 4}),
            ),
            None,
        ))
        .unwrap();
        let target = |id, user_id, username: &str| PushTarget {
            id,
            user_id,
            username: username.into(),
            endpoint: "https://fcm.googleapis.com/fcm/send/x".into(),
            p256dh: String::new(),
            auth: Zeroizing::new(String::new()),
        };
        let phone = issue_ticket(&tickets, &out, &target(10, 1, "david")).unwrap();
        let tablet = issue_ticket(&tickets, &out, &target(11, 2, "alice")).unwrap();
        assert_ne!(phone, tablet);
        let served = tickets.redeem(&tablet, DENY).unwrap();
        assert_eq!(
            (
                served.subscription,
                served.user_id,
                served.username.as_str()
            ),
            (11, 2, "alice")
        );
        assert_eq!(served.purpose, Purpose::Approval(4));
        assert_eq!(served.title, "Ordre à valider");
        // Sans boutons : pas de ticket.
        let mut plain = out;
        plain.message.buttons.clear();
        assert_eq!(
            issue_ticket(&tickets, &plain, &target(10, 1, "david")),
            None
        );
        plain.purpose = None;
        assert_eq!(
            issue_ticket(&tickets, &plain, &target(10, 1, "david")),
            None
        );
        assert_eq!(tickets.len(), 1, "seul le ticket du téléphone reste");
    }

    fn fired(event_type: &str, data: Value) -> Event {
        Event::Fired {
            event_type: event_type.into(),
            data: attributes(data),
        }
    }

    #[test]
    fn an_answered_or_forgotten_order_replaces_its_notification_quietly() {
        let core = Core::new();
        let volet: EntityId = "cover.chambre".parse().unwrap();
        core.states
            .set(
                volet,
                "open",
                attributes(json!({"friendly_name": "Volet chambre"})),
            )
            .unwrap();
        let order = json!({"id": 4, "domain": "cover", "service": "close_cover", "entity_id": "cover.chambre"});
        let mut answered = order.clone();
        answered["approved"] = true.into();
        answered["by"] = "alice".into();
        let out =
            super::outgoing(&fired(APPROVAL_ANSWERED, answered.clone()), &core.states).unwrap();
        assert_eq!(out.category, Category::Approvals);
        assert_eq!(out.message.title, "Ordre validé");
        assert_eq!(out.message.body, "alice a validé : fermer Volet chambre.");
        assert_eq!(
            out.message.tag, "persistent_notification.approval_4",
            "le même sujet que « Ordre à valider » : il le remplace"
        );
        assert!(out.message.silent);
        assert!(!out.message.urgent);
        assert_eq!(out.purpose, None, "plus de boutons");
        answered["approved"] = false.into();
        let out = super::outgoing(&fired(APPROVAL_ANSWERED, answered), &core.states).unwrap();
        assert_eq!(out.message.body, "alice a refusé : fermer Volet chambre.");
        let out = super::outgoing(&fired(APPROVAL_EXPIRED, order), &core.states).unwrap();
        assert_eq!(out.message.title, "Ordre oublié");
        assert_eq!(
            out.message.body,
            "Personne n'a répondu à temps : rien n'a été fait (fermer Volet chambre)."
        );
        // Incomplet, ou un autre événement : rien.
        assert_eq!(outgoing(&fired(APPROVAL_EXPIRED, json!({"id": 4}))), None);
        assert_eq!(
            outgoing(&fired("timer.finished", json!({"entity_id": "timer.x"}))),
            None
        );
        let payload: Value = serde_json::from_slice(&out.message.payload(None)).unwrap();
        assert_eq!(payload["silent"], true);
    }

    #[test]
    fn a_serious_alert_that_goes_away_is_resolved() {
        let gone = |state: State| Event::StateChanged {
            entity_id: state.entity_id().clone(),
            old_state: Some(state),
            new_state: None,
        };
        let error = notice(
            "persistent_notification.repair_integration_mqtt",
            "notifying",
            json!({"title": "MQTT en panne", "message": "…", "kind": "repair", "severity": "error"}),
        );
        let out = outgoing(&gone(error)).unwrap();
        assert_eq!(out.category, Category::Alerts);
        assert_eq!(out.message.title, "Résolu");
        assert_eq!(out.message.body, "MQTT en panne : c'est réglé.");
        assert_eq!(
            out.message.tag,
            "persistent_notification.repair_integration_mqtt"
        );
        assert!(out.message.silent);
        // Un avertissement, une alerte déjà ignorée, une notification : rien.
        for (state, kind, severity) in [
            ("notifying", "repair", "warning"),
            ("ignored", "repair", "error"),
            ("notifying", "notification", "error"),
            ("notifying", "approval", "error"),
        ] {
            let old = notice(
                "persistent_notification.x",
                state,
                json!({"title": "x", "message": "x", "kind": kind, "severity": severity}),
            );
            assert_eq!(outgoing(&gone(old)), None, "{state} {kind} {severity}");
        }
    }
}
