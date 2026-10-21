//! La liste de courses, calculée comme dans l'appli Iglou :
//! - les **aliments** du frigo et du congélateur qui sont sous leur « stock
//!   plein » (`quantite < quantiteMax`), à racheter : ce qui manque, ou la
//!   quantité d'achat ajustée dans l'appli ;
//! - les **articles libres**, ajoutés à la main (« Pain »).
//!
//! Rangée par appareil (dans l'ordre de l'appli), les épuisés d'abord, puis
//! par nom ; les articles libres à la fin.

use std::cmp::Ordering;

use crate::firestore::Document;

/// Longueur maximale d'un nom (celle des règles de l'appli).
pub const MAX_NAME: usize = 80;
/// Les unités de l'appli.
pub const UNITS: [&str; 5] = ["pièce", "g", "kg", "L", "portion"];

/// Un aliment rangé dans un appareil.
#[derive(Debug, Clone, PartialEq)]
pub struct Aliment {
    pub id: String,
    pub nom: String,
    pub quantite: f64,
    pub quantite_max: f64,
    pub unite: String,
    pub dans_panier: bool,
    /// La quantité d'achat ajustée dans l'appli (sinon : ce qui manque).
    pub achat: Option<f64>,
    pub appareil_id: String,
}

/// Un article ajouté à la main.
#[derive(Debug, Clone, PartialEq)]
pub struct Libre {
    pub id: String,
    pub nom: String,
    pub quantite: f64,
    pub unite: String,
    pub dans_panier: bool,
}

/// Un frigo ou un congélateur.
#[derive(Debug, Clone, PartialEq)]
pub struct Appareil {
    pub id: String,
    pub nom: String,
    pub ordre: f64,
}

/// Une ligne de la liste.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    /// `a:<id>` pour un aliment, `l:<id>` pour un article libre.
    pub uid: String,
    pub summary: String,
    /// « 2 L », « 3 pièces ».
    pub quantity: String,
    /// L'appareil (« Frigo cuisine »), ou « Autres articles ».
    pub group: String,
    /// Épuisé (il n'en reste plus du tout).
    pub out: bool,
    /// Dans le panier (coché).
    pub done: bool,
}

impl Item {
    /// Un article libre (qu'on peut retirer de la liste) ?
    #[must_use]
    pub fn is_libre(&self) -> bool {
        self.uid.starts_with("l:")
    }
}

/// Le groupe des articles libres.
pub const LIBRES_GROUP: &str = "Autres articles";

fn clean(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_control())
        .take(MAX_NAME)
        .collect::<String>()
        .trim()
        .to_owned()
}

fn unit(text: Option<&str>) -> String {
    text.filter(|u| UNITS.contains(u))
        .unwrap_or("pièce")
        .to_owned()
}

impl Aliment {
    #[must_use]
    pub fn from_document(doc: &Document) -> Option<Self> {
        let nom = clean(doc.text("nom")?);
        let quantite = doc.number("quantite")?;
        let quantite_max = doc.number("quantiteMax")?;
        (!nom.is_empty() && quantite >= 0.0 && quantite_max > 0.0).then(|| Self {
            id: doc.id.clone(),
            nom,
            quantite,
            quantite_max,
            unite: unit(doc.text("unite")),
            dans_panier: doc.boolean("dansPanier").unwrap_or(false),
            achat: doc.number("achatQuantite").filter(|n| *n > 0.0),
            appareil_id: doc.text("appareilId").unwrap_or_default().to_owned(),
        })
    }
}

impl Libre {
    #[must_use]
    pub fn from_document(doc: &Document) -> Option<Self> {
        let nom = clean(doc.text("nom")?);
        (!nom.is_empty()).then(|| Self {
            id: doc.id.clone(),
            nom,
            quantite: doc.number("quantite").filter(|n| *n > 0.0).unwrap_or(1.0),
            unite: unit(doc.text("unite")),
            dans_panier: doc.boolean("dansPanier").unwrap_or(false),
        })
    }
}

impl Appareil {
    #[must_use]
    pub fn from_document(doc: &Document) -> Option<Self> {
        Some(Self {
            id: doc.id.clone(),
            nom: clean(doc.text("nom")?),
            ordre: doc.number("ordre").unwrap_or(0.0),
        })
    }
}

/// Arrondi au millième, comme l'appli (pas de 0,30000000000000004).
fn round(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}

/// « 2 L », « 1,5 kg », « 3 pièces », « 1 portion » (comme l'appli).
#[must_use]
pub fn quantity(value: f64, unite: &str) -> String {
    let value = round(value);
    let number = if value.fract() == 0.0 {
        format!("{value:.0}")
    } else {
        value.to_string().replace('.', ",")
    };
    let plural = if value > 1.0 { "s" } else { "" };
    match unite {
        "pièce" => format!("{number} pièce{plural}"),
        "portion" => format!("{number} portion{plural}"),
        other => format!("{number} {other}"),
    }
}

/// Compare deux noms sans tenir compte des majuscules ni des accents.
fn by_name(a: &str, b: &str) -> Ordering {
    fold(a).cmp(&fold(b)).then_with(|| a.cmp(b))
}

