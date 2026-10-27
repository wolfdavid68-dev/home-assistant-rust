//! `ha-server automations check | list | delete` : les automatisations du
//! fichier et celles de l'éditeur, rangées dans la base.

use std::error::Error;

use ha_storage::Store;

use super::USAGE;
use super::audit::audit_cli;
use super::checks::{check_automations, check_scripts_and_scenes};
use super::terminal::local_time;

/// `ha-server automations check | list | delete <id>`.
pub async fn automations_command(args: &[&str]) -> Result<(), Box<dyn Error>> {
    let path = crate::automations_path();
    match args {
        ["check"] => {
            check_automations(&path)?;
            check_scripts_and_scenes(&crate::scripts_path(), &crate::scenes_path())?;
            // On n'ouvre la base que si elle existe : une simple
            // vérification ne doit pas créer de fichier.
            if std::path::Path::new(&crate::db_path()).exists() {
                with_editor(&path, |editor| Box::pin(check_stored(editor))).await?;
            }
            Ok(())
        }
        ["list"] => with_editor(&path, |editor| Box::pin(list_automations(editor))).await,
        ["delete", id] => {
            let store = crate::open_store().await?;
            let editor = editor_for(&path, &store)?;
            let result = editor.delete(id, "ligne de commande").await;
            let result = match result {
                Ok(true) => {
                    audit_cli(&store, "automations", args).await;
                    println!("automatisation « {id} » supprimée (ses versions sont gardées).");
                    println!("redémarre le serveur pour l'appliquer.");
                    Ok(())
                }
                Ok(false) => Err(format!("aucune automatisation « {id} » dans l'éditeur").into()),
                Err(error) => Err(error.into()),
            };
            store.close().await;
            result
        }
        _ => Err(USAGE.into()),
    }
}

fn editor_for(
    path: &std::path::Path,
    store: &Store,
) -> Result<ha_automation::Editor, Box<dyn Error>> {
    let file = ha_automation::load_configs(path)?;
    let has_location = crate::location()?.is_some();
    Ok(ha_automation::Editor::new(
        store.clone(),
        file,
        has_location,
    ))
}

/// Ouvre la base, prépare l'éditeur, lance `run` puis ferme la base.
///
/// Concept Rust : `run` renvoie un `Future` « épinglé » dans une `Box`
/// (`Pin<Box<…>>`). C'est la façon simple de passer une fonction `async`
/// qui emprunte un paramètre (`&Editor`) : le compilateur sait ainsi que le
/// `Future` ne vit pas plus longtemps que l'emprunt.
async fn with_editor<F>(path: &std::path::Path, run: F) -> Result<(), Box<dyn Error>>
where
    F: for<'a> FnOnce(
        &'a ha_automation::Editor,
    )
        -> std::pin::Pin<Box<dyn Future<Output = Result<(), Box<dyn Error>>> + 'a>>,
{
    let store = crate::open_store().await?;
    let result = match editor_for(path, &store) {
        Ok(editor) => run(&editor).await,
        Err(error) => Err(error),
    };
    store.close().await;
    result
}

/// Vérifie les automatisations de l'éditeur, rangées dans la base.
async fn check_stored(editor: &ha_automation::Editor) -> Result<(), Box<dyn Error>> {
    let entries = editor.entries().await?;
    let stored: Vec<_> = entries
        .iter()
        .filter(|entry| entry.source == ha_automation::editor::Source::Editor)
        .collect();
    let invalid: Vec<_> = stored
        .iter()
        .filter(|entry| entry.error.is_some())
        .collect();
    println!(
        "éditeur (base de données) : {} automatisation(s), {} invalide(s)",
        stored.len(),
        invalid.len()
    );
    for entry in &invalid {
        println!(
            "  - {} : {}",
            entry.id,
            entry.error.as_deref().unwrap_or_default()
        );
    }
    if invalid.is_empty() {
        Ok(())
    } else {
        Err("des automatisations de l'éditeur sont invalides : elles ne tournent pas".into())
    }
}

async fn list_automations(editor: &ha_automation::Editor) -> Result<(), Box<dyn Error>> {
    let entries = editor.entries().await?;
    if entries.is_empty() {
        println!("aucune automatisation.");
    }
    for entry in &entries {
        let source = match entry.source {
            ha_automation::editor::Source::File => "fichier",
            ha_automation::editor::Source::Editor => "éditeur",
        };
        let status = match (&entry.error, &entry.config) {
            (Some(error), _) => format!("INVALIDE : {error}"),
            (None, Some(config)) if !config.enabled() => "désactivée".to_owned(),
            _ => "active".to_owned(),
        };
        let alias = entry
            .config
            .as_ref()
            .map_or(entry.id.as_str(), ha_automation::AutomationConfig::alias);
        let modified = match (&entry.updated_by, entry.updated_at) {
            (Some(by), Some(at)) => format!(", modifiée par {by} le {}", local_time(at)),
            _ => String::new(),
        };
        println!(
            "  - {} « {alias} » ({source}, {status}{modified})",
            entry.id
        );
    }
    Ok(())
}
