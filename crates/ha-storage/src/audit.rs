//! Journal d'audit : qui a fait quoi, quand, et d'où.
//!
//! Chaque ligne est **signée** (HMAC-SHA256, avec une clé dérivée de
//! `ha.key`) et contient la signature de la ligne précédente : c'est une
//! chaîne. Quelqu'un qui modifie la base sans la clé ne peut ni changer une
//! ligne, ni en supprimer une au milieu, ni en insérer une fausse, sans que
//! [`Store::verify_audit`] le détecte.
//!
//! Limite honnête : supprimer les **dernières** lignes ne casse pas la
//! chaîne. On voit seulement qu'il en manque si on se souvient du numéro de
//! la dernière.

use std::time::SystemTime;

use crate::error::StorageError;
use crate::store::{Store, from_micros, to_micros};

/// Nombre maximal de lignes renvoyées par [`Store::audit_log`].
pub const MAX_AUDIT_LIMIT: u32 = 1_000;
/// Longueur maximale de chaque champ (en caractères).
const MAX_FIELD_CHARS: usize = 200;
/// Signature « avant la première ligne ».
const GENESIS: [u8; 32] = [0; 32];

/// Une action à enregistrer.
#[derive(Debug, Clone, Copy)]
pub struct AuditRecord<'a> {
    pub actor: &'a str,
    pub action: &'a str,
    pub target: Option<&'a str>,
    pub detail: Option<&'a str>,
    pub source: &'a str,
}

/// Une ligne relue depuis le journal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditEntry {
    pub id: i64,
    pub at: SystemTime,
    pub actor: String,
    pub action: String,
    pub target: Option<String>,
    pub detail: Option<String>,
    pub source: String,
}

/// Résultat de la vérification du journal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuditCheck {
    /// Toutes les lignes sont intactes et forment une chaîne continue.
    Intact { entries: u64 },
    /// La ligne `id` a été modifiée, ou la chaîne est rompue juste avant elle.
    Broken { id: i64, reason: &'static str },
}

type AuditRow = (
    i64,
    i64,
    String,
    String,
    Option<String>,
    Option<String>,
    String,
    Vec<u8>,
    Vec<u8>,
);

/// Nettoie un champ : longueur limitée, et aucun caractère de contrôle.
///
/// Sécurité : un nom d'utilisateur tapé par un attaquant finit dans le
/// journal. Sans ce nettoyage, il pourrait contenir des séquences
/// d'échappement qui modifient l'affichage du terminal de celui qui lit le
/// journal (effacer des lignes, changer les couleurs…).
#[must_use]
pub fn clean_field(value: &str) -> String {
    value
        .chars()
        .take(MAX_FIELD_CHARS)
        .map(|c| if c.is_control() { '\u{fffd}' } else { c })
        .collect()
}

fn mac_fields<'a>(
    id: &'a [u8; 8],
    at: &'a [u8; 8],
    record: &'a CleanRecord,
    prev_mac: &'a [u8],
) -> [&'a [u8]; 8] {
    [
        id,
        at,
        record.actor.as_bytes(),
        record.action.as_bytes(),
        record.target.as_deref().unwrap_or("\u{0}").as_bytes(),
        record.detail.as_deref().unwrap_or("\u{0}").as_bytes(),
        record.source.as_bytes(),
        prev_mac,
    ]
}

struct CleanRecord {
    actor: String,
    action: String,
    target: Option<String>,
    detail: Option<String>,
    source: String,
}

