//! La décision : faut-il chauffer ? Une fonction pure, sans horloge ni
//! réseau, donc facile à tester dans tous les cas.

use std::time::Duration;

/// Marche ou arrêt du thermostat (comme `hvac_mode` dans Home Assistant).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Heat,
    Off,
}

impl Mode {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Heat => "heat",
            Self::Off => "off",
        }
    }

    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "heat" => Some(Self::Heat),
            "off" => Some(Self::Off),
            _ => None,
        }
    }
}

/// Pourquoi le radiateur est (ou n'est pas) allumé.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    /// Thermostat arrêté.
    Off,
    /// Trop froid : on chauffe.
    Cold,
    /// Assez chaud : on ne chauffe pas.
    Warm,
    /// Dans la marge : on ne change rien (hystérésis).
    Hold,
    /// Température inconnue : par sécurité, on ne chauffe pas.
    NoTemperature,
    /// Fenêtre ouverte : on ne chauffe pas la rue.
    WindowOpen,
    /// Au-dessus de la température maximale : on coupe, quoi qu'il arrive.
    TooHot,
}

impl Reason {
    /// Une raison de **sécurité** coupe tout de suite, sans attendre la
    /// durée minimale entre deux changements.
    #[must_use]
    pub fn is_safety(self) -> bool {
        matches!(
            self,
            Self::Off | Self::NoTemperature | Self::WindowOpen | Self::TooHot
        )
    }

    /// Message pour le tableau de bord (`None` quand tout va bien).
    #[must_use]
    pub fn info(self) -> Option<&'static str> {
        match self {
            Self::NoTemperature => Some("température inconnue : chauffage coupé par sécurité"),
            Self::WindowOpen => Some("fenêtre ouverte : chauffage coupé"),
            Self::TooHot => Some("température maximale atteinte : chauffage coupé"),
            _ => None,
        }
    }
}

/// Ce que le thermostat sait.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Inputs {
    pub mode: Mode,
    pub target: f64,
    /// Marge autour de la consigne (°C).
    pub tolerance: f64,
    /// Température maximale absolue (°C).
    pub max_temp: f64,
    /// Température mesurée, `None` si inconnue ou indisponible.
    pub current: Option<f64>,
    pub window_open: bool,
    /// Le radiateur est-il allumé en ce moment ?
    pub heater_on: bool,
}

/// Faut-il que le radiateur soit allumé ? Et pourquoi.
///
/// L'**hystérésis** évite d'allumer et d'éteindre sans arrêt autour de la
/// consigne : avec 19 °C et une marge de 0,3, on allume à 18,7 °C et on
/// éteint à 19,3 °C. Entre les deux, on ne change rien.
#[must_use]
pub fn decide(inputs: Inputs) -> (bool, Reason) {
    if inputs.mode == Mode::Off {
        return (false, Reason::Off);
    }
    if inputs.window_open {
        return (false, Reason::WindowOpen);
    }
    let Some(current) = inputs.current else {
        return (false, Reason::NoTemperature);
    };
    if current >= inputs.max_temp {
        return (false, Reason::TooHot);
    }
    if current <= inputs.target - inputs.tolerance {
        (true, Reason::Cold)
    } else if current >= inputs.target + inputs.tolerance {
        (false, Reason::Warm)
    } else {
        (inputs.heater_on, Reason::Hold)
    }
}

/// Peut-on changer l'état du radiateur maintenant ?
///
/// Un relais ou une chaudière n'aiment pas les marche/arrêt trop rapprochés :
/// on attend `min_cycle` depuis le dernier changement. Sauf pour la sécurité.
#[must_use]
pub fn may_switch(
    reason: Reason,
    since_last_switch: Option<Duration>,
    min_cycle: Duration,
) -> bool {
    reason.is_safety() || since_last_switch.is_none_or(|elapsed| elapsed >= min_cycle)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs(current: Option<f64>, heater_on: bool) -> Inputs {
        Inputs {
            mode: Mode::Heat,
            target: 19.0,
            tolerance: 0.3,
            max_temp: 25.0,
            current,
            window_open: false,
            heater_on,
        }
    }

    #[test]
    fn hysteresis_around_the_target() {
        // Froid : on allume.
        assert_eq!(decide(inputs(Some(18.5), false)), (true, Reason::Cold));
        assert_eq!(decide(inputs(Some(18.7), false)), (true, Reason::Cold));
        // Dans la marge : on garde l'état actuel, quel qu'il soit.
        assert_eq!(decide(inputs(Some(19.0), false)), (false, Reason::Hold));
        assert_eq!(decide(inputs(Some(19.0), true)), (true, Reason::Hold));
        assert_eq!(decide(inputs(Some(19.2), true)), (true, Reason::Hold));
        // Assez chaud : on éteint.
        assert_eq!(decide(inputs(Some(19.3), true)), (false, Reason::Warm));
    }

    #[test]
    fn safety_always_wins() {
        let off = Inputs {
            mode: Mode::Off,
            ..inputs(Some(10.0), true)
        };
        assert_eq!(decide(off), (false, Reason::Off));
        let window = Inputs {
            window_open: true,
            ..inputs(Some(10.0), true)
        };
        assert_eq!(decide(window), (false, Reason::WindowOpen));
        assert_eq!(decide(inputs(None, true)), (false, Reason::NoTemperature));
        let hot = Inputs {
            target: 30.0,
            ..inputs(Some(25.0), true)
        };
        assert_eq!(decide(hot), (false, Reason::TooHot));
    }

    #[test]
    fn minimum_cycle_except_for_safety() {
        let min = Duration::from_mins(5);
        assert!(may_switch(Reason::Cold, None, min), "premier allumage");
        assert!(!may_switch(Reason::Cold, Some(Duration::from_mins(2)), min));
        assert!(may_switch(Reason::Warm, Some(Duration::from_mins(5)), min));
        for safety in [
            Reason::Off,
            Reason::WindowOpen,
            Reason::NoTemperature,
            Reason::TooHot,
        ] {
            assert!(may_switch(safety, Some(Duration::ZERO), min), "{safety:?}");
        }
    }
}
