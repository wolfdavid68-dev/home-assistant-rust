//! Le décodage des trames TIC (télé-information client) du compteur Linky.
//!
//! Le compteur envoie sans arrêt des **trames** : `STX` (0x02), des
//! **groupes**, puis `ETX` (0x03). Chaque groupe est une ligne `LF … CR`
//! avec une étiquette, une valeur et un caractère de contrôle :
//!
//! | Mode       | Vitesse    | Groupe                                         |
//! | ---------- | ---------- | ---------------------------------------------- |
//! | historique | 1200 bauds | `HCHC 012345678 ,` (séparateur : espace)       |
//! | standard   | 9600 bauds | `EAST⇥012345678⇥,` (séparateur : tabulation, avec parfois une date) |
//!
//! Le caractère de contrôle est la somme des octets du groupe, gardée sur
//! 6 bits, plus 0x20. Un groupe abîmé (parasite sur le fil) est ignoré.
//!
//! Sécurité : tout ce qui arrive est vérifié (taille, caractères, somme de
//! contrôle) avant d'être utilisé ; une ligne trop longue est jetée.

/// Début et fin de trame, interruption, début et fin de groupe.
const STX: u8 = 0x02;
const ETX: u8 = 0x03;
const EOT: u8 = 0x04;
const LF: u8 = 0x0A;
const CR: u8 = 0x0D;
const HT: u8 = 0x09;
const SP: u8 = 0x20;

/// Longueur maximale d'un groupe (le plus long, `PJOURF+1`, fait moins de 110).
const MAX_GROUP_LEN: usize = 128;
/// Groupes au plus dans une trame (une trame standard en a une soixantaine).
const MAX_GROUPS: usize = 96;
const MAX_LABEL_LEN: usize = 8;

/// Le mode de la TIC.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Le mode par défaut de la plupart des Linky (et des anciens compteurs) :
    /// 1200 bauds, étiquettes `BASE`, `HCHC`, `PAPP`…
    Historic,
    /// 9600 bauds, étiquettes `EAST`, `EASF01`, `SINSTS`… (production solaire).
    Standard,
}

impl Mode {
    /// La vitesse de la liaison série.
    #[must_use]
    pub fn baud_rate(self) -> u32 {
        match self {
            Self::Historic => 1200,
            Self::Standard => 9600,
        }
    }

    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Historic => "historique",
            Self::Standard => "standard",
        }
    }

    #[must_use]
    pub fn other(self) -> Self {
        match self {
            Self::Historic => Self::Standard,
            Self::Standard => Self::Historic,
        }
    }
}

/// Un groupe vérifié.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    pub label: String,
    pub value: String,
    /// Mode standard : la date jointe à certains groupes (`DATE`, `SMAXSN`…).
    pub horodate: Option<String>,
    pub mode: Mode,
}

/// Une trame complète : ses groupes valides.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub mode: Mode,
    pub groups: Vec<Group>,
}

impl Frame {
    /// La valeur d'une étiquette.
    #[must_use]
    pub fn get(&self, label: &str) -> Option<&str> {
        self.groups
            .iter()
            .find(|group| group.label == label)
            .map(|group| group.value.as_str())
    }

    /// La valeur d'une étiquette, comme nombre entier.
    #[must_use]
    pub fn number(&self, label: &str) -> Option<u64> {
        self.get(label)?.trim().parse().ok()
    }
}

/// Pourquoi un groupe est refusé.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupError {
    /// Pas la bonne forme (séparateurs manquants, étiquette invalide…).
    Malformed,
    /// La somme de contrôle ne correspond pas : le groupe a été abîmé.
    Checksum,
}

/// Le caractère de contrôle de `bytes`.
#[must_use]
pub fn checksum(bytes: &[u8]) -> u8 {
    let sum = bytes
        .iter()
        .fold(0_u32, |sum, byte| sum.wrapping_add(u32::from(*byte)));
    // `& 0x3F` garde 6 bits : le résultat tient toujours dans un octet.
    u8::try_from(sum & 0x3F).unwrap_or(0) + 0x20
}

fn valid_label(label: &[u8]) -> bool {
    (1..=MAX_LABEL_LEN).contains(&label.len())
        && label
            .iter()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || matches!(b, b'+' | b'-'))
}

fn text(bytes: &[u8]) -> Option<String> {
    bytes
        .iter()
        .all(|b| (0x20..0x7F).contains(b))
        .then(|| String::from_utf8_lossy(bytes).into_owned())
}

