//! Association avec le pont : obtention de la clé d'application et
//! mémorisation dans la base.

use std::time::{Duration, Instant, SystemTime};

use ha_storage::{Store, to_micros};
use zeroize::Zeroizing;

use crate::api;
use crate::error::HueError;
use crate::tls::{self, Fingerprint};

/// Temps laissé pour appuyer sur le bouton du pont.
pub const LINK_BUTTON_TIMEOUT: Duration = Duration::from_secs(60);

/// Ce qu'il faut savoir pour se reconnecter au pont.
pub struct Pairing {
    pub host: String,
    pub bridge_id: String,
    pub fingerprint: Fingerprint,
    pub app_key: Zeroizing<String>,
}

impl std::fmt::Debug for Pairing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pairing")
            .field("host", &self.host)
            .field("bridge_id", &self.bridge_id)
            .field("fingerprint", &tls::format_fingerprint(&self.fingerprint))
            .field("app_key", &"<masqué>")
            .finish()
    }
}

/// Vérifie l'adresse donnée : une adresse IPv4 ou un nom d'hôte simple,
/// éventuellement suivi de `:port`.
///
/// Sécurité : elle finit dans une URL (`https://<hôte>/…`) ; on refuse donc
/// tout caractère qui pourrait en changer le sens (`/`, `@`, `?`…).
pub fn check_host(host: &str) -> Result<(), HueError> {
    let (name, port) = match host.split_once(':') {
        Some((name, port)) => (name, Some(port)),
        None => (host, None),
    };
    let valid = !name.is_empty()
        && name.len() <= 253
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
        && port.is_none_or(|port| port.parse::<u16>().is_ok_and(|p| p > 0));
    if valid {
        Ok(())
    } else {
        Err(HueError::InvalidHost(host.to_owned()))
    }
}

/// Première étape : se connecter au pont, noter son certificat et son
/// identifiant, pour les montrer à l'utilisateur avant d'aller plus loin.
pub async fn identify(host: &str) -> Result<(String, Fingerprint), HueError> {
    check_host(host)?;
    let (http, verifier) = tls::client(None)?;
    let bridge_id = api::fetch_bridge_id(&http, host).await?;
    let fingerprint = verifier
        .seen()
        .ok_or_else(|| HueError::Bridge("aucun certificat reçu".into()))?;
    Ok((bridge_id, fingerprint))
}

/// Deuxième étape : attendre l'appui sur le bouton du pont et récupérer la
/// clé. La connexion n'accepte que le certificat vu à la première étape.
pub async fn request_key(
    host: &str,
    bridge_id: String,
    fingerprint: Fingerprint,
) -> Result<Pairing, HueError> {
    let (http, _) = tls::client(Some(fingerprint))?;
    let start = Instant::now();
    loop {
        if let Some(app_key) = api::request_app_key(&http, host).await? {
            return Ok(Pairing {
                host: host.to_owned(),
                bridge_id,
                fingerprint,
                app_key,
            });
        }
        if start.elapsed() > LINK_BUTTON_TIMEOUT {
            return Err(HueError::LinkButtonNotPressed);
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

/// Enregistre l'association (en remplaçant l'éventuelle précédente).
pub async fn save(store: &Store, pairing: &Pairing) -> Result<(), HueError> {
    sqlx::query(
        "INSERT INTO hue_bridge (id, host, bridge_id, cert_sha256, app_key, paired_at)
         VALUES (1, ?1, ?2, ?3, ?4, ?5)
         ON CONFLICT (id) DO UPDATE SET
             host = excluded.host, bridge_id = excluded.bridge_id,
             cert_sha256 = excluded.cert_sha256, app_key = excluded.app_key,
             paired_at = excluded.paired_at",
    )
    .bind(&pairing.host)
    .bind(&pairing.bridge_id)
    .bind(pairing.fingerprint.as_slice())
    .bind(pairing.app_key.as_str())
    .bind(to_micros(SystemTime::now())?)
    .execute(store.pool())
    .await?;
    Ok(())
}

/// Relit l'association, s'il y en a une.
pub async fn load(store: &Store) -> Result<Option<Pairing>, HueError> {
    let row: Option<(String, String, Vec<u8>, String)> =
        sqlx::query_as("SELECT host, bridge_id, cert_sha256, app_key FROM hue_bridge WHERE id = 1")
            .fetch_optional(store.pool())
            .await?;
    let Some((host, bridge_id, fingerprint, app_key)) = row else {
        return Ok(None);
    };
    // On ne fait pas confiance aveuglément au contenu du fichier.
    check_host(&host)?;
    let fingerprint: Fingerprint = fingerprint
        .try_into()
        .map_err(|_| HueError::Bridge("empreinte enregistrée invalide".into()))?;
    Ok(Some(Pairing {
        host,
        bridge_id,
        fingerprint,
        app_key: Zeroizing::new(app_key),
    }))
}

/// Oublie le pont. Renvoie `false` s'il n'y en avait pas.
///
/// Pense aussi à retirer « ha_rust » de la liste des applications autorisées
/// dans l'appli Hue, pour que la clé ne soit plus valable du tout.
pub async fn forget(store: &Store) -> Result<bool, HueError> {
    let deleted = sqlx::query("DELETE FROM hue_bridge WHERE id = 1")
        .execute(store.pool())
        .await?
        .rows_affected();
    Ok(deleted > 0)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn hosts() {
        for ok in [
            "192.168.1.20",
            "philips-hue.local",
            "pont",
            "127.0.0.1:8443",
        ] {
            assert!(check_host(ok).is_ok(), "{ok}");
        }
        for bad in [
            "",
            "192.168.1.20/api",
            "user@evil",
            "a:0",
            "a:http",
            "a:1:2",
            "a b",
            "é.local",
            "a?b",
        ] {
            assert!(check_host(bad).is_err(), "{bad}");
        }
    }
}
