//! Ce qu'une trame veut dire : les index de consommation, la puissance, la
//! période tarifaire… dans les deux modes de la TIC.

use crate::frame::{Frame, Mode};

/// La couleur d'un jour Tempo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Color {
    Blue,
    White,
    Red,
}

impl Color {
    /// L'état publié : « Bleu », « Blanc », « Rouge » (comme les niveaux de
    /// vigilance, pour écrire `to: Rouge` dans une automatisation).
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Blue => "Bleu",
            Self::White => "Blanc",
            Self::Red => "Rouge",
        }
    }

    /// Les deux bits d'une couleur dans le registre `STGE` (0 : pas
    /// d'annonce).
    fn from_bits(bits: u32) -> Option<Self> {
        match bits & 0b11 {
            1 => Some(Self::Blue),
            2 => Some(Self::White),
            3 => Some(Self::Red),
            _ => None,
        }
    }
}

/// Les couleurs Tempo que le compteur annonce. Un jour Tempo va de 6 h à
/// 6 h le lendemain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tempo {
    pub today: Option<Color>,
    /// `None` tant que le compteur n'a pas reçu la couleur de demain.
    pub tomorrow: Option<Color>,
}

/// Un index de consommation (en Wh, comme le compteur l'envoie).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Index {
    /// La fin de l'identifiant de l'entité : `heures_creuses`.
    pub key: String,
    /// Le nom affiché : « Heures creuses ».
    pub name: String,
    pub wh: u64,
}

/// Une lecture du compteur.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Reading {
    pub mode: Option<Mode>,
    /// Toute l'énergie consommée (somme des index), en Wh.
    pub total_wh: Option<u64>,
    pub indexes: Vec<Index>,
    /// Énergie injectée sur le réseau (production solaire), en Wh.
    pub injected_wh: Option<u64>,
    /// Puissance apparente consommée, en VA.
    pub power_va: Option<u64>,
    /// Puissance apparente injectée, en VA.
    pub injected_va: Option<u64>,
    /// Intensité (la plus forte des phases), en A.
    pub current_a: Option<u64>,
    /// Tension (phase 1), en V.
    pub voltage_v: Option<u64>,
    /// La période en cours : « Heures creuses ».
    pub period: Option<String>,
    /// L'option tarifaire : « Heures creuses », « Tempo »…
    pub tariff: Option<String>,
    /// « 30 A » (historique) ou « 6 kVA » (standard).
    pub subscribed: Option<String>,
    /// Le numéro du compteur, masqué : seuls les 4 derniers chiffres.
    pub meter: Option<String>,
    /// Les couleurs du jour et du lendemain, pour un contrat Tempo.
    pub tempo: Option<Tempo>,
}

/// Les index du mode historique : étiquette, clé, nom.
const HISTORIC_INDEXES: [(&str, &str, &str); 11] = [
    ("BASE", "base", "Base"),
    ("HCHC", "heures_creuses", "Heures creuses"),
    ("HCHP", "heures_pleines", "Heures pleines"),
    ("EJPHN", "ejp_normales", "EJP heures normales"),
    ("EJPHPM", "ejp_pointe", "EJP pointe mobile"),
    (
        "BBRHCJB",
        "tempo_bleu_creuses",
        "Tempo bleu, heures creuses",
    ),
    (
        "BBRHPJB",
        "tempo_bleu_pleines",
        "Tempo bleu, heures pleines",
    ),
    (
        "BBRHCJW",
        "tempo_blanc_creuses",
        "Tempo blanc, heures creuses",
    ),
    (
        "BBRHPJW",
        "tempo_blanc_pleines",
        "Tempo blanc, heures pleines",
    ),
    (
        "BBRHCJR",
        "tempo_rouge_creuses",
        "Tempo rouge, heures creuses",
    ),
    (
        "BBRHPJR",
        "tempo_rouge_pleines",
        "Tempo rouge, heures pleines",
    ),
];

/// Une valeur au plus raisonnable : 10⁹ kWh (un compteur domestique n'y
/// arrivera jamais). Au-delà, c'est une erreur de lecture.
const MAX_WH: u64 = 1_000_000_000_000;

fn masked(id: &str) -> Option<String> {
    let id = id.trim();
    let chars: Vec<char> = id.chars().collect();
    let tail: String = chars.iter().skip(chars.len().saturating_sub(4)).collect();
    (!tail.is_empty()).then(|| format!("…{tail}"))
}

