//! **Qui** a demandé une commande : une personne, une automatisation, un
//! script… Le journal de bord s'en sert pour écrire « Lumière du salon
//! allumée par l'automatisation « Bonne nuit » ».

/// L'origine d'un appel de service.
///
/// Concept Rust : `#[derive(Default)]` avec `#[default]` sur une variante
/// donne la valeur « par défaut » : ici, le serveur lui-même.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Origin {
    /// Le serveur lui-même (thermostat, sauvegarde…).
    #[default]
    System,
    /// Une personne, par son nom d'utilisateur (tableau de bord, API).
    User(String),
    /// Une automatisation, par son nom.
    Automation(String),
    /// Un script, par son nom.
    Script(String),
    /// Une scène, par son nom.
    Scene(String),
    /// La voix, avec Alexa.
    Voice,
}

impl Origin {
    /// « par David », « par l'automatisation « Bonne nuit » »… `None` pour
    /// le serveur lui-même.
    #[must_use]
    pub fn describe(&self) -> Option<String> {
        match self {
            Self::System => None,
            Self::User(name) => Some(format!("par {name}")),
            Self::Automation(name) => Some(format!("par l'automatisation « {name} »")),
            Self::Script(name) => Some(format!("par le script « {name} »")),
            Self::Scene(name) => Some(format!("par la scène « {name} »")),
            Self::Voice => Some("à la voix (Alexa)".to_owned()),
        }
    }
}
