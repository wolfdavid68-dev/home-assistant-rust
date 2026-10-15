//! Tests des sauvegardes, sur de vrais fichiers dans un dossier temporaire.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "dans un test, un échec doit simplement faire échouer le test"
)]

use std::path::{Path, PathBuf};

use ha_core::Attributes;
use ha_storage::{Store, backup};

/// Dossier temporaire unique, supprimé à la fin du test.
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("ha-sauvegarde-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

async fn store_with(path: &Path, value: &str) -> Store {
    let store = Store::open(path).await.unwrap();
    let core = ha_core::Core::new();
    let entity = "light.salon".parse().unwrap();
    core.states.set(entity, value, Attributes::new()).unwrap();
    store.replace_all_states(&core.states.all()).await.unwrap();
    store
}

async fn salon(store: &Store) -> String {
    store.load_states().await.unwrap()[0].state.clone()
}

#[tokio::test]
async fn a_backup_is_taken_while_the_base_is_in_use() {
    let dir = TempDir::new("en-marche");
    let store = store_with(&dir.0.join("ha.db"), "on").await;
    let saves = dir.0.join("sauvegardes");

    // La base est ouverte (le serveur tourne) : la copie se fait quand même.
    let first = store.backup_into(&saves, "2026-10-14_0330").await.unwrap();
    assert_eq!(first.name, "ha-2026-10-14_0330.db");
    assert!(first.size > 0);
    backup::verify(&first.path).await.unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&first.path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "réservée à son propriétaire");
    }

    // Même nom deux fois : refusé, la première n'est pas écrasée.
    assert!(store.backup_into(&saves, "2026-10-14_0330").await.is_err());
    // Un horodatage piégé ne peut pas sortir du dossier.
    for stamp in ["../../etc", "a/b", "", "2026 10 14"] {
        assert!(store.backup_into(&saves, stamp).await.is_err(), "{stamp:?}");
    }
    store.close().await;
}

#[tokio::test]
async fn old_backups_are_pruned_and_nothing_else_is_touched() {
    let dir = TempDir::new("rotation");
    let store = store_with(&dir.0.join("ha.db"), "on").await;
    let saves = dir.0.join("sauvegardes");
    for day in 10..=14 {
        store
            .backup_into(&saves, &format!("2026-10-{day}_0330"))
            .await
            .unwrap();
    }
    std::fs::write(saves.join("notes.txt"), "à moi").unwrap();
    std::fs::write(saves.join("ha-manuelle.db"), "à moi aussi").unwrap();

    let removed = backup::prune(&saves, 3).unwrap();
    assert_eq!(removed, ["ha-2026-10-10_0330.db", "ha-2026-10-11_0330.db"]);
    let names: Vec<String> = backup::list(&saves)
        .unwrap()
        .into_iter()
        .map(|b| b.name)
        .collect();
    assert_eq!(
        names,
        [
            "ha-2026-10-12_0330.db",
            "ha-2026-10-13_0330.db",
            "ha-2026-10-14_0330.db"
        ]
    );
    assert!(saves.join("notes.txt").exists(), "fichier étranger gardé");
    assert!(
        saves.join("ha-manuelle.db").exists(),
        "nom différent : gardé"
    );
    store.close().await;
}

#[tokio::test]
async fn restoring_brings_the_old_data_back_and_keeps_a_safety_copy() {
    let dir = TempDir::new("restauration");
    let db = dir.0.join("ha.db");
    let saves = dir.0.join("sauvegardes");
    let store = store_with(&db, "on").await;
    let copy = store.backup_into(&saves, "2026-10-14_0330").await.unwrap();
    // Après la sauvegarde, la lampe change.
    store.close().await;
    let store = store_with(&db, "off").await;
    assert_eq!(salon(&store).await, "off");
    store.close().await;

    // Serveur arrêté : on restaure.
    let safety = backup::restore(&copy.path, &db, &saves, "2026-10-15_0900")
        .await
        .unwrap()
        .unwrap();
    let store = Store::open(&db).await.unwrap();
    assert_eq!(salon(&store).await, "on", "l'ancienne valeur est revenue");
    store.close().await;
    // La base d'avant la restauration est gardée, et elle est saine. La
    // rotation ne la supprimera jamais.
    assert!(
        safety.ends_with("avant-restauration-2026-10-15_0900.db"),
        "{safety:?}"
    );
    backup::verify(&safety).await.unwrap();
    assert_eq!(backup::prune(&saves, 1).unwrap(), Vec::<String>::new());
    assert!(safety.exists());
    let before = Store::open_with_key(&safety, &db.with_extension("key"))
        .await
        .unwrap();
    assert_eq!(salon(&before).await, "off");
    before.close().await;
}

#[tokio::test]
async fn a_damaged_or_foreign_file_is_never_restored() {
    let dir = TempDir::new("refus");
    let db = dir.0.join("ha.db");
    let store = store_with(&db, "on").await;
    store.close().await;
    let fake = dir.0.join("ha-2026-01-01_0000.db");
    std::fs::write(&fake, "ceci n'est pas une base").unwrap();
    assert!(backup::restore(&fake, &db, &dir.0, "x").await.is_err());
    // Une vraie base SQLite, mais pas la nôtre.
    let other = dir.0.join("autre.db");
    let pool = sqlx::SqlitePool::connect(&format!("sqlite:{}?mode=rwc", other.display()))
        .await
        .unwrap();
    sqlx::query("CREATE TABLE t (x)")
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
    let error = backup::restore(&other, &db, &dir.0, "x").await.unwrap_err();
    assert!(
        error.to_string().contains("pas une base de ha-server"),
        "{error}"
    );

    // La base d'origine n'a pas bougé.
    let store = Store::open(&db).await.unwrap();
    assert_eq!(salon(&store).await, "on");
    store.close().await;
}
