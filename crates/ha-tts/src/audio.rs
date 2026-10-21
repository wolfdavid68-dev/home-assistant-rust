//! Le son d'une annonce : la voix, le carillon, et le fichier WAV que
//! l'enceinte télécharge.
//!
//! Piper parle à 22 050 échantillons par seconde, en mono. Les Sonos lisent
//! à coup sûr le format du CD (44 100 par seconde, stéréo, 16 bits) : on
//! convertit tout dans ce format avant d'écrire le fichier.

use std::time::Duration;

/// Le point le plus fort de la voix : -1 dB sous le maximum.
const PEAK: i16 = 29_000;
/// Le format du fichier : celui d'un CD.
pub const RATE: u32 = 44_100;
const CHANNELS: u16 = 2;

/// Du son brut : des échantillons de 16 bits, entrelacés si stéréo
/// (gauche, droite, gauche…).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pcm {
    pub rate: u32,
    pub channels: u16,
    pub samples: Vec<i16>,
}

impl Pcm {
    /// Lit des octets « petit-boutistes » (l'ordre de Piper et de Wyoming).
    /// Un octet orphelin à la fin est ignoré.
    #[must_use]
    pub fn from_le_bytes(rate: u32, channels: u16, bytes: &[u8]) -> Self {
        let samples = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| i16::from_le_bytes(*pair))
            .collect();
        Self {
            rate,
            channels,
            samples,
        }
    }

    /// Combien de temps dure ce son.
    #[must_use]
    pub fn duration(&self) -> Duration {
        if self.rate == 0 || self.channels == 0 {
            return Duration::ZERO;
        }
        let frames = self.samples.len() / usize::from(self.channels);
        // Moins d'une heure de son : la conversion est exacte.
        #[expect(
            clippy::cast_precision_loss,
            reason = "quelques millions d'échantillons au plus"
        )]
        let seconds = frames as f64 / f64::from(self.rate);
        Duration::from_secs_f64(seconds)
    }

    /// Le même son, au format du CD (44 100 Hz, stéréo).
    ///
    /// On passe d'abord en mono (moyenne des voies), puis on calcule chaque
    /// nouvel échantillon entre ses deux voisins (« interpolation linéaire ») :
    /// pour une voix, c'est inaudible.
    #[must_use]
    pub fn to_cd(&self) -> Self {
        let mono = self.mono();
        if mono.is_empty() || self.rate == 0 {
            return Self {
                rate: RATE,
                channels: CHANNELS,
                samples: Vec::new(),
            };
        }
        let step = f64::from(self.rate) / f64::from(RATE);
        #[expect(
            clippy::cast_precision_loss,
            reason = "quelques millions d'échantillons au plus"
        )]
        let length = mono.len() as f64;
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "positif et borné par la longueur du son"
        )]
        let frames = (length / step).floor() as usize;
        let mut samples = Vec::with_capacity(frames * 2);
        for frame in 0..frames {
            #[expect(clippy::cast_precision_loss, reason = "borné par la longueur du son")]
            let position = frame as f64 * step;
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "positif et inférieur à la longueur du son"
            )]
            let index = position.floor() as usize;
            let fraction = position - position.floor();
            let a = mono[index.min(mono.len() - 1)];
            let b = mono[(index + 1).min(mono.len() - 1)];
            let value = a + (b - a) * fraction;
            let sample = to_i16(value);
            samples.push(sample);
            samples.push(sample);
        }
        Self {
            rate: RATE,
            channels: CHANNELS,
            samples,
        }
    }

    fn mono(&self) -> Vec<f64> {
        let channels = usize::from(self.channels.max(1));
        self.samples
            .chunks_exact(channels)
            .map(|frame| {
                let sum: f64 = frame.iter().map(|s| f64::from(*s)).sum();
                #[expect(clippy::cast_precision_loss, reason = "quelques voies")]
                let count = channels as f64;
                sum / count
            })
            .collect()
    }

    /// Ajoute `other` à la suite (même format attendu).
    pub fn append(&mut self, other: &Self) {
        self.samples.extend_from_slice(&other.samples);
    }

    /// Baisse le son si son point le plus fort dépasse `peak` : une voix
    /// qui touche le maximum grésille sur certaines enceintes.
    pub fn limit_peak(&mut self, peak: i16) {
        let loudest = self
            .samples
            .iter()
            .map(|s| s.unsigned_abs())
            .max()
            .unwrap_or(0);
        if loudest <= peak.unsigned_abs() {
            return;
        }
        let gain = f64::from(peak) / f64::from(loudest);
        for sample in &mut self.samples {
            *sample = to_i16(f64::from(*sample) * gain);
        }
    }

    /// Du silence au format du CD.
    #[must_use]
    pub fn silence(duration: Duration) -> Self {
        Self {
            rate: RATE,
            channels: CHANNELS,
            samples: vec![0; frames_for(duration) * 2],
        }
    }
}

