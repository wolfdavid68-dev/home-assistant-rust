//! **Assist** : comprendre une phrase en français, comme l'Assist de Home
//! Assistant, sans cloud et sans modèle d'IA (étape 54).
//!
//! « Ferme le volet de la chambre », « allume les lumières du salon »,
//! « mets le chauffage à 21 degrés », « quelle est la température du
//! salon ? », « qui est à la maison ? ».
//!
//! La méthode est volontairement simple et prévisible :
//!
//! 1. la phrase est mise en minuscules, sans accents ni ponctuation ;
//! 2. un **verbe** dit quoi faire (allume, éteins, ouvre, ferme, mets à…),
//!    ou c'est une **question** ;
//! 3. l'**appareil** est trouvé par son nom (tous ses mots sont dans la
//!    phrase), sinon par sa **pièce** et son **type** (« les lumières du
//!    salon ») ;
//! 4. une **valeur** facultative : « à 50 % », « à 21 degrés ».
//!
//! Ce module ne fait que comprendre : l'API exécute les ordres avec les
//! mêmes vérifications qu'un toucher dans le tableau de bord (droits du
//! compte, mot de passe redemandé, garde, valeurs dans la plage).

use std::collections::HashSet;

use ha_core::{Attributes, EntityId, State};
use serde_json::Value;

/// Une phrase plus longue est refusée.
pub const MAX_TEXT_CHARS: usize = 300;
/// Appareils commandés au plus par une phrase (« toutes les lumières »).
pub const MAX_TARGETS: usize = 30;

/// Un appareil que la personne a le droit de voir.
#[derive(Debug, Clone)]
pub struct Device {
    pub state: State,
    /// Son nom lisible (« Volet chambre »).
    pub name: String,
    /// Sa pièce (« Chambre »), si elle est rangée dans une pièce.
    pub area: Option<String>,
}

impl Device {
    fn domain(&self) -> &str {
        self.state.entity_id().domain()
    }

    /// Le genre d'appareil, pour les mots de la phrase : une prise Hue
    /// (`light` de classe `outlet`) est une **prise**, pas une lumière.
    fn kind(&self) -> &str {
        match self.domain() {
            "light" if self.device_class() == Some("outlet") => "switch",
            domain => domain,
        }
    }

    fn attr(&self, key: &str) -> Option<&Value> {
        self.state.attributes().get(key)
    }

    fn device_class(&self) -> Option<&str> {
        self.attr("device_class").and_then(Value::as_str)
    }
}

/// Un ordre à donner à un appareil.
#[derive(Debug, Clone, PartialEq)]
pub struct Order {
    pub domain: String,
    pub service: String,
    pub entity_id: EntityId,
    pub data: Attributes,
    pub name: String,
}

/// Ce qu'Assist a compris.
#[derive(Debug, Clone, PartialEq)]
pub enum Understood {
    /// Des ordres, un par appareil.
    Orders(Vec<Order>),
    /// Une réponse lue dans l'état de la maison.
    Answer(String),
}

/// Ce qu'Assist n'a pas compris, avec de quoi reformuler.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotUnderstood {
    Empty,
    TooLong,
    /// Ni verbe connu, ni question.
    NoAction,
    /// Aucun appareil ne correspond.
    NoDevice,
    /// Plusieurs appareils possibles : leurs noms.
    Ambiguous(Vec<String>),
    /// L'action ne s'applique pas à cet appareil.
    Unsupported(String),
    /// « Ajoute » sans rien à ajouter.
    NothingToAdd,
}

impl NotUnderstood {
    /// La réponse à afficher.
    #[must_use]
    pub fn message(&self) -> String {
        match self {
            Self::Empty => "Dis-moi ce que tu veux faire.".into(),
            Self::TooLong => format!("Phrase trop longue ({MAX_TEXT_CHARS} caractères au plus)."),
            Self::NoAction => "Je n'ai pas compris quoi faire. Essaie « allume », « éteins », \
                               « ouvre », « ferme », « mets … à 21 degrés », ou pose une question."
                .into(),
            Self::NoDevice => {
                "Je n'ai pas trouvé l'appareil. Donne son nom, ou sa pièce et son type \
                 (« les lumières du salon »)."
                    .into()
            }
            Self::Ambiguous(names) => format!("Lequel ? {}.", names.join(", ")),
            Self::Unsupported(name) => format!("Je ne sais pas faire ça avec {name}."),
            Self::NothingToAdd => {
                "Que faut-il ajouter ? « Ajoute du pain à la liste de courses ».".into()
            }
        }
    }
}

/// Minuscules, sans accents, sans ponctuation ; « 21,5 » reste un nombre,
/// « % » devient « pourcent » et « ° » « degres ».
#[must_use]
pub fn normalize(text: &str) -> String {
    let chars: Vec<char> = text.to_lowercase().chars().collect();
    let mut out = String::with_capacity(text.len());
    for (i, &c) in chars.iter().enumerate() {
        let folded = match c {
            'à' | 'â' | 'ä' | 'á' => "a",
            'é' | 'è' | 'ê' | 'ë' => "e",
            'î' | 'ï' | 'í' => "i",
            'ô' | 'ö' | 'ó' => "o",
            'ù' | 'û' | 'ü' | 'ú' => "u",
            'ç' => "c",
            'œ' => "oe",
            'æ' => "ae",
            'ÿ' => "y",
            '%' => " pourcent ",
            '°' => " degres ",
            ',' | '.'
                if i > 0
                    && chars[i - 1].is_ascii_digit()
                    && chars.get(i + 1).is_some_and(char::is_ascii_digit) =>
            {
                "."
            }
            c if c.is_ascii_alphanumeric() => {
                out.push(c);
                continue;
            }
            _ => " ",
        };
        out.push_str(folded);
    }
    let words: Vec<&str> = out.split_whitespace().collect();
    spoken_numbers(&words).join(" ")
}

/// La valeur d'un nombre dit en un mot (« vingt » → 20).
fn number_word(word: &str) -> Option<u32> {
    Some(match word {
        "zero" => 0,
        "un" | "une" => 1,
        "deux" => 2,
        "trois" => 3,
        "quatre" => 4,
        "cinq" => 5,
        "six" => 6,
        "sept" => 7,
        "huit" => 8,
        "neuf" => 9,
        "dix" => 10,
        "onze" => 11,
        "douze" => 12,
        "treize" => 13,
        "quatorze" => 14,
        "quinze" => 15,
        "seize" => 16,
        "vingt" | "vingts" => 20,
        "trente" => 30,
        "quarante" => 40,
        "cinquante" => 50,
        "soixante" => 60,
        "cent" | "cents" => 100,
        _ => return None,
    })
}

/// Un nombre entier écrit en chiffres ou en lettres.
fn whole(word: &str) -> Option<u32> {
    number_word(word).or_else(|| {
        word.parse::<u32>()
            .ok()
            .filter(|_| word.bytes().all(|b| b.is_ascii_digit()))
    })
}

