//! Association avec la Freebox, appareils suivis et appareils connus.

use std::collections::HashSet;
use std::time::{Duration, Instant, SystemTime};

use ha_storage::{Store, is_sealed, to_micros};
use ha_tls::Fingerprint;
use tracing::info;
use zeroize::Zeroizing;

use crate::api::{self, ApiVersion, Session};
use crate::error::FreeboxError;
use crate::presence::{Tracked, normalize_mac};

/// Usage du jeton, pour son chiffrement dans la base.
const TOKEN_CONTEXT: &str = "freebox_box.app_token";
/// Temps laissé pour valider sur l'écran de la Freebox.
pub const AUTHORIZATION_TIMEOUT: Duration = Duration::from_mins(2);
/// Nombre maximal d'appareils suivis.
const MAX_TRACKED: usize = 50;

/// Ce qu'il faut savoir pour se reconnecter à la Freebox.
#[derive(Clone)]
pub struct Pairing {
    pub host: String,
    pub device_name: String,
    pub fingerprint: Fingerprint,
    pub app_token: Zeroizing<String>,
}

impl std::fmt::Debug for Pairing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pairing")
            .field("host", &self.host)
            .field("device_name", &self.device_name)
            .field(
                "fingerprint",
                &ha_tls::format_fingerprint(&self.fingerprint),
            )
            .field("app_token", &"<masqué>")
            .finish()
    }
}

fn check_host(host: &str) -> Result<(), FreeboxError> {
    if ha_tls::is_valid_host(host) {
        Ok(())
    } else {
        Err(FreeboxError::InvalidHost(host.to_owned()))
    }
}

/// Première étape : se connecter, vérifier que c'est une Freebox, et noter
/// son certificat.
pub async fn identify(host: &str) -> Result<(ApiVersion, Fingerprint), FreeboxError> {
    check_host(host)?;
    let (http, verifier) = ha_tls::client(None)?;
    let version = api::api_version(&http, host).await?;
    let fingerprint = verifier
        .seen()
        .ok_or_else(|| FreeboxError::Unexpected("aucun certificat reçu".into()))?;
    Ok((version, fingerprint))
}

/// Deuxième étape : demander un jeton, et attendre que tu l'acceptes sur
/// l'écran de la Freebox. Tout passe par le certificat épinglé.
pub async fn request_token(
    host: &str,
    device_name: &str,
    fingerprint: Fingerprint,
    timeout: Duration,
) -> Result<Pairing, FreeboxError> {
    check_host(host)?;
    let (http, _) = ha_tls::client(Some(fingerprint))?;
    let (app_token, track_id) = api::authorize(&http, host, "ha-rust").await?;
    let deadline = Instant::now() + timeout;
    loop {
        match api::authorization_status(&http, host, track_id)
            .await?
            .as_str()
        {
            "granted" => break,
            "denied" => return Err(FreeboxError::Denied),
            "timeout" => return Err(FreeboxError::Timeout),
            _ if Instant::now() >= deadline => return Err(FreeboxError::Timeout),
            _ => tokio::time::sleep(Duration::from_secs(1)).await,
        }
    }
    Ok(Pairing {
        host: host.to_owned(),
        device_name: ha_storage::clean_field(device_name),
        fingerprint,
        app_token,
    })
}

/// Ouvre une session avec le certificat épinglé (vérifie que le jeton
/// fonctionne). Renvoie la session et les droits accordés.
pub async fn connect(pairing: &Pairing) -> Result<(Session, Vec<String>), FreeboxError> {
    let (http, _) = ha_tls::client(Some(pairing.fingerprint))?;
    let session = Session::new(http, &pairing.host, pairing.app_token.clone());
    let permissions = session.open().await?;
    Ok((session, permissions))
}

/// Enregistre l'association (en remplaçant l'éventuelle précédente).
pub async fn save(store: &Store, pairing: &Pairing) -> Result<(), FreeboxError> {
    sqlx::query(
        "INSERT INTO freebox_box (id, host, device_name, cert_sha256, app_token, paired_at)
         VALUES (1, ?1, ?2, ?3, ?4, ?5)
         ON CONFLICT (id) DO UPDATE SET
             host = excluded.host, device_name = excluded.device_name,
             cert_sha256 = excluded.cert_sha256, app_token = excluded.app_token,
             paired_at = excluded.paired_at",
    )
    .bind(&pairing.host)
    .bind(&pairing.device_name)
    .bind(pairing.fingerprint.as_slice())
    .bind(store.seal(TOKEN_CONTEXT, &pairing.app_token)?)
    .bind(to_micros(SystemTime::now())?)
    .execute(store.pool())
    .await?;
    Ok(())
}

