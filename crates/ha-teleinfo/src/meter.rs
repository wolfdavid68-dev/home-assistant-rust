//! Ce qu'une trame veut dire : les index de consommation, la puissance, la
//! période tarifaire… dans les deux modes de la TIC.

use crate::frame::{Frame, Mode};

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
}
