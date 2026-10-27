//! Les notifications du téléphone, relayées au hub (étape 103).
//!
//! Certains appareils n'ont ni API locale ni intégration officielle (la
//! tondeuse Mammotion, par exemple), mais leur appli envoie des notifications
//! au téléphone : « Tonte terminée », « Tondeuse bloquée »… Une appli Android
//! comme MacroDroid les recopie vers `POST /api/relais`. Le hub en tire l'état
//! de l'appareil (`sensor.tondeuse`) et publie l'événement
//! `notification_telephone`, utilisable dans les automatisations.
//!
//! Pas de compte ni de jeton de session : le téléphone envoie la **clé du
//! relais** (`Authorization: Bearer …`, fichier `HA_RELAIS_KEY_FILE`). Elle ne
//! permet rien d'autre que cette route : mettre à jour la tondeuse et publier
//! l'événement. Sans clé configurée, la route n'existe pas (404).
//!
//! Corps accepté : JSON (`{"app", "titre", "texte"}`) ou formulaire
//! (`app=…&titre=…&texte=…`), ce que MacroDroid sait envoyer.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, header};
use ha_core::{Attributes, EntityId};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::ApiState;
use crate::error::ApiError;
use crate::json::format_timestamp;

/// L'entité mise à jour par les notifications de la tondeuse.
const MOWER_ENTITY: &str = "sensor.tondeuse";
/// L'événement publié pour chaque notification reçue.
const EVENT: &str = "notification_telephone";
/// Le « propriétaire » de l'entité : le relais n'est pas une intégration
/// suivie, la tondeuse ne passe donc jamais « indisponible » à cause de lui.
const OWNER: &str = "relais";
/// Une clé plus courte se devinerait trop facilement.
const MIN_KEY_LEN: usize = 32;
/// Au-delà, les notifications sont refusées jusqu'à la minute suivante.
const MAX_PER_MINUTE: u32 = 30;
const WINDOW: Duration = Duration::from_mins(1);
const MAX_TITLE: usize = 200;
const MAX_TEXT: usize = 1000;
const MAX_APP: usize = 100;

/// La clé du relais (son empreinte seulement) et sa limite de débit.
#[derive(Debug)]
pub(crate) struct Relay {
    key_digest: [u8; 32],
    window: Mutex<(Instant, u32)>,
}

impl Relay {
    /// La clé, telle qu'écrite dans le fichier (espaces autour retirés).
    pub(crate) fn new(key: &str) -> Result<Self, String> {
        let key = key.trim();
        if key.chars().count() < MIN_KEY_LEN {
            return Err(format!(
                "clé du relais trop courte : {MIN_KEY_LEN} caractères au moins"
            ));
        }
        if key.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err("clé du relais : ni espace ni caractère de contrôle".into());
        }
        Ok(Self {
            key_digest: Sha256::digest(key.as_bytes()).into(),
            window: Mutex::new((Instant::now(), 0)),
        })
    }

    /// On compare les empreintes : la durée de la comparaison ne dit rien
    /// de la clé.
    fn accepts(&self, key: &str) -> bool {
        let digest: [u8; 32] = Sha256::digest(key.trim().as_bytes()).into();
        digest == self.key_digest
    }

    /// Compte une notification ; `Err` donne le temps à attendre.
    fn take(&self, now: Instant) -> Result<(), Duration> {
        let mut window = self
            .window
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let elapsed = now.saturating_duration_since(window.0);
        if elapsed >= WINDOW {
            *window = (now, 0);
        }
        if window.1 >= MAX_PER_MINUTE {
            return Err(WINDOW.saturating_sub(elapsed));
        }
        window.1 += 1;
        Ok(())
    }
}

/// Une notification, nettoyée.
#[derive(Debug, PartialEq, Eq)]
struct Notification {
    app: String,
    title: String,
    text: String,
}

#[derive(Deserialize)]
struct JsonBody {
    #[serde(default)]
    app: Option<String>,
    #[serde(default)]
    titre: Option<String>,
    #[serde(default)]
    texte: Option<String>,
}

/// Les caractères de contrôle deviennent des espaces, et le texte est coupé.
fn clean(value: Option<&str>, max: usize) -> String {
    let value: String = value
        .unwrap_or_default()
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    value
        .trim()
        .chars()
        .take(max)
        .collect::<String>()
        .trim_end()
        .to_owned()
}

fn parse(content_type: &str, body: &[u8]) -> Result<Notification, ApiError> {
    let mime = content_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    let (app, title, text) = match mime.as_str() {
        "application/json" => {
            let body: JsonBody = serde_json::from_slice(body)
                .map_err(|error| ApiError::BadRequest(format!("JSON invalide : {error}")))?;
            (body.app, body.titre, body.texte)
        }
        "application/x-www-form-urlencoded" => {
            let fields: HashMap<String, String> =
                url::form_urlencoded::parse(body).into_owned().collect();
            (
                fields.get("app").cloned(),
                fields.get("titre").cloned(),
                fields.get("texte").cloned(),
            )
        }
        _ => {
            return Err(ApiError::BadRequest(
                "corps attendu : JSON ou formulaire (application/x-www-form-urlencoded)".into(),
            ));
        }
    };
    let notification = Notification {
        app: clean(app.as_deref(), MAX_APP),
        title: clean(title.as_deref(), MAX_TITLE),
        text: clean(text.as_deref(), MAX_TEXT),
    };
    if notification.title.is_empty() && notification.text.is_empty() {
        return Err(ApiError::BadRequest(
            "notification vide : il faut un titre ou un texte".into(),
        ));
    }
    Ok(notification)
}

