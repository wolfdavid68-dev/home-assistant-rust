//! Ce que dit un e-mail d'Amazon : où en est le colis (d'après l'objet), et
//! quel article (« Plaid »). Puis l'état de tous les colis, d'après les
//! e-mails des deux dernières semaines.

use std::collections::BTreeMap;

use jiff::Timestamp;

/// Où en est un colis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Step {
    Ordered,
    Shipped,
    Delayed,
    OutForDelivery,
    Problem,
    Delivered,
}

impl Step {
    /// Le nom de l'état, dans les attributs.
    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Self::Ordered => "commande",
            Self::Shipped => "expedie",
            Self::Delayed => "retard",
            Self::OutForDelivery => "en_livraison",
            Self::Problem => "probleme",
            Self::Delivered => "livre",
        }
    }

    /// La phrase de la fenêtre : « Plaid va arriver aujourd'hui. ».
    #[must_use]
    pub fn sentence(self, article: &str) -> String {
        match self {
            Self::Ordered => format!("{article} est commandé."),
            Self::Shipped => format!("{article} est en route."),
            Self::Delayed => format!("{article} aura du retard."),
            Self::OutForDelivery => format!("{article} va arriver aujourd'hui."),
            Self::Problem => format!("{article} n'a pas pu être livré."),
            Self::Delivered => format!("{article} a été livré."),
        }
    }

    /// Le colis est-il en route (expédié, pas encore livré) ?
    #[must_use]
    pub const fn on_the_way(self) -> bool {
        matches!(
            self,
            Self::Shipped | Self::Delayed | Self::OutForDelivery | Self::Problem
        )
    }
}

/// Minuscules, sans accents ni apostrophes courbes.
fn normalize(text: &str) -> String {
    text.chars()
        .flat_map(char::to_lowercase)
        .map(|c| match c {
            'à' | 'â' | 'ä' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'î' | 'ï' => 'i',
            'ô' | 'ö' => 'o',
            'ù' | 'û' | 'ü' => 'u',
            'ç' => 'c',
            '\u{2019}' | '`' => '\'',
            '\u{a0}' | '\u{202f}' => ' ',
            other => other,
        })
        .collect()
}

/// Où en est le colis, d'après l'objet de l'e-mail. `None` : un e-mail
/// d'Amazon qui ne parle pas d'un colis (publicité, facture…).
#[must_use]
pub fn classify(subject: &str) -> Option<Step> {
    let s = normalize(subject);
    let has = |words: &[&str]| words.iter().any(|w| s.contains(w));
    // L'ordre compte : « Livraison aujourd'hui » n'est pas « livré ».
    if has(&[
        "tentative",
        "echec de livraison",
        "n'a pas pu etre livre",
        "attempted",
        "undeliverable",
    ]) {
        Some(Step::Problem)
    } else if has(&["retard", "delayed", "running late"]) {
        Some(Step::Delayed)
    } else if has(&[
        "a ete livre",
        "livre :",
        "livree :",
        "livre:",
        "livree:",
        "delivered",
    ]) || s.starts_with("livre") && !s.starts_with("livraison")
    {
        Some(Step::Delivered)
    } else if has(&[
        "en cours de livraison",
        "arrive aujourd",
        "arrivera aujourd",
        "livraison aujourd",
        "livre aujourd",
        "out for delivery",
        "arriving today",
    ]) {
        Some(Step::OutForDelivery)
    } else if has(&["expedie", "shipped", "en route", "a ete envoye"]) {
        Some(Step::Shipped)
    } else if has(&["commande", "ordered", "your order"])
        && !has(&["annul", "cancel", "rembours", "refund", "retour"])
    {
        Some(Step::Ordered)
    } else {
        None
    }
}

