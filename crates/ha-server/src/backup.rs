//! Sauvegardes automatiques de la base, chaque nuit, et commandes
//! `ha-server backup now | list | restore <fichier>`.
//!
//! Variables d'environnement :
//! - `HA_BACKUP_DIR` : dossier des sauvegardes (sans elle : pas de sauvegarde
//!   automatique ; l'image Docker la règle sur `/sauvegardes`) ;
//! - `HA_BACKUP_TIME` : heure de la sauvegarde (par défaut `03:30`) ;
//! - `HA_BACKUP_KEEP` : nombre de sauvegardes gardées (par défaut 7).

use std::error::Error;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use ha_core::{Attributes, Background, CancellationToken, Core, EntityId};
use ha_storage::{Store, backup};
use jiff::civil::Time;
use serde_json::{Value, json};
use tracing::{error, info, warn};

const DEFAULT_TIME: Time = Time::constant(3, 30, 0, 0);
const DEFAULT_KEEP: usize = 7;
/// Si la dernière sauvegarde a plus de 26 heures (machine éteinte la nuit),
/// on en fait une peu après le démarrage.
const OVERDUE: Duration = Duration::from_hours(26);
const CATCH_UP_DELAY: Duration = Duration::from_mins(2);
/// L'entité qui montre la dernière sauvegarde.
pub const ENTITY: &str = "sensor.derniere_sauvegarde";

/// Réglages des sauvegardes automatiques.
#[derive(Debug, Clone)]
pub struct Settings {
    pub dir: PathBuf,
    pub at: Time,
    pub keep: usize,
}

/// Lit `HA_BACKUP_DIR`, `HA_BACKUP_TIME` et `HA_BACKUP_KEEP`. `None` sans dossier.
pub fn settings() -> Result<Option<Settings>, String> {
    let Some(dir) = std::env::var_os("HA_BACKUP_DIR") else {
        return Ok(None);
    };
    let at = match std::env::var("HA_BACKUP_TIME") {
        Err(_) => DEFAULT_TIME,
        Ok(value) => parse_time(&value)
            .ok_or_else(|| format!("HA_BACKUP_TIME invalide : {value:?} (exemple : 03:30)"))?,
    };
    let keep = match std::env::var("HA_BACKUP_KEEP") {
        Err(_) => DEFAULT_KEEP,
        Ok(value) => value
            .trim()
            .parse::<usize>()
            .ok()
            .filter(|keep| (1..=365).contains(keep))
            .ok_or_else(|| format!("HA_BACKUP_KEEP invalide : {value:?} (entre 1 et 365)"))?,
    };
    Ok(Some(Settings {
        dir: dir.into(),
        at,
        keep,
    }))
}

fn parse_time(value: &str) -> Option<Time> {
    let (hours, minutes) = value.trim().split_once(':')?;
    Time::new(hours.parse().ok()?, minutes.parse().ok()?, 0, 0).ok()
}

/// Horodatage du nom de fichier : `2026-10-15_033000` (heure locale).
fn stamp() -> String {
    jiff::Zoned::now().strftime("%Y-%m-%d_%H%M%S").to_string()
}

/// La tâche des sauvegardes automatiques.
#[derive(Debug)]
pub struct Scheduler(Background);

impl Scheduler {
    pub fn start(settings: Settings, store: Store, core: Arc<Core>) -> Self {
        publish(&core, &settings.dir, None);
        Self(Background::spawn("les sauvegardes", move |token| {
            run(settings, store, core, token)
        }))
    }

    pub async fn stop(self) {
        self.0.stop().await;
    }
}

async fn run(settings: Settings, store: Store, core: Arc<Core>, token: CancellationToken) {
    info!(
        "sauvegardes : chaque jour à {}, les {} dernières gardées, dans {}",
        settings.at.strftime("%H:%M"),
        settings.keep,
        settings.dir.display()
    );
    let mut first = true;
    loop {
        let delay = if first && overdue(&settings.dir) {
            info!("sauvegardes : la dernière date de plus d'un jour, on en fait une bientôt");
            CATCH_UP_DELAY
        } else {
            until(settings.at)
        };
        first = false;
        tokio::select! {
            () = token.cancelled() => break,
            () = tokio::time::sleep(delay) => {}
        }
        take(&settings, &store, &core).await;
    }
}

