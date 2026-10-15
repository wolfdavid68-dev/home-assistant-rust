//! Le décodage de la **carte de vigilance** de Météo-France (le produit
//! `CDP_CARTE_EXTERNE`), d'après son descriptif technique (version du
//! 11/09/2023).
//!
//! La carte donne, pour chaque département et chaque littoral (`67`,
//! `2910`…), le niveau de chaque phénomène (vent, orages, canicule…) sur
//! deux périodes : aujourd'hui (`J`) et demain (`J1`), avec les plages
//! horaires de chaque couleur. On ne garde que ce qui concerne la maison.

use jiff::Timestamp;
use serde::Deserialize;

/// Une vraie carte fait quelques centaines de Kio.
pub const MAX_BODY: usize = 4 * 1024 * 1024;
/// Au plus : deux périodes sont prévues (J et J1).
const MAX_PERIODS: usize = 4;

/// Le niveau de vigilance (la couleur).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Green,
    Yellow,
    Orange,
    Red,
}

impl Level {
    /// `color_id` : 1 vert, 2 jaune, 3 orange, 4 rouge.
    fn from_id(id: u64) -> Option<Self> {
        match id {
            1 => Some(Self::Green),
            2 => Some(Self::Yellow),
            3 => Some(Self::Orange),
            4 => Some(Self::Red),
            _ => None,
        }
    }

    /// Le nom, tel qu'il sert d'état : `Orange`.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Green => "Vert",
            Self::Yellow => "Jaune",
            Self::Orange => "Orange",
            Self::Red => "Rouge",
        }
    }
}

/// Les phénomènes suivis (`phenomenon_id` de 1 à 9).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Phenomenon {
    Wind,
    Rain,
    Storms,
    Floods,
    SnowIce,
    Heat,
    Cold,
    Avalanches,
    Waves,
}

impl Phenomenon {
    fn from_id(id: u64) -> Option<Self> {
        Some(match id {
            1 => Self::Wind,
            2 => Self::Rain,
            3 => Self::Storms,
            4 => Self::Floods,
            5 => Self::SnowIce,
            6 => Self::Heat,
            7 => Self::Cold,
            8 => Self::Avalanches,
            9 => Self::Waves,
            _ => return None,
        })
    }

    /// Le nom affiché, celui du site de Météo-France.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Wind => "Vent violent",
            Self::Rain => "Pluie-inondation",
            Self::Storms => "Orages",
            Self::Floods => "Crues",
            Self::SnowIce => "Neige-verglas",
            Self::Heat => "Canicule",
            Self::Cold => "Grand froid",
            Self::Avalanches => "Avalanches",
            Self::Waves => "Vagues-submersion",
        }
    }
}

/// Une plage horaire d'une même couleur.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Range {
    pub begin: Timestamp,
    pub end: Timestamp,
    pub level: Level,
}

/// Un phénomène sur la période, pour la maison.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hazard {
    pub phenomenon: Phenomenon,
    /// Le niveau le plus haut sur la période.
    pub level: Level,
    /// Les plages horaires, dans l'ordre (vides pour les crues).
    pub ranges: Vec<Range>,
    /// Le niveau vient du littoral du département, pas de l'intérieur.
    pub coast: bool,
}

impl Hazard {
    /// Le niveau à un instant donné.
    #[must_use]
    pub fn level_at(&self, now: Timestamp) -> Level {
        if self.ranges.is_empty() {
            return self.level;
        }
        self.ranges
            .iter()
            .find(|range| range.begin <= now && now < range.end)
            .map_or(Level::Green, |range| range.level)
    }
}

/// Une période de la carte (en général, un jour).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Period {
    pub begin: Timestamp,
    pub end: Timestamp,
    pub level: Level,
    /// Le plus grave d'abord.
    pub hazards: Vec<Hazard>,
}

impl Period {
    /// Le niveau à un instant donné, tous phénomènes confondus.
    #[must_use]
    pub fn level_at(&self, now: Timestamp) -> Level {
        self.hazards
            .iter()
            .map(|hazard| hazard.level_at(now))
            .max()
            .unwrap_or(Level::Green)
    }
}

