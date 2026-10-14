//! Présence et nouveaux appareils, à partir de la liste des appareils vus
//! par la Freebox. Aucune connexion réseau ici : tout se teste facilement.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use ha_core::{Attributes, EntityId};
use serde_json::{Value, json};

use crate::api::{ConnectionStatus, LanHost};

/// Un téléphone qui se met en veille coupe parfois son Wi-Fi quelques
/// minutes. On ne le déclare « absent » qu'après ce délai sans activité.
pub const CONSIDER_HOME: Duration = Duration::from_mins(10);
/// Longueur maximale d'un nom d'appareil (venu du réseau) qu'on garde.
const MAX_NAME_CHARS: usize = 64;

/// Normalise une adresse MAC : `aa-bb-cc-dd-ee-ff` → `AA:BB:CC:DD:EE:FF`.
/// `None` si ce n'en est pas une.
#[must_use]
pub fn normalize_mac(value: &str) -> Option<String> {
    let parts: Vec<&str> = value.trim().split([':', '-']).collect();
    let valid = parts.len() == 6
        && parts
            .iter()
            .all(|part| part.len() == 2 && part.bytes().all(|b| b.is_ascii_hexdigit()));
    valid.then(|| parts.join(":").to_ascii_uppercase())
}

/// Nom lisible d'un appareil, nettoyé (il vient du réseau : n'importe quel
/// appareil peut s'annoncer avec n'importe quel nom).
#[must_use]
pub fn display_name(host: &LanHost) -> String {
    let name = ha_storage::clean_field(host.primary_name.trim());
    let name: String = name.chars().take(MAX_NAME_CHARS).collect();
    if name.is_empty() {
        "appareil sans nom".to_owned()
    } else {
        name
    }
}

/// Un appareil suivi : son adresse MAC et le nom de son entité.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tracked {
    pub mac: String,
    pub name: String,
}

/// Ce qu'il faut publier après un relevé.
#[derive(Debug, Default)]
pub struct Update {
    pub states: Vec<(EntityId, String, Attributes)>,
    /// Appareils jamais vus, à signaler : (MAC, nom, fabricant).
    pub new_devices: Vec<(String, String, String)>,
    /// Appareils jamais vus, à mémoriser : (MAC, nom). Au premier relevé,
    /// tout le réseau est mémorisé sans être signalé.
    pub to_remember: Vec<(String, String)>,
}

/// L'état entre deux relevés.
#[derive(Debug)]
pub struct Presence {
    tracked: Vec<Tracked>,
    known: HashSet<String>,
    last_seen: HashMap<String, Instant>,
}

impl Presence {
    /// `known` : les adresses MAC déjà vues (relues dans la base).
    #[must_use]
    pub fn new(tracked: Vec<Tracked>, known: HashSet<String>) -> Self {
        Self {
            tracked,
            known,
            last_seen: HashMap::new(),
        }
    }

    /// Calcule les états à partir d'un relevé de la Freebox.
    pub fn update(
        &mut self,
        hosts: &[LanHost],
        connection: Option<&ConnectionStatus>,
        now: Instant,
    ) -> Update {
        let mut update = Update::default();
        // Premier relevé sans aucun appareil connu : on mémorise tout le
        // réseau actuel sans rien signaler (sinon, des dizaines d'alertes).
        let first_run = self.known.is_empty();

        for host in hosts {
            let Some(mac) = host.mac() else { continue };
            if host.active {
                self.last_seen.insert(mac.clone(), now);
            }
            if self.known.insert(mac.clone()) {
                update.to_remember.push((mac.clone(), display_name(host)));
                if !first_run {
                    update.new_devices.push((
                        mac,
                        display_name(host),
                        ha_storage::clean_field(&host.vendor_name),
                    ));
                }
            }
        }

        for tracked in &self.tracked {
            let home = self
                .last_seen
                .get(&tracked.mac)
                .is_some_and(|seen| now.saturating_duration_since(*seen) < CONSIDER_HOME);
            if let Ok(entity_id) = format!("device_tracker.{}", tracked.name).parse::<EntityId>() {
                update.states.push((
                    entity_id,
                    if home { "home" } else { "not_home" }.to_owned(),
                    attributes(json!({
                        "friendly_name": tracked.name.replace('_', " "),
                        "source_type": "router",
                    })),
                ));
            }
        }

        if let Some(connection) = connection {
            publish_connection(&mut update, connection);
        }
        update
    }

    /// Toutes les adresses MAC connues (pour les enregistrer).
    #[must_use]
    pub fn known(&self) -> &HashSet<String> {
        &self.known
    }
}

fn publish_connection(update: &mut Update, connection: &ConnectionStatus) {
    let online = connection.state == "up";
    let entities = [
        (
            "binary_sensor.freebox_internet",
            if online { "on" } else { "off" }.to_owned(),
            json!({"friendly_name": "Internet", "device_class": "connectivity",
                   "media": ha_storage::clean_field(&connection.media)}),
        ),
        (
            "sensor.freebox_debit_descendant",
            mbit_per_s(connection.rate_down),
            json!({"friendly_name": "Débit descendant", "unit_of_measurement": "Mbit/s",
                   "device_class": "data_rate"}),
        ),
        (
            "sensor.freebox_debit_montant",
            mbit_per_s(connection.rate_up),
            json!({"friendly_name": "Débit montant", "unit_of_measurement": "Mbit/s",
                   "device_class": "data_rate"}),
        ),
    ];
    for (entity, state, attrs) in entities {
        if let Ok(entity_id) = entity.parse() {
            update.states.push((entity_id, state, attributes(attrs)));
        }
    }
}

