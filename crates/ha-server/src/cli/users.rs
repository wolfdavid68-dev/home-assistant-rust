//! Commandes `ha-server user …` : gestion des comptes en ligne de commande.
//!
//! Il n'y a volontairement pas de route d'API pour créer le premier compte :
//! quelqu'un qui joint le serveur par le réseau ne doit pas pouvoir devenir
//! administrateur. Il faut avoir accès à la machine elle-même.

use std::error::Error;
use std::io::{BufRead, IsTerminal};

use ha_auth::{Auth, MIN_PASSWORD_LEN};
use zeroize::Zeroizing;

use super::USAGE;
use super::audit::audit_cli;
use super::terminal::ask_line;

fn parse_role(value: &str) -> Result<ha_auth::Role, Box<dyn Error>> {
    ha_auth::Role::parse(value)
        .ok_or_else(|| format!("rôle « {value} » inconnu : admin, famille ou invité").into())
}

/// `user add <nom> [rôle]`. Sans rôle : le tout premier compte est
/// administrateur, les suivants sont « famille » (le moins de droits utile).
async fn add_user(auth: &Auth, name: &str, role: Option<&str>) -> Result<(), Box<dyn Error>> {
    let role = match role {
        Some(role) => parse_role(role)?,
        None if auth.list_users().await?.is_empty() => ha_auth::Role::Admin,
        None => ha_auth::Role::Family,
    };
    let password = ask_new_password()?;
    auth.create_user_as(name, &password, role).await?;
    println!("compte « {name} » créé ({})", role.label());
    if role == ha_auth::Role::Family {
        println!("pour en faire un administrateur : ha-server user role {name} admin");
    }
    Ok(())
}

pub async fn user_command(args: &[&str]) -> Result<(), Box<dyn Error>> {
    let store = crate::open_store().await?;
    let auth = Auth::new(&store).await?;

    let result = match args {
        ["add", name] => add_user(&auth, name, None).await,
        ["add", name, role] => add_user(&auth, name, Some(role)).await,
        ["role", name, role] => {
            let role = parse_role(role)?;
            if auth.set_role(name, role).await? {
                println!(
                    "« {name} » est maintenant {} (tout de suite, même connecté)",
                    role.label()
                );
                Ok(())
            } else {
                Err(format!("le compte « {name} » n'existe pas").into())
            }
        }
        ["passwd", name] => {
            let password = ask_new_password()?;
            if auth.set_password(name, &password).await? {
                println!("mot de passe de « {name} » changé ; toutes ses sessions sont fermées");
                Ok(())
            } else {
                Err(format!("le compte « {name} » n'existe pas").into())
            }
        }
        ["delete", name] => {
            if auth.delete_user(name).await? {
                println!("compte « {name} » supprimé");
                Ok(())
            } else {
                Err(format!("le compte « {name} » n'existe pas").into())
            }
        }
        ["list"] => {
            for user in auth.list_users().await? {
                let two_factor = auth.has_totp(&user.username).await? == Some(true);
                let suffix = if two_factor {
                    "  (double authentification)"
                } else {
                    ""
                };
                println!("{:<20} {}{suffix}", user.username, user.role.label());
            }
            Ok(())
        }
        ["totp", name] => enable_totp(&auth, name).await,
        ["token", name] => program_token(&auth, name, None).await,
        ["token", name, days] => program_token(&auth, name, Some(days)).await,
        ["totp-off", name] => {
            if auth.disable_totp(name).await? {
                println!("double authentification désactivée pour « {name} »");
                Ok(())
            } else {
                Err(format!("le compte « {name} » n'existe pas").into())
            }
        }
        _ => Err(USAGE.into()),
    };
    if result.is_ok() {
        audit_cli(&store, "user", args).await;
    }
    store.close().await;
    result
}

