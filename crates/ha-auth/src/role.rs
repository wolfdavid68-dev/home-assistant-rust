//! Les **rôles** des comptes, et ce que chacun a le droit de faire.
//!
//! | | Administrateur | Famille | Invité |
//! |---|---|---|---|
//! | Voir et commander les appareils | tout | tout | une partie |
//! | Historique, journal de bord | oui | oui | non |
//! | Lire les automatisations | oui | oui | non |
//! | Modifier les automatisations | oui | non | non |
//! | Écrire un état à la main, journal d'audit | oui | non | non |
//!
//! Ce que voit et commande un invité dépend de l'appareil : voir `ha-api`.

/// Le rôle d'un compte.
///
/// Concept Rust : `Copy` sur un petit `enum` sans données permet de le
/// passer par valeur partout, comme un nombre.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// Tout, y compris modifier les automatisations.
    Admin,
    /// Tout piloter, tout voir, mais pas modifier les automatisations.
    Family,
    /// Quelques appareils seulement (lumières, prises…), pas la présence.
    Guest,
}

/// Ce qu'on peut vouloir faire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Permission {
    /// Historique et journal de bord (qui était là, quand).
    ReadHistory,
    /// Lire les automatisations.
    ReadAutomations,
    /// Créer, modifier, supprimer des automatisations.
    EditAutomations,
    /// Écrire ou supprimer un état directement (sans passer par l'appareil).
    WriteStates,
    /// Lire le journal d'audit.
    ReadAudit,
    /// Voir et commander **tous** les appareils (sinon : seulement ceux
    /// qu'un invité peut voir et commander).
    AllDevices,
    /// Changer l'apparence commune du tableau de bord (le dessin de la maison).
    EditDashboard,
}

impl Role {
    /// Le nom rangé dans la base.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Admin => "admin",
            Self::Family => "family",
            Self::Guest => "guest",
        }
    }

    /// Le nom affiché.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Admin => "administrateur",
            Self::Family => "famille",
            Self::Guest => "invité",
        }
    }

    /// Lit un rôle, en anglais (base) ou en français (ligne de commande).
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "admin" | "administrateur" => Some(Self::Admin),
            "family" | "famille" => Some(Self::Family),
            "guest" | "invite" | "invité" => Some(Self::Guest),
            _ => None,
        }
    }

    /// Ce rôle a-t-il le droit de… ?
    ///
    /// Concept Rust : `matches!` sur deux valeurs à la fois. Tout ce qui
    /// n'est pas listé est refusé : un droit ajouté plus tard est refusé par
    /// défaut, jamais donné par erreur.
    #[must_use]
    pub fn can(self, permission: Permission) -> bool {
        matches!(
            (self, permission),
            (Self::Admin, _)
                | (
                    Self::Family,
                    Permission::ReadHistory | Permission::ReadAutomations | Permission::AllDevices
                )
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roles_are_read_and_written() {
        for role in [Role::Admin, Role::Family, Role::Guest] {
            assert_eq!(Role::parse(role.as_str()), Some(role));
        }
        assert_eq!(Role::parse("famille"), Some(Role::Family));
        assert_eq!(Role::parse("invité"), Some(Role::Guest));
        assert_eq!(Role::parse("root"), None);
    }

    #[test]
    fn each_role_gets_only_its_rights() {
        let all = [
            Permission::ReadHistory,
            Permission::ReadAutomations,
            Permission::EditAutomations,
            Permission::WriteStates,
            Permission::ReadAudit,
            Permission::AllDevices,
            Permission::EditDashboard,
        ];
        let allowed = |role: Role| all.into_iter().filter(|p| role.can(*p)).collect::<Vec<_>>();
        assert_eq!(allowed(Role::Admin), all);
        assert_eq!(
            allowed(Role::Family),
            [
                Permission::ReadHistory,
                Permission::ReadAutomations,
                Permission::AllDevices
            ]
        );
        assert_eq!(allowed(Role::Guest), Vec::new());
    }
}
