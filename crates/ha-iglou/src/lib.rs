//! **Iglou** (étape 81) : la liste de courses de l'appli Iglou (frigo et
//! congélateur partagés de la famille), dans la maison.
//!
//! Quand un compte Iglou est associé (`ha-server iglou pair`), la liste de
//! courses de la maison, `todo.courses`, **est** celle d'Iglou :
//! - ce qui manque au frigo et au congélateur, et les articles ajoutés à la
//!   main, relus chaque minute ;
//! - `todo.add_item` ajoute un article (il apparaît sur les téléphones) ;
//! - `todo.update_item` coche ou décoche (« dans le panier »), ou renomme
//!   un article ajouté à la main ;
//! - `todo.remove_item` retire un article ajouté à la main.
//!
//! Ranger les courses (remettre le frigo à plein) se fait dans l'appli : la
//! maison ne touche jamais aux quantités.
//!
//! - [`client`] : l'API REST de Firebase ;
//! - [`firestore`] : ses valeurs ;
//! - [`list`] : le calcul de la liste, comme dans l'appli.

pub mod client;
pub mod firestore;
pub mod list;

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use ha_core::{Attributes, Background, CancellationToken, Core, EntityId, Event};
use serde_json::{Value, json};
use tokio::sync::broadcast::error::RecvError;
use tracing::{info, warn};

pub use client::{Client, Endpoints, IglouError, Maison};
use list::Item;

/// L'identifiant de l'intégration, propriétaire de son entité.
pub const ID: &str = "iglou";
/// Son nom affiché.
pub const NAME: &str = "Iglou";
/// L'entité : la liste de courses de la maison.
pub const ENTITY: &str = "todo.courses";
/// Son nom affiché.
pub const FRIENDLY_NAME: &str = "Liste de courses";

/// Entre deux lectures de la liste.
const POLL: Duration = Duration::from_mins(1);
/// Après une erreur passagère, au plus.
const MAX_BACKOFF: Duration = Duration::from_mins(5);
/// Après un refus durable (mot de passe, compte retiré de la maison).
const LASTING_WAIT: Duration = Duration::from_mins(30);

/// L'intégration en marche.
#[derive(Debug)]
pub struct Iglou(Background);

impl Iglou {
    /// Publie la liste, puis la relit chaque minute et répond aux ordres.
    #[must_use]
    pub fn start(client: Client, maison: String, core: Arc<Core>) -> Self {
        let shared = Arc::new(Shared { client, maison });
        publish(&core, "unavailable", &[]);
        core.answers_commands(ID);
        let supervisor = Arc::clone(&core);
        Self(ha_core::supervise(supervisor, ID, NAME, move |token| {
            run(Arc::clone(&shared), Arc::clone(&core), token)
        }))
    }

    pub async fn stop(self) {
        self.0.stop().await;
    }
}

struct Shared {
    client: Client,
    maison: String,
}

async fn run(shared: Arc<Shared>, core: Arc<Core>, token: CancellationToken) {
    let mut events = core.bus.subscribe();
    let mut items: Vec<Item> = Vec::new();
    let mut next = tokio::time::Instant::now();
    let mut backoff = Duration::from_secs(5);
    loop {
        tokio::select! {
            () = token.cancelled() => break,
            () = tokio::time::sleep_until(next) => {
                match shared.client.shopping_list(&shared.maison).await {
                    Ok(list) => {
                        if backoff > Duration::from_secs(5) || items.is_empty() && !list.is_empty() {
                            info!("Iglou : {} article(s) dans la liste de courses", list.len());
                        }
                        items = list;
                        publish(&core, &left(&items), &items);
                        core.report(ID, NAME, ha_core::Health::Connected);
                        backoff = Duration::from_secs(5);
                        next = tokio::time::Instant::now() + POLL;
                    }
                    Err(error) => {
                        let wait = if error.is_lasting() { LASTING_WAIT } else { backoff };
                        warn!("Iglou : {error}, nouvel essai dans {} s", wait.as_secs());
                        core.report(ID, NAME, ha_core::Health::Retrying {
                            error: error.to_string(),
                            retry_at: SystemTime::now() + wait,
                        });
                        publish(&core, "unavailable", &items);
                        backoff = (backoff * 2).min(MAX_BACKOFF);
                        next = tokio::time::Instant::now() + wait;
                    }
                }
            }
            received = events.recv() => match received {
                Ok(event) => {
                    if handle(&shared, &core, &event, &items).await {
                        // Relire tout de suite : la liste a changé.
                        next = tokio::time::Instant::now();
                    }
                }
                Err(RecvError::Lagged(missed)) => warn!("Iglou : {missed} événements manqués"),
                Err(RecvError::Closed) => break,
            },
        }
    }
    info!("Iglou : arrêté");
}

/// Ce qui reste à acheter (pas encore dans le panier).
fn left(items: &[Item]) -> String {
    items.iter().filter(|i| !i.done).count().to_string()
}

fn publish(core: &Core, state: &str, items: &[Item]) {
    let Ok(id) = EntityId::new(ENTITY) else {
        return;
    };
    if let Err(error) = core.states.set_by(ID, id, state, attributes(items)) {
        warn!("{ENTITY} : état refusé ({error})");
    }
}