/// Lit un nombre dit en mots à partir de `words[start]` : sa valeur et le
/// nombre de mots lus. « vingt et un », « quatre vingt dix », « cent
/// vingt », « dix neuf », et « 20 et 1 » (ce que la reconnaissance vocale
/// écrit parfois pour « vingt et un »).
fn read_number(words: &[&str], start: usize) -> Option<(u32, usize)> {
    let word = |i: usize| words.get(start + i).copied();
    let mut total = whole(word(0)?)?;
    let mut used = 1;
    // « quatre vingt » : 80.
    if total == 4 && matches!(word(1), Some("vingt" | "vingts")) {
        total = 80;
        used = 2;
    }
    // « deux cents », « cent ».
    if total < 10 && matches!(word(used), Some("cent" | "cents")) {
        total = total.max(1) * 100;
        used += 1;
    }
    // Après les centaines : la suite (« cent vingt et un »).
    if total >= 100 && total % 100 == 0 {
        if let Some((rest, n)) = read_number(words, start + used).filter(|(rest, _)| *rest < 100) {
            total += rest;
            used += n;
        }
        return Some((total, used));
    }
    // Les dizaines, puis les unités : « vingt deux », « vingt et un »,
    // « soixante et onze », « quatre vingt dix neuf ».
    let tens = total % 100;
    let teens_allowed = tens == 60 || tens == 80;
    if (20..=90).contains(&tens) && tens % 10 == 0 {
        let (and, next) = match word(used) {
            Some("et") => (1, word(used + 1)),
            other => (0, other),
        };
        // Écrit en chiffres (« 20 et 1 ») : seulement avec « et ».
        let digits = number_word(words[start]).is_none();
        if let Some(unit) = next.and_then(whole).filter(|_| !digits || and == 1) {
            let ok = if and == 1 {
                unit == 1 || (teens_allowed && unit == 11)
            } else {
                (1..=9).contains(&unit) || (teens_allowed && (10..=19).contains(&unit))
            };
            if ok {
                total += unit;
                used += and + 1;
                // « soixante dix sept » : 77.
                if teens_allowed
                    && unit == 10
                    && let Some(more) = word(used).and_then(number_word)
                    && (1..=9).contains(&more)
                {
                    total += more;
                    used += 1;
                }
            }
        }
    } else if total == 10
        && let Some(unit) = word(used).and_then(number_word)
        && (7..=9).contains(&unit)
    {
        // « dix sept ».
        total += unit;
        used += 1;
    }
    Some((total, used))
}

/// Remplace les nombres dits en lettres par des chiffres : « à vingt et un
/// degres » → « a 21 degres », « vingt et un et demi » → « 21.5 ». « un »
/// et « une » seuls restent des mots (« allume une lampe »).
fn spoken_numbers(words: &[&str]) -> Vec<String> {
    let mut out = Vec::with_capacity(words.len());
    let mut i = 0;
    while i < words.len() {
        // « pour cent » n'est pas un nombre.
        if words[i] == "pour" && words.get(i + 1) == Some(&"cent") {
            out.push("pourcent".to_owned());
            i += 2;
            continue;
        }
        let spelled = number_word(words[i]).is_some();
        let half_after = |next: usize| {
            matches!(
                (words.get(next), words.get(next + 1)),
                (Some(&"et"), Some(&("demi" | "demie")))
            )
        };
        match read_number(words, i) {
            Some((n, used))
                if (spelled || used > 1 || half_after(i + used))
                    && !(used == 1 && matches!(words[i], "un" | "une")) =>
            {
                i += used;
                let half = half_after(i);
                if half {
                    i += 2;
                    out.push(format!("{n}.5"));
                } else {
                    out.push(n.to_string());
                }
            }
            _ => {
                out.push(words[i].to_owned());
                i += 1;
            }
        }
    }
    out
}

/// Les petits mots qui ne servent pas à trouver un appareil.
const STOP_WORDS: [&str; 33] = [
    "le", "la", "les", "l", "un", "une", "des", "de", "du", "d", "au", "aux", "a", "et", "en",
    "mon", "ma", "mes", "ton", "ta", "tes", "notre", "nos", "tout", "toute", "tous", "toutes",
    "stp", "svp", "s", "il", "te", "plait",
];

/// « volets » → « volet », « bureaux » → « bureau ».
fn stem(word: &str) -> &str {
    if word.len() > 3 && (word.ends_with('s') || word.ends_with('x')) {
        &word[..word.len() - 1]
    } else {
        word
    }
}

/// Les mots qui comptent, au singulier.
fn keywords(normalized: &str) -> Vec<String> {
    normalized
        .split(' ')
        .filter(|word| !word.is_empty() && !STOP_WORDS.contains(word))
        .map(|word| stem(word).to_owned())
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Verb {
    On,
    Off,
    Open,
    Close,
    Stop,
    Lock,
    Unlock,
    Set,
}

fn verb(words: &[&str]) -> Option<Verb> {
    words.iter().find_map(|word| {
        Some(match *word {
            "allume" | "allumer" | "allumez" | "active" | "activer" | "activez" | "demarre"
            | "demarrer" | "lance" | "lancer" | "lancez" => Verb::On,
            "eteins" | "eteint" | "eteindre" | "eteignez" | "coupe" | "couper" | "coupez"
            | "desactive" | "desactiver" | "desactivez" => Verb::Off,
            "ouvre" | "ouvrir" | "ouvrez" | "monte" | "monter" | "montez" | "remonte"
            | "remonter" | "leve" | "lever" => Verb::Open,
            "ferme" | "fermer" | "fermez" | "descends" | "descendre" | "descendez" | "baisse"
            | "baisser" | "baissez" => Verb::Close,
            "arrete" | "arreter" | "arretez" | "stoppe" | "stop" => Verb::Stop,
            "verrouille" | "verrouiller" | "verrouillez" => Verb::Lock,
            "deverrouille" | "deverrouiller" | "deverrouillez" => Verb::Unlock,
            "mets" | "met" | "mettre" | "mettez" | "regle" | "regler" | "reglez" | "passe"
            | "passer" => Verb::Set,
            _ => return None,
        })
    })
}

/// Les types d'appareils que désignent certains mots.
fn domain_hint(word: &str) -> Option<&'static [&'static str]> {
    Some(match word {
        "lumiere" | "lampe" | "eclairage" | "plafonnier" | "spot" | "ampoule" | "applique"
        | "guirlande" => &["light"],
        "volet" | "store" | "rideau" | "portail" => &["cover"],
        "prise" => &["switch"],
        "chauffage" | "radiateur" | "thermostat" | "clim" | "climatisation" => &["climate"],
        "ventilateur" | "ventilo" => &["fan"],
        "scene" | "ambiance" => &["scene"],
        "serrure" | "verrou" => &["lock"],
        "vanne" | "arrosage" => &["valve"],
        _ => return None,
    })
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Unit {
    Percent,
    Degrees,
    None,
}

/// « à 50 % », « à 21,5 degrés », « à 30 ». Un nombre ne compte que
/// derrière « à », ou suivi d'une unité : celui d'un nom (« la chambre 2 »,
/// « la lampe 2 ») n'est pas une valeur.
fn value(words: &[&str]) -> Option<(f64, Unit)> {
    words.iter().enumerate().find_map(|(index, word)| {
        let number = word.parse::<f64>().ok().filter(|n| n.is_finite())?;
        let unit = match words.get(index + 1).copied() {
            Some("pourcent") => Unit::Percent,
            Some("pour") if words.get(index + 2) == Some(&"cent") => Unit::Percent,
            Some("degre" | "degres") => Unit::Degrees,
            _ => Unit::None,
        };
        // « 21 degres et demi ».
        let half = unit == Unit::Degrees
            && words.get(index + 2) == Some(&"et")
            && matches!(words.get(index + 3).copied(), Some("demi" | "demie"));
        let number = if half { number + 0.5 } else { number };
        let after_a = index > 0 && words[index - 1] == "a";
        (after_a || unit != Unit::None).then_some((number, unit))
    })
}

fn data(pairs: &[(&str, Value)]) -> Attributes {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect()
}

/// Le nombre, entier quand il l'est (« 50 » plutôt que « 50.0 »).
fn number_value(n: f64) -> Value {
    if n.fract() == 0.0 && n.abs() < 1e9 {
        #[allow(
            clippy::cast_possible_truncation,
            reason = "entier vérifié juste avant"
        )]
        let whole = n as i64;
        Value::from(whole)
    } else {
        Value::from(n)
    }
}

