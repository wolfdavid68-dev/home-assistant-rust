//! Commandes `ha-server user …` : gestion des comptes en ligne de commande.
//!
//! Il n'y a volontairement pas de route d'API pour créer le premier compte :
//! quelqu'un qui joint le serveur par le réseau ne doit pas pouvoir devenir
//! administrateur. Il faut avoir accès à la machine elle-même.

use std::error::Error;
use std::io::{BufRead, IsTerminal};

use ha_auth::{Auth, MIN_PASSWORD_LEN};
use ha_hue::{pairing, tls};
use ha_somfy::cloud::{CloudSession, SOMFY_EUROPE};
use ha_storage::Store;
use zeroize::Zeroizing;

pub const USAGE: &str = "\
utilisation :
  ha-server                       démarre le serveur
  ha-server user add <nom>        crée un compte
  ha-server user passwd <nom>     change un mot de passe
  ha-server user delete <nom>     supprime un compte
  ha-server user list             liste les comptes
  ha-server user totp <nom>       active la double authentification
  ha-server user totp-off <nom>   la désactive
  ha-server hue pair <adresse>    associe le pont Philips Hue
  ha-server hue status            affiche le pont associé
  ha-server hue forget            oublie le pont
  ha-server somfy pair <adresse>  associe la box Somfy TaHoma
  ha-server somfy devices         liste les appareils de la box
  ha-server somfy status          affiche la box associée
  ha-server somfy forget          oublie la box
  ha-server automations check     vérifie le fichier d'automatisations
  ha-server healthcheck           vérifie qu'un serveur répond (pour Docker)";

