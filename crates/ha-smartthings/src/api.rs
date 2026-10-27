//! L'API de SmartThings (`api.smartthings.com`) : l'association OAuth, ses
//! jetons, la liste des appareils, leur état et leurs commandes.
//!
//! Tout passe en HTTPS, sans suivre de redirection, réponses limitées à
//! [`MAX_BYTES`]. Les jetons ne sont jamais écrits dans les journaux ni
//! dans les messages d'erreur.

use std::time::{Duration, SystemTime};

use base64::Engine;
use ha_storage::SmartThingsTokens;
use reqwest::StatusCode;
use serde_json::{Value, json};
use zeroize::Zeroizing;

use crate::SmartThingsError;

/// L'adresse de l'API.
pub const API: &str = "https://api.smartthings.com";
/// Ce que la maison demande : lire les appareils et les commander, lire
/// les pièces (pour les noms).
pub const SCOPES: &str = "r:devices:* x:devices:* r:locations:*";
/// Une réponse fait au plus cette taille.
const MAX_BYTES: usize = 4 << 20;
/// Pages de la liste des appareils, au plus.
const MAX_PAGES: usize = 20;
/// Appareils, au plus.
pub const MAX_DEVICES: usize = 300;

/// Où joindre SmartThings : le vrai service, ou un faux sur cette machine
/// pour les essais (`HA_SMARTTHINGS_API=http://127.0.0.1:…`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    base: String,
}

impl Endpoint {
    #[must_use]
    pub fn smartthings() -> Self {
        Self { base: API.into() }
    }

    /// Un faux SmartThings sur cette machine seulement.
    #[must_use]
    pub fn local(base: &str) -> Option<Self> {
        let rest = base.strip_prefix("http://127.0.0.1:")?;
        rest.trim_end_matches('/')
            .parse::<u16>()
            .ok()
            .map(|_| Self {
                base: base.trim_end_matches('/').to_owned(),
            })
    }

    /// Le client HTTP qui va avec : HTTPS seulement, sauf pour le faux local.
    pub fn http(&self) -> Result<reqwest::Client, SmartThingsError> {
        ha_tls::web_client(self.base.starts_with("https://"))
            .map_err(|error| SmartThingsError::Unexpected(error.to_string()))
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }
}

/// Un identifiant SmartThings (appareil, lieu, pièce) : un UUID. Vérifié
/// avant d'entrer dans une adresse.
#[must_use]
pub fn is_id(id: &str) -> bool {
    (1..=64).contains(&id.len()) && id.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
}

/// L'adresse où envoyer la personne pour autoriser la maison.
#[must_use]
pub fn authorize_url(
    endpoint: &Endpoint,
    client_id: &str,
    redirect_uri: &str,
    state: &str,
) -> String {
    let mut url = url::Url::parse(&endpoint.url("/v1/oauth/authorize")).unwrap_or_else(|_| {
        url::Url::parse(API).unwrap_or_else(|_| unreachable!("adresse fixe valide"))
    });
    url.query_pairs_mut()
        .append_pair("client_id", client_id)
        .append_pair("response_type", "code")
        .append_pair("redirect_uri", redirect_uri)
        .append_pair("scope", SCOPES)
        .append_pair("state", state);
    url.into()
}

/// Ce que contient l'adresse de retour : `?code=…&state=…`. Accepte aussi
/// le code seul.
pub fn read_callback(text: &str, state: &str) -> Result<String, String> {
    let text = text.trim();
    if !text.contains('?') && !text.contains('=') {
        return if text.is_empty() || text.len() > 512 || !text.chars().all(|c| c.is_ascii_graphic())
        {
            Err("code vide ou invalide".into())
        } else {
            Ok(text.to_owned())
        };
    }
    let query = text.split_once('?').map_or(text, |(_, query)| query);
    let pairs: Vec<(String, String)> = url::form_urlencoded::parse(query.as_bytes())
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    let get = |key: &str| {
        pairs
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    };
    if let Some(error) = get("error") {
        return Err(format!("SmartThings a refusé : {error}"));
    }
    if get("state") != Some(state) {
        return Err("cette adresse ne vient pas de cette demande (state différent)".into());
    }
    match get("code") {
        Some(code) if !code.is_empty() && code.len() <= 512 => Ok(code.to_owned()),
        _ => Err("pas de code dans l'adresse".into()),
    }
}