fn frames_for(duration: Duration) -> usize {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "quelques secondes de son"
    )]
    let frames = (duration.as_secs_f64() * f64::from(RATE)).round() as usize;
    frames
}

fn to_i16(value: f64) -> i16 {
    #[expect(clippy::cast_possible_truncation, reason = "borné juste avant")]
    let sample = value
        .round()
        .clamp(f64::from(i16::MIN), f64::from(i16::MAX)) as i16;
    sample
}

/// Le carillon « ding-dong » : deux notes (mi, puis do) qui s'éteignent
/// doucement, comme une sonnette. Calculé ici : aucun fichier à fournir.
#[must_use]
pub fn chime() -> Pcm {
    let mut samples = Vec::new();
    for (frequency, seconds) in [(659.25, 0.45), (523.25, 0.9)] {
        let frames = frames_for(Duration::from_secs_f64(seconds));
        for frame in 0..frames {
            #[expect(clippy::cast_precision_loss, reason = "moins d'une seconde de son")]
            let t = frame as f64 / f64::from(RATE);
            // Une attaque de 5 ms (pas de « clic »), puis une décroissance.
            let attack = (t / 0.005).min(1.0);
            let envelope = attack * (-t * 4.0).exp();
            let tone = (std::f64::consts::TAU * frequency * t).sin()
                + 0.3 * (std::f64::consts::TAU * 2.0 * frequency * t).sin();
            let sample = to_i16(tone * envelope * 0.35 * f64::from(i16::MAX) / 1.3);
            samples.push(sample);
            samples.push(sample);
        }
    }
    Pcm {
        rate: RATE,
        channels: CHANNELS,
        samples,
    }
}

/// Une annonce prête à être lue : le fichier WAV et sa durée.
#[derive(Debug, Clone)]
pub struct Announcement {
    pub wav: Vec<u8>,
    pub duration: Duration,
}

/// Assemble une annonce : un court silence (certaines enceintes mangent le
/// début du son), le carillon s'il est demandé, puis la voix.
#[must_use]
pub fn announcement(with_chime: bool, voice: Option<&Pcm>) -> Announcement {
    let mut sound = Pcm::silence(Duration::from_millis(300));
    if with_chime {
        sound.append(&chime());
    }
    if let Some(voice) = voice {
        if with_chime {
            sound.append(&Pcm::silence(Duration::from_millis(150)));
        }
        let mut voice = voice.to_cd();
        voice.limit_peak(PEAK);
        sound.append(&voice);
    }
    sound.append(&Pcm::silence(Duration::from_millis(300)));
    Announcement {
        duration: sound.duration(),
        wav: wav(&sound),
    }
}

