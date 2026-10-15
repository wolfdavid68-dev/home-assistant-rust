//! Sauvegardes de la base : création **sans arrêter le serveur**,
//! vérification, rotation et restauration.
//!
//! La copie utilise `VACUUM INTO` de SQLite : une copie **cohérente** de la
//! base (comme si on l'avait figée un instant), même pendant que le serveur
//! écrit, et compactée au passage.

use std::path::{Path, PathBuf};
use std::str::FromStr;

use sqlx::sqlite::SqliteConnectOptions;
use sqlx::{ConnectOptions, Connection};

use crate::error::StorageError;
use crate::store::Store;

const PREFIX: &str = "ha-";
const SUFFIX: &str = ".db";

/// Une sauvegarde trouvée dans le dossier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupFile {
    pub path: PathBuf,
    /// `ha-2026-10-15_0330.db`
    pub name: String,
    pub size: u64,
}

/// Le nom du fichier pour une date : `ha-<stamp>.db`. `stamp` ne contient que
/// des chiffres, `-` et `_` (par exemple `2026-10-15_0330`), pour que les
/// noms se trient dans l'ordre chronologique et ne puissent rien contenir
/// d'autre (ni `/`, ni `..`).
pub fn backup_name(stamp: &str) -> Result<String, StorageError> {
    let valid = !stamp.is_empty()
        && stamp.len() <= 32
        && stamp
            .bytes()
            .all(|b| b.is_ascii_digit() || b == b'-' || b == b'_');
    if !valid {
        return Err(StorageError::Backup(format!(
            "horodatage invalide {stamp:?}"
        )));
    }
    Ok(format!("{PREFIX}{stamp}{SUFFIX}"))
}

fn is_backup_name(name: &str) -> bool {
    name.strip_prefix(PREFIX)
        .and_then(|rest| rest.strip_suffix(SUFFIX))
        .is_some_and(|stamp| backup_name(stamp).is_ok())
}

impl Store {
    /// Copie la base dans `dir`, sous le nom `ha-<stamp>.db`, puis vérifie la
    /// copie. Renvoie la sauvegarde créée.
    pub async fn backup_into(&self, dir: &Path, stamp: &str) -> Result<BackupFile, StorageError> {
        let name = backup_name(stamp)?;
        create_private_dir(dir)?;
        let path = dir.join(&name);
        let target = path
            .to_str()
            .ok_or_else(|| StorageError::Backup("chemin du dossier illisible".into()))?
            .to_owned();
        // Le fichier est créé vide, réservé à son propriétaire (droits 600) :
        // une sauvegarde contient tout l'historique de la maison.
        create_empty_private_file(&path)?;
        // Concept SQL : même le chemin passe en **paramètre** (`?1`), jamais
        // collé dans la requête : aucun nom de fichier ne peut la détourner.
        let copied = sqlx::query("VACUUM INTO ?1")
            .bind(&target)
            .execute(self.pool())
            .await;
        if let Err(error) = copied {
            let _ = std::fs::remove_file(&path);
            return Err(error.into());
        }
        if let Err(error) = verify(&path).await {
            let _ = std::fs::remove_file(&path);
            return Err(error);
        }
        let size = std::fs::metadata(&path)?.len();
        Ok(BackupFile { path, name, size })
    }
}

/// Vérifie qu'un fichier est une base saine, et bien une base de ce serveur.
pub async fn verify(path: &Path) -> Result<(), StorageError> {
    let mut connection = SqliteConnectOptions::new()
        .filename(path)
        .read_only(true)
        .connect()
        .await?;
    let (check,): (String,) = sqlx::query_as("PRAGMA integrity_check")
        .fetch_one(&mut connection)
        .await?;
    // La table des migrations n'existe que dans une base de ce serveur.
    let ours: Result<(i64,), _> = sqlx::query_as("SELECT COUNT(*) FROM _sqlx_migrations")
        .fetch_one(&mut connection)
        .await;
    connection.close().await?;
    if check != "ok" {
        return Err(StorageError::Backup(format!("base abîmée : {check}")));
    }
    if ours.is_err() {
        return Err(StorageError::Backup(
            "ce fichier n'est pas une base de ha-server".into(),
        ));
    }
    Ok(())
}

