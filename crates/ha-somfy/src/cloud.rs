//! Obtention du jeton de l'API locale, **une seule fois**, à l'association.
//!
//! Somfy ne permet de créer ce jeton qu'en passant par son serveur, avec ton
//! compte Somfy (procédure officielle du « mode développeur »). Ensuite, tout
//! se passe en local : le serveur Somfy n'est plus jamais contacté.
//!
//! Sécurité :
//! - la connexion au serveur Somfy est vérifiée comme dans un navigateur
//!   (autorités de confiance du système) ;
//! - le mot de passe n'est **jamais enregistré** : il sert à cette connexion,
//!   puis il est effacé de la mémoire ;
//! - on se déconnecte du serveur Somfy à la fin.

use std::fmt::Write as _;

use serde_json::{Value, json};
use zeroize::Zeroizing;

use crate::api::read_json;
use crate::error::SomfyError;

/// Serveur Somfy pour l'Europe.
pub const SOMFY_EUROPE: &str = "https://ha101-1.overkiz.com/enduser-mobile-web/enduserAPI";

/// Une session ouverte sur le serveur Somfy.
pub struct CloudSession {
    http: reqwest::Client,
    base: String,
    cookie: Zeroizing<String>,
}

impl CloudSession {
    /// Se connecte avec le compte Somfy (e-mail et mot de passe).
    pub async fn login(
        http: reqwest::Client,
        base: &str,
        email: &str,
        password: &str,
    ) -> Result<Self, SomfyError> {
        let form = Zeroizing::new(format!(
            "userId={}&userPassword={}",
            form_encode(email),
            form_encode(password)
        ));
        let response = http
            .post(format!("{base}/login"))
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(form.as_str().to_owned())
            .send()
            .await?;
        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Err(SomfyError::BadCredentials);
        }
        // La session est un cookie `JSESSIONID`.
        let cookie = response
            .headers()
            .get_all(reqwest::header::SET_COOKIE)
            .iter()
            .filter_map(|value| value.to_str().ok())
            .find_map(|value| {
                value
                    .split(';')
                    .next()
                    .filter(|c| c.starts_with("JSESSIONID="))
            })
            .map(str::to_owned);
        let reply: Value = read_json(response).await?;
        let Some(cookie) = cookie.filter(|_| reply.get("success") == Some(&json!(true))) else {
            return Err(SomfyError::BadCredentials);
        };
        Ok(Self {
            http,
            base: base.to_owned(),
            cookie: Zeroizing::new(cookie),
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
            .header(reqwest::header::COOKIE, self.cookie.as_str())
            .header("Content-Type", "application/json")
            .body(body.to_string())
            .send()
            .await?;
        let _: Value = read_json(response).await?;
        Ok(token)
    }

    /// Ferme la session sur le serveur Somfy.
    pub async fn logout(self) {
        let _ = self
            .http
            .post(format!("{}/logout", self.base))
            .header(reqwest::header::COOKIE, self.cookie.as_str())
            .send()
            .await;
    }

    async fn get(&self, path: &str) -> Result<Value, SomfyError> {
        let response = self
            .http
            .get(format!("{}{path}", self.base))
            .header(reqwest::header::COOKIE, self.cookie.as_str())
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
    fn gateway_ids() {
        assert!(is_gateway_id("1234-5678-9012"));
        assert!(!is_gateway_id("1234/../setup"));
        assert!(!is_gateway_id(""));
    }
}