/// Le service à appeler pour `verb` sur un appareil de ce domaine.
fn service(
    domain: &str,
    verb: Verb,
    value: Option<(f64, Unit)>,
) -> Option<(&'static str, Attributes)> {
    let none = Attributes::new;
    let amount = value.map(|(n, _)| number_value(n));
    let unit = value.map(|(_, unit)| unit);
    Some(match (domain, verb) {
        ("light", Verb::On | Verb::Set) => match (amount, unit) {
            (Some(n), Some(Unit::Percent | Unit::None)) => {
                ("turn_on", data(&[("brightness_pct", n)]))
            }
            (Some(_), Some(Unit::Degrees)) => return None,
            _ if verb == Verb::Set => return None,
            _ => ("turn_on", none()),
        },
        (
            "light" | "switch" | "fan" | "input_boolean" | "humidifier" | "climate"
            | "water_heater",
            Verb::Off | Verb::Stop,
        ) => ("turn_off", none()),
        ("switch" | "input_boolean" | "humidifier" | "climate" | "water_heater", Verb::On)
        | ("scene" | "script", Verb::On | Verb::Set) => ("turn_on", none()),
        ("fan", Verb::On | Verb::Set) => match (amount, unit) {
            (Some(n), Some(Unit::Percent | Unit::None)) => {
                ("set_percentage", data(&[("percentage", n)]))
            }
            _ if verb == Verb::Set => return None,
            _ => ("turn_on", none()),
        },
        ("cover", Verb::Open | Verb::Set | Verb::Close) if amount.is_some() => match unit {
            Some(Unit::Degrees) => return None,
            _ => ("set_cover_position", data(&[("position", amount?)])),
        },
        ("cover", Verb::Open) => ("open_cover", none()),
        ("cover", Verb::Close) => ("close_cover", none()),
        ("cover", Verb::Stop) => ("stop_cover", none()),
        ("valve", Verb::Open) => ("open_valve", none()),
        ("valve", Verb::Close | Verb::Off) => ("close_valve", none()),
        ("climate" | "water_heater", Verb::Set) => match (amount, unit) {
            (Some(n), Some(Unit::Degrees | Unit::None)) => {
                ("set_temperature", data(&[("temperature", n)]))
            }
            _ => return None,
        },
        ("lock", Verb::Lock | Verb::Close) => ("lock", none()),
        ("lock", Verb::Unlock | Verb::Open) => ("unlock", none()),
        ("button" | "input_button", Verb::On) => ("press", none()),
        _ => return None,
    })
}

/// Les domaines visés par défaut quand on ne nomme que la pièce
/// (« éteins le salon »).
fn default_domains(verb: Option<Verb>, value: Option<(f64, Unit)>) -> &'static [&'static str] {
    match (verb, value.map(|(_, unit)| unit)) {
        (_, Some(Unit::Degrees)) => &["climate"],
        (Some(Verb::Open | Verb::Close | Verb::Stop), _) => &["cover"],
        (Some(Verb::Lock | Verb::Unlock), _) => &["lock"],
        _ => &["light"],
    }
}

/// Ce que la phrase désigne.
struct Target<'a> {
    devices: Vec<&'a Device>,
}

/// Trouve les appareils désignés par `words`.
fn find<'a>(
    devices: &'a [Device],
    words: &[String],
    plural: bool,
    hint: Option<&[&str]>,
    fallback: &[&str],
) -> Result<Target<'a>, NotUnderstood> {
    let present: HashSet<&str> = words.iter().map(String::as_str).collect();
    let all_present = |text: &str| {
        let wanted = keywords(&normalize(text));
        !wanted.is_empty() && wanted.iter().all(|word| present.contains(word.as_str()))
    };
    let allowed = |device: &&Device| hint.is_none_or(|domains| domains.contains(&device.kind()));
    // La pièce la plus précise nommée dans la phrase.
    let area = devices
        .iter()
        .filter_map(|device| device.area.as_deref())
        .filter(|area| all_present(area))
        .max_by_key(|area| keywords(&normalize(area)).len());
    let area_words: Vec<String> = area.map(|a| keywords(&normalize(a))).unwrap_or_default();

    // 1. Par le nom, ou par l'identifiant (« salon » dans `cover.salon`) :
    //    tous ses mots sont dans la phrase. Un nom qui n'est que celui de la
    //    pièce (« Salon ») ne compte que si le type est dit.
    let matched = |device: &Device| -> Option<Vec<String>> {
        let object = device.state.entity_id().object_id().replace('_', " ");
        [
            keywords(&normalize(&device.name)),
            keywords(&normalize(&object)),
        ]
        .into_iter()
        .filter(|wanted| {
            !wanted.is_empty() && wanted.iter().all(|word| present.contains(word.as_str()))
        })
        .max_by_key(Vec::len)
    };
    let named: Vec<(&Device, usize)> = devices
        .iter()
        .filter(allowed)
        .filter_map(|device| matched(device).map(|words| (device, words)))
        .filter(|(_, words)| hint.is_some() || !words.iter().all(|word| area_words.contains(word)))
        .map(|(device, words)| (device, words.len()))
        .collect();
    if let Some(longest) = named.iter().map(|(_, len)| *len).max() {
        let mut devices: Vec<&Device> = named
            .into_iter()
            .filter(|(_, len)| *len == longest)
            .map(|(device, _)| device)
            .collect();
        // La pièce dite dans la phrase départage les appareils du même nom.
        if let Some(area) = area
            && devices.iter().any(|d| d.area.as_deref() == Some(area))
        {
            devices.retain(|d| d.area.as_deref() == Some(area));
        }
        // Plusieurs appareils, au singulier : lequel ?
        if devices.len() > 1 && !plural {
            return Err(NotUnderstood::Ambiguous(
                devices.iter().take(6).map(|device| label(device)).collect(),
            ));
        }
        return Ok(Target { devices });
    }

    // 2. Par la pièce et le type : « les lumières du salon ».
    let domains = hint.unwrap_or(fallback);
    if let Some(area) = area {
        let devices: Vec<&Device> = devices
            .iter()
            .filter(|device| device.area.as_deref() == Some(area))
            .filter(|device| domains.contains(&device.kind()))
            .collect();
        return if devices.is_empty() {
            Err(NotUnderstood::NoDevice)
        } else {
            Ok(Target { devices })
        };
    }

    // 3. Par le type seul : « toutes les lumières », ou le seul volet.
    if hint.is_some() {
        let devices: Vec<&Device> = devices
            .iter()
            .filter(|device| domains.contains(&device.kind()))
            .collect();
        return match devices.len() {
            0 => Err(NotUnderstood::NoDevice),
            1 => Ok(Target { devices }),
            _ if plural => Ok(Target { devices }),
            _ => Err(NotUnderstood::Ambiguous(
                devices
                    .iter()
                    .take(6)
                    .map(|device| device.name.clone())
                    .collect(),
            )),
        };
    }
    Err(NotUnderstood::NoDevice)
}

