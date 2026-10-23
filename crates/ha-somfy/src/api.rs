//! L'API locale de la box TaHoma (« mode développeur » de Somfy, protocole
//! Overkiz). La box répond en HTTPS sur le port 8443 du réseau local.

use std::time::Duration;

use futures_util::StreamExt;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use zeroize::Zeroizing;

use crate::error::SomfyError;

/// Port de l'API locale.
pub const LOCAL_PORT: u16 = 8443;
const BASE_PATH: &str = "/enduser-mobile-web/1/enduserAPI";
const MAX_RESPONSE: usize = 4 * 1024 * 1024;
pub(crate) const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

/// Un appareil connu de la box.
#[derive(Debug, Clone, Deserialize)]
pub struct Device {
    #[serde(rename = "deviceURL")]
    pub device_url: String,
    pub label: String,
    #[serde(default, rename = "controllableName")]
    pub controllable_name: String,
    #[serde(default)]
    pub definition: Definition,
    #[serde(default)]
    pub states: Vec<StateValue>,
    #[serde(default = "available_by_default")]
    pub available: bool,
}

fn available_by_default() -> bool {
    true
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Definition {
    #[serde(default, rename = "uiClass")]
    pub ui_class: String,
    #[serde(default)]
    pub commands: Vec<CommandDefinition>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CommandDefinition {
    #[serde(rename = "commandName")]
    pub name: String,
}

/// Un état d'appareil : `core:ClosureState` = `42`…
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct StateValue {
    pub name: String,
    #[serde(default)]
    pub value: Value,
}

/// Un événement remonté par la box.
#[derive(Debug, Clone, Deserialize)]
pub struct Event {
    pub name: String,
    #[serde(default, rename = "deviceURL")]
    pub device_url: Option<String>,
    #[serde(default, rename = "deviceStates")]
    pub device_states: Vec<StateValue>,
}

/// Accès authentifié à la box.
#[derive(Clone)]
pub struct LocalApi {
    http: reqwest::Client,
    base: String,
    token: Zeroizing<String>,
}

/// Jamais le jeton dans un affichage de débogage.
impl std::fmt::Debug for LocalApi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalApi")
            .field("base", &self.base)
            .finish_non_exhaustive()
    }
}

/// Adresse de base de l'API : `https://<hôte>:8443/enduser-mobile-web/1/enduserAPI`.
/// Si l'hôte contient déjà un port, il est gardé (utile pour les tests).
#[must_use]
pub fn base_url(host: &str) -> String {
    if host.contains(':') {
        format!("https://{host}{BASE_PATH}")
    } else {
        format!("https://{host}:{LOCAL_PORT}{BASE_PATH}")
    }
}

impl LocalApi {
    #[must_use]
    pub fn new(http: reqwest::Client, host: &str, token: Zeroizing<String>) -> Self {
        Self {
            http,
            base: base_url(host),
            token,
        }
    }

    /// Tous les appareils, avec leurs états.
    pub async fn devices(&self) -> Result<Vec<Device>, SomfyError> {
        self.get("/setup/devices").await
    }

    /// Envoie une commande à un appareil, par exemple `setClosure` avec `[40]`.
    pub async fn execute(
        &self,
        device_url: &str,
        command: &str,
        parameters: Vec<Value>,
    ) -> Result<(), SomfyError> {
        let body = json!({
            "label": "ha-rust",
            "actions": [{
                "deviceURL": device_url,
                "commands": [{"name": command, "parameters": parameters}],
            }],
        });
        let _: Value = self.post("/exec/apply", &body).await?;
        Ok(())
    }

