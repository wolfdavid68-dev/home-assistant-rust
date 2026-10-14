//! Jetons d'accès : création et empreinte.

use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::error::AuthError;

/// Préfixe des jetons : il permet de les reconnaître d'un coup d'œil (et aux
/// outils de détection de secrets de les repérer s'ils fuient dans un dépôt).
pub const TOKEN_PREFIX: &str = "hat_";

/// Nombre d'octets aléatoires d'un jeton : 32 octets = 256 bits.
const TOKEN_BYTES: usize = 32;

/// Crée un nouveau jeton : `hat_` suivi de 64 caractères hexadécimaux.
///
/// Les octets viennent du générateur aléatoire **du système d'exploitation**
/// (le même que pour les clés de chiffrement), pas d'un générateur « pour les
/// jeux ». 2^256 possibilités : impossible à deviner.
///
/// Concept Rust : `Zeroizing<String>` est une `String` qui est **effacée de la
/// mémoire** (remplie de zéros) quand elle est détruite, pour qu'un secret ne
/// traîne pas en mémoire plus longtemps que nécessaire.
pub fn generate_token() -> Result<Zeroizing<String>, AuthError> {
    let mut bytes = Zeroizing::new([0u8; TOKEN_BYTES]);
    getrandom::fill(bytes.as_mut_slice()).map_err(|error| {
        AuthError::Internal(format!("générateur aléatoire indisponible : {error}"))
    })?;
    let mut token = Zeroizing::new(String::with_capacity(TOKEN_PREFIX.len() + 2 * TOKEN_BYTES));
    token.push_str(TOKEN_PREFIX);
    for byte in bytes.iter() {
        token.push(hex_digit(byte >> 4));
        token.push(hex_digit(byte & 0x0f));
    }
    Ok(token)
}

/// Empreinte SHA-256 d'un jeton : c'est elle qui est stockée dans la base.
///
/// Concept : contrairement aux mots de passe, un jeton est déjà parfaitement
/// aléatoire. Un hachage rapide (SHA-256) suffit donc : il est impossible de
/// retrouver le jeton à partir de l'empreinte, même avec beaucoup de calcul.
#[must_use]
pub fn hash_token(token: &str) -> [u8; 32] {
    Sha256::digest(token.as_bytes()).into()
}

fn hex_digit(value: u8) -> char {
    char::from(b"0123456789abcdef"[usize::from(value & 0x0f)])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_long_random_and_prefixed() {
        let a = generate_token().unwrap_or_default();
        let b = generate_token().unwrap_or_default();
        assert_eq!(a.len(), TOKEN_PREFIX.len() + 64);
        assert!(a.starts_with(TOKEN_PREFIX));
        assert!(
            a[TOKEN_PREFIX.len()..]
                .bytes()
                .all(|c| c.is_ascii_hexdigit())
        );
        assert_ne!(a, b);
    }

    #[test]
    fn hash_is_stable_and_distinct() {
        assert_eq!(hash_token("hat_abc"), hash_token("hat_abc"));
        assert_ne!(hash_token("hat_abc"), hash_token("hat_abd"));
    }
}
