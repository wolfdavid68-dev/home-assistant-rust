//! Limitation des tentatives de connexion, par adresse IP.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

/// Échecs autorisés dans la fenêtre avant blocage.
pub const MAX_FAILURES: u32 = 5;
/// Fenêtre pendant laquelle les échecs sont comptés.
pub const FAILURE_WINDOW: Duration = Duration::from_secs(15 * 60);
/// Durée du blocage.
pub const LOCKOUT: Duration = Duration::from_secs(15 * 60);
/// Nombre maximal d'adresses suivies, pour que la table ne remplisse pas la mémoire.
const MAX_TRACKED: usize = 10_000;

#[derive(Debug, Clone, Copy)]
struct Attempts {
    failures: u32,
    window_start: Instant,
    locked_until: Option<Instant>,
}

/// Après [`MAX_FAILURES`] échecs en [`FAILURE_WINDOW`], l'adresse IP est bloquée
/// pendant [`LOCKOUT`]. Même le bon mot de passe est alors refusé : un
/// attaquant ne peut plus essayer que quelques mots de passe par quart d'heure.
#[derive(Debug, Default)]
pub struct LoginLimiter {
    attempts: Mutex<HashMap<IpAddr, Attempts>>,
}

impl LoginLimiter {
    pub fn new() -> Self {
        Self::default()
    }

    /// `Err(durée)` si l'adresse est bloquée, avec le temps restant.
    pub fn check(&self, ip: IpAddr) -> Result<(), Duration> {
        self.check_at(ip, Instant::now())
    }

    pub fn record_failure(&self, ip: IpAddr) {
        self.record_failure_at(ip, Instant::now());
    }

    /// Une connexion réussie efface les échecs précédents.
    pub fn record_success(&self, ip: IpAddr) {
        self.lock().remove(&ip);
    }

    fn check_at(&self, ip: IpAddr, now: Instant) -> Result<(), Duration> {
        let attempts = self.lock();
        match attempts.get(&ip).and_then(|a| a.locked_until) {
            Some(until) if until > now => Err(until - now),
            // Table pleine (attaque depuis un très grand nombre d'adresses) :
            // on refuse les nouvelles adresses plutôt que d'oublier les anciennes.
            None if attempts.len() >= MAX_TRACKED && !attempts.contains_key(&ip) => Err(LOCKOUT),
            _ => Ok(()),
        }
    }

    fn record_failure_at(&self, ip: IpAddr, now: Instant) {
        let mut attempts = self.lock();
        if attempts.len() >= MAX_TRACKED {
            // Fait le ménage des entrées qui ne servent plus à rien.
            attempts.retain(|_, a| match a.locked_until {
                Some(until) => until > now,
                None => now.duration_since(a.window_start) < FAILURE_WINDOW,
            });
            if attempts.len() >= MAX_TRACKED && !attempts.contains_key(&ip) {
                return;
            }
        }
        let entry = attempts.entry(ip).or_insert(Attempts {
            failures: 0,
            window_start: now,
            locked_until: None,
        });
        // Fenêtre expirée ou blocage terminé : on repart de zéro.
        let lock_expired = entry.locked_until.is_some_and(|until| until <= now);
        if lock_expired || now.duration_since(entry.window_start) >= FAILURE_WINDOW {
            *entry = Attempts {
                failures: 0,
                window_start: now,
                locked_until: None,
            };
        }
        entry.failures += 1;
        if entry.failures >= MAX_FAILURES {
            entry.locked_until = Some(now + LOCKOUT);
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<IpAddr, Attempts>> {
        // Données toujours cohérentes (modifiées en une fois) : on ignore l'empoisonnement.
        self.attempts.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;

    use super::*;

    const IP: IpAddr = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 10));
    const OTHER: IpAddr = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 11));

    #[test]
    fn locks_after_too_many_failures_then_unlocks() {
        let limiter = LoginLimiter::new();
        let start = Instant::now();
        for _ in 0..MAX_FAILURES - 1 {
            limiter.record_failure_at(IP, start);
            assert!(limiter.check_at(IP, start).is_ok());
        }
        limiter.record_failure_at(IP, start);
        assert!(limiter.check_at(IP, start).is_err());
        // Une autre adresse n'est pas touchée.
        assert!(limiter.check_at(OTHER, start).is_ok());
        // Le blocage se lève tout seul.
        assert!(limiter.check_at(IP, start + LOCKOUT).is_ok());
    }

    #[test]
    fn old_failures_are_forgotten() {
        let limiter = LoginLimiter::new();
        let start = Instant::now();
        for _ in 0..MAX_FAILURES - 1 {
            limiter.record_failure_at(IP, start);
        }
        let later = start + FAILURE_WINDOW;
        limiter.record_failure_at(IP, later);
        assert!(limiter.check_at(IP, later).is_ok());
    }

    #[test]
    fn success_resets_the_counter() {
        let limiter = LoginLimiter::new();
        let start = Instant::now();
        for _ in 0..MAX_FAILURES - 1 {
            limiter.record_failure_at(IP, start);
        }
        limiter.record_success(IP);
        limiter.record_failure_at(IP, start);
        assert!(limiter.check_at(IP, start).is_ok());
    }
}