/// Vérifie un groupe (sans `LF` ni `CR`).
pub fn parse_group(line: &[u8]) -> Result<Group, GroupError> {
    let (&check, data) = line.split_last().ok_or(GroupError::Malformed)?;
    if line.contains(&HT) {
        // Standard : étiquette ⇥ [date ⇥] valeur ⇥ contrôle. Le contrôle
        // porte sur tout, jusqu'à la dernière tabulation comprise.
        if data.last() != Some(&HT) {
            return Err(GroupError::Malformed);
        }
        let fields: Vec<&[u8]> = data
            .strip_suffix(&[HT])
            .unwrap_or(data)
            .split(|b| *b == HT)
            .collect();
        let (label, horodate, value) = match fields.as_slice() {
            [label, value] => (*label, None, *value),
            [label, horodate, value] => (*label, Some(*horodate), *value),
            _ => return Err(GroupError::Malformed),
        };
        if !valid_label(label) {
            return Err(GroupError::Malformed);
        }
        let group = Group {
            label: text(label).ok_or(GroupError::Malformed)?,
            value: text(value).ok_or(GroupError::Malformed)?,
            horodate: horodate
                .map(|h| text(h).ok_or(GroupError::Malformed))
                .transpose()?,
            mode: Mode::Standard,
        };
        if checksum(data) != check {
            return Err(GroupError::Checksum);
        }
        Ok(group)
    } else {
        // Historique : étiquette ␠ valeur ␠ contrôle. Le contrôle porte sur
        // l'étiquette, l'espace et la valeur (pas sur le dernier espace) ; il
        // peut lui-même être un espace.
        let summed = data.strip_suffix(&[SP]).ok_or(GroupError::Malformed)?;
        let space = summed
            .iter()
            .position(|b| *b == SP)
            .ok_or(GroupError::Malformed)?;
        let (label, rest) = summed.split_at(space);
        let value = rest.get(1..).unwrap_or_default();
        if !valid_label(label) {
            return Err(GroupError::Malformed);
        }
        let group = Group {
            label: text(label).ok_or(GroupError::Malformed)?,
            value: text(value).ok_or(GroupError::Malformed)?,
            horodate: None,
            mode: Mode::Historic,
        };
        if checksum(summed) != check {
            return Err(GroupError::Checksum);
        }
        Ok(group)
    }
}

/// Reçoit les octets au fil de l'eau et rend les trames complètes.
///
/// Concept Rust : un **automate** dans une structure. Les octets arrivent par
/// paquets de taille quelconque (la liaison série ne respecte pas les
/// trames) : on garde où on en est entre deux appels.
#[derive(Debug, Default)]
pub struct Decoder {
    /// Dans une trame ?
    in_frame: bool,
    /// Le groupe en cours (`None` : pas dans un groupe, ou groupe trop long).
    line: Option<Vec<u8>>,
    groups: Vec<Group>,
    /// Groupes refusés depuis le début (somme de contrôle, forme).
    pub errors: u64,
}