/// La dernière sauvegarde est-elle trop ancienne (ou absente) ?
fn overdue(dir: &Path) -> bool {
    newest_time(dir).is_none_or(|at| at.elapsed().is_ok_and(|age| age > OVERDUE))
}

fn newest_time(dir: &Path) -> Option<SystemTime> {
    let newest = backup::list(dir).ok()?.pop()?;
    std::fs::metadata(newest.path).ok()?.modified().ok()
}

/// Temps jusqu'à la prochaine heure `at` (heure locale, changement d'heure compris).
fn until(at: Time) -> Duration {
    let now = jiff::Zoned::now();
    let today = now.date().to_datetime(at);
    let next = if today > now.datetime() {
        today
    } else {
        today
            .checked_add(jiff::Span::new().days(1))
            .unwrap_or(today)
    };
    next.to_zoned(now.time_zone().clone())
        .ok()
        .and_then(|next| now.duration_until(&next).try_into().ok())
        .unwrap_or(Duration::from_hours(24))
}

/// Une sauvegarde automatique : copie, vérification, rotation, et compte rendu.
async fn take(settings: &Settings, store: &Store, core: &Core) {
    match store.backup_into(&settings.dir, &stamp()).await {
        Ok(saved) => {
            info!("sauvegarde : {} ({} Kio)", saved.name, saved.size / 1024);
            crate::cli::audit_local(store, "serveur", "backup", Some(&saved.name)).await;
            match backup::prune(&settings.dir, settings.keep) {
                Ok(removed) if !removed.is_empty() => {
                    info!("sauvegardes anciennes supprimées : {}", removed.join(", "));
                }
                Ok(_) => {}
                Err(error) => warn!("sauvegardes : rotation impossible ({error})"),
            }
            publish(core, &settings.dir, None);
        }
        Err(failure) => {
            error!("sauvegarde ÉCHOUÉE : {failure}");
            crate::cli::audit_local(store, "serveur", "backup_failed", None).await;
            publish(core, &settings.dir, Some(&failure.to_string()));
            notify(
                core,
                &format!("La sauvegarde de cette nuit a échoué : {failure}"),
            );
        }
    }
}

/// Met à jour `sensor.derniere_sauvegarde` : la date de la dernière
/// sauvegarde réussie, et l'erreur de la dernière tentative s'il y en a une.
fn publish(core: &Core, dir: &Path, failure: Option<&str>) {
    let Ok(entity_id) = ENTITY.parse::<EntityId>() else {
        return;
    };
    let backups = backup::list(dir).unwrap_or_default();
    let newest = backups.last();
    let state = newest_time(dir)
        .and_then(|at| jiff::Timestamp::try_from(at).ok())
        .map_or_else(|| "unknown".to_owned(), |at| at.to_string());
    let mut attributes = match json!({
        "friendly_name": "Dernière sauvegarde",
        "device_class": "timestamp",
        "fichier": newest.map(|b| b.name.clone()),
        "taille_kio": newest.map(|b| b.size / 1024),
        "nombre": backups.len(),
        "erreur": failure,
    }) {
        Value::Object(map) => map,
        _ => Attributes::new(),
    };
    attributes.retain(|_, value| !value.is_null());
    if let Err(error) = core.states.set(entity_id, state, attributes) {
        warn!("sauvegardes : état refusé ({error})");
    }
}

/// Prévient le téléphone, si les notifications sont configurées.
fn notify(core: &Core, message: &str) {
    let Ok(entity_id) = "notify.telephone".parse::<EntityId>() else {
        return;
    };
    if core.states.get(&entity_id).is_none() {
        return;
    }
    let mut data = Attributes::new();
    data.insert("title".into(), "Sauvegarde".into());
    data.insert("message".into(), message.into());
    if let Err(error) = core.call_service("notify", "send_message", entity_id, data) {
        warn!("notification impossible : {error}");
    }
}