impl Store {
    /// Ajoute une ligne au journal.
    pub async fn audit(&self, record: AuditRecord<'_>) -> Result<(), StorageError> {
        let record = CleanRecord {
            actor: clean_field(record.actor),
            action: clean_field(record.action),
            target: record.target.map(clean_field),
            detail: record.detail.map(clean_field),
            source: clean_field(record.source),
        };
        let at = to_micros(SystemTime::now())?;

        // `BEGIN IMMEDIATE` réserve l'écriture dès le début : deux écritures
        // simultanées (le serveur et une commande `ha-server user …`, par
        // exemple) ne peuvent pas lire la même « dernière ligne » et
        // casser la chaîne. La seconde attend que la première ait fini.
        let mut tx = self.pool().begin_with("BEGIN IMMEDIATE").await?;
        let last: Option<(i64, Vec<u8>)> =
            sqlx::query_as("SELECT id, mac FROM audit_log ORDER BY id DESC LIMIT 1")
                .fetch_optional(&mut *tx)
                .await?;
        let (id, prev_mac) = match last {
            Some((id, mac)) => (id + 1, mac),
            None => (1, GENESIS.to_vec()),
        };
        let mac = self.secrets().audit_mac(&mac_fields(
            &id.to_be_bytes(),
            &at.to_be_bytes(),
            &record,
            &prev_mac,
        ));
        sqlx::query(
            "INSERT INTO audit_log
                 (id, at, actor, action, target, detail, source, prev_mac, mac)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        )
        .bind(id)
        .bind(at)
        .bind(&record.actor)
        .bind(&record.action)
        .bind(&record.target)
        .bind(&record.detail)
        .bind(&record.source)
        .bind(&prev_mac)
        .bind(mac.as_slice())
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Les `limit` dernières lignes du journal, de la plus récente à la plus
    /// ancienne.
    pub async fn audit_log(&self, limit: u32) -> Result<Vec<AuditEntry>, StorageError> {
        let rows: Vec<AuditRow> = sqlx::query_as(
            "SELECT id, at, actor, action, target, detail, source, prev_mac, mac
             FROM audit_log ORDER BY id DESC LIMIT ?1",
        )
        .bind(limit.min(MAX_AUDIT_LIMIT))
        .fetch_all(self.pool())
        .await?;
        rows.into_iter()
            .map(|(id, at, actor, action, target, detail, source, _, _)| {
                Ok(AuditEntry {
                    id,
                    at: from_micros(at)?,
                    actor,
                    action,
                    target,
                    detail,
                    source,
                })
            })
            .collect()
    }

    /// Supprime les lignes plus anciennes que `older_than`. Renvoie leur nombre.
    ///
    /// La chaîne reste vérifiable : chaque ligne garde la signature de la
    /// précédente, même quand celle-ci a été supprimée.
    pub async fn purge_audit(&self, older_than: SystemTime) -> Result<u64, StorageError> {
        let deleted = sqlx::query("DELETE FROM audit_log WHERE at < ?1")
            .bind(to_micros(older_than)?)
            .execute(self.pool())
            .await?
            .rows_affected();
        Ok(deleted)
    }

    /// Vérifie tout le journal : signatures, et continuité de la chaîne.
    pub async fn verify_audit(&self) -> Result<AuditCheck, StorageError> {
        let rows: Vec<AuditRow> = sqlx::query_as(
            "SELECT id, at, actor, action, target, detail, source, prev_mac, mac
             FROM audit_log ORDER BY id",
        )
        .fetch_all(self.pool())
        .await?;
        let mut previous: Option<(i64, Vec<u8>)> = None;
        let mut entries = 0;
        for (id, at, actor, action, target, detail, source, prev_mac, mac) in rows {
            if let Some((previous_id, previous_mac)) = &previous {
                if id != previous_id + 1 {
                    return Ok(AuditCheck::Broken {
                        id,
                        reason: "des lignes ont été supprimées juste avant",
                    });
                }
                if &prev_mac != previous_mac {
                    return Ok(AuditCheck::Broken {
                        id,
                        reason: "la chaîne est rompue (ligne précédente remplacée)",
                    });
                }
            }
            let record = CleanRecord {
                actor,
                action,
                target,
                detail,
                source,
            };
            let expected = self.secrets().audit_mac(&mac_fields(
                &id.to_be_bytes(),
                &at.to_be_bytes(),
                &record,
                &prev_mac,
            ));
            if expected.as_slice() != mac.as_slice() {
                return Ok(AuditCheck::Broken {
                    id,
                    reason: "ligne modifiée (signature invalide)",
                });
            }
            previous = Some((id, mac));
            entries += 1;
        }
        Ok(AuditCheck::Intact { entries })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_characters_are_neutralised() {
        assert_eq!(clean_field("david"), "david");
        assert_eq!(clean_field("a\u{1b}[2Jb\n"), "a\u{fffd}[2Jb\u{fffd}");
        assert_eq!(
            clean_field(&"x".repeat(500)).chars().count(),
            MAX_FIELD_CHARS
        );
    }
}
