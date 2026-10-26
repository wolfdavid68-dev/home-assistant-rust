//! **Enchaîner les phrases** (étape 96) : « allume le salon », puis « et la
//! cuisine aussi », « pareil dans la chambre », « éteins-la ».
//!
//! Après un ordre compris, Assist garde un petit [`Context`] : le verbe, la
//! valeur, le type d'appareil dit, et les appareils visés. La phrase suivante
//! peut s'en servir :
//!
//! - **sans verbe** (« et la cuisine aussi ») : le même verbe, sur ce que la
//!   nouvelle phrase désigne (une pièce, un nom) ; avec seulement une valeur
//!   (« et à 50 % »), les mêmes appareils à cette valeur ;
//! - **avec un verbe mais sans appareil** (« éteins-la », « ferme-les ») :
//!   ce verbe sur les mêmes appareils.
//!
//! Le contexte ne garde jamais le texte de la phrase. Il ne donne aucun
//! droit : les appareils repris sont cherchés dans la liste de ceux que le
//! compte peut voir **maintenant**, et l'API vérifie chaque ordre comme un
//! toucher. Une question ne crée ni n'utilise de contexte.
//!
//! Ce module est à part de `lib.rs` exprès : il réutilise `verb`, `value`,
//! `find` et `service` sans les modifier.

use ha_core::EntityId;

use super::{
    Device, MAX_TARGETS, NotUnderstood, Order, STOP_WORDS, Understood, Unit, Verb, default_domains,
    domain_hint, find, is_question, keywords, normalize, service, value, verb,
};

/// Ce qu'Assist retient d'un ordre pour comprendre la phrase suivante.
///
/// Opaque : on ne peut que le recevoir de [`understand_with`] et le lui
/// redonner. Il ne contient ni le texte dit, ni le nom du compte.
#[derive(Debug, Clone, PartialEq)]
pub struct Context {
    verb: Verb,
    value: Option<(f64, Unit)>,
    /// Le type d'appareil dit (« les volets »), sinon celui par défaut.
    hint: Option<&'static [&'static str]>,
    /// Les appareils visés, à retrouver parmi ceux encore visibles.
    entities: Vec<EntityId>,
}

impl Context {
    /// Les appareils visés par l'ordre retenu.
    #[must_use]
    pub fn entities(&self) -> &[EntityId] {
        &self.entities
    }
}

/// Les mots qui disent « la même chose, ailleurs ».
const SAME: &[&str] = &["et", "aussi", "pareil", "idem", "meme", "egalement"];

/// Les mots qui ne désignent rien de nouveau : « éteins-la aussi »,
/// « mets-les à 50 % ».
const NOTHING_NEW: &[&str] = &[
    "aussi",
    "pareil",
    "idem",
    "meme",
    "egalement",
    "lui",
    "leur",
    "elle",
    "elles",
    "eux",
    "ca",
    "cela",
    "y",
    "encore",
    "maintenant",
    "puis",
    "pourcent",
    "pour",
    "cent",
    "degre",
    "degres",
    "demi",
    "demie",
];

/// La phrase ne nomme-t-elle aucun appareil ni aucune pièce ? Seulement un
/// verbe, une valeur, des pronoms. « Éteins le garage », sans garage, ne
/// doit pas éteindre l'appareil d'avant.
fn names_nothing(raw: &[&str]) -> bool {
    raw.iter().all(|word| {
        STOP_WORDS.contains(word)
            || NOTHING_NEW.contains(word)
            || verb(&[*word]).is_some()
            || word.parse::<f64>().is_ok()
    })
}

/// Les phrases qui terminent la conversation, sans ponctuation ni accents.
const GOODBYES: &[&str] = &[
    "merci",
    "merci beaucoup",
    "merci bien",
    "non merci",
    "non",
    "non c est tout",
    "c est tout",
    "c est tout merci",
    "ce sera tout",
    "ca sera tout",
    "ce sera tout merci",
    "c est bon",
    "c est bon merci",
    "ca ira",
    "ca ira merci",
    "stop",
    "rien",
    "au revoir",
    "merci au revoir",
    "fini",
    "c est fini",
    "termine",
];

/// La phrase termine-t-elle la conversation (« merci », « c'est tout »,
/// « stop », « non merci ») ? Elle doit être dite seule : « stoppe le
/// volet » ou « arrête-le » restent des ordres.
#[must_use]
pub fn is_goodbye(text: &str) -> bool {
    let normalized = normalize(text);
    GOODBYES.contains(&normalized.as_str())
}

/// La réponse courte à un au revoir.
#[must_use]
pub fn goodbye_reply(text: &str) -> &'static str {
    if normalize(text).split(' ').any(|word| word == "merci") {
        "Avec plaisir."
    } else {
        "D'accord."
    }
}

