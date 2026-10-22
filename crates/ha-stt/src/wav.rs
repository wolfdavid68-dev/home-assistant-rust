//! Le son envoyé par la tablette : un WAV tout simple, vérifié avant
//! d'être donné à Whisper.

use crate::{MAX_SECONDS, MAX_WAV_BYTES, SttError};

/// La fréquence attendue : celle de Whisper.
pub const RATE: u32 = 16_000;
/// Une phrase dure au moins ces millisecondes.
const MIN_MILLIS: u32 = 300;
/// Sous ce niveau (1 % du plus fort possible), c'est du silence.
const SILENCE_PEAK: i32 = 328;

/// Vérifie que `bytes` est un WAV en 16 bits, mono, à [`RATE`] Hz, entre
/// 0,3 s et [`MAX_SECONDS`] s. Rend les échantillons.
pub fn check_wav(bytes: &[u8]) -> Result<&[u8], SttError> {
    if bytes.len() > MAX_WAV_BYTES {
        return Err(SttError::BadAudio("trop long"));
    }
    if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(SttError::BadAudio("ce n'est pas un fichier WAV"));
    }
    let mut format_ok = false;
    let mut at = 12;
    while at + 8 <= bytes.len() {
        let id = &bytes[at..at + 4];
        let size = u32::from_le_bytes([bytes[at + 4], bytes[at + 5], bytes[at + 6], bytes[at + 7]])
            as usize;
        let body = at + 8;
        let end = body
            .checked_add(size)
            .filter(|end| *end <= bytes.len())
            .ok_or(SttError::BadAudio("fichier WAV tronqué"))?;
        match id {
            b"fmt " => {
                if size < 16 {
                    return Err(SttError::BadAudio("format illisible"));
                }
                let u16_at = |i: usize| u16::from_le_bytes([bytes[body + i], bytes[body + i + 1]]);
                let rate = u32::from_le_bytes([
                    bytes[body + 4],
                    bytes[body + 5],
                    bytes[body + 6],
                    bytes[body + 7],
                ]);
                if u16_at(0) != 1 || u16_at(2) != 1 || rate != RATE || u16_at(14) != 16 {
                    return Err(SttError::BadAudio("attendu : PCM 16 bits, mono, 16 000 Hz"));
                }
                format_ok = true;
            }
            b"data" => {
                if !format_ok {
                    return Err(SttError::BadAudio("format manquant"));
                }
                let samples = &bytes[body..end];
                let millis = samples.len() as u64 * 1000 / u64::from(RATE * 2);
                if millis < u64::from(MIN_MILLIS) {
                    return Err(SttError::BadAudio("trop court"));
                }
                if millis > u64::from(MAX_SECONDS) * 1000 {
                    return Err(SttError::BadAudio("trop long"));
                }
                return Ok(samples);
            }
            _ => {}
        }
        // Les morceaux font un nombre pair d'octets.
        at = end + (size & 1);
    }
    Err(SttError::BadAudio("aucun son"))
}

/// Personne n'a parlé : le son ne dépasse jamais 1 % du plus fort.
pub(crate) fn is_silent(samples: &[u8]) -> bool {
    samples
        .as_chunks::<2>()
        .0
        .iter()
        .all(|pair| i32::from(i16::from_le_bytes([pair[0], pair[1]])).abs() < SILENCE_PEAK)
}

#[cfg(test)]
pub(crate) mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]

    use super::*;

    /// Un WAV de `seconds` secondes : une note, ou du silence.
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss,
        reason = "son de test"
    )]
    pub(crate) fn tone(rate: u32, seconds: f64) -> Vec<u8> {
        let count = (f64::from(rate) * if seconds > 0.0 { seconds } else { 1.0 }) as usize;
        let level = if seconds > 0.0 { 8000.0 } else { 20.0 };
        let samples: Vec<i16> = (0..count)
            .map(|i| (level * (i as f64 * 0.1).sin()) as i16)
            .collect();
        ha_wav(rate, &samples)
    }

    fn ha_wav(rate: u32, samples: &[i16]) -> Vec<u8> {
        let data = u32::try_from(samples.len() * 2).unwrap();
        let mut out = Vec::new();
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&(36 + data).to_le_bytes());
        out.extend_from_slice(b"WAVEfmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&rate.to_le_bytes());
        out.extend_from_slice(&(rate * 2).to_le_bytes());
        out.extend_from_slice(&2u16.to_le_bytes());
        out.extend_from_slice(&16u16.to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&data.to_le_bytes());
        for s in samples {
            out.extend_from_slice(&s.to_le_bytes());
        }
        out
    }

    #[test]
    fn only_a_simple_wav_is_accepted() {
        let good = tone(RATE, 1.0);
        assert_eq!(check_wav(&good).unwrap().len(), 32_000);
        assert!(!is_silent(check_wav(&good).unwrap()));
        assert!(is_silent(check_wav(&tone(RATE, 0.0)).unwrap()));

        let refused = |bytes: &[u8]| match check_wav(bytes) {
            Err(SttError::BadAudio(why)) => why,
            other => panic!("{other:?}"),
        };
        assert_eq!(refused(b"bonjour"), "ce n'est pas un fichier WAV");
        assert_eq!(
            refused(&tone(44_100, 1.0)),
            "attendu : PCM 16 bits, mono, 16 000 Hz"
        );
        assert_eq!(refused(&tone(RATE, 0.1)), "trop court");
        assert_eq!(refused(&tone(RATE, 16.0)), "trop long");
        assert_eq!(refused(&good[..good.len() - 10]), "fichier WAV tronqué");
        // Une taille de morceau énorme ne fait pas déborder.
        let mut huge = good.clone();
        huge[40..44].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(refused(&huge), "fichier WAV tronqué");
    }
}