impl Decoder {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Ajoute des octets ; renvoie les trames terminées.
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<Frame> {
        let mut frames = Vec::new();
        for &byte in bytes {
            // Lue sans la configurer en 7 bits, la parité arrive dans le 8ᵉ bit.
            let byte = byte & 0x7F;
            match byte {
                STX => {
                    self.in_frame = true;
                    self.line = None;
                    self.groups.clear();
                }
                ETX => {
                    if self.in_frame && !self.groups.is_empty() {
                        let groups = std::mem::take(&mut self.groups);
                        let standard = groups.iter().filter(|g| g.mode == Mode::Standard).count();
                        let mode = if standard * 2 > groups.len() {
                            Mode::Standard
                        } else {
                            Mode::Historic
                        };
                        frames.push(Frame { mode, groups });
                    }
                    self.in_frame = false;
                    self.line = None;
                }
                // Trame interrompue (le compteur répond à une autre demande).
                EOT => {
                    self.in_frame = false;
                    self.line = None;
                    self.groups.clear();
                }
                LF => self.line = self.in_frame.then(Vec::new),
                CR => {
                    if let Some(line) = self.line.take() {
                        match parse_group(&line) {
                            Ok(group) if self.groups.len() < MAX_GROUPS => self.groups.push(group),
                            Ok(_) => {}
                            Err(_) => self.errors += 1,
                        }
                    }
                }
                _ => {
                    if let Some(line) = &mut self.line {
                        if line.len() < MAX_GROUP_LEN {
                            line.push(byte);
                        } else {
                            self.line = None;
                            self.errors += 1;
                        }
                    }
                }
            }
        }
        frames
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing, reason = "tests")]

    use super::*;

    /// Fabrique un groupe historique avec sa somme de contrôle.
    pub(crate) fn historic(label: &str, value: &str) -> Vec<u8> {
        let data = format!("{label} {value}");
        let mut line = vec![LF];
        line.extend_from_slice(data.as_bytes());
        line.push(SP);
        line.push(checksum(data.as_bytes()));
        line.push(CR);
        line
    }

    pub(crate) fn standard(label: &str, horodate: Option<&str>, value: &str) -> Vec<u8> {
        let data = match horodate {
            Some(h) => format!("{label}\t{h}\t{value}\t"),
            None => format!("{label}\t{value}\t"),
        };
        let mut line = vec![LF];
        line.extend_from_slice(data.as_bytes());
        line.push(checksum(data.as_bytes()));
        line.push(CR);
        line
    }

    #[test]
    fn real_groups_are_accepted() {
        // Groupes relevés sur de vrais compteurs (documentation Enedis).
        let group = parse_group(b"ADCO 031762120246 9").unwrap();
        assert_eq!(group.label, "ADCO");
        assert_eq!(group.value, "031762120246");
        assert_eq!(group.mode, Mode::Historic);
        assert_eq!(
            parse_group(b"HCHC 001234567 \"").map(|g| g.value),
            Ok("001234567".into())
        );
        assert_eq!(
            parse_group(b"PTEC HP..  ").map(|g| g.value),
            Ok("HP..".into())
        );
        let group = parse_group(b"DATE\tE230601120000\t\t-").unwrap();
        assert_eq!(group.mode, Mode::Standard);
        assert_eq!(group.horodate.as_deref(), Some("E230601120000"));
        assert_eq!(group.value, "");
    }

    #[test]
    fn damaged_groups_are_refused() {
        assert_eq!(parse_group(b"HCHC 001234568 \""), Err(GroupError::Checksum));
        assert_eq!(
            parse_group(b"hchc 001234567 \""),
            Err(GroupError::Malformed)
        );
        assert_eq!(parse_group(b"HCHC001234567,"), Err(GroupError::Malformed));
        assert_eq!(parse_group(b""), Err(GroupError::Malformed));
        assert_eq!(parse_group(b"TROPLONGUE 1 X"), Err(GroupError::Malformed));
        assert_eq!(
            parse_group(b"EAST\t1\t2\t3\t4\tX"),
            Err(GroupError::Malformed)
        );
    }

    #[test]
    fn frames_are_rebuilt_from_any_chunks() {
        let mut bytes = vec![STX];
        bytes.extend(historic("ADCO", "031762120246"));
        bytes.extend(historic("HCHC", "001234567"));
        bytes.extend(b"\nHCHP 000999999 X\r"); // abîmé : ignoré
        bytes.extend(historic("PAPP", "01230"));
        bytes.push(ETX);
        bytes.push(STX);
        bytes.extend(historic("HCHC", "001234570"));
        bytes.push(ETX);
        // Avec le bit de parité (8ᵉ bit) mis sur quelques octets.
        if let Some(byte) = bytes.get_mut(5) {
            *byte |= 0x80;
        }

        let mut decoder = Decoder::new();
        let mut frames = Vec::new();
        for chunk in bytes.chunks(7) {
            frames.extend(decoder.feed(chunk));
        }
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].mode, Mode::Historic);
        assert_eq!(frames[0].number("HCHC"), Some(1_234_567));
        assert_eq!(frames[0].number("PAPP"), Some(1230));
        assert_eq!(frames[0].get("HCHP"), None);
        assert_eq!(frames[1].number("HCHC"), Some(1_234_570));
        assert_eq!(decoder.errors, 1);
    }

    #[test]
    fn a_frame_started_midway_is_dropped() {
        let mut decoder = Decoder::new();
        // On se branche au milieu d'une trame : rien jusqu'au prochain STX.
        let mut bytes = historic("HCHC", "001234567");
        bytes.push(ETX);
        bytes.push(STX);
        bytes.extend(standard("EAST", None, "000012345"));
        bytes.extend(standard("DATE", Some("E230601120000"), ""));
        bytes.push(ETX);
        let frames = decoder.feed(&bytes);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].mode, Mode::Standard);
        assert_eq!(frames[0].number("EAST"), Some(12_345));
    }

    #[test]
    fn an_endless_line_is_dropped() {
        let mut decoder = Decoder::new();
        let mut bytes = vec![STX, LF];
        bytes.extend(std::iter::repeat_n(b'A', 10_000));
        bytes.push(CR);
        bytes.extend(historic("PAPP", "00500"));
        bytes.push(ETX);
        let frames = decoder.feed(&bytes);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].groups.len(), 1);
        assert_eq!(decoder.errors, 1);
    }
}
