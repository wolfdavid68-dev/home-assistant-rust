//! Association avec la box : vérification, et sauvegarde dans la base.

use std::time::SystemTime;

use ha_storage::{Store, is_sealed, to_micros};
use ha_tls::Fingerprint;
use tracing::info;
use zeroize::Zeroizing;

use crate::api::{self, LocalApi};
use crate::cloud::is_gateway_id;
use crate::error::SomfyError;

/// Usage du jeton, pour son chiffrement dans la base.
const TOKEN_CONTEXT: &str = "somfy_box.token";

/// Ce qu'il faut savoir pour se reconnecter à la box.
pub struct Pairing {
    pub host: String,
    pub gateway_id: String,
    pub fingerprint: Fingerprint,
    pub token: Zeroizing<String>,
}

impl std::fmt::Debug for Pairing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pairing")
            .field("host", &self.host)
            .field("gateway_id", &self.gateway_id)
            .field(
                "fingerprint",
                &ha_tls::format_fingerprint(&self.fingerprint),
            )
            .field("token", &"<masqué>")
            .finish()
    }
}

fn check_host(host: &str) -> Result<(), SomfyError> {
    if ha_tls::is_valid_host(host) {
        Ok(())
    } else {
        Err(SomfyError::InvalidHost(host.to_owned()))
    }
}

/// Première étape : se connecter à la box, vérifier que c'en est une, et
/// noter son certificat.
pub async fn identify(host: &str) -> Result<(String, Fingerprint), SomfyError> {
    check_host(host)?;
    let (http, verifier) = ha_tls::client(None)?;
    let version = api::api_version(&http, host).await?;
    let fingerprint = verifier
        .seen()
        .ok_or_else(|| SomfyError::Unexpected("aucun certificat reçu".into()))?;
    Ok((version, fingerprint))
}

/// Vérifie qu'un jeton fonctionne (avec le certificat épinglé), et renvoie
/// la liste des appareils.
pub async fn verify(pairing: &Pairing) -> Result<Vec<api::Device>, SomfyError> {
    let (http, _) = ha_tls::client(Some(pairing.fingerprint))?;
    LocalApi::new(http, &pairing.host, pairing.token.clone())
        .devices()
        .await
}

/// Enregistre l'association (en remplaçant l'éventuelle précédente).
pub async fn save(store: &Store, pairing: &Pairing) -> Result<(), SomfyError> {
    sqlx::query(
        "INSERT INTO somfy_box (id, host, gateway_id, cert_sha256, token, paired_at)
         VALUES (1, ?1, ?2, ?3, ?4, ?5)
         ON CONFLICT (id) DO UPDATE SET
             host = excluded.host, gateway_id = excluded.gateway_id,
             cert_sha256 = excluded.cert_sha256, token = excluded.token,
             paired_at = excluded.paired_at",
    )
    .bind(&pairing.host)
    .bind(&pairing.gateway_id)
    .bind(pairing.fingerprint.as_slice())
    .bind(store.seal(TOKEN_CONTEXT, &pairing.token)?)
    .bind(to_micros(SystemTime::now())?)
    .execute(store.pool())
    .await?;
    Ok(())
}

/// Relit l'association, s'il y en a une.
pub async fn load(store: &Store) -> Result<Option<Pairing>, SomfyError> {
    let row: Option<(String, String, Vec<u8>, String)> =
        sqlx::query_as("SELECT host, gateway_id, cert_sha256, token FROM somfy_box WHERE id = 1")
            .fetch_optional(store.pool())
            .await?;
    let Some((host, gateway_id, fingerprint, token)) = row else {
        return Ok(None);
    };
    // On ne fait pas confiance aveuglément au contenu du fichier.
    check_host(&host)?;
    if !is_gateway_id(&gateway_id) {
        return Err(SomfyError::Unexpected(
            "PIN de box enregistré invalide".into(),
        ));
    }
    let fingerprint: Fingerprint = fingerprint
        .try_into()
        .map_err(|_| SomfyError::Unexpected("empreinte enregistrée invalide".into()))?;
    let token = Zeroizing::new(token);
    let was_sealed = is_sealed(&token);
    let pairing = Pairing {
        host,
        gateway_id,
        fingerprint,
        token: if was_sealed {
            store.unseal(TOKEN_CONTEXT, &token)?
        } else {
            token
        },
    };
    if !was_sealed {
        // Association faite avant le chiffrement : on la chiffre maintenant.
        save(store, &pairing).await?;
        info!("jeton de la box Somfy chiffré dans la base");
    }
    Ok(Some(pairing))
}

/// Oublie la box. Renvoie `false` s'il n'y en avait pas.
pub async fn forget(store: &Store) -> Result<bool, SomfyError> {
    let deleted = sqlx::query("DELETE FROM somfy_box WHERE id = 1")
        .execute(store.pool())
        .await?
        .rows_affected();
    Ok(deleted > 0)
}
