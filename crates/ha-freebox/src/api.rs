//! L'API locale de Freebox OS, en HTTPS.
//!
//! Fonctionnement :
//! 1. **Autorisation** (une seule fois) : on demande un jeton d'application,
//!    que tu acceptes sur l'écran de la Freebox ;
//! 2. **Session** : à chaque connexion, la Freebox envoie un « défi » (un
//!    texte aléatoire). On répond `HMAC-SHA1(jeton, défi)` : la Freebox sait
//!    qu'on a le jeton, sans que le jeton circule sur le réseau ;
//! 3. **Requêtes** : avec l'en-tête `X-Fbx-App-Auth: <jeton de session>`.
//!
//! On n'utilise que des routes de **lecture** : liste des appareils du réseau
//! et état de la connexion Internet.

use std::fmt::Write as _;
use std::time::Duration;

use futures_util::StreamExt;
use hmac::{Hmac, KeyInit, Mac};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use sha1::Sha1;
use tokio::sync::Mutex;
use zeroize::Zeroizing;

use crate::error::FreeboxError;

/// Adresse de la Freebox sur le réseau local.
pub const DEFAULT_HOST: &str = "mafreebox.freebox.fr";
/// Identifiant de notre application auprès de la Freebox.
pub const APP_ID: &str = "fr.ha-rust";
const API_PATH: &str = "/api/v6";
const MAX_RESPONSE: usize = 4 * 1024 * 1024;
/// Nombre maximal d'appareils lus sur le réseau.
pub const MAX_HOSTS: usize = 1_000;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

/// Réponse de `/api_version` : qui est cette Freebox ?
#[derive(Debug, Clone, Deserialize)]
pub struct ApiVersion {
    #[serde(default)]
    pub device_name: String,
    #[serde(default)]
    pub api_version: String,
    #[serde(default)]
    pub device_type: String,
}

/// Enveloppe de toutes les réponses : `{"success": …, "result": …}`.
#[derive(Debug, Deserialize)]
struct Envelope<T> {
    success: bool,
    result: Option<T>,
    #[serde(default)]
    error_code: Option<String>,
    #[serde(default)]
    msg: Option<String>,
}

