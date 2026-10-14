//! Appels à l'API locale du pont Hue (« CLIP v2 »).

use std::time::Duration;

use futures_util::StreamExt;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use zeroize::Zeroizing;

use crate::error::HueError;

/// Taille maximale d'une réponse : la liste des lampes d'une maison tient en
/// quelques dizaines de Kio.
const MAX_RESPONSE: usize = 1024 * 1024;
/// Délai maximal d'une requête ordinaire.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Une lampe telle que le pont la décrit. On ne lit que les champs utiles ;
/// les autres sont ignorés (le pont en ajoute au fil des mises à jour).
#[derive(Debug, Clone, Deserialize)]
pub struct LightResource {
    pub id: String,
    #[serde(default)]
    pub metadata: Option<Metadata>,
    #[serde(default)]
    pub on: Option<OnState>,
    #[serde(default)]
    pub dimming: Option<Dimming>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Metadata {
    pub name: String,
}

#[derive(Debug, Clone, Copy, Deserialize)]
pub struct OnState {
    pub on: bool,
}

#[derive(Debug, Clone, Copy, Deserialize)]
pub struct Dimming {
    /// Luminosité en pourcentage (0 à 100).
    pub brightness: f64,
}

#[derive(Debug, Deserialize)]
struct Envelope<T> {
    #[serde(default)]
    errors: Vec<ApiError>,
    #[serde(default = "Vec::new")]
    data: Vec<T>,
}

#[derive(Debug, Deserialize)]
struct ApiError {
    description: String,
}

/// Accès authentifié au pont.
///
/// Concept Rust : `#[derive(Clone)]` sur une structure qui contient un
/// `reqwest::Client` est peu coûteux : le client partage ses connexions en
/// interne. On peut donc en donner une copie à chaque tâche.
#[derive(Clone)]
pub struct HueApi {
    http: reqwest::Client,
    base: String,
    key: Zeroizing<String>,
}

/// Jamais la clé dans un affichage de débogage.
impl std::fmt::Debug for HueApi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HueApi")
            .field("base", &self.base)
            .finish_non_exhaustive()
    }
}

impl HueApi {
    pub fn new(http: reqwest::Client, host: &str, key: Zeroizing<String>) -> Self {
        Self {
            http,
            base: format!("https://{host}"),
            key,
        }
    }

    /// Toutes les lampes.
    pub async fn lights(&self) -> Result<Vec<LightResource>, HueError> {
        let response = self
            .http
            .get(format!("{}/clip/v2/resource/light", self.base))
            .header("hue-application-key", self.key.as_str())
            .timeout(REQUEST_TIMEOUT)
            .send()
            .await?;
        let envelope: Envelope<LightResource> = read_json(response).await?;
        if let Some(error) = envelope.errors.first() {
            return Err(HueError::Bridge(error.description.clone()));
        }
        Ok(envelope.data)
    }

    /// Envoie une commande à une lampe : `{"on": {"on": true}, "dimming": {…}}`.
    pub async fn set_light(&self, id: &str, body: &Value) -> Result<(), HueError> {
        if !is_resource_id(id) {
            return Err(HueError::Bridge(format!(
                "identifiant de lampe invalide : {id:?}"
            )));
        }
        let response = self
            .http
            .put(format!("{}/clip/v2/resource/light/{id}", self.base))
            .header("hue-application-key", self.key.as_str())
            .header("Content-Type", "application/json")
            .body(body.to_string())
            .timeout(REQUEST_TIMEOUT)
            .send()
            .await?;
        let envelope: Envelope<Value> = read_json(response).await?;
        match envelope.errors.first() {
            Some(error) => Err(HueError::Bridge(error.description.clone())),
            None => Ok(()),
        }
    }

    /// Ouvre le flux d'événements (« Server-Sent Events ») : le pont y signale
    /// chaque changement en direct, même ceux faits avec l'appli Hue ou Alexa.
    pub async fn event_stream(&self) -> Result<reqwest::Response, HueError> {
        let response = self
            .http
            .get(format!("{}/eventstream/clip/v2", self.base))
            .header("hue-application-key", self.key.as_str())
            .header("Accept", "text/event-stream")
            .send()
            .await?
            .error_for_status()?;
        Ok(response)
    }
}

