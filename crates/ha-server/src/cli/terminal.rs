//! Ce que les commandes partagent pour parler au terminal : lire une ligne
//! ou un secret au clavier, afficher une date en heure locale.

use std::error::Error;
use std::io::{BufRead, IsTerminal};

use zeroize::Zeroizing;

/// Lit une ligne (visible).
pub(super) fn ask_line(prompt: &str) -> Result<String, Box<dyn Error>> {
    use std::io::Write as _;
    print!("{prompt}");
    std::io::stdout().flush()?;
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line)?;
    Ok(line.trim().to_owned())
}

/// Lit un secret sans l'afficher (ou une ligne si l'entrée n'est pas un terminal).
pub(super) fn ask_secret(prompt: &str) -> Result<Zeroizing<String>, Box<dyn Error>> {
    if std::io::stdin().is_terminal() {
        return Ok(Zeroizing::new(rpassword::prompt_password(prompt)?));
    }
    let mut line = Zeroizing::new(String::new());
    std::io::stdin().lock().read_line(&mut line)?;
    Ok(Zeroizing::new(
        line.trim_end_matches(['\r', '\n']).to_owned(),
    ))
}

/// Une date en heure locale : `14/10/2026 23:05:12`.
pub(super) fn local_time(at: std::time::SystemTime) -> String {
    jiff::Timestamp::try_from(at).map_or_else(
        |_| "date invalide".to_owned(),
        |at| {
            at.to_zoned(jiff::tz::TimeZone::system())
                .strftime("%d/%m/%Y %H:%M:%S")
                .to_string()
        },
    )
}