/// Ce que la carte dit pour la maison.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Map {
    /// Quand Météo-France l'a publiée.
    pub updated: Timestamp,
    pub periods: Vec<Period>,
}

impl Map {
    /// La période en cours.
    ///
    /// On ne se fie pas aux noms `J` et `J1` : la carte de 16 h reste la
    /// dernière jusqu'à 6 h le lendemain, et après minuit sa période « J1 »
    /// est celle du jour.
    #[must_use]
    pub fn at(&self, now: Timestamp) -> Option<&Period> {
        self.periods
            .iter()
            .find(|period| period.begin <= now && now < period.end)
    }

    /// La période qui suit `period` (demain), si elle est déjà publiée.
    #[must_use]
    pub fn after(&self, period: &Period) -> Option<&Period> {
        self.periods.iter().find(|next| next.begin == period.end)
    }
}

// Le format brut. Les champs inconnus sont ignorés : Météo-France peut en
// ajouter sans tout casser.

#[derive(Deserialize)]
struct Raw {
    product: Product,
}

#[derive(Deserialize)]
struct Product {
    update_time: String,
    #[serde(default)]
    periods: Vec<RawPeriod>,
}

#[derive(Deserialize)]
struct RawPeriod {
    begin_validity_time: String,
    end_validity_time: String,
    timelaps: Timelaps,
}

#[derive(Deserialize)]
struct Timelaps {
    #[serde(default)]
    domain_ids: Vec<Domain>,
}

#[derive(Deserialize)]
struct Domain {
    #[serde(rename = "domain_id")]
    id: String,
    max_color_id: Id,
    #[serde(default)]
    phenomenon_items: Vec<Item>,
}

#[derive(Deserialize)]
struct Item {
    phenomenon_id: Id,
    phenomenon_max_color_id: Id,
    #[serde(default)]
    timelaps_items: Vec<Slot>,
}

#[derive(Deserialize)]
struct Slot {
    begin_time: String,
    end_time: String,
    color_id: Id,
}

/// Un identifiant, écrit `3` ou `"3"` selon les champs.
#[derive(Deserialize)]
#[serde(untagged)]
enum Id {
    Number(u64),
    Text(String),
}

impl Id {
    fn value(&self) -> Option<u64> {
        match self {
            Self::Number(value) => Some(*value),
            Self::Text(text) => text.trim().parse().ok(),
        }
    }
}

fn time(text: &str) -> Result<Timestamp, String> {
    text.parse()
        .map_err(|_| "date invalide dans la carte de vigilance".to_owned())
}

/// Décode la carte, pour un département (`67`, `2A`…).
///
/// # Errors
/// Réponse illisible, ou département absent de la carte.
pub fn parse(body: &[u8], department: &str) -> Result<Map, String> {
    if body.len() > MAX_BODY {
        return Err("carte de vigilance trop grosse".into());
    }
    let raw: Raw = serde_json::from_slice(body)
        .map_err(|error| format!("carte de vigilance illisible ({error})"))?;
    let coast = format!("{department}10");
    let mut periods = Vec::new();
    for raw_period in raw.product.periods.iter().take(MAX_PERIODS) {
        let begin = time(&raw_period.begin_validity_time)?;
        let end = time(&raw_period.end_validity_time)?;
        if begin >= end {
            continue;
        }
        let domains = &raw_period.timelaps.domain_ids;
        let Some(inland) = domains.iter().find(|d| d.id == department) else {
            return Err(format!("département {department} absent de la carte"));
        };
        let shore = domains.iter().find(|d| d.id == coast);
        let mut level = Level::Green;
        let mut hazards: Vec<Hazard> = Vec::new();
        for (domain, is_coast) in [(Some(inland), false), (shore, true)] {
            let Some(domain) = domain else { continue };
            if let Some(max) = domain.max_color_id.value().and_then(Level::from_id) {
                level = level.max(max);
            }
            for item in &domain.phenomenon_items {
                let Some(hazard) = hazard(item, is_coast) else {
                    continue;
                };
                level = level.max(hazard.level);
                // Le même phénomène à l'intérieur et sur le littoral : on
                // garde le plus grave.
                match hazards
                    .iter_mut()
                    .find(|known| known.phenomenon == hazard.phenomenon)
                {
                    Some(known) if known.level >= hazard.level => {}
                    Some(known) => *known = hazard,
                    None => hazards.push(hazard),
                }
            }
        }
        hazards.sort_by(|a, b| b.level.cmp(&a.level).then(a.phenomenon.cmp(&b.phenomenon)));
        periods.push(Period {
            begin,
            end,
            level,
            hazards,
        });
    }
    periods.sort_by_key(|period| period.begin);
    Ok(Map {
        updated: time(&raw.product.update_time)?,
        periods,
    })
}