/// Comprend `text` parmi les appareils que la personne peut voir.
pub fn understand(text: &str, devices: &[Device]) -> Result<Understood, NotUnderstood> {
    if text.chars().count() > MAX_TEXT_CHARS {
        return Err(NotUnderstood::TooLong);
    }
    let normalized = normalize(text);
    if normalized.is_empty() {
        return Err(NotUnderstood::Empty);
    }
    let raw: Vec<&str> = normalized.split(' ').collect();
    let words = keywords(&normalized);
    let plural = raw
        .iter()
        .any(|word| matches!(*word, "les" | "tous" | "toutes" | "tout"));
    let hint = words.iter().find_map(|word| domain_hint(word));

    if let Some(added) = add_to_list(text, devices) {
        return added.map(|order| Understood::Orders(vec![order]));
    }
    if is_question(text, &raw) {
        return Ok(Understood::Answer(answer(
            devices, &raw, &words, plural, hint,
        )));
    }
    let verb = verb(&raw).ok_or(NotUnderstood::NoAction)?;
    let value = value(&raw);
    let target = find(
        devices,
        &words,
        plural,
        hint,
        default_domains(Some(verb), value),
    )?;
    let mut orders = Vec::new();
    for device in target.devices.into_iter().take(MAX_TARGETS) {
        let (service, data) = service(device.domain(), verb, value)
            .ok_or_else(|| NotUnderstood::Unsupported(device.name.clone()))?;
        orders.push(Order {
            domain: device.domain().to_owned(),
            service: service.to_owned(),
            entity_id: device.state.entity_id().clone(),
            data,
            name: device.name.clone(),
        });
    }
    Ok(Understood::Orders(orders))
}

/// Une chose à ajouter fait au plus ces caractères.
pub const MAX_ITEM_CHARS: usize = 100;

/// Les verbes qui ajoutent à une liste, même sans la nommer.
const ADD_VERBS: &[&str] = &[
    "ajoute",
    "ajouter",
    "ajoutez",
    "rajoute",
    "rajouter",
    "rajoutez",
    "note",
    "noter",
    "notez",
    "inscris",
    "inscrire",
    "inscrivez",
];
/// Les verbes qui n'ajoutent que si la liste est nommée (« mets du pain sur
/// la liste de courses »).
const PUT_VERBS: &[&str] = &["mets", "met", "mettre", "mettez"];
/// Les mots qui annoncent la liste : « à la liste », « aux courses ».
const LIST_MARKERS: &[&str] = &["a", "au", "aux", "sur", "dans", "en"];
/// Les mots qu'on peut dire autour du nom de la liste.
const LIST_EXTRA: &[&str] = &["liste", "merci", "vous"];
/// Les petits mots retirés devant la chose (« du pain » → « pain »).
const ITEM_ARTICLES: &[&str] = &[
    "du", "de", "la", "le", "les", "l", "des", "un", "une", "d", "quelques", "encore",
];
/// Les mots de politesse retirés à la fin de la chose.
const ITEM_POLITE: &[&str] = &["stp", "svp", "merci", "plait", "te", "vous", "il", "s"];

/// Les mots de la phrase d'origine, accents et majuscules gardés, coupés
/// après l'apostrophe (« l'huile » → « l' », « huile »).
fn tokens(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for word in text.split_whitespace() {
        let mut rest = word;
        while let Some(cut) = rest.find(['\'', '’']) {
            let (head, tail) = rest.split_at(cut);
            let apostrophe = tail.chars().next().map_or(1, char::len_utf8);
            out.push(format!("{head}'"));
            rest = &tail[apostrophe..];
        }
        out.push(rest.to_owned());
    }
    out.into_iter()
        .map(|token| {
            token
                .trim_matches(|c: char| ",.;:!?«»\"()[]".contains(c) || c.is_whitespace())
                .to_owned()
        })
        .filter(|token| !token.is_empty())
        .collect()
}

/// Les mots qui comptent dans le nom d'une liste (« Liste de courses » →
/// « course »).
fn list_words(device: &Device) -> Vec<String> {
    let object = device.state.entity_id().object_id().replace('_', " ");
    let mut words = keywords(&normalize(&device.name));
    words.extend(keywords(&normalize(&object)));
    words.retain(|word| word != "liste");
    words
}

/// La liste dite dans la phrase.
#[derive(Clone, Copy)]
enum ListSaid<'a> {
    /// « la liste », sans dire laquelle.
    Unnamed,
    Named(&'a Device),
}

impl<'a> ListSaid<'a> {
    fn device(self) -> Option<&'a Device> {
        match self {
            Self::Unnamed => None,
            Self::Named(device) => Some(device),
        }
    }
}

/// Les mots `said` désignent-ils une liste ?
fn which_list<'a>(lists: &[&'a Device], said: &[String]) -> Option<ListSaid<'a>> {
    let said: Vec<&String> = said
        .iter()
        .filter(|word| !LIST_EXTRA.contains(&word.as_str()))
        .collect();
    if said.is_empty() {
        return Some(ListSaid::Unnamed);
    }
    lists
        .iter()
        .filter(|list| {
            let words = list_words(list);
            said.iter().all(|word| words.contains(word))
        })
        .max_by_key(|list| list_words(list).len())
        .map(|list| ListSaid::Named(list))
}

/// La chose à ajouter, propre : sans article devant, sans politesse
/// derrière, avec une majuscule (« du pain de mie » → « Pain de mie »).
fn item_text(tokens: &[String]) -> Option<String> {
    let norm = |token: &String| normalize(token);
    let mut start = 0;
    let mut end = tokens.len();
    while end > start && ITEM_POLITE.contains(&norm(&tokens[end - 1]).as_str()) {
        end -= 1;
    }
    while start + 1 < end && ITEM_ARTICLES.contains(&norm(&tokens[start]).as_str()) {
        start += 1;
    }
    let mut item = String::new();
    for token in &tokens[start..end] {
        if !item.is_empty() && !item.ends_with('\'') {
            item.push(' ');
        }
        item.push_str(token);
    }
    let mut chars = item.chars();
    let first = chars.next()?;
    Some(first.to_uppercase().chain(chars).collect())
}