/// Les attributs, comme une liste de Home Assistant (`items` avec `uid`,
/// `summary`, `status`), plus ce que l'appli montre : la quantité, le frigo
/// ou le congélateur, « épuisé ».
#[must_use]
pub fn attributes(items: &[Item]) -> Attributes {
    let list: Vec<Value> = items
        .iter()
        .map(|item| {
            json!({
                "uid": item.uid,
                "summary": item.summary,
                "status": if item.done { "completed" } else { "needs_action" },
                "quantity": item.quantity,
                "group": item.group,
                "out": item.out,
                "removable": item.is_libre(),
            })
        })
        .collect();
    let mut attributes = Attributes::new();
    attributes.insert("friendly_name".into(), json!(FRIENDLY_NAME));
    attributes.insert("items".into(), Value::Array(list));
    // Le tableau de bord : pas de « Retirer ce qui est fait », on range dans l'appli.
    attributes.insert("source".into(), json!("iglou"));
    attributes
}

/// Répond à un ordre sur la liste. Rend `true` si la liste a changé.
async fn handle(shared: &Shared, core: &Core, event: &Event, items: &[Item]) -> bool {
    let Event::CallService {
        domain,
        service,
        entity_id,
        data,
        command,
        issued_at,
        ..
    } = event
    else {
        return false;
    };
    if domain != "todo" || entity_id.as_str() != ENTITY {
        return false;
    }
    if ha_core::commands::is_stale(*issued_at) {
        core.command_done(*command, Err("ordre périmé".into()));
        return false;
    }
    let result = call(shared, service, data, items).await;
    if let Err(error) = &result {
        warn!("{ENTITY} : {service} refusé ({error})");
    }
    let changed = result.is_ok();
    core.command_done(*command, result);
    changed
}

async fn call(
    shared: &Shared,
    service: &str,
    data: &Attributes,
    items: &[Item],
) -> Result<(), String> {
    let maison = &shared.maison;
    let client = &shared.client;
    match service {
        "add_item" => {
            let text = text(data, "item")?;
            // Déjà sur la liste (pas encore dans le panier) : rien à ajouter.
            if items
                .iter()
                .any(|i| !i.done && i.summary.to_lowercase() == text.to_lowercase())
            {
                return Ok(());
            }
            if items.len() >= client::MAX_ITEMS {
                return Err(format!("la liste a déjà {} articles", client::MAX_ITEMS));
            }
            client.add(maison, &text).await.map_err(|e| e.to_string())
        }
        "update_item" => {
            let item = find(data, items)?;
            let rename = data
                .get("rename")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty());
            let status = data.get("status").and_then(Value::as_str);
            if rename.is_none() && status.is_none() {
                return Err("rien à changer (« status » ou « rename »)".into());
            }
            if let Some(name) = rename {
                client.rename(maison, &item.uid, name).await.map_err(|e| e.to_string())?;
            }
            match status {
                None => Ok(()),
                Some("completed") => client.set_in_cart(maison, &item.uid, true).await.map_err(|e| e.to_string()),
                Some("needs_action") => client.set_in_cart(maison, &item.uid, false).await.map_err(|e| e.to_string()),
                Some(_) => Err("« status » : completed ou needs_action".into()),
            }
        }
        "remove_item" => {
            let item = find(data, items)?;
            client.remove(maison, &item.uid).await.map_err(|e| e.to_string())
        }
        "remove_completed_items" => Err(
            "les courses se rangent dans l'appli Iglou (« J'ai rangé les courses ») : elle remet le frigo à plein"
                .into(),
        ),
        other => Err(format!("todo.{other} n'existe pas pour la liste Iglou")),
    }
}

fn text(data: &Attributes, key: &str) -> Result<String, String> {
    let text = data
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("« {key} » manquant"))?;
    if text.chars().count() > list::MAX_NAME {
        return Err(format!("{} caractères au plus", list::MAX_NAME));
    }
    Ok(text.to_owned())
}

/// L'article désigné par `item` : son `uid`, sinon son nom (le premier qui
/// n'est pas encore dans le panier, comme dans Home Assistant).
fn find<'a>(data: &Attributes, items: &'a [Item]) -> Result<&'a Item, String> {
    let wanted = data
        .get("item")
        .and_then(Value::as_str)
        .map(str::trim)
        .ok_or("« item » manquant")?;
    items
        .iter()
        .find(|i| i.uid == wanted)
        .or_else(|| {
            items
                .iter()
                .find(|i| !i.done && i.summary.eq_ignore_ascii_case(wanted))
        })
        .or_else(|| {
            items
                .iter()
                .find(|i| i.summary.eq_ignore_ascii_case(wanted))
        })
        .ok_or_else(|| "article introuvable dans la liste".to_owned())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]

    use super::*;

    fn item(uid: &str, summary: &str, done: bool) -> Item {
        Item {
            uid: uid.into(),
            summary: summary.into(),
            quantity: "1 pièce".into(),
            group: "Frigo".into(),
            out: false,
            done,
        }
    }

    #[test]
    fn finds_an_item_by_uid_or_name() {
        let items = [item("a:1", "Lait", true), item("l:2", "lait", false)];
        let data = |v: &str| {
            let mut a = Attributes::new();
            a.insert("item".into(), json!(v));
            a
        };
        assert_eq!(find(&data("a:1"), &items).unwrap().uid, "a:1");
        // Par le nom : d'abord celui qui n'est pas encore dans le panier.
        assert_eq!(find(&data("LAIT"), &items).unwrap().uid, "l:2");
        assert!(find(&data("Pain"), &items).is_err());
    }

    #[test]
    fn publishes_like_a_home_assistant_list() {
        let items = [item("a:1", "Lait", true), item("l:2", "Pain", false)];
        assert_eq!(left(&items), "1");
        let attributes = attributes(&items);
        let list = attributes["items"].as_array().unwrap();
        assert_eq!(list[0]["status"], "completed");
        assert_eq!(list[0]["removable"], false);
        assert_eq!(list[1]["removable"], true);
        assert_eq!(attributes["source"], "iglou");
    }
}