/// Un phénomène. `None` s'il est inconnu (un ajout futur de Météo-France).
fn hazard(item: &Item, coast: bool) -> Option<Hazard> {
    let phenomenon = item.phenomenon_id.value().and_then(Phenomenon::from_id)?;
    let level = item
        .phenomenon_max_color_id
        .value()
        .and_then(Level::from_id)?;
    let mut ranges: Vec<Range> = item
        .timelaps_items
        .iter()
        .filter_map(|slot| {
            let begin = time(&slot.begin_time).ok()?;
            let end = time(&slot.end_time).ok()?;
            let level = slot.color_id.value().and_then(Level::from_id)?;
            (begin < end).then_some(Range { begin, end, level })
        })
        .collect();
    ranges.sort_by_key(|range| range.begin);
    Some(Hazard {
        phenomenon,
        level,
        ranges,
        coast,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Une carte réduite, au format du descriptif technique.
    const SAMPLE: &str = r#"{
      "product": {
        "warning_type": "vigilance", "type_cdp": "cdp_carte_externe",
        "version_vigilance": "V6", "version_cdp": "1.0.0",
        "update_time": "2026-10-15T14:00:00Z", "domain_id": "FRA",
        "global_max_color_id": 3,
        "periods": [
          {
            "echeance": "J1",
            "begin_validity_time": "2026-10-15T22:00:00Z",
            "end_validity_time": "2026-10-16T22:00:00Z",
            "timelaps": {"domain_ids": [
              {"domain_id": "67", "max_color_id": 1, "phenomenon_items": []},
              {"domain_id": "29", "max_color_id": 2, "phenomenon_items": [
                {"phenomenon_id": "1", "phenomenon_max_color_id": 2, "timelaps_items": [
                  {"begin_time": "2026-10-15T22:00:00Z", "end_time": "2026-10-16T06:00:00Z", "color_id": 2},
                  {"begin_time": "2026-10-16T06:00:00Z", "end_time": "2026-10-16T22:00:00Z", "color_id": 1}
                ]}
              ]}
            ]},
            "max_count_items": [], "per_phenomenon_items": []
          },
          {
            "echeance": "J",
            "begin_validity_time": "2026-10-14T22:00:00Z",
            "end_validity_time": "2026-10-15T22:00:00Z",
            "text_items": {"title": "commentaire carte", "text": ["Un coup de vent."]},
            "timelaps": {"domain_ids": [
              {"domain_id": "67", "max_color_id": 1, "phenomenon_items": []},
              {"domain_id": "29", "max_color_id": 2, "phenomenon_items": [
                {"phenomenon_id": "1", "phenomenon_max_color_id": 2, "timelaps_items": [
                  {"begin_time": "2026-10-14T22:00:00Z", "end_time": "2026-10-15T12:00:00Z", "color_id": 1},
                  {"begin_time": "2026-10-15T12:00:00Z", "end_time": "2026-10-15T22:00:00Z", "color_id": 2}
                ]},
                {"phenomenon_id": "3", "phenomenon_max_color_id": 1, "timelaps_items": []},
                {"phenomenon_id": "4", "phenomenon_max_color_id": 2, "timelaps_items": []},
                {"phenomenon_id": "42", "phenomenon_max_color_id": 3, "timelaps_items": []}
              ]},
              {"domain_id": "2910", "max_color_id": 3, "phenomenon_items": [
                {"phenomenon_id": 1, "phenomenon_max_color_id": 3, "timelaps_items": [
                  {"begin_time": "2026-10-15T16:00:00Z", "end_time": "2026-10-15T20:00:00Z", "color_id": 3}
                ]},
                {"phenomenon_id": "9", "phenomenon_max_color_id": 2, "timelaps_items": [
                  {"begin_time": "2026-10-15T18:00:00+00:00", "end_time": "2026-10-15T22:00:00+00:00", "color_id": "2"}
                ]}
              ]}
            ]},
            "max_count_items": [{"color_id": 2, "color_name": "Jaune", "count": 1, "text_count": "1 département en Jaune"}],
            "per_phenomenon_items": []
          }
        ]
      },
      "meta": {"snapshot_id": "x", "generation_timestamp": "2026-10-15T14:00:00Z", "product_datetime": "2026-10-15T14:00:00Z"}
    }"#;

    fn at(text: &str) -> Timestamp {
        text.parse().unwrap()
    }

    #[test]
    fn the_department_and_its_coast_are_merged() {
        let map = parse(SAMPLE.as_bytes(), "29").unwrap();
        assert_eq!(map.updated, at("2026-10-15T14:00:00Z"));
        assert_eq!(map.periods.len(), 2);
        let today = map.at(at("2026-10-15T17:00:00Z")).unwrap();
        // Le littoral est en orange : la maison aussi.
        assert_eq!(today.level, Level::Orange);
        let names: Vec<_> = today
            .hazards
            .iter()
            .map(|h| (h.phenomenon.name(), h.level.name(), h.coast))
            .collect();
        // Le plus grave d'abord ; le phénomène 42, inconnu, est ignoré.
        assert_eq!(
            names,
            [
                ("Vent violent", "Orange", true),
                ("Crues", "Jaune", false),
                ("Vagues-submersion", "Jaune", true),
                ("Orages", "Vert", false),
            ]
        );
        assert_eq!(today.level_at(at("2026-10-15T17:00:00Z")), Level::Orange);
        // Le matin, seules les crues (sans plages horaires) restent jaunes.
        assert_eq!(today.level_at(at("2026-10-15T08:00:00Z")), Level::Yellow);
    }

    #[test]
    fn today_and_tomorrow_follow_the_clock() {
        let map = parse(SAMPLE.as_bytes(), "29").unwrap();
        let today = map.at(at("2026-10-15T10:00:00Z")).unwrap();
        let tomorrow = map.after(today).unwrap();
        assert_eq!(tomorrow.level, Level::Yellow);
        // Après minuit (heure de Paris), « demain » devient aujourd'hui, et
        // on ne connaît pas encore le surlendemain.
        let night = at("2026-10-15T23:00:00Z");
        let today = map.at(night).unwrap();
        assert_eq!(today.begin, at("2026-10-15T22:00:00Z"));
        assert!(map.after(today).is_none());
        // Après la dernière période : plus rien.
        assert!(map.at(at("2026-10-17T00:00:00Z")).is_none());
    }

    #[test]
    fn a_quiet_department_is_green() {
        let map = parse(SAMPLE.as_bytes(), "67").unwrap();
        let today = map.at(at("2026-10-15T10:00:00Z")).unwrap();
        assert_eq!(today.level, Level::Green);
        assert_eq!(today.hazards, []);
    }

    #[test]
    fn bad_maps_are_refused() {
        // Un département absent de la carte est une erreur : la maison ne
        // doit pas se croire en vert.
        let error = parse(SAMPLE.as_bytes(), "75").unwrap_err();
        assert!(error.contains("75"), "{error}");
        assert!(parse(b"<html>", "29").is_err());
        assert!(parse(br#"{"product": {}}"#, "29").is_err());
        let bad_date = SAMPLE.replace("2026-10-15T14:00:00Z\", \"domain_id", "hier\", \"domain_id");
        assert!(parse(bad_date.as_bytes(), "29").is_err());
        assert!(parse(&vec![b' '; MAX_BODY + 1], "29").is_err());
    }
}