/// Relit l'association, s'il y en a une.
pub async fn load(store: &Store) -> Result<Option<Pairing>, FreeboxError> {
    let row: Option<(String, String, Vec<u8>, String)> = sqlx::query_as(
        "SELECT host, device_name, cert_sha256, app_token FROM freebox_box WHERE id = 1",
    )
    .fetch_optional(store.pool())
    .await?;
    let Some((host, device_name, fingerprint, app_token)) = row else {
        return Ok(None);
    };
    check_host(&host)?;
    let fingerprint: Fingerprint = fingerprint
        .try_into()
        .map_err(|_| FreeboxError::Unexpected("empreinte enregistrée invalide".into()))?;
    if !is_sealed(&app_token) {
        return Err(FreeboxError::Unexpected(
            "jeton enregistré non chiffré".into(),
        ));
    }
    Ok(Some(Pairing {
        host,
        device_name,
        fingerprint,
        app_token: store.unseal(TOKEN_CONTEXT, &app_token)?,
    }))
}

/// Oublie la Freebox (et les appareils suivis). Renvoie `false` s'il n'y
/// avait pas d'association.
pub async fn forget(store: &Store) -> Result<bool, FreeboxError> {
    let mut tx = store.pool().begin().await?;
    let deleted = sqlx::query("DELETE FROM freebox_box WHERE id = 1")
        .execute(&mut *tx)
        .await?
        .rows_affected();
    sqlx::query("DELETE FROM freebox_tracked")
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM freebox_known")
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(deleted > 0)
}

/// Suit la présence de l'appareil `mac`, sous le nom `name`
/// (entité `device_tracker.<name>`).
pub async fn track(store: &Store, mac: &str, name: &str) -> Result<Tracked, FreeboxError> {
    let mac = normalize_mac(mac).ok_or_else(|| FreeboxError::InvalidMac(mac.to_owned()))?;
    if !ha_core::is_valid_name(name) {
        return Err(FreeboxError::InvalidName(name.to_owned()));
    }
    if tracked(store).await?.len() >= MAX_TRACKED {
        return Err(FreeboxError::Unexpected(format!(
            "{MAX_TRACKED} appareils suivis au plus"
        )));
    }
    sqlx::query(
        "INSERT INTO freebox_tracked (mac, name, added_at) VALUES (?1, ?2, ?3)
         ON CONFLICT (mac) DO UPDATE SET name = excluded.name",
    )
    .bind(&mac)
    .bind(name)
    .bind(to_micros(SystemTime::now())?)
    .execute(store.pool())
    .await
    .map_err(|error| match error {
        sqlx::Error::Database(db) if db.is_unique_violation() => {
            FreeboxError::InvalidName(format!("{name} (déjà utilisé)"))
        }
        other => other.into(),
    })?;
    Ok(Tracked {
        mac,
        name: name.to_owned(),
    })
}

/// Arrête de suivre un appareil (par son nom ou son adresse MAC).
pub async fn untrack(store: &Store, name_or_mac: &str) -> Result<bool, FreeboxError> {
    let mac = normalize_mac(name_or_mac).unwrap_or_default();
    let deleted = sqlx::query("DELETE FROM freebox_tracked WHERE name = ?1 OR mac = ?2")
        .bind(name_or_mac)
        .bind(mac)
        .execute(store.pool())
        .await?
        .rows_affected();
    Ok(deleted > 0)
}

/// Les appareils suivis.
pub async fn tracked(store: &Store) -> Result<Vec<Tracked>, FreeboxError> {
    let rows: Vec<(String, String)> =
        sqlx::query_as("SELECT mac, name FROM freebox_tracked ORDER BY name")
            .fetch_all(store.pool())
            .await?;
    // On revérifie ce qu'on relit.
    Ok(rows
        .into_iter()
        .filter(|(mac, name)| normalize_mac(mac).is_some() && ha_core::is_valid_name(name))
        .map(|(mac, name)| Tracked { mac, name })
        .collect())
}

/// Les adresses MAC déjà vues sur le réseau.
pub async fn known(store: &Store) -> Result<HashSet<String>, FreeboxError> {
    let rows: Vec<(String,)> = sqlx::query_as("SELECT mac FROM freebox_known")
        .fetch_all(store.pool())
        .await?;
    Ok(rows.into_iter().map(|(mac,)| mac).collect())
}

/// Mémorise des appareils vus : (MAC, nom).
pub async fn remember(store: &Store, devices: &[(String, String)]) -> Result<(), FreeboxError> {
    if devices.is_empty() {
        return Ok(());
    }
    let now = to_micros(SystemTime::now())?;
    let mut tx = store.pool().begin().await?;
    for (mac, name) in devices {
        sqlx::query(
            "INSERT INTO freebox_known (mac, name, first_seen) VALUES (?1, ?2, ?3)
             ON CONFLICT (mac) DO NOTHING",
        )
        .bind(mac)
        .bind(name)
        .bind(now)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    info!("Freebox : {} appareil(s) mémorisé(s)", devices.len());
    Ok(())
}