/// Le nom complet de l'article : entre « », entre "", ou après « : ».
/// `None` si l'objet n'en cite pas (« Votre colis arrive aujourd'hui »).
#[must_use]
pub fn article(subject: &str) -> Option<String> {
    let between = |open: char, close: char| {
        let start = subject.find(open)? + open.len_utf8();
        let end = subject[start..].rfind(close)? + start;
        Some(subject[start..end].to_owned())
    };
    let raw = between('«', '»')
        .or_else(|| between('"', '"').filter(|t| !t.is_empty()))
        .or_else(|| between('\u{201c}', '\u{201d}'))
        .or_else(|| subject.split_once(':').map(|(_, after)| after.to_owned()))?;
    let mut name = raw.trim().trim_matches(['"', '«', '»', ' ']).to_owned();
    // « Plaid et 1 autre article » / « and 2 more items ».
    let lower = normalize(&name);
    for marker in [" et ", " and "] {
        if let Some(at) = lower.rfind(marker)
            && ["autre", "more"].iter().any(|w| lower[at..].contains(w))
            && name.is_char_boundary(at)
        {
            name.truncate(at);
        }
    }
    let name = name.trim().trim_end_matches(['.', '…']).trim().to_owned();
    let generic = normalize(&name);
    if name.is_empty()
        || [
            "votre colis",
            "votre commande",
            "your package",
            "your order",
            "colis",
        ]
        .iter()
        .any(|g| generic.starts_with(g))
    {
        return None;
    }
    Some(name.chars().take(200).collect())
}

/// Le nom court, comme le dit l'Echo : « Plaid Polaire Doux 150x200 cm,
/// Couverture… » → « Plaid Polaire Doux ».
#[must_use]
pub fn short_name(article: &str) -> String {
    let cut = article
        .find([',', '|', '(', '[', ':', '/'])
        .into_iter()
        .chain(article.find(" - "))
        .chain(article.find(" – "))
        .min()
        .unwrap_or(article.len());
    let mut words: Vec<&str> = Vec::new();
    for word in article[..cut].split_whitespace() {
        if !words.is_empty() && word.chars().any(|c| c.is_ascii_digit()) {
            break;
        }
        words.push(word);
        if words.len() == 3 {
            break;
        }
    }
    let short = words.join(" ");
    if short.is_empty() {
        article.chars().take(40).collect()
    } else {
        short
    }
}

/// Un e-mail d'Amazon, compris.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    /// Son numéro dans la boîte (UID).
    pub uid: u32,
    pub step: Step,
    pub article: Option<String>,
    pub order: Option<String>,
    pub image: Option<String>,
    pub at: Timestamp,
}

/// Un colis : le dernier e-mail qui en parle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parcel {
    /// Le numéro de commande, sinon le nom de l'article.
    pub id: String,
    pub step: Step,
    pub article: String,
    pub image: Option<String>,
    pub at: Timestamp,
    pub uid: u32,
}