/// « Ajoute du pain à la liste de courses », « note appeler le plombier
/// dans les tâches », « mets du lait sur la liste ». `None` : la phrase ne
/// parle pas d'ajouter à une liste.
fn add_to_list(text: &str, devices: &[Device]) -> Option<Result<Order, NotUnderstood>> {
    let tokens = tokens(text);
    let norms: Vec<String> = tokens.iter().map(|token| normalize(token)).collect();
    // Le verbe, au début de la phrase (« tu peux ajouter… »).
    let (verb_at, strong) = norms.iter().take(4).enumerate().find_map(|(i, word)| {
        if ADD_VERBS.contains(&word.as_str()) {
            Some((i, true))
        } else if PUT_VERBS.contains(&word.as_str()) {
            Some((i, false))
        } else {
            None
        }
    })?;
    let lists: Vec<&Device> = devices.iter().filter(|d| d.domain() == "todo").collect();
    if lists.is_empty() {
        return strong.then_some(Err(NotUnderstood::NoDevice));
    }
    let said = |range: &[String]| keywords(&normalize(&range.join(" ")));
    let after = verb_at + 1;
    let marker = |i: usize| LIST_MARKERS.contains(&norms[i].as_str());
    // 1. « ajoute <chose> à <liste> » : le premier « à » qui n'est suivi
    //    que du nom d'une liste.
    let mut found = (after + 1..tokens.len())
        .filter(|&i| marker(i))
        .find_map(|i| {
            let named = said(&tokens[i + 1..]);
            // Sans « ajoute », la liste doit être dite (« mets … à » ne suffit pas).
            if !strong && named.is_empty() {
                return None;
            }
            which_list(&lists, &named).map(|list| (list, &tokens[after..i]))
        });
    // 2. « ajoute à <liste> <chose> ».
    if found.is_none() && after < tokens.len() && marker(after) {
        found = (after + 2..tokens.len()).rev().find_map(|end| {
            let named = said(&tokens[after + 1..end]);
            if named.is_empty() {
                return None;
            }
            which_list(&lists, &named)
                .filter(|list| list.device().is_some())
                .map(|list| (list, &tokens[end..]))
        });
    }
    let (list, item) = match found {
        Some(found) => found,
        None if strong => (ListSaid::Unnamed, &tokens[after..]),
        None => return None,
    };
    // Sans liste nommée : celle des courses, ou la seule.
    let list = list
        .device()
        .or_else(|| {
            lists
                .iter()
                .find(|list| list_words(list).iter().any(|word| word == "course"))
                .copied()
        })
        .or_else(|| (lists.len() == 1).then(|| lists[0]));
    let Some(list) = list else {
        return Some(Err(NotUnderstood::Ambiguous(
            lists.iter().take(6).map(|list| list.name.clone()).collect(),
        )));
    };
    let Some(item) = item_text(item) else {
        return Some(Err(NotUnderstood::NothingToAdd));
    };
    if item.chars().count() > MAX_ITEM_CHARS {
        return Some(Err(NotUnderstood::TooLong));
    }
    Some(Ok(Order {
        domain: "todo".into(),
        service: "add_item".into(),
        entity_id: list.state.entity_id().clone(),
        data: data(&[("item", Value::from(item))]),
        name: list.name.clone(),
    }))
}

fn is_question(text: &str, raw: &[&str]) -> bool {
    text.trim_end().ends_with('?')
        || matches!(
            raw.first().copied(),
            Some("quel" | "quelle" | "quels" | "quelles" | "combien" | "qui" | "est" | "y")
        )
        || raw
            .windows(2)
            .any(|pair| matches!(pair, ["est", "il" | "elle"] | ["sont", "ils" | "elles"]))
}

/// Les mots d'une question qui ne désignent rien.
const QUESTION_WORDS: &[&str] = &[
    // Les mots sont déjà au singulier (« quelles » → « quelle »).
    "quel",
    "quelle",
    "y",
    "a",
    "combien",
    "est",
    "ce",
    "que",
    "qu",
    "fait",
    "il",
    "temperature",
    "humidite",
    "chaud",
    "chaude",
    "froid",
    "froide",
    "maison",
];

/// La réponse à une question.
fn answer(
    devices: &[Device],
    raw: &[&str],
    words: &[String],
    plural: bool,
    hint: Option<&[&str]>,
) -> String {
    // « Qui est à la maison ? »
    if raw.first() == Some(&"qui") {
        let people: Vec<&Device> = devices.iter().filter(|d| d.domain() == "person").collect();
        if people.is_empty() {
            return "Je ne peux pas te le dire.".into();
        }
        let home: Vec<&str> = people
            .iter()
            .filter(|person| person.state.state() == "home")
            .map(|person| person.name.as_str())
            .collect();
        return if home.is_empty() {
            "Personne n'est à la maison.".into()
        } else {
            format!("À la maison : {}.", home.join(", "))
        };
    }
    // « Quelle est la température du salon ? »
    let measure = if words.iter().any(|w| {
        matches!(
            w.as_str(),
            "temperature" | "chaud" | "chaude" | "froid" | "froide"
        )
    }) {
        Some("temperature")
    } else if words.iter().any(|w| w == "humidite") {
        Some("humidity")
    } else {
        None
    };
    if let Some(class) = measure {
        let sensors: Vec<Device> = devices
            .iter()
            .filter(|d| d.domain() == "sensor" && d.device_class() == Some(class))
            .cloned()
            .collect();
        // Sans pièce ni nom (« quelle température fait-il ? ») : toutes les
        // mesures. Avec un mot inconnu (« la piscine ») : aucune.
        let vague = words
            .iter()
            .all(|word| QUESTION_WORDS.contains(&word.as_str()));
        let found = match find(&sensors, words, true, None, &["sensor"]) {
            Ok(target) => target.devices,
            Err(_) if vague => sensors.iter().collect(),
            Err(_) => Vec::new(),
        };
        if found.is_empty() {
            return "Je n'ai pas trouvé de mesure.".into();
        }
        return list(&found);
    }
    // « Le linge est fini ? », « où en est la machine ? » : les machines
    // suivies (lave-linge SmartThings, étape 86).
    let wanted = |machine: &str| -> bool {
        words.iter().any(|w| match w.as_str() {
            "vaisselle" => machine == "lave-vaisselle",
            "seche" | "sechage" | "sechoir" => machine == "sèche-linge",
            "linge" | "lessive" => machine != "lave-vaisselle",
            "machine" => true,
            _ => false,
        })
    };
    let machines: Vec<&Device> = devices
        .iter()
        .filter(|d| {
            d.attr("appareil")
                .and_then(Value::as_str)
                .is_some_and(wanted)
        })
        .collect();
    if !machines.is_empty() {
        return list(&machines);
    }
    match find(devices, words, plural, hint, &["light", "cover", "switch"]) {
        Ok(target) if !target.devices.is_empty() => list(&target.devices),
        Ok(_) | Err(NotUnderstood::NoDevice) => NotUnderstood::NoDevice.message(),
        Err(other) => other.message(),
    }
}

fn list(devices: &[&Device]) -> String {
    let parts: Vec<String> = devices
        .iter()
        .take(MAX_TARGETS)
        .map(|device| format!("{} : {}", label(device), describe(device)))
        .collect();
    format!("{}.", parts.join(" ; "))
}

/// Le nom, avec la pièce quand il ne la dit pas (« Température (Salon) »).
fn label(device: &Device) -> String {
    match &device.area {
        Some(area) => {
            let name = keywords(&normalize(&device.name));
            let room = keywords(&normalize(area));
            if room.iter().all(|word| name.contains(word)) {
                device.name.clone()
            } else {
                format!("{} ({area})", device.name)
            }
        }
        None => device.name.clone(),
    }
}