/// `user token <nom> [jours]` : un jeton pour un assistant IA branché par
/// MCP (étape 55). Affiché une seule fois ; 90 jours par défaut, 365 au plus.
async fn program_token(auth: &Auth, name: &str, days: Option<&str>) -> Result<(), Box<dyn Error>> {
    let days: u64 = match days {
        None => 90,
        Some(days) => days
            .parse()
            .ok()
            .filter(|days| (1..=365).contains(days))
            .ok_or("durée invalide : de 1 à 365 jours")?,
    };
    let issued = auth
        .issue_program_token(name, std::time::Duration::from_hours(days * 24))
        .await?
        .ok_or_else(|| format!("le compte « {name} » n'existe pas"))?;
    println!("Jeton de « {name} », valable {days} jours (affiché une seule fois) :");
    println!();
    println!("{}", issued.token.as_str());
    println!();
    println!(
        "À donner à l'assistant IA comme en-tête « Authorization: Bearer <jeton> » (docs/mcp.md)."
    );
    println!("Pour le révoquer : ha-server user passwd {name} (ferme toutes ses sessions).");
    Ok(())
}

/// Active la double authentification : affiche un QR code à scanner avec
/// l'appli d'authentification, puis demande un premier code pour vérifier.
async fn enable_totp(auth: &Auth, name: &str) -> Result<(), Box<dyn Error>> {
    match auth.has_totp(name).await? {
        None => return Err(format!("le compte « {name} » n'existe pas").into()),
        Some(true) => println!("(elle est déjà activée : l'ancien code sera remplacé)"),
        Some(false) => {}
    }
    let setup = ha_auth::totp::new_setup(name)?;
    // Les terminaux (PowerShell, Windows Terminal…) ont en général un fond
    // sombre : on dessine donc les cases claires du QR code avec des blocs
    // (qui s'affichent en clair), et les cases sombres avec des espaces.
    let qr = qrcode::QrCode::new(setup.uri.as_bytes())?
        .render::<qrcode::render::unicode::Dense1x2>()
        .dark_color(qrcode::render::unicode::Dense1x2::Light)
        .light_color(qrcode::render::unicode::Dense1x2::Dark)
        .quiet_zone(true)
        .build();
    println!("Scanne ce QR code avec ton appli d'authentification");
    println!("(Aegis, 2FAS, Google Authenticator, ton gestionnaire de mots de passe…).");
    println!("Vérifie que personne ne regarde ton écran.");
    println!("(Si l'appli ne le lit pas, saisis le code affiché en dessous.)\n");
    println!("{qr}");
    println!(
        "Ou saisis ce code à la main : {}\n",
        group_by_four(&setup.secret_base32).as_str()
    );

    for _ in 0..3 {
        let code = ask_line("code à 6 chiffres affiché par l'appli : ")?;
        match auth.enable_totp(name, &setup.secret, &code).await {
            Ok(true) => {
                println!(
                    "double authentification activée pour « {name} » ; \
                     toutes ses sessions sont fermées"
                );
                return Ok(());
            }
            Ok(false) => return Err(format!("le compte « {name} » n'existe pas").into()),
            Err(ha_auth::AuthError::InvalidTotpCode) => {
                println!("code incorrect (vérifie que l'heure du téléphone est juste)");
            }
            Err(error) => return Err(error.into()),
        }
    }
    Err("double authentification non activée : réessaie".into())
}

/// « ABCDEFGH… » → « ABCD EFGH … », plus facile à recopier.
fn group_by_four(text: &str) -> Zeroizing<String> {
    let mut out = Zeroizing::new(String::with_capacity(text.len() * 5 / 4));
    for (index, c) in text.chars().enumerate() {
        if index > 0 && index % 4 == 0 {
            out.push(' ');
        }
        out.push(c);
    }
    out
}

/// Demande un nouveau mot de passe deux fois, sans l'afficher à l'écran.
///
/// Si l'entrée n'est pas un terminal (script, Docker…), le mot de passe est
/// lu sur une seule ligne de l'entrée standard.
fn ask_new_password() -> Result<Zeroizing<String>, Box<dyn Error>> {
    if !std::io::stdin().is_terminal() {
        let mut line = Zeroizing::new(String::new());
        std::io::stdin().lock().read_line(&mut line)?;
        let password = Zeroizing::new(line.trim_end_matches(['\r', '\n']).to_owned());
        return Ok(password);
    }

    println!("au moins {MIN_PASSWORD_LEN} caractères ; une phrase de passe est idéale");
    let first = Zeroizing::new(rpassword::prompt_password("mot de passe : ")?);
    let second = Zeroizing::new(rpassword::prompt_password("encore une fois : ")?);
    if first != second {
        return Err("les deux mots de passe sont différents".into());
    }
    Ok(first)
}
