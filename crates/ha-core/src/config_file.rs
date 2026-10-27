//! Lire un fichier de configuration (`pieces.yaml`, `rappels.yaml`…) **sans
//! dépasser une taille**, en distinguant le fichier absent.
//!
//! Chaque intégration vérifie ensuite le contenu à sa façon : ici, on ne
//! fait que lire le texte. La taille est vérifiée avant de lire (un fichier
//! de plusieurs Go ne remplit pas la mémoire), puis pendant la lecture (au
//! cas où le fichier grandirait entre-temps).

use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

/// La taille maximale habituelle d'un fichier de configuration : 256 Kio,
/// bien plus qu'il n'en faut pour une maison.
pub const DEFAULT_MAX: u64 = 256 * 1024;

/// Pourquoi un fichier de configuration n'a pas pu être lu.
#[derive(Debug, thiserror::Error)]
pub enum ConfigFileError {
    /// Plus de `max` octets.
    #[error("fichier trop gros ({} au plus)", size(*.max))]
    TooLarge { max: u64 },

    /// Lecture impossible (droits, disque…).
    #[error(transparent)]
    Io(#[from] io::Error),
}

impl ConfigFileError {
    /// Traduit l'erreur dans le type d'erreur de l'appelant : `too_large`
    /// pour un fichier trop gros, l'erreur d'entrée-sortie sinon.
    ///
    /// Concept Rust : `E: From<io::Error>` veut dire « n'importe quel type
    /// qui sait se construire depuis une `io::Error` » ; `.into()` fait la
    /// conversion.
    pub fn into_error<E: From<io::Error>>(self, too_large: E) -> E {
        match self {
            Self::TooLarge { .. } => too_large,
            Self::Io(error) => error.into(),
        }
    }
}

/// Pour les appelants qui rangent l'erreur dans une `io::Error` : le
/// message « fichier trop gros » est gardé.
impl From<ConfigFileError> for io::Error {
    fn from(error: ConfigFileError) -> Self {
        match error {
            ConfigFileError::Io(error) => error,
            too_large @ ConfigFileError::TooLarge { .. } => {
                Self::new(io::ErrorKind::InvalidData, too_large.to_string())
            }
        }
    }
}

/// Lit le fichier `path`, `max` octets au plus. `Ok(None)` s'il n'existe
/// pas.
pub fn read(path: &Path, max: u64) -> Result<Option<String>, ConfigFileError> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if file.metadata()?.len() > max {
        return Err(ConfigFileError::TooLarge { max });
    }
    // Concept Rust : `take` limite ce que la lecture peut renvoyer ; un
    // octet de plus que `max` suffit à savoir que c'est trop.
    let mut text = String::new();
    file.take(max.saturating_add(1)).read_to_string(&mut text)?;
    if u64::try_from(text.len()).unwrap_or(u64::MAX) > max {
        return Err(ConfigFileError::TooLarge { max });
    }
    Ok(Some(text))
}

/// Une taille lisible : « 256 Kio », « 1 Mio ».
fn size(bytes: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = 1024 * KIB;
    if bytes >= MIB && bytes.is_multiple_of(MIB) {
        format!("{} Mio", bytes / MIB)
    } else if bytes >= KIB && bytes.is_multiple_of(KIB) {
        format!("{} Kio", bytes / KIB)
    } else {
        format!("{bytes} octets")
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]

    use super::*;

    /// Le type d'erreur d'une intégration, pour essayer `into_error`.
    #[derive(Debug, PartialEq)]
    enum Mine {
        TooLarge,
        Io(io::ErrorKind),
    }

    impl From<io::Error> for Mine {
        fn from(error: io::Error) -> Self {
            Self::Io(error.kind())
        }
    }

    /// Un dossier vide, propre à chaque test.
    fn dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("ha-core-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_missing_file_is_none() {
        let dir = dir("absent");
        assert!(read(&dir.join("absent.yaml"), 10).unwrap().is_none());
    }

    #[test]
    fn reads_up_to_the_limit() {
        let dir = dir("limite");
        let path = dir.join("pieces.yaml");
        std::fs::write(&path, "- salon\n").unwrap();
        assert_eq!(read(&path, 8).unwrap().as_deref(), Some("- salon\n"));
        let error = read(&path, 7).unwrap_err();
        assert!(matches!(error, ConfigFileError::TooLarge { max: 7 }));
        assert_eq!(error.to_string(), "fichier trop gros (7 octets au plus)");
    }

    #[test]
    fn errors_translate() {
        let error = ConfigFileError::TooLarge { max: 256 * 1024 };
        assert_eq!(error.to_string(), "fichier trop gros (256 Kio au plus)");
        let io: io::Error = error.into();
        assert_eq!(io.to_string(), "fichier trop gros (256 Kio au plus)");
        let error = ConfigFileError::TooLarge { max: 1024 * 1024 };
        assert_eq!(error.to_string(), "fichier trop gros (1 Mio au plus)");

        let too_large = ConfigFileError::TooLarge { max: 1 };
        assert_eq!(too_large.into_error(Mine::TooLarge), Mine::TooLarge);
        let denied = ConfigFileError::Io(io::ErrorKind::PermissionDenied.into());
        assert_eq!(
            denied.into_error(Mine::TooLarge),
            Mine::Io(io::ErrorKind::PermissionDenied)
        );
    }
}