/// Écrit un fichier WAV (en-tête RIFF de 44 octets, puis le son).
#[must_use]
pub fn wav(pcm: &Pcm) -> Vec<u8> {
    let data_len = u32::try_from(pcm.samples.len() * 2).unwrap_or(u32::MAX - 36);
    let block_align = pcm.channels * 2;
    let byte_rate = pcm.rate * u32::from(block_align);
    let mut out = Vec::with_capacity(44 + pcm.samples.len() * 2);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&pcm.channels.to_le_bytes());
    out.extend_from_slice(&pcm.rate.to_le_bytes());
    out.extend_from_slice(&byte_rate.to_le_bytes());
    out.extend_from_slice(&block_align.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for sample in &pcm.samples {
        out.extend_from_slice(&sample.to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wav_file_has_the_cd_header() {
        let sound = announcement(true, None);
        let wav = &sound.wav;
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..16], b"WAVEfmt ");
        assert_eq!(u16::from_le_bytes([wav[22], wav[23]]), 2);
        assert_eq!(
            u32::from_le_bytes([wav[24], wav[25], wav[26], wav[27]]),
            44_100
        );
        assert_eq!(u16::from_le_bytes([wav[34], wav[35]]), 16);
        let data_len = u32::from_le_bytes([wav[40], wav[41], wav[42], wav[43]]) as usize;
        assert_eq!(wav.len(), 44 + data_len);
        // 0,3 s + carillon 1,35 s + 0,3 s.
        let ms = sound.duration.as_millis();
        assert!((1940..=1960).contains(&ms), "{ms} ms");
    }

    #[test]
    fn the_voice_is_converted_to_cd_format() {
        // Une seconde de voix à 22 050 Hz, mono : une rampe.
        let samples: Vec<i16> = (0..22_050)
            .map(|i| i16::try_from(i % 1000).unwrap_or(0))
            .collect();
        let voice = Pcm {
            rate: 22_050,
            channels: 1,
            samples,
        };
        let cd = voice.to_cd();
        assert_eq!(cd.rate, 44_100);
        assert_eq!(cd.channels, 2);
        assert_eq!(cd.samples.len(), 44_100 * 2);
        // Un échantillon sur deux tombe pile sur l'original, l'autre au milieu.
        assert_eq!(cd.samples[0], 0);
        assert_eq!(cd.samples[2], 1); // 0,5 arrondi à 1
        assert_eq!(cd.samples[4], 1);
        assert_eq!(cd.samples[4], cd.samples[5], "gauche = droite");
        assert_eq!(cd.duration().as_millis(), 1000);
        let sound = announcement(false, Some(&voice));
        assert_eq!(sound.duration.as_millis(), 1600);
    }

    #[test]
    fn stereo_is_mixed_and_odd_bytes_ignored() {
        let pcm = Pcm::from_le_bytes(44_100, 2, &[100, 0, 200, 0, 7]);
        assert_eq!(pcm.samples, vec![100, 200]);
        let cd = pcm.to_cd();
        assert_eq!(cd.samples, vec![150, 150]);
        assert_eq!(
            Pcm {
                rate: 0,
                channels: 1,
                samples: vec![1]
            }
            .duration(),
            Duration::ZERO
        );
        let empty = Pcm {
            rate: 0,
            channels: 1,
            samples: vec![1],
        }
        .to_cd();
        assert_eq!(empty.samples, Vec::<i16>::new());
    }

    #[test]
    fn a_loud_voice_is_lowered() {
        let mut loud = Pcm {
            rate: 44_100,
            channels: 1,
            samples: vec![32_767, -32_768, 1000],
        };
        loud.limit_peak(PEAK);
        assert_eq!(loud.samples, vec![28_999, -29_000, 885]);
        let mut quiet = Pcm {
            rate: 44_100,
            channels: 1,
            samples: vec![100, -200],
        };
        quiet.limit_peak(PEAK);
        assert_eq!(quiet.samples, vec![100, -200]);
    }

    #[test]
    fn the_chime_rings_and_fades() {
        let chime = chime();
        let peak = chime
            .samples
            .iter()
            .map(|s| s.unsigned_abs())
            .max()
            .unwrap_or(0);
        assert!(peak > 5000, "audible : {peak}");
        let tail = chime.samples[chime.samples.len() - 200..]
            .iter()
            .map(|s| s.unsigned_abs())
            .max()
            .unwrap_or(0);
        assert!(tail < peak / 5, "il s'éteint : {tail}");
        assert_eq!(chime.samples[0], 0, "pas de clic au début");
    }
}