/// Octets par seconde → mégabits par seconde, une décimale.
fn mbit_per_s(bytes_per_second: u64) -> String {
    // Concept Rust : le calcul se fait en entiers (dixièmes de Mbit/s) pour
    // éviter les conversions approximatives vers `f64`.
    let tenths = bytes_per_second.saturating_mul(8) / 100_000;
    format!("{}.{}", tenths / 10, tenths % 10)
}

fn attributes(value: Value) -> Attributes {
    match value {
        Value::Object(map) => map,
        _ => Attributes::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host(mac: &str, name: &str, active: bool) -> LanHost {
        serde_json::from_value(json!({
            "primary_name": name, "host_type": "smartphone", "vendor_name": "Fabricant",
            "l2ident": {"id": mac, "type": "mac_address"}, "active": active,
        }))
        .unwrap()
    }

    fn state_of(update: &Update, entity: &str) -> String {
        update
            .states
            .iter()
            .find(|(id, _, _)| id.as_str() == entity)
            .map(|(_, state, _)| state.clone())
            .unwrap()
    }

    #[test]
    fn mac_addresses() {
        assert_eq!(
            normalize_mac("aa-bb-cc-dd-ee-0f").as_deref(),
            Some("AA:BB:CC:DD:EE:0F")
        );
        assert_eq!(normalize_mac("AA:BB:CC:DD:EE"), None);
        assert_eq!(normalize_mac("AA:BB:CC:DD:EE:GG"), None);
        assert_eq!(normalize_mac("AA:BB:CC:DD:EE:FF:00"), None);
        assert_eq!(normalize_mac("AAA:B:CC:DD:EE:FF"), None);
    }

    #[test]
    fn phone_is_home_then_away_after_the_delay() {
        let mac = "AA:BB:CC:DD:EE:01";
        let mut presence = Presence::new(
            vec![Tracked {
                mac: mac.into(),
                name: "telephone_david".into(),
            }],
            HashSet::new(),
        );
        let start = Instant::now();
        let update = presence.update(&[host(mac, "iPhone", true)], None, start);
        assert_eq!(state_of(&update, "device_tracker.telephone_david"), "home");

        // Wi-Fi en veille quelques minutes : toujours à la maison.
        let update = presence.update(
            &[host(mac, "iPhone", false)],
            None,
            start + Duration::from_mins(5),
        );
        assert_eq!(state_of(&update, "device_tracker.telephone_david"), "home");

        // Plus de 10 minutes sans activité : parti.
        let update = presence.update(&[], None, start + Duration::from_mins(11));
        assert_eq!(
            state_of(&update, "device_tracker.telephone_david"),
            "not_home"
        );
    }

    #[test]
    fn new_devices_are_reported_except_on_the_first_run() {
        let mut presence = Presence::new(Vec::new(), HashSet::new());
        let now = Instant::now();
        let update = presence.update(
            &[
                host("AA:BB:CC:DD:EE:01", "PC", true),
                host("AA:BB:CC:DD:EE:02", "TV", false),
            ],
            None,
            now,
        );
        assert!(update.new_devices.is_empty(), "premier relevé : silence");
        assert_eq!(update.to_remember.len(), 2, "mais tout est mémorisé");
        assert_eq!(presence.known().len(), 2);

        let update = presence.update(
            &[
                host("AA:BB:CC:DD:EE:01", "PC", true),
                host("aa:bb:cc:dd:ee:03", "Inconnu\u{1b}[2J", true),
            ],
            None,
            now,
        );
        assert_eq!(update.new_devices.len(), 1);
        let (mac, name, _) = &update.new_devices[0];
        assert_eq!(mac, "AA:BB:CC:DD:EE:03");
        assert!(!name.contains('\u{1b}'), "nom nettoyé : {name:?}");

        assert_eq!(update.to_remember.len(), 1);

        // Revu ensuite : plus signalé, plus rien à mémoriser.
        let update = presence.update(&[host("AA:BB:CC:DD:EE:03", "x", true)], None, now);
        assert_eq!(update.new_devices, Vec::new());
        assert_eq!(update.to_remember, Vec::new());
    }

    #[test]
    fn connection_entities() {
        let mut presence = Presence::new(Vec::new(), HashSet::from(["X".to_owned()]));
        let connection = ConnectionStatus {
            state: "up".into(),
            media: "ftth".into(),
            rate_down: 125_000_000, // 1 Gbit/s
            rate_up: 1_250_000,     // 10 Mbit/s
        };
        let update = presence.update(&[], Some(&connection), Instant::now());
        assert_eq!(state_of(&update, "binary_sensor.freebox_internet"), "on");
        assert_eq!(
            state_of(&update, "sensor.freebox_debit_descendant"),
            "1000.0"
        );
        assert_eq!(state_of(&update, "sensor.freebox_debit_montant"), "10.0");
        assert_eq!(mbit_per_s(0), "0.0");
        assert_ne!(mbit_per_s(u64::MAX), "");
    }
}
