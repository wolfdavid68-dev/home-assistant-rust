//! Mots de passe : règles et hachage Argon2id.

use argon2::Argon2;
use argon2::password_hash::phc::PasswordHash;
use argon2::password_hash::{PasswordHasher, PasswordVerifier};

use crate::error::AuthError;

/// Longueur minimale d'un mot de passe, en caractères.
pub const MIN_PASSWORD_LEN: usize = 12;
/// Longueur maximale : au-delà, ce n'est plus un mot de passe mais une
/// tentative de faire travailler le serveur pour rien.
pub const MAX_PASSWORD_LEN: usize = 256;

/// Vérifie la longueur d'un nouveau mot de passe.
///
/// On suit les recommandations actuelles (ANSSI, NIST) : la **longueur** compte
/// plus que les règles « une majuscule, un chiffre, un symbole ». Une phrase de
/// passe comme `cafe chaud sur la terrasse` est à la fois solide et mémorisable.
pub fn check_password_policy(password: &str) -> Result<(), AuthError> {
    let len = password.chars().count();
    if len < MIN_PASSWORD_LEN {
        return Err(AuthError::PasswordTooShort {
            min: MIN_PASSWORD_LEN,
        });
    }
    if len > MAX_PASSWORD_LEN {
        return Err(AuthError::PasswordTooLong {
            max: MAX_PASSWORD_LEN,
        });
    }
    Ok(())
}

/// Calcule l'empreinte Argon2id d'un mot de passe, avec un sel aléatoire.
///
/// Le résultat (format « PHC ») ressemble à `$argon2id$v=19$m=19456,t=2,p=1$<sel>$<empreinte>` :
/// il contient tout ce qu'il faut pour vérifier plus tard, sauf le mot de passe.
///
/// Concept : Argon2id est **volontairement lent** et gourmand en mémoire
/// (environ 19 Mo par calcul avec les paramètres par défaut, ceux recommandés
/// par l'OWASP). Pour toi, une connexion prend quelques dizaines de
/// millisecondes ; pour un attaquant qui a volé la base et essaie des
/// milliards de mots de passe, cela devient hors de prix.
pub fn hash_password(password: &str) -> Result<String, AuthError> {
    let hash: PasswordHash = Argon2::default()
        .hash_password(password.as_bytes())
        .map_err(|error| AuthError::Internal(format!("hachage impossible : {error}")))?;
    Ok(hash.to_string())
}

/// Vérifie un mot de passe contre une empreinte. Renvoie `false` aussi si
/// l'empreinte est illisible.
#[must_use]
pub fn verify_password(password: &str, hash: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(hash) else {
        return false;
    };
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_then_verify() {
        let hash = hash_password("une phrase de passe solide").unwrap_or_default();
        assert!(hash.starts_with("$argon2id$"));
        assert!(verify_password("une phrase de passe solide", &hash));
        assert!(!verify_password("une phrase de passe solidE", &hash));
    }

    #[test]
    fn same_password_gives_different_hashes() {
        // Le sel aléatoire rend chaque empreinte unique.
        let a = hash_password("une phrase de passe solide").unwrap_or_default();
        let b = hash_password("une phrase de passe solide").unwrap_or_default();
        assert_ne!(a, b);
    }

    #[test]
    fn garbage_hash_never_matches() {
        assert!(!verify_password("n'importe quoi", "pas une empreinte"));
        assert!(!verify_password("", ""));
    }

    #[test]
    fn policy() {
        assert!(check_password_policy("trop court").is_err());
        assert!(check_password_policy("douze carac.").is_ok());
        assert!(check_password_policy(&"é".repeat(MAX_PASSWORD_LEN)).is_ok());
        assert!(check_password_policy(&"a".repeat(MAX_PASSWORD_LEN + 1)).is_err());
    }
}