/// La valeur demandée, pour la réponse : « à 40 % », « à 21 °C ».
#[must_use]
pub fn amount(order: &Order) -> Option<String> {
    let (field, unit) = [
        ("position", " %"),
        ("brightness_pct", " %"),
        ("percentage", " %"),
        ("temperature", " °C"),
    ]
    .into_iter()
    .find(|(field, _)| order.data.contains_key(*field))?;
    let n = order.data.get(field)?.as_f64()?;
    Some(format!("à {}{unit}", show(n)))
}

/// L'état d'un appareil, en français.
#[must_use]
pub fn describe(device: &Device) -> String {
    let state = device.state.state();
    // Une machine (lave-linge SmartThings, étape 86) : « en marche, fini vers 11:20 ».
    if device.attr("appareil").is_some() {
        let lower = state.to_lowercase();
        return match device.attr("fin_heure").and_then(Value::as_str) {
            Some(end) if matches!(state, "En marche" | "En pause") => {
                format!("{lower}, fini vers {end}")
            }
            _ => lower,
        };
    }
    let unit = device
        .attr("unit_of_measurement")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if let Ok(number) = state.parse::<f64>()
        && number.is_finite()
    {
        let shown = state.replace('.', ",");
        return if unit.is_empty() {
            shown
        } else {
            format!("{shown} {unit}")
        };
    }
    let opening = matches!(
        device.device_class(),
        Some("door" | "window" | "opening" | "garage_door")
    );
    let text = match (device.domain(), state) {
        (_, "unavailable") => "injoignable",
        (_, "unknown") => "inconnu",
        ("binary_sensor", "on") if opening => "ouvert",
        ("binary_sensor", "off") if opening => "fermé",
        ("binary_sensor", "on") if device.device_class() == Some("motion") => "mouvement détecté",
        ("binary_sensor", "off") if device.device_class() == Some("motion") => "calme",
        ("binary_sensor", "on") => "oui",
        ("binary_sensor", "off") => "non",
        ("climate", mode) => {
            let target = device.attr("temperature").and_then(Value::as_f64);
            let current = device.attr("current_temperature").and_then(Value::as_f64);
            let mode = match mode {
                "off" => "arrêté",
                "heat" => "chauffe",
                "cool" => "refroidit",
                "auto" => "automatique",
                other => other,
            };
            return match (target, current) {
                (Some(t), Some(c)) => {
                    format!("{mode}, consigne {} °C, il fait {} °C", show(t), show(c))
                }
                (Some(t), None) => format!("{mode}, consigne {} °C", show(t)),
                _ => mode.to_owned(),
            };
        }
        (_, "on") => "allumé",
        (_, "off") => "éteint",
        (_, "open") => "ouvert",
        (_, "closed") => "fermé",
        (_, "opening") => "en train de s'ouvrir",
        (_, "closing") => "en train de se fermer",
        (_, "locked") => "verrouillé",
        (_, "unlocked") => "déverrouillé",
        (_, "home") => "à la maison",
        (_, "not_home") => "absent",
        (_, other) => other,
    };
    text.to_owned()
}

