//! Codes à usage unique (TOTP, RFC 6238), pour la double authentification.
//!
//! L'appli d'authentification du téléphone (Google Authenticator, Aegis,
//! 2FAS, celle de ton gestionnaire de mots de passe…) et le serveur
//! partagent un **secret**. Toutes les 30 secondes, chacun calcule un code
//! à 6 chiffres à partir de ce secret et de l'heure :
//!
//! ```text
//! code = HMAC-SHA1(secret, numéro du créneau de 30 s) → 6 chiffres
//! ```
//!
//! Un mot de passe volé ne suffit plus : il faut aussi le téléphone.

use std::time::{SystemTime, UNIX_EPOCH};

use hmac::{Hmac, KeyInit, Mac};
use sha1::Sha1;
use zeroize::Zeroizing;

use crate::AuthError;

/// Durée d'un créneau.
pub const PERIOD_SECS: u64 = 30;
/// Nombre de chiffres du code.
pub const DIGITS: usize = 6;
/// 10^6 : on garde les 6 derniers chiffres.
const MODULUS: u32 = 1_000_000;
/// Taille du secret : 160 bits, comme recommandé par la RFC 4226.
pub const SECRET_LEN: usize = 20;
/// Nom affiché dans l'appli d'authentification.
pub const ISSUER: &str = "ha-rust";

/// Un nouveau secret, prêt à être montré à l'utilisateur.
pub struct Setup {
    pub secret: Zeroizing<Vec<u8>>,
    /// Le secret en base 32 (à saisir à la main si le QR code ne passe pas).
    pub secret_base32: Zeroizing<String>,
    /// Adresse `otpauth://` à mettre dans le QR code.
    pub uri: Zeroizing<String>,
}

/// Le secret ne doit jamais apparaître dans un log.
impl std::fmt::Debug for Setup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Setup(<masqué>)")
    }
}

/// Crée un secret aléatoire pour le compte `username`.
pub fn new_setup(username: &str) -> Result<Setup, AuthError> {
    let mut secret = Zeroizing::new(vec![0u8; SECRET_LEN]);
    getrandom::fill(&mut secret)
        .map_err(|error| AuthError::Internal(format!("générateur aléatoire : {error}")))?;
    let secret_base32 = Zeroizing::new(base32_encode(&secret));
    // Le nom de compte n'a que des caractères sûrs (a-z, 0-9, _ - .) : pas
    // besoin d'encodage dans l'adresse.
    let uri = Zeroizing::new(format!(
        "otpauth://totp/{ISSUER}:{username}?secret={}&issuer={ISSUER}&algorithm=SHA1&digits={DIGITS}&period={PERIOD_SECS}",
        secret_base32.as_str()
    ));
    Ok(Setup {
        secret,
        secret_base32,
        uri,
    })
}

/// Numéro du créneau de 30 s qui contient `now`.
#[must_use]
pub fn step_at(now: SystemTime) -> u64 {
    now.duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs() / PERIOD_SECS)
}

/// Le code d'un créneau (RFC 4226, « HOTP »).
#[must_use]
pub fn code_at(secret: &[u8], step: u64) -> u32 {
    let Ok(mut mac) = <Hmac<Sha1> as KeyInit>::new_from_slice(secret) else {
        // HMAC accepte des clés de toute taille : ce cas n'arrive pas.
        return u32::MAX;
    };
    mac.update(&step.to_be_bytes());
    let digest = mac.finalize().into_bytes();
    // « Troncature dynamique » : les 4 bits de poids faible du dernier octet
    // disent où lire 4 octets dans l'empreinte.
    let offset = usize::from(digest.last().copied().unwrap_or(0) & 0x0f);
    let Some(&[a, b, c, d]) = digest.get(offset..offset + 4).and_then(|s| s.first_chunk()) else {
        return u32::MAX;
    };
    let value = u32::from_be_bytes([a & 0x7f, b, c, d]);
    value % MODULUS
}

/// Vérifie `code` pour le créneau `now_step`, en tolérant un créneau d'écart
/// (horloge du téléphone un peu en avance ou en retard).
///
/// Un code d'un créneau déjà utilisé (`<= last_step`) est refusé : un code
/// intercepté ne peut pas être rejoué. Renvoie le créneau accepté.
#[must_use]
pub fn verify(secret: &[u8], code: &str, now_step: u64, last_step: Option<u64>) -> Option<u64> {
    let code: String = code.chars().filter(|c| !c.is_whitespace()).collect();
    if code.len() != DIGITS || !code.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let expected: u32 = code.parse().ok()?;
    let mut accepted = None;
    // On calcule toujours les trois codes, qu'un code corresponde ou non,
    // pour que le temps de réponse ne dise rien.
    for step in [
        now_step.saturating_sub(1),
        now_step,
        now_step.saturating_add(1),
    ] {
        let fresh = last_step.is_none_or(|last| step > last);
        if code_at(secret, step) == expected && fresh && accepted.is_none() {
            accepted = Some(step);
        }
    }
    accepted
}