/// Comme [`super::understand`], avec le contexte de la phrase d'avant.
///
/// Renvoie ce qui est compris et, quand ce sont des ordres, le contexte à
/// retenir pour la phrase suivante (`None` : garder l'ancien).
///
/// # Errors
///
/// Les mêmes que [`super::understand`].
pub fn understand_with(
    text: &str,
    devices: &[Device],
    context: Option<&Context>,
) -> Result<(Understood, Option<Context>), NotUnderstood> {
    let first = super::understand(text, devices);
    let normalized = normalize(text);
    let raw: Vec<&str> = normalized.split(' ').collect();
    let said = Said {
        verb: verb(&raw),
        value: value(&raw),
        hint: keywords(&normalized)
            .iter()
            .find_map(|word| domain_hint(word)),
    };
    match (first, context) {
        (Ok(understood), _) => {
            let next = remember(&understood, &said);
            Ok((understood, next))
        }
        // « Éteins-la » : le verbe, sur les mêmes appareils.
        (Err(NotUnderstood::NoDevice), Some(context))
            if !is_question(text, &raw) && names_nothing(&raw) =>
        {
            let Some(verb) = said.verb else {
                return Err(NotUnderstood::NoDevice);
            };
            let value = said.value.or(match verb {
                Verb::Set => context.value,
                _ => None,
            });
            again(devices, context, verb, value, context.hint).ok_or(NotUnderstood::NoDevice)?
        }
        // « Et la cuisine aussi » : le même verbe, ailleurs.
        (Err(NotUnderstood::NoAction), Some(context)) if !is_question(text, &raw) => {
            follow(devices, context, &said, &normalized, &raw)
        }
        (Err(error), _) => Err(error),
    }
}

/// Ce que dit la nouvelle phrase.
struct Said {
    verb: Option<Verb>,
    value: Option<(f64, Unit)>,
    hint: Option<&'static [&'static str]>,
}

/// Le contexte d'un ordre compris (rien pour une réponse ou une liste).
fn remember(understood: &Understood, said: &Said) -> Option<Context> {
    let Understood::Orders(orders) = understood else {
        return None;
    };
    let verb = said.verb?;
    if orders.is_empty() || orders.iter().any(|order| order.domain == "todo") {
        return None;
    }
    Some(Context {
        verb,
        value: said.value,
        hint: said.hint,
        entities: orders.iter().map(|order| order.entity_id.clone()).collect(),
    })
}

/// Une suite sans verbe : « et la cuisine », « pareil dans la chambre »,
/// « et à 50 % ».
fn follow(
    devices: &[Device],
    context: &Context,
    said: &Said,
    normalized: &str,
    raw: &[&str],
) -> Result<(Understood, Option<Context>), NotUnderstood> {
    // Avec une valeur, « mets » : « à 50 % » allume une lumière à 50 %,
    // place un volet à 50 %, règle un chauffage.
    let (verb, value) = match said.value {
        Some(value) => (Verb::Set, Some(value)),
        None => (context.verb, context.value),
    };
    let words = keywords(normalized);
    let plural = raw
        .iter()
        .any(|word| matches!(*word, "les" | "tous" | "toutes" | "tout"));
    // Le type dit maintenant, sinon celui d'avant, sinon celui par défaut.
    let fallback = context
        .hint
        .unwrap_or_else(|| default_domains(Some(context.verb), context.value));
    match find(devices, &words, plural, said.hint, fallback) {
        Ok(target) => {
            let orders = orders(&target.devices, verb, value)?;
            let next = Context {
                verb,
                value,
                hint: said.hint.or(context.hint),
                entities: orders.iter().map(|order| order.entity_id.clone()).collect(),
            };
            Ok((Understood::Orders(orders), Some(next)))
        }
        // Rien de nouveau de désigné, mais une valeur : les mêmes appareils.
        Err(NotUnderstood::NoDevice) if said.value.is_some() && names_nothing(raw) => {
            again(devices, context, verb, value, context.hint).ok_or(NotUnderstood::NoDevice)?
        }
        // « Et le garage » sans garage : on dit qu'on n'a pas trouvé. Une
        // phrase qui ne ressemble pas à une suite (« oui ») reste « pas
        // compris ».
        Err(NotUnderstood::NoDevice) => {
            let looks_like_more = raw.iter().any(|word| SAME.contains(word));
            Err(if looks_like_more {
                NotUnderstood::NoDevice
            } else {
                NotUnderstood::NoAction
            })
        }
        Err(error) => Err(error),
    }
}