fn show(n: f64) -> String {
    let text = if n.fract() == 0.0 {
        format!("{n:.0}")
    } else {
        format!("{n}")
    };
    text.replace('.', ",")
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]

    use super::*;
    use ha_core::Core;
    use serde_json::json;

    fn device(
        core: &Core,
        id: &str,
        state: &str,
        name: &str,
        area: Option<&str>,
        attrs: Value,
    ) -> Device {
        let entity_id: EntityId = id.parse().unwrap();
        let Value::Object(attrs) = attrs else {
            panic!()
        };
        core.states.set(entity_id.clone(), state, attrs).unwrap();
        Device {
            state: core.states.get(&entity_id).unwrap(),
            name: name.into(),
            area: area.map(str::to_owned),
        }
    }

    fn home() -> Vec<Device> {
        let core = Core::new();
        vec![
            device(
                &core,
                "light.plafonnier_salon",
                "off",
                "Plafonnier salon",
                Some("Salon"),
                json!({}),
            ),
            device(
                &core,
                "light.lampe_salon",
                "on",
                "Lampe d'appoint",
                Some("Salon"),
                json!({}),
            ),
            device(
                &core,
                "light.chambre",
                "off",
                "Lampe chambre",
                Some("Chambre"),
                json!({}),
            ),
            // Une prise Hue : une « lampe » de classe outlet (un radiateur
            // d'appoint y est branché).
            device(
                &core,
                "light.prise_bureau",
                "on",
                "Prise d'appoint",
                Some("Salon"),
                json!({"device_class": "outlet"}),
            ),
            device(
                &core,
                "cover.chambre",
                "open",
                "Volet chambre",
                Some("Chambre"),
                json!({}),
            ),
            device(
                &core,
                "cover.salon",
                "open",
                "Volet salon",
                Some("Salon"),
                json!({}),
            ),
            device(
                &core,
                "climate.salon",
                "heat",
                "Salon",
                Some("Salon"),
                json!({"temperature": 20, "current_temperature": 19.5}),
            ),
            device(
                &core,
                "sensor.temperature_salon",
                "21.5",
                "Température salon",
                Some("Salon"),
                json!({"device_class": "temperature", "unit_of_measurement": "°C"}),
            ),
            device(
                &core,
                "sensor.temperature_chambre",
                "19",
                "Température chambre",
                Some("Chambre"),
                json!({"device_class": "temperature", "unit_of_measurement": "°C"}),
            ),
            device(
                &core,
                "scene.bonne_nuit",
                "scening",
                "Bonne nuit",
                None,
                json!({}),
            ),
            device(
                &core,
                "lock.entree",
                "locked",
                "Porte d'entrée",
                None,
                json!({}),
            ),
            device(&core, "person.david", "home", "David", None, json!({})),
            device(&core, "person.alice", "not_home", "Alice", None, json!({})),
        ]
    }

    fn orders(text: &str) -> Vec<(String, String, Attributes)> {
        match understand(text, &home()).unwrap() {
            Understood::Orders(orders) => orders
                .into_iter()
                .map(|o| {
                    (
                        format!("{}.{}", o.domain, o.service),
                        o.entity_id.to_string(),
                        o.data,
                    )
                })
                .collect(),
            Understood::Answer(answer) => panic!("réponse inattendue : {answer}"),
        }
    }

    fn one(text: &str) -> (String, String, Value) {
        let mut found = orders(text);
        assert_eq!(found.len(), 1, "{text} : {found:?}");
        let (service, entity, data) = found.remove(0);
        (service, entity, Value::Object(data))
    }

    fn said(text: &str) -> String {
        match understand(text, &home()).unwrap() {
            Understood::Answer(answer) => answer,
            Understood::Orders(orders) => panic!("ordres inattendus : {orders:?}"),
        }
    }

    #[test]
    fn sentences_are_normalized() {
        assert_eq!(
            normalize("Éteins   l'Éclairage, STP !"),
            "eteins l eclairage stp"
        );
        assert_eq!(normalize("Mets à 21,5°"), "mets a 21.5 degres");
        assert_eq!(normalize("à 50%"), "a 50 pourcent");
        assert_eq!(
            keywords("ferme les volets de la chambre"),
            ["ferme", "volet", "chambre"]
        );
    }

    #[test]
    fn a_device_is_found_by_its_name() {
        assert_eq!(
            one("Ferme le volet de la chambre"),
            (
                "cover.close_cover".into(),
                "cover.chambre".into(),
                json!({})
            )
        );
        assert_eq!(
            one("ouvre le volet du salon à 30 %"),
            (
                "cover.set_cover_position".into(),
                "cover.salon".into(),
                json!({"position": 30})
            )
        );
        assert_eq!(
            one("Allume le plafonnier du salon").1,
            "light.plafonnier_salon"
        );
        assert_eq!(
            one("active la scène bonne nuit"),
            ("scene.turn_on".into(), "scene.bonne_nuit".into(), json!({}))
        );
        assert_eq!(one("verrouille la porte d'entrée").0, "lock.lock");
        assert_eq!(one("déverrouille la porte d'entrée").0, "lock.unlock");
    }

    #[test]
    fn by_room_and_type() {
        let lights = orders("éteins les lumières du salon");
        let ids: Vec<&str> = lights.iter().map(|(_, id, _)| id.as_str()).collect();
        assert_eq!(ids, ["light.plafonnier_salon", "light.lampe_salon"]);
        assert!(
            lights
                .iter()
                .all(|(service, _, _)| service == "light.turn_off")
        );
        // La pièce seule : les lumières ; « ferme » : les volets.
        assert_eq!(orders("éteins le salon").len(), 2);
        // Une prise Hue n'est pas une lumière : « la prise » la désigne.
        assert_eq!(
            one("éteins la prise du salon"),
            (
                "light.turn_off".into(),
                "light.prise_bureau".into(),
                json!({})
            )
        );
        assert_eq!(one("ferme la chambre").1, "cover.chambre");
        // « Salon » est aussi le nom du thermostat : avec le type, c'est lui.
        assert_eq!(
            one("mets le chauffage du salon à 21 degrés"),
            (
                "climate.set_temperature".into(),
                "climate.salon".into(),
                json!({"temperature": 21})
            )
        );
        assert_eq!(
            one("règle le thermostat à 19,5").2,
            json!({"temperature": 19.5})
        );
        assert_eq!(
            one("mets la lampe de la chambre à 40 %"),
            (
                "light.turn_on".into(),
                "light.chambre".into(),
                json!({"brightness_pct": 40})
            )
        );
    }

    #[test]
    fn several_devices_need_a_plural_or_a_name() {
        assert_eq!(orders("ferme tous les volets").len(), 2);
        assert_eq!(
            understand("ferme le volet", &home()),
            Err(NotUnderstood::Ambiguous(vec![
                "Volet chambre".into(),
                "Volet salon".into()
            ]))
        );
    }

    #[test]
    fn what_cannot_be_understood_says_why() {
        let devices = home();
        assert_eq!(understand("  ", &devices), Err(NotUnderstood::Empty));
        assert_eq!(
            understand(&"a".repeat(400), &devices),
            Err(NotUnderstood::TooLong)
        );
        assert_eq!(
            understand("bonjour la maison", &devices),
            Err(NotUnderstood::NoAction)
        );
        assert_eq!(
            understand("allume la piscine", &devices),
            Err(NotUnderstood::NoDevice)
        );
        assert_eq!(
            understand("mets le volet de la chambre à 20 degrés", &devices),
            Err(NotUnderstood::Unsupported("Volet chambre".into()))
        );
        assert_eq!(
            understand("ouvre le chauffage", &devices),
            Err(NotUnderstood::Unsupported("Salon".into()))
        );
        assert!(
            NotUnderstood::Ambiguous(vec!["A".into(), "B".into()])
                .message()
                .contains("A, B")
        );
    }

    #[test]
    fn questions_are_answered_from_the_house() {
        assert_eq!(
            said("Quelle est la température du salon ?"),
            "Température salon : 21,5 °C."
        );
        assert_eq!(
            said("quelle température fait-il ?"),
            "Température salon : 21,5 °C ; Température chambre : 19 °C."
        );
        assert_eq!(said("Qui est à la maison ?"), "À la maison : David.");
        assert_eq!(
            said("le volet de la chambre est-il fermé ?"),
            "Volet chambre : ouvert."
        );
        assert_eq!(
            said("est-ce que le chauffage du salon marche ?"),
            "Salon : chauffe, consigne 20 °C, il fait 19,5 °C."
        );
        assert_eq!(
            said("est-ce que la piscine est chaude ?"),
            "Je n'ai pas trouvé de mesure."
        );
    }

    #[test]
    fn answers_say_the_room_and_orders_their_value() {
        let core = Core::new();
        let devices = vec![device(
            &core,
            "sensor.t",
            "19.9",
            "Température",
            Some("Salon"),
            json!({"device_class": "temperature", "unit_of_measurement": "°C"}),
        )];
        assert_eq!(
            understand("quelle est la température du salon ?", &devices),
            Ok(Understood::Answer("Température (Salon) : 19,9 °C.".into()))
        );
        let Understood::Orders(orders) =
            understand("ouvre le volet du salon à 40 %", &home()).unwrap()
        else {
            panic!()
        };
        assert_eq!(amount(&orders[0]).as_deref(), Some("à 40 %"));
        let Understood::Orders(orders) =
            understand("mets le chauffage du salon à 21,5", &home()).unwrap()
        else {
            panic!()
        };
        assert_eq!(amount(&orders[0]).as_deref(), Some("à 21,5 °C"));
        let Understood::Orders(orders) = understand("ferme le volet du salon", &home()).unwrap()
        else {
            panic!()
        };
        assert_eq!(amount(&orders[0]), None);
    }

    #[test]
    fn numbers_in_names_are_not_values() {
        // Revue de la PR #60 : « chambre 2 » donnait une position de 2 %.
        let core = Core::new();
        let devices = vec![
            device(
                &core,
                "cover.ch2",
                "closed",
                "Volet",
                Some("Chambre 2"),
                json!({}),
            ),
            device(&core, "light.l2", "off", "Lampe 2", None, json!({})),
        ];
        let one = |text: &str| match understand(text, &devices).unwrap() {
            Understood::Orders(mut orders) => {
                assert_eq!(orders.len(), 1, "{text}");
                let order = orders.remove(0);
                (order.service, Value::Object(order.data))
            }
            Understood::Answer(answer) => panic!("{answer}"),
        };
        assert_eq!(
            one("ouvre le volet de la chambre 2"),
            ("open_cover".into(), json!({}))
        );
        assert_eq!(one("allume la lampe 2"), ("turn_on".into(), json!({})));
        assert_eq!(
            one("mets la lampe 2 à 40 %"),
            ("turn_on".into(), json!({"brightness_pct": 40}))
        );
        assert_eq!(
            one("ouvre le volet de la chambre 2 à 30"),
            ("set_cover_position".into(), json!({"position": 30}))
        );
    }

    #[test]
    fn the_room_said_chooses_between_devices_of_the_same_name() {
        // Revue de la PR #60 : deux « Lampe » ; « la lampe du salon »
        // allumait aussi celle de la chambre.
        let core = Core::new();
        let devices = vec![
            device(&core, "light.a", "off", "Lampe", Some("Salon"), json!({})),
            device(&core, "light.b", "off", "Lampe", Some("Chambre"), json!({})),
        ];
        let Understood::Orders(orders) = understand("allume la lampe du salon", &devices).unwrap()
        else {
            panic!()
        };
        let ids: Vec<&str> = orders.iter().map(|o| o.entity_id.as_str()).collect();
        assert_eq!(ids, ["light.a"]);
        // Sans pièce, au singulier : laquelle ?
        assert_eq!(
            understand("allume la lampe", &devices),
            Err(NotUnderstood::Ambiguous(vec![
                "Lampe (Salon)".into(),
                "Lampe (Chambre)".into()
            ]))
        );
        // Au pluriel : les deux.
        let Understood::Orders(orders) = understand("allume les lampes", &devices).unwrap() else {
            panic!()
        };
        assert_eq!(orders.len(), 2);
    }

    #[test]
    fn a_device_without_a_name_is_found_by_its_identifier() {
        let core = Core::new();
        let devices = vec![
            device(&core, "cover.salon", "open", "cover.salon", None, json!({})),
            device(
                &core,
                "cover.garage",
                "open",
                "cover.garage",
                None,
                json!({}),
            ),
        ];
        let Understood::Orders(orders) = understand("ferme le volet du salon", &devices).unwrap()
        else {
            panic!()
        };
        assert_eq!(orders.len(), 1);
        assert_eq!(orders[0].entity_id.as_str(), "cover.salon");
    }

    #[test]
    fn a_guest_without_people_gets_no_presence() {
        let devices: Vec<Device> = home()
            .into_iter()
            .filter(|d| d.domain() != "person")
            .collect();
        assert_eq!(
            understand("qui est à la maison ?", &devices),
            Ok(Understood::Answer("Je ne peux pas te le dire.".into()))
        );
    }

    #[test]
    fn a_machine_says_when_it_ends() {
        let core = Core::new();
        let washer = device(
            &core,
            "sensor.lave_linge",
            "En marche",
            "Lave-linge",
            Some("Buanderie"),
            json!({"appareil": "lave-linge", "fin_heure": "11:20"}),
        );
        let expected = Ok(Understood::Answer(
            "Lave-linge (Buanderie) : en marche, fini vers 11:20.".into(),
        ));
        assert_eq!(
            understand(
                "Est-ce que le lave-linge a fini ?",
                std::slice::from_ref(&washer)
            ),
            expected
        );
        assert_eq!(
            understand("Le linge est fini ?", std::slice::from_ref(&washer)),
            expected
        );
        assert_eq!(
            understand("Où en est la machine ?", std::slice::from_ref(&washer)),
            expected
        );
    }

    #[test]
    fn spoken_numbers_become_digits() {
        let cases = [
            (
                "mets le chauffage à vingt et un degrés",
                "mets le chauffage a 21 degres",
            ),
            ("à 20 et 1°", "a 21 degres"),
            ("à dix-neuf degrés et demi", "a 19 degres et demi"),
            ("à vingt et un et demi", "a 21.5"),
            ("à 21 et demi", "a 21.5"),
            ("soixante-dix-sept", "77"),
            ("soixante et onze", "71"),
            ("quatre-vingt-dix pour cent", "90 pourcent"),
            ("quatre vingts", "80"),
            ("cent", "100"),
            ("deux cent vingt-deux", "222"),
            ("dix sept", "17"),
            ("allume une lampe et un spot", "allume une lampe et un spot"),
            ("les deux volets", "les 2 volets"),
            ("la chambre 2", "la chambre 2"),
            ("à 20 1", "a 20 1"),
        ];
        for (said, expected) in cases {
            assert_eq!(normalize(said), expected, "{said}");
        }
        assert_eq!(
            one("mets le chauffage du salon à vingt et un degrés et demi"),
            (
                "climate.set_temperature".into(),
                "climate.salon".into(),
                json!({"temperature": 21.5})
            )
        );
        assert_eq!(
            one("Met le chauffage du salon à 20 et 1°."),
            (
                "climate.set_temperature".into(),
                "climate.salon".into(),
                json!({"temperature": 21})
            )
        );
        assert_eq!(
            one("ouvre le volet du salon à cinquante pour cent"),
            (
                "cover.set_cover_position".into(),
                "cover.salon".into(),
                json!({"position": 50})
            )
        );
    }

    fn with_lists() -> Vec<Device> {
        let core = Core::new();
        let mut devices = home();
        devices.push(device(
            &core,
            "todo.courses",
            "3",
            "Liste de courses",
            None,
            json!({}),
        ));
        devices.push(device(&core, "todo.taches", "0", "Tâches", None, json!({})));
        devices
    }

    fn added(text: &str) -> (String, String) {
        match understand(text, &with_lists()) {
            Ok(Understood::Orders(orders)) => {
                assert_eq!(orders.len(), 1, "{text}");
                assert_eq!(orders[0].service, "add_item");
                (
                    orders[0].entity_id.to_string(),
                    orders[0].data["item"].as_str().unwrap().to_owned(),
                )
            }
            other => panic!("{text} : {other:?}"),
        }
    }

    #[test]
    fn things_are_added_to_a_list() {
        let courses = |item: &str| ("todo.courses".to_owned(), item.to_owned());
        assert_eq!(
            added("Ajoute du pain à la liste de courses."),
            courses("Pain")
        );
        assert_eq!(added("ajoute du lait aux courses"), courses("Lait"));
        assert_eq!(added("Ajoute de l'huile d'olive"), courses("Huile d'olive"));
        assert_eq!(
            added("ajoute de la sauce à la tomate à la liste de courses s'il te plaît"),
            courses("Sauce à la tomate")
        );
        assert_eq!(added("mets des œufs sur la liste"), courses("Œufs"));
        assert_eq!(
            added("Tu peux rajouter 6 yaourts sur ma liste de courses ?"),
            courses("6 yaourts")
        );
        assert_eq!(
            added("ajoute à la liste de courses du beurre"),
            courses("Beurre")
        );
        assert_eq!(
            added("note appeler le plombier dans les tâches"),
            ("todo.taches".into(), "Appeler le plombier".into())
        );
        // « mets … à » reste un ordre à un appareil.
        assert_eq!(
            understand("mets le chauffage du salon à 21", &with_lists()).unwrap(),
            understand("mets le chauffage du salon à 21", &home()).unwrap()
        );
        assert_eq!(
            understand("ajoute", &with_lists()),
            Err(NotUnderstood::NothingToAdd)
        );
        assert_eq!(
            understand("ajoute du pain", &home()),
            Err(NotUnderstood::NoDevice)
        );
        // Sans liste de courses, plusieurs listes : laquelle ?
        let core = Core::new();
        let others = vec![
            device(&core, "todo.maison", "0", "Maison", None, json!({})),
            device(&core, "todo.jardin", "0", "Jardin", None, json!({})),
        ];
        assert!(matches!(
            understand("ajoute du pain", &others),
            Err(NotUnderstood::Ambiguous(_))
        ));
    }
}