/// L'application SmartThings de la maison : de quoi demander des jetons.
#[derive(Clone)]
pub struct App {
    pub client_id: String,
    pub client_secret: Zeroizing<String>,
    pub redirect_uri: String,
}

impl App {
    fn basic(&self) -> String {
        let raw = Zeroizing::new(format!(
            "{}:{}",
            self.client_id,
            self.client_secret.as_str()
        ));
        format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode(raw.as_bytes())
        )
    }

    /// Échange le code reçu au retour contre les premiers jetons.
    pub async fn exchange(
        &self,
        http: &reqwest::Client,
        endpoint: &Endpoint,
        code: &str,
    ) -> Result<SmartThingsTokens, SmartThingsError> {
        let body = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("grant_type", "authorization_code")
            .append_pair("code", code)
            .append_pair("client_id", &self.client_id)
            .append_pair("redirect_uri", &self.redirect_uri)
            .finish();
        self.token(http, endpoint, Zeroizing::new(body)).await
    }

    /// Renouvelle les jetons. L'ancien jeton de renouvellement ne sert plus
    /// ensuite : les nouveaux doivent être enregistrés aussitôt.
    pub async fn refresh(
        &self,
        http: &reqwest::Client,
        endpoint: &Endpoint,
        refresh_token: &str,
    ) -> Result<SmartThingsTokens, SmartThingsError> {
        let body = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("grant_type", "refresh_token")
            .append_pair("refresh_token", refresh_token)
            .append_pair("client_id", &self.client_id)
            .finish();
        self.token(http, endpoint, Zeroizing::new(body)).await
    }

    async fn token(
        &self,
        http: &reqwest::Client,
        endpoint: &Endpoint,
        body: Zeroizing<String>,
    ) -> Result<SmartThingsTokens, SmartThingsError> {
        let response = http
            .post(endpoint.url("/v1/oauth/token"))
            .header("Authorization", self.basic())
            .header("Content-Type", "application/x-www-form-urlencoded")
            .header("Accept", "application/json")
            .body(body.to_string())
            .send()
            .await
            .map_err(|_| SmartThingsError::Unreachable)?;
        let status = response.status();
        if matches!(
            status,
            StatusCode::BAD_REQUEST | StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
        ) {
            return Err(SmartThingsError::Unauthorized);
        }
        let value = read_json(response).await?;
        let text = |key: &str| {
            value
                .get(key)
                .and_then(Value::as_str)
                .filter(|t| !t.is_empty() && t.len() <= 4096)
                .map(|t| Zeroizing::new(t.to_owned()))
                .ok_or_else(|| SmartThingsError::Unexpected(format!("{key} manquant")))
        };
        let lifetime = value
            .get("expires_in")
            .and_then(Value::as_u64)
            .unwrap_or(86_399)
            .clamp(60, 30 * 86_400);
        Ok(SmartThingsTokens {
            access_token: text("access_token")?,
            refresh_token: text("refresh_token")?,
            expires_at: SystemTime::now() + Duration::from_secs(lifetime),
        })
    }
}

/// Un appareil, tel que SmartThings le décrit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    pub id: String,
    pub label: String,
    pub location_id: Option<String>,
    pub room_id: Option<String>,
    /// Les capacités du composant principal (`switch`, `windowShade`…).
    pub capabilities: Vec<String>,
    /// Ses catégories (`Light`, `Blind`, `Television`…).
    pub categories: Vec<String>,
}