/// « HEURES CREUSES » → « Heures creuses ».
fn sentence(text: &str) -> Option<String> {
    let lower = text.trim().to_lowercase();
    let mut chars = lower.chars();
    let first = chars.next()?;
    Some(first.to_uppercase().chain(chars).collect())
}

fn historic_period(ptec: &str) -> String {
    match ptec.trim_end_matches('.') {
        "TH" => "Toutes heures".into(),
        "HC" => "Heures creuses".into(),
        "HP" => "Heures pleines".into(),
        "HN" => "Heures normales".into(),
        "PM" => "Pointe mobile".into(),
        "HCJB" => "Heures creuses, jour bleu".into(),
        "HPJB" => "Heures pleines, jour bleu".into(),
        "HCJW" => "Heures creuses, jour blanc".into(),
        "HPJW" => "Heures pleines, jour blanc".into(),
        "HCJR" => "Heures creuses, jour rouge".into(),
        "HPJR" => "Heures pleines, jour rouge".into(),
        other => other.to_owned(),
    }
}

/// La couleur du jour d'après la période (`HPJR` : heures pleines, jour
/// rouge).
fn historic_today(ptec: &str) -> Option<Color> {
    match ptec.trim_end_matches('.').get(2..)? {
        "JB" => Some(Color::Blue),
        "JW" => Some(Color::White),
        "JR" => Some(Color::Red),
        _ => None,
    }
}

/// La couleur de demain (`DEMAIN`) : `----` tant qu'elle n'est pas connue.
fn historic_tomorrow(demain: &str) -> Option<Color> {
    match demain.trim() {
        "BLEU" => Some(Color::Blue),
        "BLAN" => Some(Color::White),
        "ROUG" => Some(Color::Red),
        _ => None,
    }
}

/// Les couleurs du registre d'état `STGE` (8 chiffres hexadécimaux) : bits
/// 24-25 pour aujourd'hui, 26-27 pour demain. Hors contrat Tempo, ils
/// restent à zéro : on ne publie alors rien.
fn standard_tempo(stge: &str) -> Option<Tempo> {
    let stge = stge.trim();
    // `from_str_radix` accepterait un signe « + » : on veut 8 chiffres.
    if stge.len() != 8 || !stge.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let bits = u32::from_str_radix(stge, 16).ok()?;
    let today = Color::from_bits(bits >> 24);
    let tomorrow = Color::from_bits(bits >> 26);
    (today.is_some() || tomorrow.is_some()).then_some(Tempo { today, tomorrow })
}

fn historic_tariff(optarif: &str) -> String {
    let optarif = optarif.trim_end_matches('.');
    if optarif.starts_with("BBR") {
        return "Tempo".into();
    }
    match optarif {
        "BASE" => "Base".into(),
        "HC" => "Heures creuses".into(),
        "EJP" => "EJP".into(),
        other => other.to_owned(),
    }
}

/// Les index du mode standard, selon le nom du tarif (`NGTF`).
fn standard_index(tariff: &str, number: usize) -> (String, String) {
    let tariff = tariff.to_uppercase();
    let named = if tariff.contains("TEMPO") {
        [
            "tempo_bleu_creuses",
            "tempo_bleu_pleines",
            "tempo_blanc_creuses",
            "tempo_blanc_pleines",
            "tempo_rouge_creuses",
            "tempo_rouge_pleines",
        ]
        .get(number.wrapping_sub(1))
        .copied()
    } else if tariff.contains("CREUSE") || tariff.contains("HC") {
        ["heures_creuses", "heures_pleines"]
            .get(number.wrapping_sub(1))
            .copied()
    } else if tariff.contains("BASE") {
        (number == 1).then_some("base")
    } else {
        None
    };
    match named {
        Some(key) => {
            let name = HISTORIC_INDEXES
                .iter()
                .find(|(_, k, _)| *k == key)
                .map_or(key, |(_, _, name)| name);
            (key.to_owned(), name.to_owned())
        }
        None => (format!("index_{number:02}"), format!("Index {number}")),
    }
}