    /// Ouvre un « écouteur » d'événements. Il faut le relever régulièrement
    /// avec [`LocalApi::fetch_events`], sinon la box l'oublie (~10 min).
    pub async fn register_listener(&self) -> Result<String, SomfyError> {
        let reply: Value = self.post("/events/register", &json!({})).await?;
        let id = reply
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| is_safe_path_part(id))
            .ok_or_else(|| SomfyError::Unexpected("identifiant d'écouteur invalide".into()))?;
        Ok(id.to_owned())
    }

    /// Les événements arrivés depuis le dernier relevé.
    pub async fn fetch_events(&self, listener: &str) -> Result<Vec<Event>, SomfyError> {
        if !is_safe_path_part(listener) {
            return Err(SomfyError::Unexpected(
                "identifiant d'écouteur invalide".into(),
            ));
        }
        self.post(&format!("/events/{listener}/fetch"), &json!({}))
            .await
    }

    async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T, SomfyError> {
        let response = self
            .http
            .get(format!("{}{path}", self.base))
            .bearer_auth(self.token.as_str())
            .timeout(REQUEST_TIMEOUT)
            .send()
            .await?;
        read_json(response).await
    }

    async fn post<T: DeserializeOwned>(&self, path: &str, body: &Value) -> Result<T, SomfyError> {
        let response = self
            .http
            .post(format!("{}{path}", self.base))
            .bearer_auth(self.token.as_str())
            .header("Content-Type", "application/json")
            .body(body.to_string())
            .timeout(REQUEST_TIMEOUT)
            .send()
            .await?;
        read_json(response).await
    }
}

/// Vérifie qu'on parle bien à une box TaHoma (route publique `apiVersion`).
pub async fn api_version(http: &reqwest::Client, host: &str) -> Result<String, SomfyError> {
    let response = http
        .get(format!("{}/apiVersion", base_url(host)))
        .timeout(REQUEST_TIMEOUT)
        .send()
        .await?;
    let reply: Value = read_json(response).await?;
    reply
        .get("protocolVersion")
        .and_then(Value::as_str)
        .map(|v| v.chars().take(40).collect())
        .ok_or_else(|| SomfyError::Unexpected("ce n'est pas une box TaHoma".into()))
}

/// Lit une réponse JSON, en refusant les réponses démesurées.
pub(crate) async fn read_json<T: DeserializeOwned>(
    response: reqwest::Response,
) -> Result<T, SomfyError> {
    let status = response.status();
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        return Err(SomfyError::Unauthorized);
    }
    let mut body = Vec::new();
    let mut chunks = response.bytes_stream();
    while let Some(chunk) = chunks.next().await {
        let chunk = chunk?;
        if body.len() + chunk.len() > MAX_RESPONSE {
            return Err(SomfyError::TooLarge);
        }
        body.extend_from_slice(&chunk);
    }
    if !status.is_success() {
        let detail: String = String::from_utf8_lossy(&body)
            .chars()
            .filter(|c| !c.is_control())
            .take(200)
            .collect();
        return Err(SomfyError::Unexpected(format!("{status} {detail}")));
    }
    serde_json::from_slice(&body).map_err(|error| SomfyError::Unexpected(format!("JSON : {error}")))
}

/// Un morceau d'adresse sûr (identifiant d'écouteur…) : lettres, chiffres, `-`.
pub(crate) fn is_safe_path_part(value: &str) -> bool {
    (1..=100).contains(&value.len())
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls() {
        assert_eq!(
            base_url("192.168.1.30"),
            "https://192.168.1.30:8443/enduser-mobile-web/1/enduserAPI"
        );
        assert_eq!(
            base_url("127.0.0.1:9000"),
            "https://127.0.0.1:9000/enduser-mobile-web/1/enduserAPI"
        );
        assert!(is_safe_path_part("8b4ef3d1-0a40-4c6f-a8f2-14bd2c6e0d17"));
        assert!(!is_safe_path_part("../setup"));
        assert!(!is_safe_path_part(""));
    }

    #[test]
    fn device_json_is_understood() {
        let json = r#"{
            "deviceURL": "io://1234-5678-9012/1111111",
            "label": "Volet salon",
            "controllableName": "io:RollerShutterGenericIOComponent",
            "definition": {"uiClass": "RollerShutter",
                           "commands": [{"commandName": "open", "nparams": 0}]},
            "states": [{"name": "core:ClosureState", "type": 1, "value": 30}],
            "available": true, "enabled": true, "oid": "abc"
        }"#;
        let device: Device = serde_json::from_str(json).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(device.definition.ui_class, "RollerShutter");
        assert_eq!(device.states[0].value, 30);
        assert_eq!(device.definition.commands[0].name, "open");
    }
}