const BASE32: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

/// Encodage base 32 (RFC 4648), sans `=` : le format des applis TOTP.
#[must_use]
pub fn base32_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(5) * 8);
    let mut buffer: u32 = 0;
    let mut bits = 0;
    for &byte in bytes {
        buffer = (buffer << 8) | u32::from(byte);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(char::from(BASE32[((buffer >> bits) & 31) as usize]));
        }
    }
    if bits > 0 {
        out.push(char::from(BASE32[((buffer << (5 - bits)) & 31) as usize]));
    }
    out
}

/// Décodage base 32 (majuscules, sans `=`). `None` si un caractère est invalide.
#[must_use]
pub fn base32_decode(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() * 5 / 8);
    let mut buffer: u32 = 0;
    let mut bits = 0;
    for c in text.bytes() {
        let value = BASE32.iter().position(|&b| b == c)?;
        buffer = (buffer << 5) | u32::try_from(value).ok()?;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push(u8::try_from((buffer >> bits) & 0xff).ok()?);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Secret des exemples de la RFC 6238 (SHA-1) : « 12345678901234567890 ».
    const RFC_SECRET: &[u8] = b"12345678901234567890";

    #[test]
    fn rfc_6238_test_vectors() {
        // Les codes de la RFC ont 8 chiffres : on garde les 6 derniers.
        for (time, code) in [
            (59, 94_287_082),
            (1_111_111_109, 7_081_804),
            (1_111_111_111, 14_050_471),
            (1_234_567_890, 89_005_924),
            (2_000_000_000, 69_279_037),
            (20_000_000_000, 65_353_130),
        ] {
            let step = step_at(UNIX_EPOCH + std::time::Duration::from_secs(time));
            assert_eq!(code_at(RFC_SECRET, step), code % 1_000_000, "t = {time}");
        }
    }

    #[test]
    fn accepts_neighbour_steps_and_refuses_replays() {
        let step = 1_000;
        let code = format!("{:06}", code_at(RFC_SECRET, step));
        assert_eq!(verify(RFC_SECRET, &code, step, None), Some(step));
        // Horloge du téléphone décalée d'un créneau : accepté.
        assert_eq!(verify(RFC_SECRET, &code, step + 1, None), Some(step));
        assert_eq!(verify(RFC_SECRET, &code, step - 1, None), Some(step));
        // Deux créneaux d'écart : refusé.
        assert_eq!(verify(RFC_SECRET, &code, step + 2, None), None);
        // Déjà utilisé : refusé.
        assert_eq!(verify(RFC_SECRET, &code, step, Some(step)), None);
        // Espaces tolérés (« 123 456 »).
        let spaced = format!("{} {}", &code[..3], &code[3..]);
        assert_eq!(verify(RFC_SECRET, &spaced, step, None), Some(step));
    }

    #[test]
    fn malformed_codes_are_refused() {
        for code in ["", "12345", "1234567", "abcdef", "12345a", "١٢٣٤٥٦"] {
            assert_eq!(verify(RFC_SECRET, code, 1_000, None), None, "{code:?}");
        }
    }

    #[test]
    fn base32_matches_rfc_4648() {
        assert_eq!(base32_encode(b""), "");
        assert_eq!(base32_encode(b"f"), "MY");
        assert_eq!(base32_encode(b"foobar"), "MZXW6YTBOI");
        assert_eq!(base32_decode("MZXW6YTBOI").unwrap(), b"foobar");
        assert_eq!(base32_decode("mzxw"), None);
        let setup = new_setup("david").unwrap();
        assert_eq!(setup.secret.len(), SECRET_LEN);
        assert_eq!(
            base32_decode(&setup.secret_base32).unwrap(),
            setup.secret.as_slice()
        );
        assert!(
            setup
                .uri
                .starts_with("otpauth://totp/ha-rust:david?secret=")
        );
        assert_eq!(format!("{setup:?}"), "Setup(<masqué>)");
    }
}
