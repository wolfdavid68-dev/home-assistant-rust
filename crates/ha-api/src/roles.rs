//! Ce que chaque compte peut voir et commander (voir `ha_auth::Role`).
//!
//! Un **invité** ne voit pas la présence de la famille (personnes,
//! téléphones, zone Maison), ni l'alarme, ni les serrures et sirènes, ni
//! l'agenda et les listes, ni les notifications. Il ne
//! commande que des appareils sans risque : lumières, prises, ventilateurs,
//! scènes, et les volets (jamais un portail, une porte de garage ou une
//! porte).

use ha_auth::{Permission, User};
use ha_core::{EntityId, State};
use serde_json::{Value, json};

use crate::error::ApiError;

/// Domaines cachés aux invités : la présence de la famille, les messages, et
/// ce qui dit si la maison est fermée (serrures, sirènes).
const HIDDEN_FROM_GUESTS: [&str; 8] = [
    "person",
    "device_tracker",
    "zone",
    "notify",
    "lock",
    "siren",
    // L'agenda dit quand la maison est vide (« Vacances »), les listes sont
    // celles de la famille.
    "calendar",
    "todo",
];
/// Domaines qu'un invité peut commander.
const GUEST_CONTROLS: [&str; 5] = ["light", "switch", "fan", "scene", "cover"];
/// Volets qu'un invité ne commande jamais (comme Alexa).
const FORBIDDEN_COVER_CLASSES: [&str; 3] = ["garage", "gate", "door"];

/// Refuse (403) si le compte n'a pas ce droit.
pub(crate) fn require(user: &User, permission: Permission) -> Result<(), ApiError> {
    if user.can(permission) {
        return Ok(());
    }
    Err(ApiError::Forbidden(format!(
        "ton compte ({}) ne permet pas de {}",
        user.role.label(),
        match permission {
            Permission::ReadHistory => "voir l'historique",
            Permission::ReadAutomations => "voir les automatisations",
            Permission::EditAutomations => "modifier les automatisations",
            Permission::WriteStates => "modifier un état directement",
            Permission::ReadAudit => "voir le journal d'audit",
            Permission::AllDevices => "voir tous les appareils",
        }
    )))
}

fn device_class(state: Option<&State>) -> Option<&str> {
    state?.attributes().get("device_class")?.as_str()
}

/// Ce compte peut-il voir cet appareil ?
pub(crate) fn can_see(user: &User, entity_id: &EntityId, state: Option<&State>) -> bool {
    if user.can(Permission::AllDevices) {
        return true;
    }
    let domain = entity_id.domain();
    let alarm = domain == "sensor" && device_class(state) == Some("alarm");
    !HIDDEN_FROM_GUESTS.contains(&domain) && !alarm
}

/// Ce compte peut-il commander cet appareil ?
pub(crate) fn can_control(user: &User, entity_id: &EntityId, state: Option<&State>) -> bool {
    if user.can(Permission::AllDevices) {
        return true;
    }
    let domain = entity_id.domain();
    if !GUEST_CONTROLS.contains(&domain) {
        return false;
    }
    // Un volet sans classe connue est accepté ; un portail ou un garage, jamais.
    !(domain == "cover"
        && device_class(state).is_some_and(|c| FORBIDDEN_COVER_CLASSES.contains(&c)))
}

/// Les commandes qui ouvrent la maison : même un administrateur doit
/// redonner son mot de passe (valable 15 minutes), comme pour modifier une
/// automatisation. Un téléphone oublié déverrouillé ne suffit pas.
#[must_use]
pub(crate) fn needs_confirmation(domain: &str, service: &str) -> bool {
    matches!((domain, service), ("lock", "unlock" | "open"))
}

/// Les droits du compte, pour que le tableau de bord cache ce qui ne sert à
/// rien (le serveur vérifie de toute façon chaque demande).
pub(crate) fn permissions_json(user: &User) -> Value {
    json!({
        "history": user.can(Permission::ReadHistory),
        "automations": user.can(Permission::ReadAutomations),
        "edit_automations": user.can(Permission::EditAutomations),
        "write_states": user.can(Permission::WriteStates),
        "audit": user.can(Permission::ReadAudit),
        "all_devices": user.can(Permission::AllDevices),
        // Ce que le compte peut commander, avec les mêmes règles que
        // `can_control` : `null` veut dire « tout ».
        "controls": if user.can(Permission::AllDevices) {
            Value::Null
        } else {
            json!({
                "domains": GUEST_CONTROLS,
                "forbidden_cover_classes": FORBIDDEN_COVER_CLASSES,
            })
        },
    })
}