/// Les colis, d'après les e-mails (du plus ancien au plus récent).
/// Les e-mails d'une même commande (ou, sans numéro, du même article) sont
/// un même colis ; son état est celui du dernier e-mail. Un e-mail sans
/// article ni commande (« Votre colis arrive aujourd'hui ») complète le
/// dernier colis en route s'il n'y en a qu'un.
#[must_use]
pub fn parcels(notices: &[Notice]) -> Vec<Parcel> {
    let mut sorted: Vec<&Notice> = notices.iter().collect();
    sorted.sort_by_key(|n| (n.at, n.uid));
    let mut by_id: BTreeMap<String, Parcel> = BTreeMap::new();
    for notice in sorted {
        let id = notice
            .order
            .clone()
            .or_else(|| notice.article.as_deref().map(normalize));
        let id = id.unwrap_or_else(|| {
            let on_the_way: Vec<&String> = by_id
                .iter()
                .filter(|(_, p)| p.step.on_the_way() || p.step == Step::Ordered)
                .map(|(id, _)| id)
                .collect();
            if let [only] = on_the_way.as_slice() {
                (*only).clone()
            } else {
                format!("mail-{}", notice.uid)
            }
        });
        let previous = by_id.get(&id);
        let article = notice
            .article
            .clone()
            .or_else(|| previous.map(|p| p.article.clone()))
            .unwrap_or_else(|| "Un colis".to_owned());
        let image = notice
            .image
            .clone()
            .or_else(|| previous.and_then(|p| p.image.clone()));
        by_id.insert(
            id.clone(),
            Parcel {
                id,
                step: notice.step,
                article,
                image,
                at: notice.at,
                uid: notice.uid,
            },
        );
    }
    let mut list: Vec<Parcel> = by_id.into_values().collect();
    list.sort_by_key(|p| (p.at, p.uid));
    list
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]

    use super::*;

    #[test]
    fn subjects_are_classified() {
        let cases = [
            (
                "Expédié : « Plaid polaire » et 1 autre article",
                Some(Step::Shipped),
            ),
            (
                "Votre colis Amazon arrive aujourd'hui",
                Some(Step::OutForDelivery),
            ),
            (
                "En cours de livraison : « Plaid »",
                Some(Step::OutForDelivery),
            ),
            (
                "Livraison aujourd’hui : « Plaid »",
                Some(Step::OutForDelivery),
            ),
            ("Livré : « Plaid polaire »", Some(Step::Delivered)),
            ("Votre colis a été livré", Some(Step::Delivered)),
            ("Delivered: \"Blanket\"", Some(Step::Delivered)),
            ("Commandé : « Plaid »", Some(Step::Ordered)),
            ("Votre commande a du retard", Some(Step::Delayed)),
            ("Tentative de livraison : « Plaid »", Some(Step::Problem)),
            ("Remboursement de votre commande", None),
            ("Les offres du Black Friday", None),
        ];
        for (subject, expected) in cases {
            assert_eq!(classify(subject), expected, "{subject}");
        }
    }

    #[test]
    fn article_names() {
        assert_eq!(
            article(
                "Expédié : « Plaid Polaire Doux 150x200 cm, Couverture... » et 1 autre article"
            )
            .as_deref(),
            Some("Plaid Polaire Doux 150x200 cm, Couverture")
        );
        assert_eq!(article("Livré : Plaid").as_deref(), Some("Plaid"));
        assert_eq!(
            article("Delivered: \"Blanket\" and 2 more items").as_deref(),
            Some("Blanket")
        );
        assert_eq!(article("Votre colis Amazon arrive aujourd'hui"), None);
        assert_eq!(article("En cours de livraison : votre colis"), None);
        assert_eq!(
            short_name("Plaid Polaire Doux 150x200 cm, Couverture"),
            "Plaid Polaire Doux"
        );
        assert_eq!(short_name("Plaid 150x200"), "Plaid");
        assert_eq!(short_name("Anker - Câble USB-C"), "Anker");
    }

    #[test]
    fn mails_become_parcels() {
        let at = |s: &str| s.parse::<Timestamp>().unwrap();
        let notice = |uid, step, article: Option<&str>, order: Option<&str>, when| Notice {
            uid,
            step,
            article: article.map(Into::into),
            order: order.map(Into::into),
            image: (uid == 1).then(|| "https://m.media-amazon.com/images/I/a.jpg".into()),
            at: at(when),
        };
        let list = parcels(&[
            notice(3, Step::OutForDelivery, None, None, "2026-10-22T07:15:00Z"),
            notice(
                1,
                Step::Shipped,
                Some("Plaid"),
                Some("405-1234567-1234567"),
                "2026-10-20T09:00:00Z",
            ),
            notice(
                2,
                Step::Delivered,
                Some("Câble"),
                None,
                "2026-10-21T15:00:00Z",
            ),
        ]);
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].article, "Câble");
        assert_eq!(list[0].step, Step::Delivered);
        // « Votre colis arrive aujourd'hui » : le seul en route, le plaid.
        assert_eq!(list[1].id, "405-1234567-1234567");
        assert_eq!(list[1].step, Step::OutForDelivery);
        assert_eq!(list[1].article, "Plaid");
        assert!(list[1].image.is_some());
    }
}