/// Les sauvegardes du dossier, de la plus ancienne à la plus récente.
pub fn list(dir: &Path) -> Result<Vec<BackupFile>, StorageError> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    let mut backups = Vec::new();
    for entry in entries {
        let entry = entry?;
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        let metadata = entry.metadata()?;
        if metadata.is_file() && is_backup_name(&name) {
            backups.push(BackupFile {
                path: entry.path(),
                name,
                size: metadata.len(),
            });
        }
    }
    backups.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(backups)
}

/// Garde les `keep` sauvegardes les plus récentes et supprime les autres.
/// Seuls les fichiers nommés `ha-<date>.db` sont concernés : rien d'autre
/// dans le dossier n'est jamais touché. Renvoie les noms supprimés.
pub fn prune(dir: &Path, keep: usize) -> Result<Vec<String>, StorageError> {
    let backups = list(dir)?;
    let excess = backups.len().saturating_sub(keep.max(1));
    let mut removed = Vec::new();
    for backup in backups.into_iter().take(excess) {
        std::fs::remove_file(&backup.path)?;
        removed.push(backup.name);
    }
    Ok(removed)
}

/// Remplace la base `db` par la sauvegarde `backup`. **Le serveur doit être
/// arrêté.**
///
/// Avant de toucher à quoi que ce soit, la sauvegarde est vérifiée, et la
/// base actuelle est mise de côté dans `safety_dir` sous le nom
/// `avant-restauration-<stamp>.db` : une restauration ratée se défait. Ce
/// nom n'est jamais supprimé par la rotation.
pub async fn restore(
    backup: &Path,
    db: &Path,
    safety_dir: &Path,
    stamp: &str,
) -> Result<Option<PathBuf>, StorageError> {
    verify(backup).await?;
    let safety = if db.exists() {
        let name = format!(
            "avant-restauration-{}",
            backup_name(stamp)?.trim_start_matches(PREFIX)
        );
        create_private_dir(safety_dir)?;
        let path = safety_dir.join(name);
        create_empty_private_file(&path)?;
        let target = path
            .to_str()
            .ok_or_else(|| StorageError::Backup("chemin du dossier illisible".into()))?
            .to_owned();
        let mut connection = SqliteConnectOptions::from_str("sqlite:")?
            .filename(db)
            .connect()
            .await?;
        // Les dernières écritures (journal WAL) sont d'abord intégrées à la base.
        sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
            .execute(&mut connection)
            .await?;
        sqlx::query("VACUUM INTO ?1")
            .bind(&target)
            .execute(&mut connection)
            .await?;
        connection.close().await?;
        Some(path)
    } else {
        None
    };
    // Copie à côté, puis renommage : la base n'est jamais à moitié écrite.
    let temporary = db.with_extension("db.restauration");
    let _ = std::fs::remove_file(&temporary);
    std::fs::copy(backup, &temporary)?;
    restrict(&temporary)?;
    std::fs::rename(&temporary, db)?;
    for extension in ["db-wal", "db-shm"] {
        let _ = std::fs::remove_file(db.with_extension(extension));
    }
    Ok(safety)
}

fn create_private_dir(dir: &Path) -> Result<(), StorageError> {
    if dir.is_dir() {
        return Ok(());
    }
    std::fs::create_dir_all(dir)?;
    restrict_dir(dir)
}

fn create_empty_private_file(path: &Path) -> Result<(), StorageError> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path).map(|_| ()).map_err(|error| {
        if error.kind() == std::io::ErrorKind::AlreadyExists {
            StorageError::Backup(format!("{} existe déjà", path.display()))
        } else {
            error.into()
        }
    })
}

#[cfg(unix)]
fn restrict(path: &Path) -> Result<(), StorageError> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(())
}

#[cfg(not(unix))]
#[expect(clippy::unnecessary_wraps, reason = "même signature que sous Unix")]
fn restrict(_path: &Path) -> Result<(), StorageError> {
    Ok(())
}

#[cfg(unix)]
fn restrict_dir(dir: &Path) -> Result<(), StorageError> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    Ok(())
}

#[cfg(not(unix))]
#[expect(clippy::unnecessary_wraps, reason = "même signature que sous Unix")]
fn restrict_dir(_dir: &Path) -> Result<(), StorageError> {
    Ok(())
}