fn fold(text: &str) -> String {
    text.chars()
        .flat_map(char::to_lowercase)
        .map(|c| match c {
            'à' | 'â' | 'ä' => 'a',
            'ç' => 'c',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'î' | 'ï' => 'i',
            'ô' | 'ö' => 'o',
            'ù' | 'û' | 'ü' => 'u',
            other => other,
        })
        .collect()
}

/// La liste de courses.
#[must_use]
pub fn shopping_list(aliments: &[Aliment], appareils: &[Appareil], libres: &[Libre]) -> Vec<Item> {
    let mut appareils: Vec<&Appareil> = appareils.iter().collect();
    appareils.sort_by(|a, b| {
        a.ordre
            .total_cmp(&b.ordre)
            .then_with(|| by_name(&a.nom, &b.nom))
    });
    let rank = |id: &str| {
        appareils
            .iter()
            .position(|a| a.id == id)
            .unwrap_or(appareils.len())
    };
    let mut wanted: Vec<&Aliment> = aliments
        .iter()
        .filter(|a| a.quantite < a.quantite_max)
        .collect();
    wanted.sort_by(|a, b| {
        rank(&a.appareil_id)
            .cmp(&rank(&b.appareil_id))
            .then_with(|| (b.quantite <= 0.0).cmp(&(a.quantite <= 0.0)))
            .then_with(|| by_name(&a.nom, &b.nom))
    });
    let mut items: Vec<Item> = wanted
        .into_iter()
        .map(|a| {
            let missing = round((a.quantite_max - a.quantite).max(0.0));
            Item {
                uid: format!("a:{}", a.id),
                summary: a.nom.clone(),
                quantity: quantity(a.achat.unwrap_or(missing), &a.unite),
                group: appareils
                    .iter()
                    .find(|x| x.id == a.appareil_id)
                    .map_or_else(|| "Autres".to_owned(), |x| x.nom.clone()),
                out: a.quantite <= 0.0,
                done: a.dans_panier,
            }
        })
        .collect();
    let mut libres: Vec<&Libre> = libres.iter().collect();
    libres.sort_by(|a, b| by_name(&a.nom, &b.nom));
    items.extend(libres.into_iter().map(|l| Item {
        uid: format!("l:{}", l.id),
        summary: l.nom.clone(),
        quantity: quantity(l.quantite, &l.unite),
        group: LIBRES_GROUP.to_owned(),
        out: false,
        done: l.dans_panier,
    }));
    items
}

#[cfg(test)]
mod tests {
    use super::*;

    fn aliment(id: &str, nom: &str, quantite: f64, max: f64, appareil: &str) -> Aliment {
        Aliment {
            id: id.into(),
            nom: nom.into(),
            quantite,
            quantite_max: max,
            unite: "L".into(),
            dans_panier: false,
            achat: None,
            appareil_id: appareil.into(),
        }
    }

    #[test]
    fn writes_quantities_like_the_app() {
        assert_eq!(quantity(2.0, "L"), "2 L");
        assert_eq!(quantity(1.5, "kg"), "1,5 kg");
        assert_eq!(quantity(3.0, "pièce"), "3 pièces");
        assert_eq!(quantity(1.0, "pièce"), "1 pièce");
        assert_eq!(quantity(0.1 + 0.2, "L"), "0,3 L");
        assert_eq!(quantity(2.0, "portion"), "2 portions");
    }

    #[test]
    fn lists_what_is_missing_by_appliance_then_the_free_items() {
        let appareils = [
            Appareil {
                id: "c".into(),
                nom: "Congélateur".into(),
                ordre: 1.0,
            },
            Appareil {
                id: "f".into(),
                nom: "Frigo".into(),
                ordre: 0.0,
            },
        ];
        let mut beurre = aliment("b", "beurre", 1.0, 2.0, "f");
        beurre.achat = Some(3.0);
        let aliments = [
            aliment("l", "Lait", 0.0, 2.0, "f"),
            beurre,
            aliment("o", "Œufs", 6.0, 6.0, "f"),
            aliment("g", "Glace", 0.5, 1.0, "c"),
            aliment("x", "Mystère", 0.0, 1.0, "inconnu"),
        ];
        let libres = [
            Libre {
                id: "p".into(),
                nom: "Pain".into(),
                quantite: 1.0,
                unite: "pièce".into(),
                dans_panier: true,
            },
            Libre {
                id: "e".into(),
                nom: "éponges".into(),
                quantite: 2.0,
                unite: "pièce".into(),
                dans_panier: false,
            },
        ];
        let items = shopping_list(&aliments, &appareils, &libres);
        let names: Vec<_> = items
            .iter()
            .map(|i| (i.summary.as_str(), i.group.as_str(), i.quantity.as_str()))
            .collect();
        assert_eq!(
            names,
            [
                // Le frigo (ordre 0) d'abord, le lait épuisé en tête ; les œufs sont pleins.
                ("Lait", "Frigo", "2 L"),
                ("beurre", "Frigo", "3 L"),
                ("Glace", "Congélateur", "0,5 L"),
                ("Mystère", "Autres", "1 L"),
                ("éponges", LIBRES_GROUP, "2 pièces"),
                ("Pain", LIBRES_GROUP, "1 pièce"),
            ]
        );
        assert!(items[0].out && !items[1].out);
        assert_eq!(items[0].uid, "a:l");
        assert!(items[5].done && items[5].is_libre());
    }
}