impl Device {
    fn parse(value: &Value) -> Option<Self> {
        let id = value
            .get("deviceId")?
            .as_str()
            .filter(|id| is_id(id))?
            .to_owned();
        let label = ["label", "name"]
            .into_iter()
            .find_map(|key| {
                value
                    .get(key)
                    .and_then(Value::as_str)
                    .filter(|t| !t.trim().is_empty())
            })
            .unwrap_or("SmartThings")
            .chars()
            .filter(|c| !c.is_control())
            .take(100)
            .collect::<String>()
            .trim()
            .to_owned();
        let main = value
            .get("components")?
            .as_array()?
            .iter()
            .find(|c| c.get("id").and_then(Value::as_str) == Some("main"))?;
        let names = |key: &str, field: &str| -> Vec<String> {
            main.get(key)
                .and_then(Value::as_array)
                .map(|list| {
                    list.iter()
                        .filter_map(|item| item.get(field).and_then(Value::as_str))
                        .filter(|name| name.len() <= 100)
                        .take(100)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default()
        };
        let id_of = |key: &str| {
            value
                .get(key)
                .and_then(Value::as_str)
                .filter(|id| is_id(id))
                .map(str::to_owned)
        };
        Some(Self {
            id,
            label,
            location_id: id_of("locationId"),
            room_id: id_of("roomId"),
            capabilities: names("capabilities", "id"),
            categories: names("categories", "name"),
        })
    }
}

/// Une commande à un appareil (`switch` / `on`).
#[derive(Debug, Clone, PartialEq)]
pub struct Command {
    pub capability: &'static str,
    pub command: &'static str,
    pub arguments: Vec<Value>,
}

/// Les appels à l'API, avec un jeton d'accès.
#[derive(Clone)]
pub struct Api {
    http: reqwest::Client,
    endpoint: Endpoint,
    access: Zeroizing<String>,
}

impl Api {
    #[must_use]
    pub fn new(http: reqwest::Client, endpoint: Endpoint, access: Zeroizing<String>) -> Self {
        Self {
            http,
            endpoint,
            access,
        }
    }

    async fn send(&self, request: reqwest::RequestBuilder) -> Result<Value, SmartThingsError> {
        let response = request
            .bearer_auth(self.access.as_str())
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(|_| SmartThingsError::Unreachable)?;
        match response.status() {
            StatusCode::UNAUTHORIZED => Err(SmartThingsError::Unauthorized),
            StatusCode::TOO_MANY_REQUESTS => {
                let wait = response
                    .headers()
                    .get("x-ratelimit-reset")
                    .or_else(|| response.headers().get("retry-after"))
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.parse::<u64>().ok())
                    // `x-ratelimit-reset` est en millisecondes.
                    .map_or(60, |v| if v > 1000 { v / 1000 } else { v })
                    .clamp(1, 900);
                Err(SmartThingsError::RateLimited(wait))
            }
            status if !status.is_success() => Err(SmartThingsError::Status(status.as_u16())),
            _ => read_json(response).await,
        }
    }

    /// Tous les appareils, page après page.
    pub async fn devices(&self) -> Result<Vec<Device>, SmartThingsError> {
        let mut devices = Vec::new();
        let mut next = Some(self.endpoint.url("/v1/devices"));
        for _ in 0..MAX_PAGES {
            let Some(url) = next.take() else { break };
            let page = self.send(self.http.get(&url)).await?;
            let items = page.get("items").and_then(Value::as_array).ok_or_else(|| {
                SmartThingsError::Unexpected("liste des appareils illisible".into())
            })?;
            devices.extend(items.iter().filter_map(Device::parse));
            if devices.len() >= MAX_DEVICES {
                devices.truncate(MAX_DEVICES);
                break;
            }
            // La page suivante, seulement sur la même adresse.
            next = page
                .pointer("/_links/next/href")
                .and_then(Value::as_str)
                .filter(|href| href.starts_with(&format!("{}/", self.endpoint.base)))
                .map(str::to_owned);
        }
        Ok(devices)
    }

    /// L'état du composant principal d'un appareil :
    /// `{"switch": {"switch": {"value": "on"}}, …}`.
    pub async fn status(&self, device_id: &str) -> Result<Value, SmartThingsError> {
        if !is_id(device_id) {
            return Err(SmartThingsError::Unexpected("identifiant invalide".into()));
        }
        let url = self
            .endpoint
            .url(&format!("/v1/devices/{device_id}/components/main/status"));
        self.send(self.http.get(url)).await
    }

    /// Les noms des pièces d'un lieu : identifiant → nom.
    pub async fn rooms(
        &self,
        location_id: &str,
    ) -> Result<Vec<(String, String)>, SmartThingsError> {
        if !is_id(location_id) {
            return Err(SmartThingsError::Unexpected("identifiant invalide".into()));
        }
        let url = self
            .endpoint
            .url(&format!("/v1/locations/{location_id}/rooms"));
        let page = self.send(self.http.get(url)).await?;
        Ok(page
            .get("items")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|room| {
                        let id = room.get("roomId")?.as_str().filter(|id| is_id(id))?;
                        let name = room.get("name")?.as_str()?;
                        Some((
                            id.to_owned(),
                            name.chars().filter(|c| !c.is_control()).take(100).collect(),
                        ))
                    })
                    .take(200)
                    .collect()
            })
            .unwrap_or_default())
    }

    /// Envoie des commandes à un appareil.
    pub async fn command(
        &self,
        device_id: &str,
        commands: &[Command],
    ) -> Result<(), SmartThingsError> {
        if !is_id(device_id) {
            return Err(SmartThingsError::Unexpected("identifiant invalide".into()));
        }
        let body = json!({
            "commands": commands.iter().map(|c| json!({
                "component": "main",
                "capability": c.capability,
                "command": c.command,
                "arguments": c.arguments,
            })).collect::<Vec<_>>(),
        });
        let url = self
            .endpoint
            .url(&format!("/v1/devices/{device_id}/commands"));
        self.send(
            self.http
                .post(url)
                .header("Content-Type", "application/json")
                .body(body.to_string()),
        )
        .await
        .map(|_| ())
    }
}

