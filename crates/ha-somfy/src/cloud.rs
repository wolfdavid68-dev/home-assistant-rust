//! Le serveur Somfy (le « cloud », comme l'appli TaHoma).
//!
//! Deux usages :
//! - l'obtention du jeton de l'API locale, **une seule fois**, à
//!   l'association (procédure officielle du « mode développeur ») ;
//! - le chauffage Atlantic (voir [`crate::heating`]) : la box ne le montre
//!   pas à son API locale, seul le serveur Somfy le connaît.
//!
//! Sécurité :
//! - la connexion au serveur Somfy est vérifiée comme dans un navigateur
//!   (autorités de confiance du système) ;
//! - à l'association, le mot de passe n'est **jamais enregistré** : il sert à
//!   cette connexion, puis il est effacé de la mémoire. Pour le chauffage, il
//!   est lu dans un fichier hors du dépôt (`HA_SOMFY_PASSWORD_FILE`) ;
//! - le jeton de session n'apparaît jamais dans les journaux.
//!
//! Connexion : comme l'appli TaHoma, on demande un jeton au serveur des
//! comptes Somfy (`accounts.somfy.com`, OAuth2 « password »), puis on l'envoie
//! à chaque requête (`Authorization: Bearer …`). L'ancienne connexion
//! (`/login` avec `userId` et `userPassword`) est refusée par Somfy.

use std::fmt::Write as _;

use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use zeroize::Zeroizing;

use crate::api::{Device, Event, REQUEST_TIMEOUT, is_safe_path_part, read_json, read_limited};
use crate::error::SomfyError;

/// Un serveur Somfy : celui des comptes (la connexion) et celui de l'API.
#[derive(Debug, Clone, Copy)]
pub struct SomfyServer<'a> {
    /// Le serveur des comptes, qui donne le jeton.
    pub accounts: &'a str,
    /// L'API des appareils.
    pub api: &'a str,
}

/// Serveur Somfy pour l'Europe.
pub const SOMFY_EUROPE: SomfyServer<'static> = SomfyServer {
    accounts: "https://accounts.somfy.com",
    api: "https://ha101-1.overkiz.com/enduser-mobile-web/enduserAPI",
};

/// L'identifiant de l'appli TaHoma auprès du serveur des comptes Somfy.
/// Ce n'est pas un secret personnel : il est le même pour tout le monde, et
/// publié (bibliothèque `pyoverkiz`, utilisée par Home Assistant).
const CLIENT_ID: &str =
    "0d8e920c-1478-11e7-a377-02dd59bd3041_1ewvaqmclfogo4kcsoo0c8k4kso884owg08sg8c40sk4go4ksg";
const CLIENT_SECRET: &str = "12k73w1n540g8o4cokg0cw84cog840k84cwggscwg884004kgk";

/// Une session ouverte sur le serveur Somfy.
#[derive(Clone)]
pub struct CloudSession {
    http: reqwest::Client,
    base: String,
    /// `Bearer <jeton>`, prêt pour l'en-tête `Authorization`.
    authorization: Zeroizing<String>,
}

/// Jamais le jeton de session dans un affichage de débogage.
impl std::fmt::Debug for CloudSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CloudSession")
            .field("base", &self.base)
            .finish_non_exhaustive()
    }
}

impl CloudSession {
    /// Se connecte avec le compte Somfy (e-mail et mot de passe).
    pub async fn login(
        http: reqwest::Client,
        server: SomfyServer<'_>,
        email: &str,
        password: &str,
    ) -> Result<Self, SomfyError> {
        let form = Zeroizing::new(format!(
            "grant_type=password&client_id={}&client_secret={}&username={}&password={}",
            form_encode(CLIENT_ID),
            form_encode(CLIENT_SECRET),
            form_encode(email),
            form_encode(password)
        ));
        let response = http
            .post(format!("{}/oauth/oauth/v2/token/jwt", server.accounts))
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(form.as_str().to_owned())
            .timeout(REQUEST_TIMEOUT)
            .send()
            .await?;
        let status = response.status();
        // La réponse contient le jeton : elle n'est jamais recopiée dans une erreur.
        let body = Zeroizing::new(read_limited(response).await?);
        let reply: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
        let refused = reply.get("message").and_then(Value::as_str) == Some("error.invalid.grant")
            || reply.get("error").and_then(Value::as_str) == Some("invalid_grant")
            || status == reqwest::StatusCode::UNAUTHORIZED;
        if refused {
            return Err(SomfyError::BadCredentials);
        }
        if !status.is_success() {
            return Err(SomfyError::Unexpected(format!(
                "serveur des comptes Somfy : {status}"
            )));
        }
        let token = reply
            .get("access_token")
            .and_then(Value::as_str)
            .filter(|t| is_header_token(t))
            .ok_or_else(|| SomfyError::Unexpected("pas de jeton Somfy dans la réponse".into()))?;
        Ok(Self {
            http,
            base: server.api.to_owned(),
            authorization: Zeroizing::new(format!("Bearer {token}")),
        })
    }