/// Un appareil vu sur le réseau local.
#[derive(Debug, Clone, Deserialize)]
pub struct LanHost {
    #[serde(default)]
    pub primary_name: String,
    #[serde(default)]
    pub host_type: String,
    #[serde(default)]
    pub vendor_name: String,
    #[serde(default)]
    pub l2ident: Option<L2Ident>,
    /// La Freebox a vu de l'activité récemment.
    #[serde(default)]
    pub active: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct L2Ident {
    #[serde(default)]
    pub id: String,
    #[serde(default, rename = "type")]
    pub kind: String,
}

impl LanHost {
    /// L'adresse MAC de l'appareil, normalisée, s'il en a une valide.
    #[must_use]
    pub fn mac(&self) -> Option<String> {
        self.l2ident
            .as_ref()
            .filter(|ident| ident.kind == "mac_address")
            .and_then(|ident| crate::presence::normalize_mac(&ident.id))
    }
}

/// État de la connexion Internet.
#[derive(Debug, Clone, Deserialize)]
pub struct ConnectionStatus {
    /// `up`, `down`, `going_up`, `going_down`.
    #[serde(default)]
    pub state: String,
    /// `ftth`, `xdsl`…
    #[serde(default)]
    pub media: String,
    /// Débit instantané, en octets par seconde.
    #[serde(default)]
    pub rate_down: u64,
    #[serde(default)]
    pub rate_up: u64,
}

#[derive(Debug, Deserialize)]
struct Interface {
    name: String,
}

#[derive(Debug, Deserialize)]
struct Authorization {
    app_token: String,
    track_id: u64,
}

#[derive(Debug, Deserialize)]
struct AuthorizationStatus {
    status: String,
}

#[derive(Debug, Deserialize)]
struct Challenge {
    challenge: String,
}

#[derive(Debug, Deserialize)]
struct SessionReply {
    session_token: String,
    #[serde(default)]
    permissions: serde_json::Map<String, Value>,
}

#[must_use]
pub fn api_url(host: &str) -> String {
    format!("https://{host}{API_PATH}")
}

/// Vérifie qu'on parle bien à une Freebox (route publique `/api_version`).
pub async fn api_version(http: &reqwest::Client, host: &str) -> Result<ApiVersion, FreeboxError> {
    let response = http
        .get(format!("https://{host}/api_version"))
        .timeout(REQUEST_TIMEOUT)
        .send()
        .await?;
    let body = read_body(response).await?;
    let version: ApiVersion = serde_json::from_slice(&body)
        .map_err(|_| FreeboxError::Unexpected("ce n'est pas une Freebox".into()))?;
    if version.api_version.is_empty() {
        return Err(FreeboxError::Unexpected("ce n'est pas une Freebox".into()));
    }
    Ok(version)
}

/// Demande un jeton d'application. Il faudra ensuite l'accepter sur l'écran
/// de la Freebox ([`authorization_status`]).
pub async fn authorize(
    http: &reqwest::Client,
    host: &str,
    device_name: &str,
) -> Result<(Zeroizing<String>, u64), FreeboxError> {
    let body = json!({
        "app_id": APP_ID,
        "app_name": "ha-rust (domotique)",
        "app_version": env!("CARGO_PKG_VERSION"),
        "device_name": device_name,
    });
    let response = http
        .post(format!("{}/login/authorize/", api_url(host)))
        .header("Content-Type", "application/json")
        .body(body.to_string())
        .timeout(REQUEST_TIMEOUT)
        .send()
        .await?;
    let reply: Authorization = read_envelope(response).await?;
    let token = Zeroizing::new(reply.app_token);
    if token.is_empty() || token.len() > 200 {
        return Err(FreeboxError::Unexpected(
            "jeton d'application invalide".into(),
        ));
    }
    Ok((token, reply.track_id))
}

/// Où en est l'autorisation : `pending`, `granted`, `denied`, `timeout`…
pub async fn authorization_status(
    http: &reqwest::Client,
    host: &str,
    track_id: u64,
) -> Result<String, FreeboxError> {
    let response = http
        .get(format!("{}/login/authorize/{track_id}", api_url(host)))
        .timeout(REQUEST_TIMEOUT)
        .send()
        .await?;
    let reply: AuthorizationStatus = read_envelope(response).await?;
    Ok(reply.status)
}

/// Réponse au défi : `HMAC-SHA1(jeton, défi)`, en hexadécimal.
#[must_use]
pub fn challenge_password(app_token: &str, challenge: &str) -> String {
    let Ok(mut mac) = <Hmac<Sha1> as KeyInit>::new_from_slice(app_token.as_bytes()) else {
        // HMAC accepte des clés de toute taille : ce cas n'arrive pas.
        return String::new();
    };
    mac.update(challenge.as_bytes());
    let digest = mac.finalize().into_bytes();
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        // Écrire dans une `String` ne peut pas échouer.
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}

/// Une session ouverte avec la Freebox.
pub struct Session {
    http: reqwest::Client,
    base: String,
    app_token: Zeroizing<String>,
    current: Mutex<Option<Zeroizing<String>>>,
}

/// Jamais de jeton dans un affichage de débogage.
impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("base", &self.base)
            .finish_non_exhaustive()
    }
}

impl Session {
    #[must_use]
    pub fn new(http: reqwest::Client, host: &str, app_token: Zeroizing<String>) -> Self {
        Self {
            http,
            base: api_url(host),
            app_token,
            current: Mutex::new(None),
        }
    }

    /// Ouvre une session et renvoie les droits accordés à l'application.
    pub async fn open(&self) -> Result<Vec<String>, FreeboxError> {
        let response = self
            .http
            .get(format!("{}/login/", self.base))
            .timeout(REQUEST_TIMEOUT)
            .send()
            .await?;
        let Challenge { challenge } = read_envelope(response).await?;
        let body = json!({
            "app_id": APP_ID,
            "password": challenge_password(&self.app_token, &challenge),
        });
        let response = self
            .http
            .post(format!("{}/login/session/", self.base))
            .header("Content-Type", "application/json")
            .body(body.to_string())
            .timeout(REQUEST_TIMEOUT)
            .send()
            .await?;
        let reply: SessionReply = read_envelope(response).await?;
        let permissions = reply
            .permissions
            .iter()
            .filter(|(_, granted)| granted.as_bool() == Some(true))
            .map(|(name, _)| ha_storage::clean_field(name))
            .collect();
        *self.current.lock().await = Some(Zeroizing::new(reply.session_token));
        Ok(permissions)
    }

    /// Tous les appareils du réseau local, toutes interfaces confondues.
    pub async fn hosts(&self) -> Result<Vec<LanHost>, FreeboxError> {
        let interfaces: Vec<Interface> = match self.get("/lan/browser/interfaces/").await {
            Ok(interfaces) => interfaces,
            // Freebox en mode « bridge » : pas de réseau local à lire.
            Err(FreeboxError::Api(code)) if code.starts_with("nodev") => return Ok(Vec::new()),
            Err(error) => return Err(error),
        };
        let mut hosts = Vec::new();
        for interface in interfaces {
            if !is_safe_path_part(&interface.name) {
                continue;
            }
            let list: Option<Vec<LanHost>> = self
                .get_optional(&format!("/lan/browser/{}/", interface.name))
                .await?;
            hosts.extend(list.unwrap_or_default());
            if hosts.len() > MAX_HOSTS {
                return Err(FreeboxError::TooLarge);
            }
        }
        Ok(hosts)
    }