/// Lit l'identifiant du pont (`001788FFFE…`). Cette route ne demande pas de clé.
pub async fn fetch_bridge_id(http: &reqwest::Client, host: &str) -> Result<String, HueError> {
    let response = http
        .get(format!("https://{host}/api/0/config"))
        .timeout(REQUEST_TIMEOUT)
        .send()
        .await?;
    let config: Value = read_json(response).await?;
    let bridge_id = config
        .get("bridgeid")
        .and_then(Value::as_str)
        .filter(|id| id.len() == 16 && id.bytes().all(|b| b.is_ascii_hexdigit()))
        .ok_or_else(|| HueError::Bridge("ce n'est pas un pont Hue (pas de bridgeid)".into()))?;
    Ok(bridge_id.to_ascii_uppercase())
}

/// Demande une clé d'application. `Ok(None)` tant que le bouton du pont n'a
/// pas été pressé.
pub async fn request_app_key(
    http: &reqwest::Client,
    host: &str,
) -> Result<Option<Zeroizing<String>>, HueError> {
    let response = http
        .post(format!("https://{host}/api"))
        .header("Content-Type", "application/json")
        .body(r#"{"devicetype": "ha_rust#serveur", "generateclientkey": true}"#)
        .timeout(REQUEST_TIMEOUT)
        .send()
        .await?;
    let reply: Value = read_json(response).await?;
    let first = reply
        .get(0)
        .ok_or_else(|| HueError::Bridge("réponse vide".into()))?;
    if let Some(key) = first.pointer("/success/username").and_then(Value::as_str) {
        let valid = (20..=100).contains(&key.len())
            && key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-');
        if !valid {
            return Err(HueError::Bridge("clé d'application invalide".into()));
        }
        return Ok(Some(Zeroizing::new(key.to_owned())));
    }
    // Erreur 101 : « link button not pressed ».
    if first.pointer("/error/type").and_then(Value::as_u64) == Some(101) {
        return Ok(None);
    }
    let description = first
        .pointer("/error/description")
        .and_then(Value::as_str)
        .unwrap_or("erreur inconnue");
    Err(HueError::Bridge(description.chars().take(200).collect()))
}

/// Les identifiants Hue v2 sont des UUID : `8-4-4-4-12` caractères hexadécimaux.
///
/// Sécurité : l'identifiant vient du pont et finit dans une adresse
/// (`…/resource/light/<id>`). On vérifie qu'il ne contient rien d'autre.
pub fn is_resource_id(id: &str) -> bool {
    let parts: Vec<&str> = id.split('-').collect();
    parts.len() == 5
        && parts
            .iter()
            .zip([8, 4, 4, 4, 12])
            .all(|(part, len)| part.len() == len && part.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// Lit une réponse JSON en refusant les réponses démesurées.
async fn read_json<T: DeserializeOwned>(response: reqwest::Response) -> Result<T, HueError> {
    let mut body = Vec::new();
    let mut chunks = response.bytes_stream();
    while let Some(chunk) = chunks.next().await {
        let chunk = chunk?;
        if body.len() + chunk.len() > MAX_RESPONSE {
            return Err(HueError::TooLarge);
        }
        body.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&body)
        .map_err(|error| HueError::Bridge(format!("JSON invalide : {error}")))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn resource_ids() {
        assert!(is_resource_id("3f8d6e6a-1b2c-4d5e-8f90-0123456789ab"));
        for bad in [
            "",
            "3f8d6e6a",
            "3f8d6e6a-1b2c-4d5e-8f90-0123456789ab/../../api",
            "3f8d6e6a-1b2c-4d5e-8f90-0123456789zz",
            "3f8d6e6a-1b2c-4d5e-8f90-0123456789ab-00",
        ] {
            assert!(!is_resource_id(bad), "{bad}");
        }
    }
}
