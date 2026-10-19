//! Ce que chaque compte peut voir et commander (voir `ha_auth::Role`).
//!
//! Un **invité** ne voit pas la présence de la famille (personnes,
//! téléphones, zone Maison), ni l'alarme, ni les serrures et sirènes, ni
//! l'agenda et les listes, ni les notifications et les rappels. Il ne
//! commande que des appareils sans risque : lumières, prises, ventilateurs,
//! scènes, et les volets (jamais un portail, une porte de garage ou une
//! porte). Les interrupteurs du serveur lui-même (simulation de présence,
//! délestage, protection solaire) lui sont cachés.

use ha_auth::{Permission, User};
use ha_core::{EntityId, State};
use serde_json::{Value, json};

use crate::error::ApiError;

/// Domaines cachés aux invités : la présence de la famille, les messages, et
/// ce qui dit si la maison est fermée (serrures, sirènes).
const HIDDEN_FROM_GUESTS: [&str; 11] = [
    // Les notifications, alertes et rappels parlent de la maison (porte
    // ouverte…).
    "persistent_notification",
    "alert",
    "person",
    "device_tracker",
    "zone",
    "notify",
    "lock",
    "siren",
    "alarm_control_panel",
    // L'agenda dit quand la maison est vide (« Vacances »), les listes sont
    // celles de la famille.
    "calendar",
    "todo",
];
/// Les interrupteurs du serveur lui-même, cachés aux invités : la simulation
/// de présence dit quand la maison est vide (étape 58), le délestage protège
/// le compteur (étape 62), la protection solaire garde la maison au frais
/// (étape 70).
const HIDDEN_SWITCHES: [&str; 3] = [
    "switch.simulation_presence",
    "switch.delestage",
    "switch.protection_solaire",
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
            Permission::EditDashboard => "changer le dessin de la maison",
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
    !HIDDEN_FROM_GUESTS.contains(&domain)
        && !alarm
        && !HIDDEN_SWITCHES.contains(&entity_id.as_str())
}

/// Ce compte peut-il commander cet appareil ?
pub(crate) fn can_control(user: &User, entity_id: &EntityId, state: Option<&State>) -> bool {
    if user.can(Permission::AllDevices) {
        return true;
    }
    let domain = entity_id.domain();
    if !GUEST_CONTROLS.contains(&domain) || HIDDEN_SWITCHES.contains(&entity_id.as_str()) {
        return false;
    }
    // Un volet sans classe connue est accepté ; un portail ou un garage, jamais.
    !(domain == "cover"
        && device_class(state).is_some_and(|c| FORBIDDEN_COVER_CLASSES.contains(&c)))
}

/// Les commandes qui ouvrent la maison (déverrouiller, désarmer l'alarme) :
/// même un administrateur doit redonner son mot de passe (valable 15
/// minutes), comme pour modifier une automatisation. Un téléphone oublié
/// déverrouillé ne suffit pas.
#[must_use]
pub(crate) fn needs_confirmation(domain: &str, service: &str) -> bool {
    matches!(
        (domain, service),
        ("lock", "unlock" | "open") | ("alarm_control_panel", "alarm_disarm")
    )
}

/// Comme [`can_control`], et pour un **groupe** (`light.lumieres_du_bas`),
/// chacun de ses membres aussi : un invité ne doit pas pouvoir ouvrir la
/// porte du garage en passant par un groupe de volets qui la contient.
pub(crate) fn can_control_with_members(
    user: &User,
    entity_id: &EntityId,
    states: &ha_core::StateMachine,
) -> bool {
    let state = states.get(entity_id);
    if !can_control(user, entity_id, state.as_ref()) {
        return false;
    }
    let Some(members) = state
        .as_ref()
        .and_then(|s| s.attributes().get("entity_id"))
        .and_then(Value::as_array)
    else {
        return true;
    };
    members.iter().all(|member| {
        member
            .as_str()
            .and_then(|id| id.parse::<EntityId>().ok())
            .is_some_and(|id| can_control(user, &id, states.get(&id).as_ref()))
    })
}

/// Ce qu'un assistant IA ne commande **jamais**, même en fermant (étape
/// 55) : ce qui ferme ou ouvre la maison. Les serrures, l'alarme, les
/// sirènes, et les portes de garage, portails et portes ; pour un groupe,
/// chacun de ses membres aussi.
pub(crate) fn guards_the_house(entity_id: &EntityId, states: &ha_core::StateMachine) -> bool {
    let sensitive = |id: &EntityId, state: Option<&State>| {
        matches!(id.domain(), "lock" | "alarm_control_panel" | "siren")
            || (id.domain() == "cover"
                && device_class(state).is_some_and(|c| FORBIDDEN_COVER_CLASSES.contains(&c)))
    };
    let state = states.get(entity_id);
    if sensitive(entity_id, state.as_ref()) {
        return true;
    }
    state
        .as_ref()
        .and_then(|s| s.attributes().get("entity_id"))
        .and_then(Value::as_array)
        .is_some_and(|members| {
            members
                .iter()
                .filter_map(Value::as_str)
                .filter_map(|id| id.parse::<EntityId>().ok())
                .any(|id| sensitive(&id, states.get(&id).as_ref()))
        })
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
        "edit_dashboard": user.can(Permission::EditDashboard),
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