pub async fn user_command(args: &[&str]) -> Result<(), Box<dyn Error>> {
    let store = crate::open_store().await?;
    let auth = Auth::new(&store).await?;

    let result = match args {
        ["add", name] => {
            let password = ask_new_password()?;
            auth.create_user(name, &password).await?;
            println!("compte « {name} » créé");
            Ok(())
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
                println!("{}{suffix}", user.username);
            }
            Ok(())
        }
        ["totp", name] => enable_totp(&auth, name).await,
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
    store.close().await;
    result
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

pub async fn hue_command(args: &[&str]) -> Result<(), Box<dyn Error>> {
    let store = crate::open_store().await?;
    let result = hue_inner(args, &store).await;
    store.close().await;
    result
}

async fn hue_inner(args: &[&str], store: &Store) -> Result<(), Box<dyn Error>> {
    match args {
        ["pair", host] => {
            let (bridge_id, fingerprint) = pairing::identify(host).await?;
            println!("pont Hue trouvé : {bridge_id}");
            println!("empreinte de son certificat :");
            println!("  {}", tls::format_fingerprint(&fingerprint));
            println!("seul ce certificat sera accepté à l'avenir.");
            println!();
            println!(
                "appuie maintenant sur le gros bouton rond du pont (tu as {} secondes)…",
                pairing::LINK_BUTTON_TIMEOUT.as_secs()
            );
            let paired = pairing::request_key(host, bridge_id, fingerprint).await?;
            pairing::save(store, &paired).await?;
            println!("associé ! relance « ha-server » pour piloter tes lampes.");
            Ok(())
        }
        ["status"] => {
            match pairing::load(store).await? {
                Some(paired) => {
                    println!("pont {} à l'adresse {}", paired.bridge_id, paired.host);
                    println!(
                        "certificat : {}",
                        tls::format_fingerprint(&paired.fingerprint)
                    );
                }
                None => println!("aucun pont associé (ha-server hue pair <adresse>)"),
            }
            Ok(())
        }
        ["forget"] => {
            if pairing::forget(store).await? {
                println!("pont oublié.");
                println!("retire aussi « ha_rust » des applications autorisées dans l'appli Hue");
                println!("(Paramètres > Ponts Hue > i > Applications) pour révoquer la clé.");
            } else {
                println!("aucun pont associé.");
            }
            Ok(())
        }
        _ => Err(USAGE.into()),
    }
}

/// Vérifie le fichier d'automatisations sans démarrer le serveur.
pub fn check_automations(path: &std::path::Path) -> Result<(), Box<dyn Error>> {
    if !path.exists() {
        println!("{} n'existe pas : aucune automatisation.", path.display());
        return Ok(());
    }
    let automations = ha_automation::load(path)?;
    println!(
        "{} : {} automatisation(s) valide(s)",
        path.display(),
        automations.len()
    );
    for automation in &automations {
        println!(
            "  - {} ({} déclencheur(s), {} condition(s), {} action(s))",
            automation.alias,
            automation.triggers.len(),
            automation.conditions.len(),
            automation.actions.len()
        );
    }
    Ok(())
}

pub async fn somfy_command(args: &[&str]) -> Result<(), Box<dyn Error>> {
    let store = crate::open_store().await?;
    let result = somfy_inner(args, &store).await;
    store.close().await;
    result
}

async fn somfy_inner(args: &[&str], store: &Store) -> Result<(), Box<dyn Error>> {
    match args {
        ["pair", host] => {
            let (version, fingerprint) = ha_somfy::pairing::identify(host).await?;
            println!("box TaHoma trouvée (API {version})");
            println!("empreinte de son certificat :");
            println!("  {}", tls::format_fingerprint(&fingerprint));
            println!("seul ce certificat sera accepté à l'avenir.");
            println!();
            println!("Somfy impose de créer le jeton de l'API locale avec ton compte Somfy,");
            println!(
                "une seule fois. Ton mot de passe sert à cette connexion et n'est pas enregistré."
            );
            let email = ask_line("e-mail du compte Somfy : ")?;
            let password = ask_secret("mot de passe Somfy : ")?;
            let session =
                CloudSession::login(ha_tls::public_client()?, SOMFY_EUROPE, &email, &password)
                    .await?;
            drop(password);
            let gateways = session.gateways().await?;
            let gateway_id = match gateways.as_slice() {
                [] => return Err("aucune box TaHoma sur ce compte Somfy".into()),
                [only] => only.clone(),
                several => {
                    for (n, id) in several.iter().enumerate() {
                        println!("  {}. {id}", n + 1);
                    }
                    let choice: usize = ask_line("numéro de la box : ")?.parse()?;
                    several
                        .get(choice.wrapping_sub(1))
                        .cloned()
                        .ok_or("numéro invalide")?
                }
            };
            let token = session.create_local_token(&gateway_id).await?;
            session.logout().await;
            pair_with_token(store, host, gateway_id, fingerprint, token).await
        }
        ["pair", host, "--token"] => {
            // Pour qui a déjà un jeton (créé ailleurs) : rien ne passe par Somfy.
            let (_, fingerprint) = ha_somfy::pairing::identify(host).await?;
            println!(
                "empreinte du certificat : {}",
                tls::format_fingerprint(&fingerprint)
            );
            let gateway_id = ask_line("PIN de la box (sous la box, ex. 1234-5678-9012) : ")?;
            if !ha_somfy::cloud::is_gateway_id(&gateway_id) {
                return Err("PIN invalide".into());
            }
            let token = ask_secret("jeton de l'API locale : ")?;
            pair_with_token(store, host, gateway_id, fingerprint, token).await
        }
        ["devices"] => {
            let Some(paired) = ha_somfy::pairing::load(store).await? else {
                return Err("aucune box associée (ha-server somfy pair <adresse>)".into());
            };
            for device in ha_somfy::pairing::verify(&paired).await? {
                let kind = ha_somfy::devices::Kind::from_ui_class(&device.definition.ui_class);
                println!(
                    "{} — {} [{}]{}",
                    device.label,
                    device.definition.ui_class,
                    device.controllable_name,
                    if kind.is_some() {
                        ""
                    } else {
                        " (non pris en charge)"
                    }
                );
                let commands: Vec<&str> = device
                    .definition
                    .commands
                    .iter()
                    .map(|c| c.name.as_str())
                    .collect();
                if !commands.is_empty() {
                    println!("    commandes : {}", commands.join(", "));
                }
                for state in &device.states {
                    println!("    {} = {}", state.name, state.value);
                }
            }
            Ok(())
        }
        ["status"] => {
            match ha_somfy::pairing::load(store).await? {
                Some(paired) => {
                    println!("box {} à l'adresse {}", paired.gateway_id, paired.host);
                    println!(
                        "certificat : {}",
                        tls::format_fingerprint(&paired.fingerprint)
                    );
                }
                None => println!("aucune box associée (ha-server somfy pair <adresse>)"),
            }
            Ok(())
        }
        ["forget"] => {
            if ha_somfy::pairing::forget(store).await? {
                println!("box oubliée.");
                println!("pour révoquer complètement le jeton, désactive puis réactive le mode");
                println!("développeur dans ton espace Somfy.");
            } else {
                println!("aucune box associée.");
            }
            Ok(())
        }
        _ => Err(USAGE.into()),
    }
}

/// Vérifie le jeton auprès de la box, puis enregistre l'association.
async fn pair_with_token(
    store: &Store,
    host: &str,
    gateway_id: String,
    fingerprint: ha_tls::Fingerprint,
    token: Zeroizing<String>,
) -> Result<(), Box<dyn Error>> {
    let paired = ha_somfy::Pairing {
        host: (*host).to_owned(),
        gateway_id,
        fingerprint,
        token,
    };
    // Un jeton tout juste créé peut mettre quelques secondes à être actif.
    let mut attempt = 0;
    let devices = loop {
        match ha_somfy::pairing::verify(&paired).await {
            Ok(devices) => break devices,
            Err(ha_somfy::SomfyError::Unauthorized) if attempt < 5 => {
                attempt += 1;
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            }
            Err(error) => return Err(error.into()),
        }
    };
    ha_somfy::pairing::save(store, &paired).await?;
    println!(
        "associée ! {} appareils sur la box. Relance « ha-server ».",
        devices.len()
    );
    Ok(())
}

/// Lit une ligne (visible).
fn ask_line(prompt: &str) -> Result<String, Box<dyn Error>> {
    use std::io::Write as _;
    print!("{prompt}");
    std::io::stdout().flush()?;
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line)?;
    Ok(line.trim().to_owned())
}

/// Lit un secret sans l'afficher (ou une ligne si l'entrée n'est pas un terminal).
fn ask_secret(prompt: &str) -> Result<Zeroizing<String>, Box<dyn Error>> {
    if std::io::stdin().is_terminal() {
        return Ok(Zeroizing::new(rpassword::prompt_password(prompt)?));
    }
    let mut line = Zeroizing::new(String::new());
    std::io::stdin().lock().read_line(&mut line)?;
    Ok(Zeroizing::new(
        line.trim_end_matches(['\r', '\n']).to_owned(),
    ))
}