    /// Les box TaHoma du compte (leur « PIN », par exemple `1234-5678-9012`).
    pub async fn gateways(&self) -> Result<Vec<String>, SomfyError> {
        let reply: Value = self.get("/setup/gateways").await?;
        let ids = reply
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|gateway| gateway.get("gatewayId").and_then(Value::as_str))
            .filter(|id| is_gateway_id(id))
            .map(str::to_owned)
            .collect();
        Ok(ids)
    }

    /// Crée un jeton pour l'API locale de la box et l'active.
    pub async fn create_local_token(
        &self,
        gateway_id: &str,
    ) -> Result<Zeroizing<String>, SomfyError> {
        if !is_gateway_id(gateway_id) {
            return Err(SomfyError::Unexpected(format!(
                "PIN de box invalide : {gateway_id:?}"
            )));
        }
        let reply: Value = self
            .get(&format!("/config/{gateway_id}/local/tokens/generate"))
            .await?;
        let token = reply
            .get("token")
            .and_then(Value::as_str)
            .filter(|t| {
                (10..=200).contains(&t.len()) && t.bytes().all(|b| b.is_ascii_alphanumeric())
            })
            .map(|t| Zeroizing::new(t.to_owned()))
            .ok_or_else(|| SomfyError::Unexpected("jeton généré invalide".into()))?;

        let body = json!({"label": "ha-rust", "token": token.as_str(), "scope": "devmode"});
        let response = self
            .http
            .post(format!("{}/config/{gateway_id}/local/tokens", self.base))
            .header(reqwest::header::AUTHORIZATION, self.authorization.as_str())
            .header("Content-Type", "application/json")
            .body(body.to_string())
            .send()
            .await?;
        let _: Value = read_json(response).await?;
        Ok(token)
    }

    /// Tous les appareils du compte, avec leurs états.
    pub async fn devices(&self) -> Result<Vec<Device>, SomfyError> {
        self.get("/setup/devices").await
    }

    /// Envoie des commandes à un appareil, en un seul ordre, par exemple
    /// `setDerogatedTargetTemperature` avec `[20]`.
    pub async fn execute(
        &self,
        device_url: &str,
        commands: &[(String, Vec<Value>)],
    ) -> Result<(), SomfyError> {
        let commands: Vec<Value> = commands
            .iter()
            .map(|(name, parameters)| json!({"name": name, "parameters": parameters}))
            .collect();
        let body = json!({
            "label": "ha-rust",
            "actions": [{"deviceURL": device_url, "commands": commands}],
        });
        let _: Value = self.post("/exec/apply", &body).await?;
        Ok(())
    }

    /// Ouvre un « écouteur » d'événements, à relever avec
    /// [`CloudSession::fetch_events`] (sinon le serveur l'oublie).
    pub async fn register_listener(&self) -> Result<String, SomfyError> {
        let reply: Value = self.post("/events/register", &json!({})).await?;
        reply
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| is_safe_path_part(id))
            .map(str::to_owned)
            .ok_or_else(|| SomfyError::Unexpected("identifiant d'écouteur invalide".into()))
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

    /// Ferme la session sur le serveur Somfy.
    pub async fn logout(self) {
        let _ = self
            .http
            .post(format!("{}/logout", self.base))
            .header(reqwest::header::AUTHORIZATION, self.authorization.as_str())
            .timeout(REQUEST_TIMEOUT)
            .send()
            .await;
    }

    async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T, SomfyError> {
        let response = self
            .http
            .get(format!("{}{path}", self.base))
            .header(reqwest::header::AUTHORIZATION, self.authorization.as_str())
            .timeout(REQUEST_TIMEOUT)
            .send()
            .await?;
        read_json(response).await
    }

    async fn post<T: DeserializeOwned>(&self, path: &str, body: &Value) -> Result<T, SomfyError> {
        let response = self
            .http
            .post(format!("{}{path}", self.base))
            .header(reqwest::header::AUTHORIZATION, self.authorization.as_str())
            .header("Content-Type", "application/json")
            .body(body.to_string())
            .timeout(REQUEST_TIMEOUT)
            .send()
            .await?;
        read_json(response).await
    }
}

/// PIN d'une box : `1234-5678-9012` (chiffres et tirets).
#[must_use]
pub fn is_gateway_id(value: &str) -> bool {
    (4..=40).contains(&value.len()) && value.bytes().all(|b| b.is_ascii_digit() || b == b'-')
}

/// Un jeton envoyable tel quel dans un en-tête : caractères visibles, sans
/// espace, de taille raisonnable.
fn is_header_token(value: &str) -> bool {
    (10..=8192).contains(&value.len()) && value.bytes().all(|b| b.is_ascii_graphic())
}

/// Encodage d'un champ de formulaire (`application/x-www-form-urlencoded`).
fn form_encode(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len() * 3);
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.~".contains(&byte) {
            encoded.push(char::from(byte));
        } else {
            // Écrire dans une `String` ne peut pas échouer.
            let _ = write!(encoded, "%{byte:02X}");
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn form_encoding() {
        assert_eq!(form_encode("moi@exemple.fr"), "moi%40exemple.fr");
        assert_eq!(form_encode("a&b=c d"), "a%26b%3Dc%20d");
        assert_eq!(form_encode("é"), "%C3%A9");
    }

    #[test]
    fn header_tokens() {
        assert!(is_header_token("eyJhbGciOi.eyJzdWIiOi.c2lnbmF0dXJl"));
        assert!(!is_header_token("court"));
        assert!(!is_header_token("avec espace dans le jeton"));
        assert!(!is_header_token(
            "retour
X-Autre: 1"
        ));
    }

    #[test]
    fn gateway_ids() {
        assert!(is_gateway_id("1234-5678-9012"));
        assert!(!is_gateway_id("1234/../setup"));
        assert!(!is_gateway_id(""));
    }
}