/// Les règles, dans l'ordre : la première qui convient donne l'état. Une
/// règle est une liste de débuts de mots qui doivent **tous** être présents
/// (« charge » et « terminée » : la batterie est pleine, la tonte n'est pas
/// forcément finie).
const RULES: &[(&str, &[&[&str]])] = &[
    (
        "bloquée",
        &[
            &["bloqu"],
            &["coinc"],
            &["soulev"],
            &["renvers"],
            &["stuck"],
            &["trapped"],
            &["lifted"],
            &["tilt"],
        ],
    ),
    (
        "erreur",
        &[
            &["erreur"],
            &["échec"],
            &["echec"],
            &["anomalie"],
            &["défaut"],
            &["defaut"],
            &["panne"],
            &["error"],
            &["fault"],
            &["fail"],
        ],
    ),
    ("en pause (pluie)", &[&["pluie"], &["pluvi"], &["rain"]]),
    (
        "chargée",
        &[
            &["charg", "termin"],
            &["charg", "complet"],
            &["charg", "fini"],
            &["batterie", "pleine"],
            &["fully", "charged"],
        ],
    ),
    (
        "tonte terminée",
        &[&["termin"], &["achev"], &["fini"], &["complet"], &["done"]],
    ),
    ("en pause", &[&["pause"], &["suspend"]]),
    ("retour à la base", &[&["retour"], &["rentre"], &["return"]]),
    ("en charge", &[&["charg"]]),
    (
        "en tonte",
        &[
            &["commenc"],
            &["démarr"],
            &["demarr"],
            &["lanc"],
            &["start"],
            &["mowing"],
            &["tonte", "cours"],
        ],
    ),
    (
        "batterie faible",
        &[&["batterie", "faible"], &["low", "battery"]],
    ),
];

/// L'état que décrit la notification, s'il est reconnu.
fn classify(title: &str, text: &str) -> Option<&'static str> {
    let lower = format!("{title} {text}").to_lowercase();
    let words: Vec<&str> = lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect();
    let has = |prefix: &str| words.iter().any(|word| word.starts_with(prefix));
    RULES
        .iter()
        .find(|(_, alternatives)| {
            alternatives
                .iter()
                .any(|all| all.iter().all(|prefix| has(prefix)))
        })
        .map(|(state, _)| *state)
}

/// La notification vient-elle de l'appli de la tondeuse ?
fn from_mower(app: &str) -> bool {
    app.to_lowercase().contains("mammotion")
}

/// L'état à garder quand la notification n'en décrit aucun.
fn kept_state(previous: Option<&str>) -> String {
    match previous {
        Some(state) if state != "unavailable" && state != "unknown" => state.to_owned(),
        _ => "inconnu".to_owned(),
    }
}

fn publish_mower(
    state: &ApiState,
    notification: &Notification,
    recognised: Option<&str>,
) -> Result<String, ApiError> {
    let entity_id =
        EntityId::new(MOWER_ENTITY).map_err(|error| ApiError::Internal(error.to_string()))?;
    let value = match recognised {
        Some(value) => value.to_owned(),
        None => kept_state(
            state
                .core
                .states
                .get(&entity_id)
                .as_deref()
                .map(ha_core::State::state),
        ),
    };
    let last = if notification.text.is_empty() {
        &notification.title
    } else {
        &notification.text
    };
    let mut attributes = Attributes::new();
    attributes.insert("friendly_name".into(), json!("Tondeuse"));
    attributes.insert("icon".into(), json!("mdi:robot-mower"));
    attributes.insert("dernier_message".into(), json!(last));
    attributes.insert("titre".into(), json!(notification.title));
    attributes.insert("recu".into(), json!(format_timestamp(SystemTime::now())));
    attributes.insert("source".into(), json!("notifications du téléphone"));
    state
        .core
        .states
        .set_by(OWNER, entity_id, value.clone(), attributes)
        .map_err(|error| ApiError::Internal(error.to_string()))?;
    Ok(value)
}