/// `verb` sur les appareils du contexte encore visibles. `None` : plus
/// aucun n'est visible.
fn again(
    devices: &[Device],
    context: &Context,
    verb: Verb,
    value: Option<(f64, Unit)>,
    hint: Option<&'static [&'static str]>,
) -> Option<Result<(Understood, Option<Context>), NotUnderstood>> {
    let targets: Vec<&Device> = context
        .entities
        .iter()
        .filter_map(|id| devices.iter().find(|d| d.state.entity_id() == id))
        .collect();
    if targets.is_empty() {
        return None;
    }
    Some(orders(&targets, verb, value).map(|orders| {
        let next = Context {
            verb,
            value,
            hint,
            entities: orders.iter().map(|order| order.entity_id.clone()).collect(),
        };
        (Understood::Orders(orders), Some(next))
    }))
}

/// Un ordre par appareil, comme [`super::understand`].
fn orders(
    targets: &[&Device],
    verb: Verb,
    value: Option<(f64, Unit)>,
) -> Result<Vec<Order>, NotUnderstood> {
    targets
        .iter()
        .take(MAX_TARGETS)
        .map(|device| {
            let (service, data) = service(device.domain(), verb, value)
                .ok_or_else(|| NotUnderstood::Unsupported(device.name.clone()))?;
            Ok(Order {
                domain: device.domain().to_owned(),
                service: service.to_owned(),
                entity_id: device.state.entity_id().clone(),
                data,
                name: device.name.clone(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]

    use std::sync::LazyLock;

    use ha_core::Core;
    use serde_json::json;

    use super::*;

    static CORE: LazyLock<Core> = LazyLock::new(Core::new);

    fn device(id: &str, name: &str, area: Option<&str>) -> Device {
        let entity_id: EntityId = id.parse().unwrap();
        CORE.states
            .set(entity_id.clone(), "off", serde_json::Map::new())
            .unwrap();
        Device {
            state: CORE.states.get(&entity_id).unwrap(),
            name: name.into(),
            area: area.map(str::to_owned),
            aliases: Vec::new(),
        }
    }

    fn house() -> Vec<Device> {
        vec![
            device("light.salon", "Lampe salon", Some("Salon")),
            device("light.cuisine", "Plafonnier cuisine", Some("Cuisine")),
            device("light.chambre", "Lampe chambre", Some("Chambre")),
            device("cover.salon", "Volet salon", Some("Salon")),
            device("cover.chambre", "Volet chambre", Some("Chambre")),
            device("climate.salon", "Chauffage salon", Some("Salon")),
        ]
    }

    /// Les ordres compris : `(service, entité)`.
    fn said(text: &str, context: Option<&Context>) -> (Vec<(String, String)>, Option<Context>) {
        let (understood, next) = understand_with(text, &house(), context).expect("phrase comprise");
        let Understood::Orders(orders) = understood else {
            panic!("des ordres attendus");
        };
        let orders = orders
            .into_iter()
            .map(|order| (order.service, order.entity_id.to_string()))
            .collect();
        (orders, next)
    }

    fn pair(service: &str, entity: &str) -> (String, String) {
        (service.to_owned(), entity.to_owned())
    }

    #[test]
    fn a_first_sentence_gives_a_context() {
        let (orders, context) = said("Allume le salon", None);
        assert_eq!(orders, vec![pair("turn_on", "light.salon")]);
        let context = context.expect("contexte retenu");
        assert_eq!(context.entities()[0].to_string(), "light.salon");
    }

    #[test]
    fn and_the_kitchen_too_does_the_same_elsewhere() {
        let (_, context) = said("Allume le salon", None);
        for text in [
            "et la cuisine aussi",
            "Et la cuisine.",
            "pareil dans la cuisine",
            "la cuisine aussi",
        ] {
            let (orders, _) = said(text, context.as_ref());
            assert_eq!(orders, vec![pair("turn_on", "light.cuisine")], "{text}");
        }
    }

    #[test]
    fn the_type_said_first_is_kept() {
        let (_, context) = said("ferme le volet du salon", None);
        let (orders, _) = said("et la chambre", context.as_ref());
        assert_eq!(orders, vec![pair("close_cover", "cover.chambre")]);
    }

    #[test]
    fn a_pronoun_targets_the_same_device() {
        let (_, context) = said("allume la lampe du salon", None);
        let (orders, context) = said("éteins-la", context.as_ref());
        assert_eq!(orders, vec![pair("turn_off", "light.salon")]);
        // Et encore, sur le contexte mis à jour.
        let (orders, _) = said("allume-la aussi", context.as_ref());
        assert_eq!(orders, vec![pair("turn_on", "light.salon")]);
    }

    #[test]
    fn an_unknown_place_never_falls_back_on_the_device_before() {
        let (_, context) = said("allume la lampe du salon", None);
        assert_eq!(
            understand_with("éteins le garage", &house(), context.as_ref()),
            Err(NotUnderstood::NoDevice)
        );
        assert_eq!(
            understand_with("et à 30 % dans le garage", &house(), context.as_ref()),
            Err(NotUnderstood::NoDevice)
        );
    }

    #[test]
    fn a_value_alone_changes_the_same_devices() {
        let (_, context) = said("allume la lampe du salon", None);
        let (orders, context) = said("et à 30 %", context.as_ref());
        assert_eq!(orders, vec![pair("turn_on", "light.salon")]);
        // La valeur suit dans la phrase d'après.
        let (understood, _) = understand_with("pareil dans la chambre", &house(), context.as_ref())
            .expect("comprise");
        let Understood::Orders(orders) = understood else {
            panic!("ordres attendus");
        };
        assert_eq!(orders[0].entity_id.to_string(), "light.chambre");
        assert_eq!(orders[0].data.get("brightness_pct"), Some(&json!(30)));
    }

    #[test]
    fn a_pronoun_with_a_value_sets_the_same_device() {
        let (_, context) = said("allume la lampe du salon", None);
        let (understood, _) =
            understand_with("mets-la à 30 %", &house(), context.as_ref()).expect("comprise");
        let Understood::Orders(orders) = understood else {
            panic!("ordres attendus");
        };
        assert_eq!(orders[0].entity_id.to_string(), "light.salon");
        assert_eq!(orders[0].data.get("brightness_pct"), Some(&json!(30)));
        // Une autre valeur, ailleurs.
        let (understood, _) =
            understand_with("et 50 % dans la chambre", &house(), context.as_ref())
                .expect("comprise");
        let Understood::Orders(orders) = understood else {
            panic!("ordres attendus");
        };
        assert_eq!(orders[0].entity_id.to_string(), "light.chambre");
        assert_eq!(orders[0].data.get("brightness_pct"), Some(&json!(50)));
    }

    #[test]
    fn a_follow_up_follows_the_last_sentence() {
        let (_, context) = said("allume le salon", None);
        let (_, context) = said("et la cuisine", context.as_ref());
        let (orders, _) = said("éteins-la", context.as_ref());
        assert_eq!(orders, vec![pair("turn_off", "light.cuisine")]);
    }

    #[test]
    fn without_context_nothing_changes() {
        assert_eq!(
            understand_with("et la cuisine aussi", &house(), None),
            Err(NotUnderstood::NoAction)
        );
        assert_eq!(
            understand_with("éteins-la", &house(), None),
            Err(NotUnderstood::NoDevice)
        );
    }

    #[test]
    fn a_full_sentence_ignores_the_context() {
        let (_, context) = said("allume le salon", None);
        let (orders, _) = said("ferme le volet de la chambre", context.as_ref());
        assert_eq!(orders, vec![pair("close_cover", "cover.chambre")]);
    }

    #[test]
    fn questions_neither_use_nor_make_a_context() {
        let (_, context) = said("allume le salon", None);
        let (understood, next) = understand_with(
            "quelle est la température du salon ?",
            &house(),
            context.as_ref(),
        )
        .expect("question comprise");
        assert!(matches!(understood, Understood::Answer(_)));
        assert!(next.is_none(), "une question ne remplace pas le contexte");
    }

    #[test]
    fn a_device_no_longer_visible_is_not_reused() {
        let (_, context) = said("allume la lampe du salon", None);
        let without: Vec<Device> = house()
            .into_iter()
            .filter(|d| d.state.entity_id().to_string() != "light.salon")
            .collect();
        assert_eq!(
            understand_with("éteins-la", &without, context.as_ref()),
            Err(NotUnderstood::NoDevice)
        );
    }

    #[test]
    fn something_else_is_still_not_understood() {
        let (_, context) = said("allume le salon", None);
        assert_eq!(
            understand_with("oui", &house(), context.as_ref()),
            Err(NotUnderstood::NoAction)
        );
        assert_eq!(
            understand_with("et le garage", &house(), context.as_ref()),
            Err(NotUnderstood::NoDevice)
        );
    }

    #[test]
    fn an_unsupported_follow_up_says_so() {
        let (_, context) = said("ouvre le volet du salon", None);
        // Ouvrir le chauffage : pas possible.
        assert_eq!(
            understand_with("et le chauffage aussi", &house(), context.as_ref()),
            Err(NotUnderstood::Unsupported("Chauffage salon".into()))
        );
    }

    #[test]
    fn goodbyes_end_the_conversation() {
        for text in [
            "Merci.",
            "merci beaucoup !",
            "C'est tout.",
            "Stop",
            "non merci",
            "Ça ira",
        ] {
            assert!(is_goodbye(text), "{text}");
        }
        for text in [
            "stoppe le volet du salon",
            "arrête-le",
            "allume le salon merci",
            "et la cuisine",
            "",
        ] {
            assert!(!is_goodbye(text), "{text}");
        }
        assert_eq!(goodbye_reply("Merci !"), "Avec plaisir.");
        assert_eq!(goodbye_reply("C'est tout"), "D'accord.");
    }
}