/// Lit une réponse JSON, [`MAX_BYTES`] au plus.
async fn read_json(response: reqwest::Response) -> Result<Value, SmartThingsError> {
    let body = ha_tls::read_limited(response, MAX_BYTES)
        .await
        .map_err(|error| match error {
            ha_tls::ReadError::Http(_) => SmartThingsError::Unreachable,
            ha_tls::ReadError::TooLarge => SmartThingsError::Unexpected(error.to_string()),
        })?;
    if body.is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_slice(&body).map_err(|_| SmartThingsError::Unexpected("JSON illisible".into()))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]

    use super::*;

    #[test]
    fn the_authorize_url_asks_for_the_right_things() {
        let url = authorize_url(
            &Endpoint::smartthings(),
            "client-1",
            "https://192.168.1.10:8123/smartthings",
            "abc",
        );
        assert!(url.starts_with("https://api.smartthings.com/v1/oauth/authorize?client_id=client-1&response_type=code&redirect_uri=https%3A%2F%2F192.168.1.10%3A8123%2Fsmartthings&scope=r%3Adevices%3A*+x%3Adevices%3A*+r%3Alocations%3A*&state=abc"), "{url}");
    }

    #[test]
    fn the_callback_gives_the_code_if_it_is_ours() {
        let page = "https://192.168.1.10:8123/smartthings?code=XyZ12&state=abc";
        assert_eq!(read_callback(page, "abc").unwrap(), "XyZ12");
        assert!(read_callback(page, "autre").unwrap_err().contains("state"));
        assert_eq!(read_callback("  XyZ12 ", "abc").unwrap(), "XyZ12");
        assert!(
            read_callback("https://x/?error=access_denied&state=abc", "abc")
                .unwrap_err()
                .contains("refusé")
        );
        assert!(read_callback("https://x/?state=abc", "abc").is_err());
    }

    #[test]
    fn local_endpoints_are_on_this_machine_only() {
        assert!(Endpoint::local("http://127.0.0.1:18400").is_some());
        assert!(Endpoint::local("http://192.168.1.2:18400").is_none());
        assert!(Endpoint::local("https://api.smartthings.com").is_none());
        assert!(is_id("2d3f2c1a-0b5e-4c2a-9d1e-6f7a8b9c0d1e"));
        assert!(!is_id("../locations"));
    }

    #[test]
    fn a_device_is_read_from_its_main_component() {
        let device = Device::parse(&json!({
            "deviceId": "2d3f2c1a-0b5e-4c2a-9d1e-6f7a8b9c0d1e",
            "name": "c2c-switch",
            "label": "Lampe salon",
            "locationId": "aaaa-bbbb",
            "roomId": "cccc",
            "components": [
                {"id": "other", "capabilities": [{"id": "battery"}]},
                {"id": "main", "capabilities": [{"id": "switch"}, {"id": "switchLevel"}],
                 "categories": [{"name": "Light", "categoryType": "manufacturer"}]}
            ]
        }))
        .unwrap();
        assert_eq!(device.label, "Lampe salon");
        assert_eq!(device.capabilities, ["switch", "switchLevel"]);
        assert_eq!(device.categories, ["Light"]);
        assert_eq!(device.room_id.as_deref(), Some("cccc"));
        assert!(Device::parse(&json!({"deviceId": "../x", "components": []})).is_none());
    }
}