/// `POST /api/relais` : une notification reçue par le téléphone.
pub(crate) async fn receive(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<Value>, ApiError> {
    let Some(relay) = state.relay.get() else {
        return Err(ApiError::NotFound);
    };
    let key = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .ok_or(ApiError::Unauthorized)?;
    if !relay.accepts(key) {
        tracing::warn!("relais des notifications : clé refusée");
        return Err(ApiError::Unauthorized);
    }
    relay
        .take(Instant::now())
        .map_err(|retry_after| ApiError::TooManyRequests { retry_after })?;
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    let notification = parse(content_type, &body)?;
    let recognised = classify(&notification.title, &notification.text);
    let mower = if from_mower(&notification.app) {
        Some(publish_mower(&state, &notification, recognised)?)
    } else {
        None
    };

    let mut data = Attributes::new();
    data.insert("app".into(), json!(notification.app));
    data.insert("titre".into(), json!(notification.title));
    data.insert("texte".into(), json!(notification.text));
    data.insert("etat".into(), json!(recognised));
    state
        .core
        .fire(EVENT, data)
        .map_err(|error| ApiError::Internal(error.to_string()))?;
    Ok(Json(json!({
        "result": "ok",
        "etat": recognised,
        "tondeuse": mower,
    })))
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        reason = "dans un test, un échec doit faire échouer le test"
    )]

    use super::*;

    #[test]
    fn mower_messages_are_recognised() {
        for (title, text, expected) in [
            ("Mammotion", "Votre tondeuse est bloquée", Some("bloquée")),
            (
                "Alerte",
                "Robot soulevé, arrêt de sécurité",
                Some("bloquée"),
            ),
            ("Luba", "Erreur moteur de lame", Some("erreur")),
            (
                "Luba",
                "Pluie détectée, la tonte est mise en pause",
                Some("en pause (pluie)"),
            ),
            (
                "Luba",
                "Tonte terminée, retour à la station",
                Some("tonte terminée"),
            ),
            ("Luba", "Charge terminée", Some("chargée")),
            ("Luba", "Tâche mise en pause", Some("en pause")),
            (
                "Luba",
                "Retour à la station de charge",
                Some("retour à la base"),
            ),
            ("Luba", "En charge", Some("en charge")),
            ("Luba", "La tonte a commencé", Some("en tonte")),
            ("Luba", "Tonte en cours", Some("en tonte")),
            ("Luba", "Batterie faible", Some("batterie faible")),
            ("Luba", "Task completed", Some("tonte terminée")),
            ("Luba", "Mower is stuck", Some("bloquée")),
            ("Luba", "Nouvelle version disponible", None),
        ] {
            assert_eq!(classify(title, text), expected, "{text}");
        }
    }

    #[test]
    fn keywords_match_the_start_of_a_word() {
        // « terrain » contient « rain », mais ne parle pas de pluie.
        assert_eq!(classify("", "Carte du terrain mise à jour"), None);
        // « restart » ne commence pas par « start ».
        assert_eq!(classify("", "Please restart the app"), None);
    }

    #[test]
    fn an_unknown_message_keeps_the_state() {
        assert_eq!(kept_state(Some("en tonte")), "en tonte");
        assert_eq!(kept_state(Some("unavailable")), "inconnu");
        assert_eq!(kept_state(Some("unknown")), "inconnu");
        assert_eq!(kept_state(None), "inconnu");
    }

    #[test]
    fn only_the_mower_app_updates_the_mower() {
        assert!(from_mower("Mammotion"));
        assert!(from_mower("com.agilexrobotics.mammotion"));
        assert!(!from_mower("WhatsApp"));
    }

    #[test]
    fn bodies_are_parsed_and_cleaned() {
        let form = parse(
            "application/x-www-form-urlencoded; charset=UTF-8",
            "app=Mammotion&titre=Luba&texte=Tondeuse+bloqu%C3%A9e%0A".as_bytes(),
        )
        .unwrap();
        assert_eq!(
            form,
            Notification {
                app: "Mammotion".into(),
                title: "Luba".into(),
                text: "Tondeuse bloquée".into(),
            }
        );
        let json = parse(
            "application/json",
            br#"{"app": "Mammotion", "texte": "a\u0007b"}"#,
        )
        .unwrap();
        assert_eq!(json.text, "a b");
        assert_eq!(json.title, "");

        let long = "x".repeat(5000);
        let json = parse(
            "application/json",
            json!({"texte": long}).to_string().as_bytes(),
        )
        .unwrap();
        assert_eq!(json.text.chars().count(), MAX_TEXT);

        assert!(parse("application/json", br#"{"app": "Mammotion"}"#).is_err());
        assert!(parse("text/plain", b"bonjour").is_err());
    }

    #[test]
    fn the_key_is_checked() {
        assert!(Relay::new("trop-courte").is_err());
        assert!(Relay::new(&format!("{} avec espace", "a".repeat(32))).is_err());
        let key = "k".repeat(40);
        let relay = Relay::new(&format!("{key}\n")).unwrap();
        assert!(relay.accepts(&key));
        assert!(!relay.accepts(&"k".repeat(39)));
    }

    #[test]
    fn the_rate_is_limited() {
        let relay = Relay::new(&"k".repeat(40)).unwrap();
        let start = Instant::now();
        for _ in 0..MAX_PER_MINUTE {
            relay.take(start).unwrap();
        }
        assert!(relay.take(start).is_err());
        assert!(relay.take(start + WINDOW).is_ok());
    }
}