impl Reading {
    /// Lit une trame.
    #[must_use]
    pub fn from_frame(frame: &Frame) -> Self {
        let wh = |label: &str| frame.number(label).filter(|v| *v <= MAX_WH);
        let mut reading = Self {
            mode: Some(frame.mode),
            ..Self::default()
        };
        match frame.mode {
            Mode::Historic => {
                for (label, key, name) in HISTORIC_INDEXES {
                    if let Some(value) = wh(label) {
                        reading.indexes.push(Index {
                            key: key.into(),
                            name: name.into(),
                            wh: value,
                        });
                    }
                }
                reading.power_va = frame.number("PAPP");
                reading.current_a = ["IINST", "IINST1", "IINST2", "IINST3"]
                    .iter()
                    .filter_map(|label| frame.number(label))
                    .max();
                reading.period = frame.get("PTEC").map(historic_period);
                reading.tariff = frame.get("OPTARIF").map(historic_tariff);
                reading.subscribed = frame.number("ISOUSC").map(|a| format!("{a} A"));
                reading.meter = frame.get("ADCO").and_then(masked);
                let tempo = frame.get("OPTARIF").is_some_and(|o| o.starts_with("BBR"));
                let today = frame.get("PTEC").and_then(historic_today);
                if tempo || today.is_some() {
                    reading.tempo = Some(Tempo {
                        today,
                        tomorrow: frame.get("DEMAIN").and_then(historic_tomorrow),
                    });
                }
            }
            Mode::Standard => {
                let tariff = frame.get("NGTF").unwrap_or_default();
                for number in 1..=10 {
                    let Some(value) = wh(&format!("EASF{number:02}")) else {
                        continue;
                    };
                    // Les index inutilisés restent à zéro.
                    if value == 0 {
                        continue;
                    }
                    let (key, name) = standard_index(tariff, number);
                    reading.indexes.push(Index {
                        key,
                        name,
                        wh: value,
                    });
                }
                reading.total_wh = wh("EAST");
                reading.injected_wh = wh("EAIT");
                reading.power_va = frame.number("SINSTS");
                reading.injected_va = frame.number("SINSTI");
                reading.current_a = ["IRMS1", "IRMS2", "IRMS3"]
                    .iter()
                    .filter_map(|label| frame.number(label))
                    .max();
                reading.voltage_v = frame.number("URMS1");
                reading.period = frame.get("LTARF").and_then(sentence);
                reading.tariff = sentence(tariff);
                reading.subscribed = frame.number("PREF").map(|kva| format!("{kva} kVA"));
                reading.meter = frame.get("ADSC").and_then(masked);
                reading.tempo = frame.get("STGE").and_then(standard_tempo);
            }
        }
        if reading.total_wh.is_none() && !reading.indexes.is_empty() {
            reading.total_wh = Some(reading.indexes.iter().map(|index| index.wh).sum());
        }
        reading
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing, reason = "tests")]

    use super::*;
    use crate::frame::Group;

    fn frame(mode: Mode, groups: &[(&str, &str)]) -> Frame {
        Frame {
            mode,
            groups: groups
                .iter()
                .map(|(label, value)| Group {
                    label: (*label).into(),
                    value: (*value).into(),
                    horodate: None,
                    mode,
                })
                .collect(),
        }
    }

    #[test]
    fn a_historic_off_peak_meter() {
        let reading = Reading::from_frame(&frame(
            Mode::Historic,
            &[
                ("ADCO", "031762120246"),
                ("OPTARIF", "HC.."),
                ("ISOUSC", "30"),
                ("HCHC", "012345678"),
                ("HCHP", "023456789"),
                ("PTEC", "HC.."),
                ("IINST", "004"),
                ("PAPP", "00920"),
            ],
        ));
        assert_eq!(reading.total_wh, Some(35_802_467));
        let keys: Vec<&str> = reading.indexes.iter().map(|i| i.key.as_str()).collect();
        assert_eq!(keys, ["heures_creuses", "heures_pleines"]);
        assert_eq!(reading.power_va, Some(920));
        assert_eq!(reading.current_a, Some(4));
        assert_eq!(reading.period.as_deref(), Some("Heures creuses"));
        assert_eq!(reading.tariff.as_deref(), Some("Heures creuses"));
        assert_eq!(reading.subscribed.as_deref(), Some("30 A"));
        // Le numéro du compteur identifie le foyer : il n'est jamais montré en entier.
        assert_eq!(reading.meter.as_deref(), Some("…0246"));
    }

    #[test]
    fn a_standard_meter_with_solar_panels() {
        let reading = Reading::from_frame(&frame(
            Mode::Standard,
            &[
                ("ADSC", "041876097521"),
                ("NGTF", "  H PLEINE/CREUSE"),
                ("LTARF", "  HEURES PLEINES"),
                ("EAST", "001600000"),
                ("EASF01", "000600000"),
                ("EASF02", "001000000"),
                ("EASF03", "000000000"),
                ("EAIT", "000250000"),
                ("IRMS1", "005"),
                ("URMS1", "232"),
                ("PREF", "06"),
                ("SINSTS", "01150"),
                ("SINSTI", "00000"),
            ],
        ));
        assert_eq!(reading.total_wh, Some(1_600_000));
        let keys: Vec<(&str, u64)> = reading
            .indexes
            .iter()
            .map(|i| (i.key.as_str(), i.wh))
            .collect();
        assert_eq!(
            keys,
            [("heures_creuses", 600_000), ("heures_pleines", 1_000_000)]
        );
        assert_eq!(reading.injected_wh, Some(250_000));
        assert_eq!(reading.voltage_v, Some(232));
        assert_eq!(reading.period.as_deref(), Some("Heures pleines"));
        assert_eq!(reading.subscribed.as_deref(), Some("6 kVA"));
        assert_eq!(reading.meter.as_deref(), Some("…7521"));
    }

    #[test]
    fn unknown_tariffs_and_absurd_values() {
        let reading = Reading::from_frame(&frame(
            Mode::Standard,
            &[
                ("NGTF", "SPECIAL"),
                ("EASF03", "000000100"),
                ("EAST", "999999999999999"),
            ],
        ));
        assert_eq!(reading.indexes[0].key, "index_03");
        assert_eq!(reading.indexes[0].name, "Index 3");
        // Une valeur absurde est ignorée : le total vient alors des index.
        assert_eq!(reading.total_wh, Some(100));
        let tempo = Reading::from_frame(&frame(
            Mode::Historic,
            &[("OPTARIF", "BBR("), ("PTEC", "HPJR")],
        ));
        assert_eq!(tempo.tariff.as_deref(), Some("Tempo"));
        assert_eq!(tempo.period.as_deref(), Some("Heures pleines, jour rouge"));
    }

    #[test]
    fn tempo_colors_in_historic_mode() {
        let colors =
            |groups: &[(&str, &str)]| Reading::from_frame(&frame(Mode::Historic, groups)).tempo;
        assert_eq!(
            colors(&[("OPTARIF", "BBR("), ("PTEC", "HCJB"), ("DEMAIN", "ROUG")]),
            Some(Tempo {
                today: Some(Color::Blue),
                tomorrow: Some(Color::Red)
            })
        );
        // Demain pas encore annoncé.
        assert_eq!(
            colors(&[("OPTARIF", "BBR("), ("PTEC", "HPJW"), ("DEMAIN", "----")]),
            Some(Tempo {
                today: Some(Color::White),
                tomorrow: None
            })
        );
        // Un autre contrat : pas de Tempo.
        assert_eq!(colors(&[("OPTARIF", "HC.."), ("PTEC", "HC..")]), None);
        assert_eq!(colors(&[("OPTARIF", "BASE"), ("PTEC", "TH..")]), None);
    }

    #[test]
    fn tempo_colors_in_standard_mode() {
        let colors = |stge: &str| {
            Reading::from_frame(&frame(Mode::Standard, &[("NGTF", "TEMPO"), ("STGE", stge)])).tempo
        };
        // Bits 24-25 : 01 (bleu) ; bits 26-27 : 11 (rouge) ; le reste (contact
        // sec, coupure…) ne compte pas.
        assert_eq!(
            colors("0D0A0001"),
            Some(Tempo {
                today: Some(Color::Blue),
                tomorrow: Some(Color::Red)
            })
        );
        assert_eq!(
            colors("02000000"),
            Some(Tempo {
                today: Some(Color::White),
                tomorrow: None
            })
        );
        // Hors Tempo, les bits restent à zéro ; une valeur abîmée est ignorée.
        assert_eq!(colors("003A0001"), None);
        assert_eq!(colors("0D0A00"), None);
        assert_eq!(colors("0D0A00ZZ"), None);
        assert_eq!(colors("+D0A0001"), None);
    }
}
