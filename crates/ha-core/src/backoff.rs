//! L'attente entre deux tentatives de reconnexion, commune à toutes les
//! intégrations (Hue, MQTT, Somfy, Freebox…).
//!
//! - L'attente **double** à chaque échec, sans dépasser un maximum : un
//!   appareil en panne n'est pas harcelé, et une coupure courte se rattrape
//!   vite.
//! - Elle repart du début (`reset`) dès que la connexion réussit.
//! - Un peu de **hasard** (« jitter ») s'y ajoute : jusqu'à 20 % de plus.
//!   Après une coupure du réseau, les intégrations ne se reconnectent donc
//!   pas toutes à la même seconde. L'attente n'est jamais plus courte que
//!   prévu : un service qui demande de patienter est respecté.
//!
//! Le hasard n'a pas besoin d'être imprévisible (ce n'est pas un secret) :
//! un petit générateur « xorshift », amorcé une fois, suffit. Pas de
//! dépendance en plus.

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::time::Duration;

/// Part maximale de hasard ajoutée à l'attente : 1/5, soit 20 %.
const JITTER_DIVISOR: u32 = 5;

/// L'attente avant la prochaine tentative.
///
/// ```
/// use std::time::Duration;
/// use ha_core::Backoff;
///
/// let mut backoff = Backoff::new(Duration::from_secs(1), Duration::from_mins(1));
/// let first = backoff.next(); // entre 1 et 1,2 s
/// let second = backoff.next(); // entre 2 et 2,4 s
/// assert!(first < second);
/// backoff.reset(); // connecté : on repart de 1 s
/// ```
#[derive(Debug, Clone)]
pub struct Backoff {
    initial: Duration,
    max: Duration,
    /// L'attente de base de la prochaine tentative (sans le hasard).
    current: Duration,
    /// L'état du petit générateur de hasard (jamais nul).
    seed: u64,
}

impl Backoff {
    /// Première attente `initial`, puis le double à chaque échec, sans
    /// dépasser `max` (hors hasard).
    #[must_use]
    pub fn new(initial: Duration, max: Duration) -> Self {
        // Concept Rust : `RandomState` est ce que `HashMap` utilise contre
        // les attaques par collisions ; ses clés sont tirées au hasard par
        // le système. Hacher « rien » avec lui donne un nombre différent à
        // chaque `Backoff`, sans aucune dépendance.
        let seed = RandomState::new().build_hasher().finish();
        Self::with_seed(initial, max, seed)
    }

    /// Comme [`Backoff::new`], avec un hasard choisi (pour les tests).
    #[must_use]
    pub fn with_seed(initial: Duration, max: Duration, seed: u64) -> Self {
        let initial = initial.min(max);
        Self {
            initial,
            max,
            current: initial,
            // Xorshift reste bloqué sur 0 : on le remplace par une constante.
            seed: if seed == 0 {
                0x9E37_79B9_7F4A_7C15
            } else {
                seed
            },
        }
    }

    /// L'attente à faire maintenant (avec le hasard), et double celle de la
    /// fois suivante.
    ///
    /// Concept Rust : `&mut self` car la méthode modifie l'état. Le nom
    /// `next` n'a rien à voir avec un `Iterator` (d'où l'autorisation).
    #[allow(clippy::should_implement_trait, reason = "pas un itérateur")]
    pub fn next(&mut self) -> Duration {
        let base = self.current;
        self.current = self.current.saturating_mul(2).min(self.max);
        base + self.jitter(base)
    }

    /// La connexion a réussi : la prochaine attente repart du début.
    pub fn reset(&mut self) {
        self.current = self.initial;
    }

    /// Vrai s'il y a eu au moins un échec depuis la dernière réussite (ou
    /// depuis le départ).
    #[must_use]
    pub fn is_retrying(&self) -> bool {
        self.current > self.initial
    }

    /// Un supplément au hasard, entre 0 et 1/5 de `base`.
    fn jitter(&mut self, base: Duration) -> Duration {
        // Xorshift 64 bits (Marsaglia, 2003) : trois décalages et « ou
        // exclusifs », un nombre d'apparence aléatoire à chaque appel.
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 7;
        self.seed ^= self.seed << 17;
        let spread = base / JITTER_DIVISOR;
        let nanos = u64::try_from(spread.as_nanos()).unwrap_or(u64::MAX);
        if nanos == 0 {
            return Duration::ZERO;
        }
        Duration::from_nanos(self.seed % (nanos + 1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECOND: Duration = Duration::from_secs(1);
    const MINUTE: Duration = Duration::from_mins(1);

    /// Vrai si `wait` est entre `base` et `base` + 20 %.
    fn around(wait: Duration, base: Duration) -> bool {
        wait >= base && wait <= base + base / JITTER_DIVISOR
    }

    #[test]
    fn doubles_up_to_the_maximum() {
        let mut backoff = Backoff::new(SECOND, MINUTE);
        for base in [1, 2, 4, 8, 16, 32, 60, 60, 60] {
            let wait = backoff.next();
            assert!(
                around(wait, Duration::from_secs(base)),
                "{wait:?} pour {base} s"
            );
        }
    }

    #[test]
    fn reset_starts_over() {
        let mut backoff = Backoff::new(SECOND * 5, MINUTE * 5);
        assert!(!backoff.is_retrying());
        backoff.next();
        backoff.next();
        assert!(backoff.is_retrying());
        backoff.reset();
        assert!(!backoff.is_retrying());
        assert!(around(backoff.next(), SECOND * 5));
    }

    #[test]
    fn jitter_stays_in_range_and_varies() {
        let mut waits = std::collections::HashSet::new();
        for seed in 0..200 {
            let mut backoff = Backoff::with_seed(SECOND * 10, MINUTE, seed);
            let wait = backoff.next();
            assert!(around(wait, SECOND * 10), "{wait:?}");
            waits.insert(wait);
        }
        // Le hasard en est vraiment un : presque toutes les attentes diffèrent.
        assert!(waits.len() > 150, "{} attentes différentes", waits.len());
    }

    #[test]
    fn initial_above_maximum_is_capped() {
        let mut backoff = Backoff::new(MINUTE * 10, MINUTE);
        assert!(around(backoff.next(), MINUTE));
        assert!(around(backoff.next(), MINUTE));
    }

    #[test]
    fn zero_delay_stays_zero() {
        let mut backoff = Backoff::new(Duration::ZERO, MINUTE);
        assert_eq!(backoff.next(), Duration::ZERO);
        assert!(!backoff.is_retrying());
    }
}