/// Le dossier pour la ligne de commande : `HA_BACKUP_DIR`, ou `sauvegardes`.
fn cli_dir() -> PathBuf {
    std::env::var_os("HA_BACKUP_DIR").map_or_else(|| PathBuf::from("sauvegardes"), PathBuf::from)
}

/// `ha-server backup now | list | restore <fichier>`.
pub async fn command(args: &[&str]) -> Result<(), Box<dyn Error>> {
    let dir = cli_dir();
    match args {
        ["now"] => {
            let store = crate::open_store().await?;
            let result = store.backup_into(&dir, &stamp()).await;
            if let Ok(saved) = &result {
                crate::cli::audit_local(&store, "ligne de commande", "backup", Some(&saved.name))
                    .await;
            }
            store.close().await;
            let saved = result?;
            println!(
                "sauvegarde vérifiée : {} ({} Kio)",
                saved.path.display(),
                saved.size / 1024
            );
            println!("la clé (ha.key) n'est pas dans la sauvegarde : garde-la à part.");
            Ok(())
        }
        ["list"] => {
            let backups = backup::list(&dir)?;
            if backups.is_empty() {
                println!("aucune sauvegarde dans {}", dir.display());
            }
            for saved in backups {
                println!("  {}  ({} Kio)", saved.name, saved.size / 1024);
            }
            Ok(())
        }
        ["restore", name] => restore(&dir, name, false).await,
        ["restore", name, "--force"] => restore(&dir, name, true).await,
        _ => Err(crate::cli::USAGE.into()),
    }
}

async fn restore(dir: &Path, name: &str, force: bool) -> Result<(), Box<dyn Error>> {
    // Un nom seul désigne un fichier du dossier des sauvegardes.
    let source = if Path::new(name).components().count() == 1 {
        dir.join(name)
    } else {
        PathBuf::from(name)
    };
    if !source.is_file() {
        return Err(format!("{} introuvable", source.display()).into());
    }
    // Restaurer sous un serveur en marche mélangerait deux bases : on refuse.
    if let Ok(bind) = crate::parse_bind()
        && crate::health::is_running(bind).await
    {
        return Err(
            "le serveur est en marche : arrête-le d'abord (docker compose stop ha-server)".into(),
        );
    }
    let db = PathBuf::from(crate::db_path());
    // Deuxième vérification, qui marche aussi entre deux conteneurs Docker :
    // un serveur en marche garde un journal `ha.db-wal`, effacé à l'arrêt
    // propre. S'il reste après un arrêt brutal, `--force` permet de passer.
    let journal = db.with_extension("db-wal");
    if !force && std::fs::metadata(&journal).is_ok_and(|m| m.len() > 0) {
        return Err(format!(
            "{} existe : le serveur est peut-être en marche. Arrête-le ; s'il est \
             vraiment arrêté (après une coupure, par exemple), ajoute --force",
            journal.display()
        )
        .into());
    }
    let safety = backup::restore(&source, &db, dir, &stamp()).await?;
    println!("base restaurée depuis {}", source.display());
    if let Some(safety) = safety {
        println!("l'ancienne base est gardée dans {}", safety.display());
    }
    let store = crate::open_store().await?;
    crate::cli::audit_local(&store, "ligne de commande", "backup_restore", Some(name)).await;
    store.close().await;
    println!("tu peux redémarrer le serveur.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_are_read_strictly() {
        assert_eq!(parse_time("03:30"), Some(Time::constant(3, 30, 0, 0)));
        assert_eq!(parse_time(" 23:05 "), Some(Time::constant(23, 5, 0, 0)));
        for bad in ["24:00", "3h30", "", "03:61"] {
            assert_eq!(parse_time(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn the_next_run_is_within_a_day() {
        let wait = until(Time::constant(3, 30, 0, 0));
        assert!(wait <= Duration::from_hours(25), "{wait:?}");
        assert!(wait > Duration::ZERO);
    }

    #[test]
    fn stamps_are_valid_backup_names() {
        assert!(backup::backup_name(&stamp()).is_ok());
    }
}