    /// État de la connexion Internet.
    pub async fn connection(&self) -> Result<ConnectionStatus, FreeboxError> {
        self.get("/connection/").await
    }

    async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T, FreeboxError> {
        self.get_optional(path)
            .await?
            .ok_or_else(|| FreeboxError::Unexpected("réponse vide".into()))
    }

    /// GET authentifié. Si la session a expiré, on en rouvre une et on
    /// réessaie une fois.
    async fn get_optional<T: DeserializeOwned>(
        &self,
        path: &str,
    ) -> Result<Option<T>, FreeboxError> {
        if self.current.lock().await.is_none() {
            self.open().await?;
        }
        match self.get_once(path).await {
            Err(FreeboxError::Unauthorized) => {
                self.open().await?;
                self.get_once(path).await
            }
            other => other,
        }
    }

    async fn get_once<T: DeserializeOwned>(&self, path: &str) -> Result<Option<T>, FreeboxError> {
        let token = self
            .current
            .lock()
            .await
            .clone()
            .ok_or(FreeboxError::Unauthorized)?;
        let response = self
            .http
            .get(format!("{}{path}", self.base))
            .header("X-Fbx-App-Auth", token.as_str())
            .timeout(REQUEST_TIMEOUT)
            .send()
            .await?;
        read_envelope_optional(response).await
    }
}

async fn read_body(response: reqwest::Response) -> Result<Vec<u8>, FreeboxError> {
    let mut body = Vec::new();
    let mut chunks = response.bytes_stream();
    while let Some(chunk) = chunks.next().await {
        let chunk = chunk?;
        if body.len() + chunk.len() > MAX_RESPONSE {
            return Err(FreeboxError::TooLarge);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

async fn read_envelope<T: DeserializeOwned>(
    response: reqwest::Response,
) -> Result<T, FreeboxError> {
    read_envelope_optional(response)
        .await?
        .ok_or_else(|| FreeboxError::Unexpected("réponse vide".into()))
}

/// Lit `{"success": …, "result": …}` et traduit les codes d'erreur.
async fn read_envelope_optional<T: DeserializeOwned>(
    response: reqwest::Response,
) -> Result<Option<T>, FreeboxError> {
    let body = read_body(response).await?;
    let envelope: Envelope<T> = serde_json::from_slice(&body)
        .map_err(|error| FreeboxError::Unexpected(format!("JSON : {error}")))?;
    if envelope.success {
        return Ok(envelope.result);
    }
    let code = envelope.error_code.unwrap_or_default();
    Err(match code.as_str() {
        "auth_required" | "invalid_session" | "invalid_token" => FreeboxError::Unauthorized,
        "insufficient_rights" => FreeboxError::InsufficientRights,
        _ => FreeboxError::Api(ha_storage::clean_field(&format!(
            "{code} {}",
            envelope.msg.unwrap_or_default()
        ))),
    })
}

/// Un morceau d'adresse sûr (nom d'interface…) : lettres, chiffres, `-`, `_`.
fn is_safe_path_part(value: &str) -> bool {
    (1..=32).contains(&value.len())
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn challenge_answer_is_hmac_sha1_hex() {
        // Valeur de référence calculée indépendamment (RFC 2202, cas n° 2).
        assert_eq!(
            challenge_password("Jefe", "what do ya want for nothing?"),
            "effcdf6ae5eb2fa2d27416d5f184df9c259a7c79"
        );
    }

    #[test]
    fn host_json_is_understood() {
        let host: LanHost = serde_json::from_str(
            r#"{"id": "ether-aa:bb:cc:dd:ee:ff", "primary_name": "Téléphone de David",
                "host_type": "smartphone", "vendor_name": "Apple",
                "l2ident": {"id": "aa:bb:cc:dd:ee:ff", "type": "mac_address"},
                "active": true, "reachable": true, "last_activity": 1700000000,
                "names": [], "l3connectivities": []}"#,
        )
        .unwrap();
        assert_eq!(host.mac().as_deref(), Some("AA:BB:CC:DD:EE:FF"));
        assert!(host.active);
    }

    #[test]
    fn path_parts() {
        assert!(is_safe_path_part("pub"));
        assert!(is_safe_path_part("wifiguest"));
        assert!(!is_safe_path_part("../connection"));
        assert!(!is_safe_path_part(""));
    }
}
