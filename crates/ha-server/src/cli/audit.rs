//! Le journal d'audit : les lignes écrites par la ligne de commande, et
//! `ha-server audit [nombre] | verify` pour le lire.

use std::error::Error;

use ha_storage::Store;

use super::USAGE;
use super::terminal::local_time;

/// Enregistre dans le journal d'audit les commandes qui changent quelque
/// chose (pas celles qui ne font que lire).
pub(super) async fn audit_cli(store: &Store, domain: &str, args: &[&str]) {
    let Some(entry) = audit_entry(domain, args) else {
        return;
    };
    let record = ha_storage::AuditRecord {
        actor: "ligne de commande",
        action: &entry.action,
        target: entry.target,
        detail: entry.detail,
        source: "local",
    };
    if let Err(error) = store.audit(record).await {
        eprintln!("attention : journal d'audit non écrit ({error})");
    }
}

/// Une ligne du journal d'audit pour une commande.
#[derive(Debug, PartialEq, Eq)]
struct Entry<'a> {
    action: String,
    target: Option<&'a str>,
    detail: Option<&'a str>,
}

/// La ligne d'audit d'une commande, ou `None` si elle ne change rien.
///
/// Concept Rust : la durée de vie `'a` dit que `target` et `detail`
/// empruntent les textes de `args`, sans les copier.
fn audit_entry<'a>(domain: &str, args: &[&'a str]) -> Option<Entry<'a>> {
    let (action, target) = match (domain, args) {
        (
            "user",
            [
                command @ ("add" | "passwd" | "delete" | "totp" | "totp-off" | "token"),
                name,
                ..,
            ],
        ) => (format!("user_{}", command.replace('-', "_")), Some(*name)),
        // Le rôle donné (`admin`…) va dans le détail : qui est devenu
        // administrateur, et quand, doit se retrouver.
        ("user", ["role", name, role]) => {
            return Some(Entry {
                action: "user_role".to_owned(),
                target: Some(name),
                detail: Some(role),
            });
        }
        ("hue" | "somfy", ["pair", host, ..]) => (format!("{domain}_pair"), Some(*host)),
        ("freebox", ["pair", rest @ ..]) => (
            "freebox_pair".to_owned(),
            Some(
                rest.first()
                    .copied()
                    .unwrap_or(ha_freebox::api::DEFAULT_HOST),
            ),
        ),
        ("freebox", ["track", mac, _]) => ("freebox_track".to_owned(), Some(*mac)),
        ("freebox", ["untrack", name]) => ("freebox_untrack".to_owned(), Some(*name)),
        (
            "hue" | "somfy" | "freebox" | "timetree" | "iglou" | "smartthings" | "colis",
            ["forget"],
        ) => (format!("{domain}_forget"), None),
        ("timetree", ["pair"]) => ("timetree_pair".to_owned(), None),
        ("iglou", ["pair"]) => ("iglou_pair".to_owned(), None),
        ("smartthings", ["pair"]) => ("smartthings_pair".to_owned(), None),
        ("colis", ["pair"]) => ("colis_pair".to_owned(), None),
        ("automations", ["delete", id]) => ("automation_delete".to_owned(), Some(*id)),
        _ => return None,
    };
    Some(Entry {
        action,
        target,
        detail: None,
    })
}

/// Ajoute une ligne « locale » (faite sur la machine elle-même) au journal.
pub async fn audit_local(store: &Store, actor: &str, action: &str, target: Option<&str>) {
    let record = ha_storage::AuditRecord {
        actor,
        action,
        target,
        detail: None,
        source: "local",
    };
    if let Err(error) = store.audit(record).await {
        eprintln!("attention : journal d'audit non écrit ({error})");
    }
}

/// `ha-server audit [nombre]` et `ha-server audit verify`.
pub async fn audit_command(args: &[&str]) -> Result<(), Box<dyn Error>> {
    let store = crate::open_store().await?;
    let result = audit_inner(args, &store).await;
    store.close().await;
    result
}

async fn audit_inner(args: &[&str], store: &Store) -> Result<(), Box<dyn Error>> {
    let limit = match args {
        ["verify"] => {
            return match store.verify_audit().await? {
                ha_storage::AuditCheck::Intact { entries } => {
                    println!("journal intact : {entries} lignes, signatures et chaîne valides");
                    Ok(())
                }
                ha_storage::AuditCheck::Broken { id, reason } => {
                    Err(format!("journal MODIFIÉ à la ligne {id} : {reason}").into())
                }
            };
        }
        [] => 50,
        [count] => count
            .parse::<u32>()
            .ok()
            .filter(|count| (1..=ha_storage::MAX_AUDIT_LIMIT).contains(count))
            .ok_or_else(|| {
                format!(
                    "nombre de lignes invalide (entre 1 et {})",
                    ha_storage::MAX_AUDIT_LIMIT
                )
            })?,
        _ => return Err(USAGE.into()),
    };
    let mut entries = store.audit_log(limit).await?;
    // Le plus récent en bas, comme dans un terminal.
    entries.reverse();
    if entries.is_empty() {
        println!("journal vide");
    }
    for entry in entries {
        let when = local_time(entry.at);
        let what = [entry.target.as_deref(), entry.detail.as_deref()]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" ");
        println!(
            "{when}  {:<18} {:<34} {what}  ({})",
            entry.actor,
            action_label(&entry.action),
            entry.source
        );
    }
    Ok(())
}

/// Libellé lisible d'une action du journal.
fn action_label(action: &str) -> &str {
    match action {
        "login" => "connexion",
        "login_failed" => "ÉCHEC de connexion",
        "logout" => "déconnexion",
        "service" => "commande",
        "state_set" => "état modifié",
        "state_delete" => "état supprimé",
        "user_add" => "compte créé",
        "user_passwd" => "mot de passe changé",
        "user_delete" => "compte supprimé",
        "user_totp" => "double authentification activée",
        "user_totp_off" => "double authentification désactivée",
        "hue_pair" => "pont Hue associé",
        "hue_forget" => "pont Hue oublié",
        "somfy_pair" => "box Somfy associée",
        "somfy_forget" => "box Somfy oubliée",
        "freebox_pair" => "Freebox associée",
        "freebox_forget" => "Freebox oubliée",
        "freebox_track" => "présence suivie",
        "freebox_untrack" => "présence plus suivie",
        "freebox_new_device" => "NOUVEL appareil sur le réseau",
        "confirm" => "identité confirmée",
        "confirm_failed" => "ÉCHEC de confirmation",
        "automation_create" => "automatisation créée",
        "automation_update" => "automatisation modifiée",
        "automation_delete" => "automatisation supprimée",
        "backup" => "sauvegarde",
        "backup_failed" => "ÉCHEC de sauvegarde",
        "backup_restore" => "sauvegarde restaurée",
        "server_start" => "démarrage du serveur",
        "server_stop" => "arrêt du serveur",
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::{Entry, audit_entry};

    #[test]
    fn role_changes_are_audited() {
        assert_eq!(
            audit_entry("user", &["role", "alice", "admin"]),
            Some(Entry {
                action: "user_role".to_owned(),
                target: Some("alice"),
                detail: Some("admin"),
            })
        );
        assert_eq!(
            audit_entry("user", &["totp-off", "alice"]),
            Some(Entry {
                action: "user_totp_off".to_owned(),
                target: Some("alice"),
                detail: None,
            })
        );
        // Lire ne change rien : pas de ligne.
        assert_eq!(audit_entry("user", &["list"]), None);
    }
}
