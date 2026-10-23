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
  ha-server --version             affiche la version
  ha-server user add <nom> [rôle] crée un compte (admin, famille ou invité)
  ha-server user role <nom> <rôle> change le rôle d'un compte
  ha-server user passwd <nom>     change un mot de passe
  ha-server user delete <nom>     supprime un compte
  ha-server user list             liste les comptes
  ha-server user totp <nom>       active la double authentification
  ha-server user totp-off <nom>   la désactive
  ha-server user token <nom> [jours]  jeton pour un assistant IA (MCP), 90 jours par défaut
  ha-server hue pair <adresse>    associe le pont Philips Hue
  ha-server hue status            affiche le pont associé
  ha-server hue forget            oublie le pont
  ha-server somfy pair <adresse>  associe la box Somfy TaHoma
  ha-server somfy devices         liste les appareils de la box
  ha-server somfy status          affiche la box associée
  ha-server somfy forget          oublie la box
  ha-server freebox pair [adresse]     associe la Freebox (lecture seule)
  ha-server freebox status             affiche la Freebox associée
  ha-server freebox devices            liste les appareils du réseau
  ha-server freebox track <MAC> <nom>  suit la présence d'un appareil
  ha-server freebox untrack <nom>      arrête de le suivre
  ha-server freebox forget             oublie la Freebox
  ha-server timetree pair         associe le compte TimeTree de la maison (lecture seule)
  ha-server timetree status       affiche le compte et ses agendas
  ha-server timetree forget       oublie le compte
  ha-server iglou pair            relie la liste de courses de l'appli Iglou
  ha-server iglou status          affiche le compte, sa maison et la liste
  ha-server iglou forget          oublie le compte (la liste locale reprend)
  ha-server smartthings pair      associe la maison SmartThings de Samsung (docs/smartthings.md)
  ha-server smartthings status    affiche l'association et ses jetons
  ha-server smartthings devices   liste les appareils SmartThings et leurs entités
  ha-server smartthings forget    oublie l'association
  ha-server colis pair            associe la boîte Gmail des colis Amazon (docs/colis.md)
  ha-server colis status          vérifie la boîte et affiche les colis
  ha-server colis forget          oublie la boîte
  ha-server sonos                 liste les enceintes Sonos trouvées
  ha-server voix \"<phrase>\"       essaie la voix des annonces (écrit essai-voix.wav)
  ha-server automations check     vérifie automatisations, scripts et scènes
  ha-server automations list      liste les automatisations (fichier et éditeur)
  ha-server automations delete <id>  supprime une automatisation de l'éditeur
  ha-server areas check           vérifie le fichier des pièces
  ha-server calendriers check     vérifie le fichier des agendas en ligne (ICS)
  ha-server simulation check      vérifie la simulation de présence
  ha-server delestage check       vérifie le délestage
  ha-server modeles check         vérifie les capteurs modèles
  ha-server rappels check         vérifie les rappels insistants
  ha-server protection-solaire check  vérifie la protection solaire des volets
  ha-server guard check           vérifie la garde (pièces protégées, heures calmes)
  ha-server alexa check           vérifie le fichier des appareils pour Alexa
  ha-server alexa relay [pont]    relais pour Alexa quand le serveur est dans Docker Desktop
  ha-server thermostats check     vérifie le fichier des thermostats
  ha-server persons check         vérifie le fichier des personnes
  ha-server inputs check          vérifie le fichier des entrées
  ha-server derived check         vérifie le fichier des assistants calculés
  ha-server energy check          vérifie le fichier du bilan énergétique
  ha-server vigilance test        vérifie la clé de la vigilance Météo-France
  ha-server audit [nombre]        affiche le journal d'audit
  ha-server audit verify          vérifie que le journal n'a pas été modifié
  ha-server backup now            sauvegarde la base (même serveur en marche)
  ha-server backup list           liste les sauvegardes
  ha-server backup restore <fichier> [--force]  restaure une sauvegarde (serveur arrêté)
  ha-server https                 l'autorité de la maison (HTTPS) : empreinte, noms
  ha-server healthcheck           vérifie qu'un serveur répond (pour Docker)";

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

pub async fn hue_command(args: &[&str]) -> Result<(), Box<dyn Error>> {
    let store = crate::open_store().await?;
    let result = hue_inner(args, &store).await;
    if result.is_ok() {
        audit_cli(&store, "hue", args).await;
    }
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
    if result.is_ok() {
        audit_cli(&store, "somfy", args).await;
    }
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

/// Enregistre dans le journal d'audit les commandes qui changent quelque
/// chose (pas celles qui ne font que lire).
async fn audit_cli(store: &Store, domain: &str, args: &[&str]) {
    let (action, target) = match (domain, args) {
        (
            "user",
            [
                command @ ("add" | "passwd" | "delete" | "totp" | "totp-off" | "token"),
                name,
                ..,
            ],
        ) => (format!("user_{}", command.replace('-', "_")), Some(*name)),
        ("hue" | "somfy", ["pair", host, ..]) => (format!("{domain}_pair"), Some(*host)),
        ("freebox", ["pair", rest @ ..]) => (
            "freebox_pair".to_owned(),
            Some(
                rest.first()
                    .copied()
                    .unwrap_or(ha_freebox::api::DEFAULT_HOST),
            ),
        ),
        ("freebox", ["track", mac, _]) => ("freebox_track".to_owned(), Some(*mac)),
        ("freebox", ["untrack", name]) => ("freebox_untrack".to_owned(), Some(*name)),
        (
            "hue" | "somfy" | "freebox" | "timetree" | "iglou" | "smartthings" | "colis",
            ["forget"],
        ) => (format!("{domain}_forget"), None),
        ("timetree", ["pair"]) => ("timetree_pair".to_owned(), None),
        ("iglou", ["pair"]) => ("iglou_pair".to_owned(), None),
        ("smartthings", ["pair"]) => ("smartthings_pair".to_owned(), None),
        ("colis", ["pair"]) => ("colis_pair".to_owned(), None),
        ("automations", ["delete", id]) => ("automation_delete".to_owned(), Some(*id)),
        _ => return,
    };
    audit_local(store, "ligne de commande", &action, target).await;
}

/// Ajoute une ligne « locale » (faite sur la machine elle-même) au journal.
pub async fn audit_local(store: &Store, actor: &str, action: &str, target: Option<&str>) {
    let record = ha_storage::AuditRecord {
        actor,
        action,
        target,
        detail: None,
        source: "local",
    };
    if let Err(error) = store.audit(record).await {
        eprintln!("attention : journal d'audit non écrit ({error})");
    }
}

/// `ha-server audit [nombre]` et `ha-server audit verify`.
pub async fn audit_command(args: &[&str]) -> Result<(), Box<dyn Error>> {
    let store = crate::open_store().await?;
    let result = audit_inner(args, &store).await;
    store.close().await;
    result
}

async fn audit_inner(args: &[&str], store: &Store) -> Result<(), Box<dyn Error>> {
    let limit = match args {
        ["verify"] => {
            return match store.verify_audit().await? {
                ha_storage::AuditCheck::Intact { entries } => {
                    println!("journal intact : {entries} lignes, signatures et chaîne valides");
                    Ok(())
                }
                ha_storage::AuditCheck::Broken { id, reason } => {
                    Err(format!("journal MODIFIÉ à la ligne {id} : {reason}").into())
                }
            };
        }
        [] => 50,
        [count] => count
            .parse::<u32>()
            .ok()
            .filter(|count| (1..=ha_storage::MAX_AUDIT_LIMIT).contains(count))
            .ok_or_else(|| {
                format!(
                    "nombre de lignes invalide (entre 1 et {})",
                    ha_storage::MAX_AUDIT_LIMIT
                )
            })?,
        _ => return Err(USAGE.into()),
    };
    let mut entries = store.audit_log(limit).await?;
    // Le plus récent en bas, comme dans un terminal.
    entries.reverse();
    if entries.is_empty() {
        println!("journal vide");
    }
    for entry in entries {
        let when = local_time(entry.at);
        let what = [entry.target.as_deref(), entry.detail.as_deref()]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" ");
        println!(
            "{when}  {:<18} {:<34} {what}  ({})",
            entry.actor,
            action_label(&entry.action),
            entry.source
        );
    }
    Ok(())
}

/// Libellé lisible d'une action du journal.
fn action_label(action: &str) -> &str {
    match action {
        "login" => "connexion",
        "login_failed" => "ÉCHEC de connexion",
        "logout" => "déconnexion",
        "service" => "commande",
        "state_set" => "état modifié",
        "state_delete" => "état supprimé",
        "user_add" => "compte créé",
        "user_passwd" => "mot de passe changé",
        "user_delete" => "compte supprimé",
        "user_totp" => "double authentification activée",
        "user_totp_off" => "double authentification désactivée",
        "hue_pair" => "pont Hue associé",
        "hue_forget" => "pont Hue oublié",
        "somfy_pair" => "box Somfy associée",
        "somfy_forget" => "box Somfy oubliée",
        "freebox_pair" => "Freebox associée",
        "freebox_forget" => "Freebox oubliée",
        "freebox_track" => "présence suivie",
        "freebox_untrack" => "présence plus suivie",
        "freebox_new_device" => "NOUVEL appareil sur le réseau",
        "confirm" => "identité confirmée",
        "confirm_failed" => "ÉCHEC de confirmation",
        "automation_create" => "automatisation créée",
        "automation_update" => "automatisation modifiée",
        "automation_delete" => "automatisation supprimée",
        "backup" => "sauvegarde",
        "backup_failed" => "ÉCHEC de sauvegarde",
        "backup_restore" => "sauvegarde restaurée",
        "server_start" => "démarrage du serveur",
        "server_stop" => "arrêt du serveur",
        other => other,
    }
}

/// `ha-server freebox …`
pub async fn freebox_command(args: &[&str]) -> Result<(), Box<dyn Error>> {
    let store = crate::open_store().await?;
    let result = freebox_inner(args, &store).await;
    if result.is_ok() {
        audit_cli(&store, "freebox", args).await;
    }
    store.close().await;
    result
}

async fn freebox_inner(args: &[&str], store: &Store) -> Result<(), Box<dyn Error>> {
    use ha_freebox::pairing as freebox;

    match args {
        ["pair"] => freebox_pair(store, ha_freebox::api::DEFAULT_HOST).await,
        ["pair", host] => freebox_pair(store, host).await,
        ["status"] => {
            match freebox::load(store).await? {
                Some(pairing) => {
                    println!(
                        "Freebox associée : {} ({})",
                        pairing.device_name, pairing.host
                    );
                    println!(
                        "empreinte : {}",
                        tls::format_fingerprint(&pairing.fingerprint)
                    );
                    for tracked in freebox::tracked(store).await? {
                        println!("suivi : device_tracker.{}  ({})", tracked.name, tracked.mac);
                    }
                }
                None => println!("aucune Freebox associée (ha-server freebox pair)"),
            }
            Ok(())
        }
        ["devices"] => {
            let pairing = freebox::load(store)
                .await?
                .ok_or("aucune Freebox associée (ha-server freebox pair)")?;
            let (session, _) = freebox::connect(&pairing).await?;
            let tracked = freebox::tracked(store).await?;
            let mut hosts = session.hosts().await?;
            hosts.sort_by_key(|host| (!host.active, ha_freebox::presence::display_name(host)));
            println!("  {:<17}  {:<15} nom", "adresse MAC", "type");
            for host in &hosts {
                let Some(mac) = host.mac() else { continue };
                let followed = tracked
                    .iter()
                    .find(|t| t.mac == mac)
                    .map(|t| format!("  → device_tracker.{}", t.name))
                    .unwrap_or_default();
                println!(
                    "{} {mac}  {:<15} {}{followed}",
                    if host.active { "●" } else { "○" },
                    ha_storage::clean_field(&host.host_type),
                    ha_freebox::presence::display_name(host),
                );
            }
            println!("\n● actif   ○ inactif");
            println!("Pour suivre ta présence : ha-server freebox track <adresse MAC> <nom>");
            Ok(())
        }
        ["track", mac, name] => {
            let tracked = freebox::track(store, mac, name).await?;
            println!(
                "présence de {} suivie : device_tracker.{} (redémarre le serveur)",
                tracked.mac, tracked.name
            );
            Ok(())
        }
        ["untrack", name] => {
            if freebox::untrack(store, name).await? {
                println!("« {name} » n'est plus suivi (redémarre le serveur)");
                Ok(())
            } else {
                Err(format!("« {name} » n'était pas suivi").into())
            }
        }
        ["forget"] => {
            if freebox::forget(store).await? {
                println!("Freebox oubliée.");
                println!(
                    "Retire aussi « ha-rust » dans Freebox OS : Paramètres > Gestion des accès > Applications."
                );
            } else {
                println!("aucune Freebox n'était associée");
            }
            Ok(())
        }
        _ => Err(USAGE.into()),
    }
}

async fn freebox_pair(store: &Store, host: &str) -> Result<(), Box<dyn Error>> {
    use ha_freebox::pairing as freebox;

    let (version, fingerprint) = freebox::identify(host).await?;
    println!(
        "Freebox trouvée : {} (API {})",
        ha_storage::clean_field(&version.device_name),
        ha_storage::clean_field(&version.api_version)
    );
    println!("empreinte de son certificat :");
    println!("  {}", tls::format_fingerprint(&fingerprint));
    println!("seul ce certificat sera accepté à l'avenir.\n");
    println!("→ Va voir l'écran de la Freebox et accepte l'application « ha-rust »");
    println!(
        "  (tu as {} secondes).",
        freebox::AUTHORIZATION_TIMEOUT.as_secs()
    );
    let pairing = freebox::request_token(
        host,
        &version.device_name,
        fingerprint,
        freebox::AUTHORIZATION_TIMEOUT,
    )
    .await?;
    let (session, permissions) = freebox::connect(&pairing).await?;
    freebox::save(store, &pairing).await?;
    // On mémorise le réseau actuel : seuls les appareils qui arriveront
    // ensuite seront signalés comme nouveaux.
    let current: Vec<(String, String)> = session
        .hosts()
        .await?
        .iter()
        .filter_map(|host| Some((host.mac()?, ha_freebox::presence::display_name(host))))
        .collect();
    freebox::remember(store, &current).await?;
    println!(
        "\nFreebox associée ({} appareils sur le réseau).",
        current.len()
    );
    if !permissions.is_empty() {
        println!("droits accordés à « ha-rust » : {}", permissions.join(", "));
        println!(
            "Garde le minimum : décoche-les dans Freebox OS (Paramètres > Gestion des accès > \
             Applications), puis vérifie avec « freebox devices ». En cas de « droits \
             insuffisants », recoche le dernier retiré."
        );
    }
    println!("Ensuite : ha-server freebox devices, puis freebox track <adresse MAC> <nom>.");
    Ok(())
}

/// `ha-server areas check` : vérifie le fichier des pièces.
pub fn check_areas(path: &std::path::Path) -> Result<(), Box<dyn Error>> {
    if !path.exists() {
        println!("{} : fichier absent (aucune pièce)", path.display());
        return Ok(());
    }
    let areas = ha_areas::load(path)?;
    println!("{} : {} pièce(s) valide(s)", path.display(), areas.len());
    for area in &areas {
        println!("  - {} ({} appareil(s))", area.name, area.entities.len());
    }
    Ok(())
}

/// `ha-server calendriers check` : vérifie `calendriers.yaml` (sans
/// télécharger : l'adresse n'est pas affichée, seulement le site).
pub fn check_calendars(path: &std::path::Path) -> Result<(), Box<dyn Error>> {
    if !path.exists() {
        println!(
            "{} : fichier absent (aucun agenda en ligne)",
            path.display()
        );
        return Ok(());
    }
    let calendars = ha_ics::load(path)?;
    println!(
        "{} : {} agenda(s) en ligne valide(s)",
        path.display(),
        calendars.len()
    );
    for calendar in &calendars {
        println!(
            "  - {} → {} ({})",
            calendar.name,
            calendar.entity_id,
            calendar.source.label()
        );
    }
    Ok(())
}

/// `ha-server simulation check` : vérifie `simulation.yaml`.
pub fn check_simulation(path: &std::path::Path) -> Result<(), Box<dyn Error>> {
    match ha_simulation::load(path)? {
        None => println!(
            "{} : fichier absent (pas de simulation de présence)",
            path.display()
        ),
        Some(config) => {
            println!(
                "{} : simulation valide, rejoue il y a {} jours :",
                path.display(),
                config.days_ago
            );
            for device in &config.devices {
                println!("  - {device}");
            }
        }
    }
    Ok(())
}

/// `ha-server modeles check` : vérifie `modeles.yaml`.
pub fn check_templates(path: &std::path::Path) -> Result<(), Box<dyn Error>> {
    let entities = ha_automation::load_template_entities(path)?;
    if entities.is_empty() {
        println!(
            "{} : absent ou vide (pas de capteur modèle)",
            path.display()
        );
        return Ok(());
    }
    println!(
        "{} : {} capteur(s) modèle(s) valide(s)",
        path.display(),
        entities.len()
    );
    for entity in &entities {
        let reads: Vec<&str> = entity
            .template
            .entity_ids()
            .map(ha_core::EntityId::as_str)
            .collect();
        println!(
            "  - {} ({}) : lit {}",
            entity.entity_id,
            entity.name,
            if reads.is_empty() {
                "rien".to_owned()
            } else {
                reads.join(", ")
            }
        );
    }
    Ok(())
}

/// `ha-server protection-solaire check` : vérifie `protection-solaire.yaml`.
pub fn check_shade(path: &std::path::Path) -> Result<(), Box<dyn Error>> {
    let Some(config) = ha_shade::load(path)? else {
        println!(
            "{} : fichier absent (pas de protection solaire)",
            path.display()
        );
        return Ok(());
    };
    let heat = config.temperature.as_ref().map_or_else(
        || "sans mesure de température".to_owned(),
        |sensor| {
            format!(
                "au-dessus de {} °C ({sensor}), jusqu'à {} °C",
                config.threshold,
                config.threshold - config.hysteresis
            )
        },
    );
    println!(
        "{} : protection solaire valide, {heat} ; {} ; pause de {} h après un geste à la main",
        path.display(),
        config.weather.as_ref().map_or_else(
            || "quel que soit le temps".to_owned(),
            |w| format!("par soleil ({w})")
        ),
        config.manual_pause.as_secs_f64() / 3600.0
    );
    for cover in &config.covers {
        println!(
            "  - {} : façade à {}° (soleil à ±{}°, au moins {}° de haut), descend à {} % d'ouverture",
            cover.entity_id,
            cover.orientation,
            cover.half_angle,
            cover.min_elevation,
            cover.position
        );
    }
    Ok(())
}

/// `ha-server rappels check` : vérifie `rappels.yaml`.
pub fn check_alerts(path: &std::path::Path) -> Result<(), Box<dyn Error>> {
    let alerts = ha_alert::load(path)?;
    if alerts.is_empty() {
        println!("{} : absent ou vide (pas de rappel)", path.display());
        return Ok(());
    }
    println!("{} : {} rappel(s) valide(s)", path.display(), alerts.len());
    for alert in &alerts {
        let notifiers: Vec<&str> = alert
            .notifiers
            .iter()
            .map(ha_core::EntityId::as_str)
            .collect();
        println!(
            "  - {} ({}) : quand {} est « {} » ; {} ; vers {}{}",
            alert.entity_id,
            alert.name,
            alert.watched,
            alert.state,
            alert.rhythm(),
            notifiers.join(", "),
            if alert.can_acknowledge {
                ""
            } else {
                " (sans « Compris »)"
            }
        );
    }
    Ok(())
}

/// `ha-server delestage check` : vérifie `delestage.yaml`.
pub fn check_shedding(path: &std::path::Path) -> Result<(), Box<dyn Error>> {
    match ha_shedding::load(path)? {
        None => println!("{} : fichier absent (pas de délestage)", path.display()),
        Some(config) => {
            let limit = config.limit.map_or_else(
                || "la puissance souscrite du Linky".to_owned(),
                |limit| format!("{limit}"),
            );
            println!(
                "{} : délestage valide, mesure {}, limite {limit}, retour après {} min ; \
                 coupés dans cet ordre :",
                path.display(),
                config.measure,
                config.restore_after.as_secs() / 60
            );
            for device in &config.devices {
                println!("  - {device}");
            }
        }
    }
    Ok(())
}

/// `ha-server guard check` : vérifie la garde (`guard.yaml`).
pub fn check_guard(areas: &std::path::Path, guard: &std::path::Path) -> Result<(), Box<dyn Error>> {
    let areas = ha_areas::load(areas)?;
    let policy = ha_areas::guard::load(guard, &areas)?;
    if policy.is_off() {
        println!(
            "{} : absent ou vide (la garde ne retient rien)",
            guard.display()
        );
        return Ok(());
    }
    println!("{} : valide", guard.display());
    let mut protected: Vec<_> = policy.protected.iter().collect();
    protected.sort();
    for (entity_id, room) in protected {
        println!("  - {entity_id} ({room}) protégé");
    }
    match policy.quiet_hours {
        Some(hours) if policy.protected.is_empty() => {
            println!("  heures calmes {} : toute la maison", hours.describe());
        }
        Some(hours) => println!(
            "  heures calmes {} : la protection ne vaut que pendant ces heures",
            hours.describe()
        ),
        None => println!("  pas d'heures calmes : la protection vaut toute la journée"),
    }
    Ok(())
}

/// Vérifie `scripts.yaml` et `scenes.yaml` (avec `automations check`).
pub fn check_scripts_and_scenes(
    scripts: &std::path::Path,
    scenes: &std::path::Path,
) -> Result<(), Box<dyn Error>> {
    let loaded = ha_automation::scripts::load_file(scripts, ha_automation::parse_scripts)
        .map_err(|error| format!("{} : {error}", scripts.display()))?;
    println!(
        "{} : {} script(s) valide(s)",
        scripts.display(),
        loaded.len()
    );
    for script in &loaded {
        println!(
            "  - script.{} « {} » ({} action(s))",
            script.id,
            script.alias,
            script.actions.len()
        );
    }
    let loaded = ha_automation::scripts::load_file(scenes, ha_automation::parse_scenes)
        .map_err(|error| format!("{} : {error}", scenes.display()))?;
    println!("{} : {} scène(s) valide(s)", scenes.display(), loaded.len());
    for scene in &loaded {
        println!(
            "  - scene.{} « {} » ({} appareil(s))",
            scene.id,
            scene.name,
            scene.actions.len()
        );
    }
    Ok(())
}

/// `ha-server automations check | list | delete <id>`.
pub async fn automations_command(args: &[&str]) -> Result<(), Box<dyn Error>> {
    let path = crate::automations_path();
    match args {
        ["check"] => {
            check_automations(&path)?;
            check_scripts_and_scenes(&crate::scripts_path(), &crate::scenes_path())?;
            // On n'ouvre la base que si elle existe : une simple
            // vérification ne doit pas créer de fichier.
            if std::path::Path::new(&crate::db_path()).exists() {
                with_editor(&path, |editor| Box::pin(check_stored(editor))).await?;
            }
            Ok(())
        }
        ["list"] => with_editor(&path, |editor| Box::pin(list_automations(editor))).await,
        ["delete", id] => {
            let store = crate::open_store().await?;
            let editor = editor_for(&path, &store)?;
            let result = editor.delete(id, "ligne de commande").await;
            let result = match result {
                Ok(true) => {
                    audit_cli(&store, "automations", args).await;
                    println!("automatisation « {id} » supprimée (ses versions sont gardées).");
                    println!("redémarre le serveur pour l'appliquer.");
                    Ok(())
                }
                Ok(false) => Err(format!("aucune automatisation « {id} » dans l'éditeur").into()),
                Err(error) => Err(error.into()),
            };
            store.close().await;
            result
        }
        _ => Err(USAGE.into()),
    }
}

fn editor_for(
    path: &std::path::Path,
    store: &Store,
) -> Result<ha_automation::Editor, Box<dyn Error>> {
    let file = ha_automation::load_configs(path)?;
    let has_location = crate::location()?.is_some();
    Ok(ha_automation::Editor::new(
        store.clone(),
        file,
        has_location,
    ))
}

/// Ouvre la base, prépare l'éditeur, lance `run` puis ferme la base.
///
/// Concept Rust : `run` renvoie un `Future` « épinglé » dans une `Box`
/// (`Pin<Box<…>>`). C'est la façon simple de passer une fonction `async`
/// qui emprunte un paramètre (`&Editor`) : le compilateur sait ainsi que le
/// `Future` ne vit pas plus longtemps que l'emprunt.
async fn with_editor<F>(path: &std::path::Path, run: F) -> Result<(), Box<dyn Error>>
where
    F: for<'a> FnOnce(
        &'a ha_automation::Editor,
    )
        -> std::pin::Pin<Box<dyn Future<Output = Result<(), Box<dyn Error>>> + 'a>>,
{
    let store = crate::open_store().await?;
    let result = match editor_for(path, &store) {
        Ok(editor) => run(&editor).await,
        Err(error) => Err(error),
    };
    store.close().await;
    result
}

/// Vérifie les automatisations de l'éditeur, rangées dans la base.
async fn check_stored(editor: &ha_automation::Editor) -> Result<(), Box<dyn Error>> {
    let entries = editor.entries().await?;
    let stored: Vec<_> = entries
        .iter()
        .filter(|entry| entry.source == ha_automation::editor::Source::Editor)
        .collect();
    let invalid: Vec<_> = stored
        .iter()
        .filter(|entry| entry.error.is_some())
        .collect();
    println!(
        "éditeur (base de données) : {} automatisation(s), {} invalide(s)",
        stored.len(),
        invalid.len()
    );
    for entry in &invalid {
        println!(
            "  - {} : {}",
            entry.id,
            entry.error.as_deref().unwrap_or_default()
        );
    }
    if invalid.is_empty() {
        Ok(())
    } else {
        Err("des automatisations de l'éditeur sont invalides : elles ne tournent pas".into())
    }
}

async fn list_automations(editor: &ha_automation::Editor) -> Result<(), Box<dyn Error>> {
    let entries = editor.entries().await?;
    if entries.is_empty() {
        println!("aucune automatisation.");
    }
    for entry in &entries {
        let source = match entry.source {
            ha_automation::editor::Source::File => "fichier",
            ha_automation::editor::Source::Editor => "éditeur",
        };
        let status = match (&entry.error, &entry.config) {
            (Some(error), _) => format!("INVALIDE : {error}"),
            (None, Some(config)) if !config.enabled() => "désactivée".to_owned(),
            _ => "active".to_owned(),
        };
        let alias = entry
            .config
            .as_ref()
            .map_or(entry.id.as_str(), ha_automation::AutomationConfig::alias);
        let modified = match (&entry.updated_by, entry.updated_at) {
            (Some(by), Some(at)) => format!(", modifiée par {by} le {}", local_time(at)),
            _ => String::new(),
        };
        println!(
            "  - {} « {alias} » ({source}, {status}{modified})",
            entry.id
        );
    }
    Ok(())
}

/// Une date en heure locale : `14/10/2026 23:05:12`.
fn local_time(at: std::time::SystemTime) -> String {
    jiff::Timestamp::try_from(at).map_or_else(
        |_| "date invalide".to_owned(),
        |at| {
            at.to_zoned(jiff::tz::TimeZone::system())
                .strftime("%d/%m/%Y %H:%M:%S")
                .to_string()
        },
    )
}

/// `ha-server alexa check` : vérifie `alexa.yaml`.
pub fn check_alexa(path: &std::path::Path) -> Result<(), Box<dyn Error>> {
    if !path.exists() {
        println!(
            "{} n'existe pas : aucun appareil pour Alexa.",
            path.display()
        );
        return Ok(());
    }
    let devices = ha_alexa::load(path).map_err(|error| format!("{} : {error}", path.display()))?;
    println!(
        "{} : {} appareil(s) pour Alexa",
        path.display(),
        devices.len()
    );
    for device in &devices {
        match &device.name {
            Some(name) => println!("  - {} « {name} »", device.entity_id),
            None => println!("  - {} (nom de l'appareil)", device.entity_id),
        }
    }
    Ok(())
}

/// `ha-server alexa relay` : le relais, sur la machine, devant un serveur
/// lancé dans Docker Desktop (voir `ha_alexa::relay`). Tourne jusqu'à Ctrl+C.
pub async fn run_alexa_relay(
    settings: ha_alexa::relay::RelaySettings,
    stop: impl std::future::Future<Output = std::io::Result<()>>,
) -> Result<(), Box<dyn Error>> {
    let target = settings.target;
    let relay = ha_alexa::relay::Relay::start(settings).await?;
    println!(
        "Relais Alexa : l'Echo trouvera le pont sur http://{}/, relayé vers {target}. Ctrl+C pour arrêter.",
        relay.address
    );
    let stopped = stop.await;
    relay.stop().await;
    Ok(stopped?)
}

/// `ha-server thermostats check` : vérifie `thermostats.yaml`.
pub fn check_thermostats(path: &std::path::Path) -> Result<(), Box<dyn Error>> {
    if !path.exists() {
        println!("{} n'existe pas : aucun thermostat.", path.display());
        return Ok(());
    }
    let thermostats =
        ha_thermostat::load(path).map_err(|error| format!("{} : {error}", path.display()))?;
    println!(
        "{} : {} thermostat(s) valide(s)",
        path.display(),
        thermostats.len()
    );
    for t in &thermostats {
        let window = t
            .window
            .as_ref()
            .map_or_else(String::new, |w| format!(", fenêtre {w}"));
        println!(
            "  - climate.{} « {} » : {} d'après {}{window}, {}–{} °C",
            t.id, t.name, t.heater, t.sensor, t.min_temp, t.max_temp
        );
    }
    Ok(())
}

/// `ha-server persons check` : vérifie `persons.yaml`.
pub fn check_persons(path: &std::path::Path) -> Result<(), Box<dyn Error>> {
    if !path.exists() {
        println!("{} n'existe pas : aucune personne.", path.display());
        return Ok(());
    }
    let persons = ha_person::load(path).map_err(|error| format!("{} : {error}", path.display()))?;
    println!("{} : {} personne(s)", path.display(), persons.len());
    for person in &persons {
        let trackers: Vec<&str> = person
            .trackers
            .iter()
            .map(ha_core::EntityId::as_str)
            .collect();
        println!(
            "  - person.{} « {} » : {}",
            person.id,
            person.name,
            trackers.join(", ")
        );
    }
    Ok(())
}

pub fn check_inputs(path: &std::path::Path) -> Result<(), Box<dyn Error>> {
    if !path.exists() {
        println!("{} n'existe pas : aucune entrée.", path.display());
        return Ok(());
    }
    let config = ha_input::load(path).map_err(|error| format!("{} : {error}", path.display()))?;
    println!("{} : {} entrée(s)", path.display(), config.helpers.len());
    for helper in &config.helpers {
        println!("  - {} « {} »", helper.entity_id, helper.name);
    }
    Ok(())
}

pub fn check_derived(path: &std::path::Path) -> Result<(), Box<dyn Error>> {
    if !path.exists() {
        println!("{} n'existe pas : aucun assistant calculé.", path.display());
        return Ok(());
    }
    let derived =
        ha_derived::load(path).map_err(|error| format!("{} : {error}", path.display()))?;
    println!(
        "{} : {} assistant(s) calculé(s)",
        path.display(),
        derived.len()
    );
    for item in &derived {
        println!("  - {} « {} »", item.entity_id, item.name);
    }
    Ok(())
}

/// `ha-server energy check` : vérifie le fichier du bilan énergétique.
pub fn check_energy(path: &std::path::Path) -> Result<(), Box<dyn Error>> {
    if !path.exists() {
        println!(
            "{} n'existe pas : les index du compteur Linky seront comptés automatiquement.",
            path.display()
        );
        return Ok(());
    }
    let config = ha_energy::load(path).map_err(|error| format!("{} : {error}", path.display()))?;
    println!("{} : {} compteur(s)", path.display(), config.sources.len());
    for source in &config.sources {
        let kind = match source.kind {
            ha_energy::Kind::Consumption => "consommation",
            ha_energy::Kind::Production => "production",
        };
        let price = source
            .price
            .map_or_else(String::new, |price| format!(", {price} €/kWh"));
        println!("  - {} ({kind}{price})", source.entity_id);
    }
    if let Some(subscription) = config.subscription {
        println!("  abonnement : {subscription} € par mois");
    }
    Ok(())
}

/// `ha-server vigilance test` : interroge Météo-France une fois et affiche
/// la vigilance du département.
pub async fn test_vigilance(
    config: &ha_weather::vigilance::VigilanceConfig,
) -> Result<(), Box<dyn Error>> {
    let department = config.department().as_str();
    let map = ha_weather::vigilance::fetch_once(config).await?;
    let zone = jiff::tz::TimeZone::system();
    let local = |time: jiff::Timestamp| time.to_zoned(zone.clone()).strftime("%d/%m à %H:%M");
    println!("Clé acceptée. Carte publiée le {}.", local(map.updated));
    let now = jiff::Timestamp::now();
    let Some(today) = map.at(now) else {
        println!("La carte ne couvre pas l'heure actuelle (elle sera bientôt republiée).");
        return Ok(());
    };
    let mut periods = vec![("Aujourd'hui", today)];
    if let Some(tomorrow) = map.after(today) {
        periods.push(("Demain", tomorrow));
    }
    for (label, period) in periods {
        println!(
            "{label} ({department}) : vigilance {}",
            period.level.name().to_lowercase()
        );
        for hazard in &period.hazards {
            if hazard.level > ha_weather::carte::Level::Green {
                let coast = if hazard.coast { " (littoral)" } else { "" };
                println!(
                    "  - {}{coast} : {}",
                    hazard.phenomenon.name(),
                    hazard.level.name().to_lowercase()
                );
            }
        }
    }
    Ok(())
}

/// `sonos` : les enceintes trouvées, par groupe, et le nom de leur entité.
pub(crate) async fn list_sonos(hosts: &[String]) -> Result<(), Box<dyn Error>> {
    if hosts.is_empty() {
        println!("recherche des enceintes Sonos sur le réseau local (3 s)…");
    }
    let groups = ha_sonos::find(hosts).await?;
    for group in &groups {
        let names: Vec<String> = group
            .members
            .iter()
            .map(|m| {
                let slug = ha_core::slugify(&m.name);
                format!("{} (media_player.{slug}, {})", m.name, m.host)
            })
            .collect();
        if names.len() == 1 {
            println!("- {}", names[0]);
        } else {
            println!("- groupe : {}", names.join(" + "));
        }
    }
    println!(
        "{} enceinte(s). Pour le serveur : HA_SONOS={}",
        groups.iter().map(|g| g.members.len()).sum::<usize>(),
        groups
            .first()
            .and_then(|g| g.coordinator())
            .map_or("auto", |m| m.host.trim_end_matches(":1400"))
    );
    Ok(())
}

/// `voix "<phrase>"` : fait dire la phrase à Piper, et l'écrit dans
/// `essai-voix.wav` (avec le carillon), à écouter sur le PC.
pub(crate) async fn test_voice(tts: &ha_tts::Tts, phrase: &str) -> Result<(), Box<dyn Error>> {
    println!("voix : {}", tts.describe());
    let started = std::time::Instant::now();
    let voice = tts.speak(phrase).await?;
    let sound = ha_tts::announcement(true, Some(&voice));
    std::fs::write("essai-voix.wav", &sound.wav)?;
    println!(
        "essai-voix.wav écrit : {:.1} s de son, préparé en {:.1} s",
        sound.duration.as_secs_f64(),
        started.elapsed().as_secs_f64()
    );
    Ok(())
}

/// `ecoute <fichier.wav>` : fait écouter un enregistrement (16 bits, mono,
/// 16 000 Hz) à Whisper, comme une phrase dite à la tablette.
pub(crate) async fn test_listening(stt: &ha_stt::Stt, file: &str) -> Result<(), Box<dyn Error>> {
    println!("écoute : {}", stt.describe());
    let wav = std::fs::read(file)?;
    let started = std::time::Instant::now();
    let text = stt.transcribe(&wav).await?;
    println!(
        "entendu en {:.1} s : « {text} »",
        started.elapsed().as_secs_f64()
    );
    Ok(())
}

/// `smartthings pair|status|devices|forget` : la maison SmartThings de
/// Samsung (étape 86).
pub(crate) async fn smartthings_command(args: &[&str]) -> Result<(), Box<dyn Error>> {
    let store = crate::open_store().await?;
    let result = smartthings_inner(args, &store).await;
    if result.is_ok() {
        audit_cli(&store, "smartthings", args).await;
    }
    store.close().await;
    result
}

async fn smartthings_inner(args: &[&str], store: &Store) -> Result<(), Box<dyn Error>> {
    use ha_smartthings::api::{self, Api, App};

    let endpoint = crate::smartthings_endpoint();
    let http = endpoint.http()?;
    match args {
        ["pair"] => {
            println!("Il faut l'application SmartThings créée pour la maison (une seule fois,");
            println!(
                "avec « smartthings apps:create », voir docs/smartthings.md) : son Client ID,"
            );
            println!("son Client Secret et l'adresse de retour (redirect URI) qu'elle connaît.");
            println!();
            let client_id = ask_line("Client ID : ")?;
            if client_id.is_empty()
                || client_id.len() > 128
                || !client_id.chars().all(|c| c.is_ascii_graphic())
            {
                return Err("Client ID invalide".into());
            }
            let client_secret = ask_secret("Client Secret : ")?;
            if client_secret.is_empty() || client_secret.len() > 256 {
                return Err("Client Secret invalide".into());
            }
            let redirect_uri = ask_line("Adresse de retour (redirect URI) : ")?;
            if !redirect_uri.starts_with("https://")
                || redirect_uri.len() > 512
                || !redirect_uri.chars().all(|c| c.is_ascii_graphic())
            {
                return Err("l'adresse de retour doit commencer par https://".into());
            }
            let mut random = [0u8; 16];
            getrandom::fill(&mut random)
                .map_err(|error| format!("aléatoire indisponible : {error}"))?;
            let state: String = random
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<Vec<_>>()
                .concat();
            println!();
            println!("1. Ouvre cette adresse, connecte-toi à ton compte Samsung, choisis le lieu");
            println!("   et autorise « la maison » :");
            println!();
            println!(
                "{}",
                api::authorize_url(&endpoint, &client_id, &redirect_uri, &state)
            );
            println!();
            println!("2. Tu arrives sur l'adresse de retour (la page peut afficher une erreur :");
            println!("   ce n'est pas grave). Copie l'adresse complète de cette page.");
            let pasted = ask_line("Colle-la ici : ")?;
            let code = api::read_callback(&pasted, &state)?;
            let app = App {
                client_id,
                client_secret,
                redirect_uri,
            };
            let tokens = app.exchange(&http, &endpoint, &code).await?;
            let devices = Api::new(http.clone(), endpoint.clone(), tokens.access_token.clone())
                .devices()
                .await?;
            store
                .save_smartthings_account(&ha_storage::SmartThingsAccount {
                    client_id: app.client_id,
                    client_secret: app.client_secret,
                    redirect_uri: app.redirect_uri,
                    tokens,
                })
                .await?;
            println!("associé !");
            print_smartthings_devices(devices);
            println!("Relance le serveur (sudo systemctl restart ha-maison) pour les voir.");
            Ok(())
        }
        ["status" | "devices"] => {
            let Some(account) = store.smartthings_account().await? else {
                println!("aucune maison SmartThings associée (ha-server smartthings pair)");
                return Ok(());
            };
            println!("application : {}", account.client_id);
            println!("adresse de retour : {}", account.redirect_uri);
            if let Some((refreshed, paired)) = store.smartthings_dates().await? {
                println!("associée le {}", local_time(paired));
                println!("jetons renouvelés le {}", local_time(refreshed));
            }
            let left = account
                .tokens
                .expires_at
                .duration_since(std::time::SystemTime::now())
                .unwrap_or_default();
            // On ne renouvelle pas les jetons ici : c'est le serveur qui
            // s'en charge (un jeton de renouvellement ne sert qu'une fois).
            if left.is_zero() {
                println!(
                    "jeton d'accès expiré : le serveur le renouvellera à son prochain passage"
                );
                return Ok(());
            }
            println!("jeton d'accès valable encore {} h", left.as_secs() / 3600);
            if args == ["devices"] {
                let devices = Api::new(http, endpoint, account.tokens.access_token)
                    .devices()
                    .await?;
                print_smartthings_devices(devices);
            }
            Ok(())
        }
        ["forget"] => {
            if store.forget_smartthings_account().await? {
                println!(
                    "association SmartThings oubliée. Pour couper aussi l'accès côté Samsung :"
                );
                println!("« smartthings apps:delete » sur l'application de la maison.");
            } else {
                println!("aucune maison SmartThings associée");
            }
            Ok(())
        }
        _ => Err(USAGE.into()),
    }
}

/// Les appareils et les entités qu'ils donneront.
fn print_smartthings_devices(devices: Vec<ha_smartthings::api::Device>) {
    let core = ha_core::Core::new();
    let mut mapped = ha_smartthings::devices::Devices::default();
    let total = devices.len();
    mapped.sync(devices, &std::collections::HashMap::new(), &core.states);
    println!("{total} appareils, dont {} utilisables :", mapped.len());
    for (label, entities) in mapped.summary() {
        println!("  {label} : {}", entities.join(", "));
    }
}

/// `colis pair|status|forget` : la boîte aux lettres où arrivent les
/// e-mails d'Amazon (étape 87).
pub(crate) async fn colis_command(args: &[&str]) -> Result<(), Box<dyn Error>> {
    let store = crate::open_store().await?;
    let result = colis_inner(args, &store).await;
    if result.is_ok() {
        audit_cli(&store, "colis", args).await;
    }
    store.close().await;
    result
}

async fn colis_inner(args: &[&str], store: &Store) -> Result<(), Box<dyn Error>> {
    let mailbox = crate::colis_mailbox();
    match args {
        ["pair"] => {
            println!("La maison lit les e-mails d'Amazon (lecture seule) pour suivre les colis.");
            println!("Il faut un « mot de passe d'application » Google, pas ton mot de passe");
            println!("habituel : voir docs/colis.md (myaccount.google.com/apppasswords).");
            println!();
            let host = ask_line("Serveur IMAP [imap.gmail.com] : ")?;
            let host = if host.is_empty() {
                "imap.gmail.com".to_owned()
            } else {
                host
            };
            if !ha_tls::is_valid_host(&host) || host.contains(':') {
                return Err("serveur invalide".into());
            }
            let username = ask_line("Adresse e-mail : ")?;
            if !username.contains('@')
                || username.len() > 254
                || !username.chars().all(|c| c.is_ascii_graphic())
            {
                return Err("adresse e-mail invalide".into());
            }
            if is_microsoft_mailbox(&host, &username) {
                return Err(MICROSOFT_MAILBOX.into());
            }
            let pasted = ask_secret("Mot de passe d'application (16 lettres) : ")?;
            // Google l'affiche en quatre groupes : les espaces ne comptent pas.
            let password = Zeroizing::new(pasted.split_whitespace().collect::<String>());
            if password.is_empty() || password.len() > 256 {
                return Err("mot de passe invalide".into());
            }
            let account = ha_storage::MailAccount {
                host,
                port: 993,
                username,
                password,
            };
            println!("connexion…");
            let report =
                ha_colis::check(&mailbox, &account)
                    .await
                    .map_err(|error| match error {
                        ha_colis::ColisError::Unauthorized => {
                            "compte refusé : vérifie l'adresse et le mot de passe d'application"
                                .to_owned()
                        }
                        other => other.to_string(),
                    })?;
            store.save_mail_account(&account).await?;
            println!("associée !");
            print_report(&report);
            println!();
            println!("Redémarre le serveur : sudo systemctl restart ha-maison");
            Ok(())
        }
        ["status"] => {
            let Some(account) = store.mail_account().await? else {
                println!("aucune boîte associée (ha-server colis pair)");
                return Ok(());
            };
            println!("boîte : {} sur {}", account.username, account.host);
            let report = ha_colis::check(&mailbox, &account).await?;
            print_report(&report);
            Ok(())
        }
        ["forget"] => {
            if store.forget_mail_account().await? {
                println!("boîte oubliée. Pour couper aussi l'accès côté Google : supprime le mot");
                println!("de passe d'application sur myaccount.google.com/apppasswords.");
            } else {
                println!("aucune boîte associée");
            }
            Ok(())
        }
        _ => Err(USAGE.into()),
    }
}

/// Hotmail, Outlook.com, Live, MSN : Microsoft refuse les mots de passe
/// aux programmes (IMAP), il faut passer par une redirection vers Gmail.
const MICROSOFT_MAILBOX: &str = "\
Hotmail / Outlook.com n'accepte plus de mot de passe pour lire la boîte depuis un
programme (Microsoft exige une application enregistrée chez Azure).
Le plus simple : une règle Hotmail qui redirige les e-mails d'Amazon vers une
boîte Gmail, et associer cette boîte Gmail ici. Voir docs/colis.md, « Avec Hotmail ».";

fn is_microsoft_mailbox(host: &str, username: &str) -> bool {
    let host = host.to_ascii_lowercase();
    let domain = username
        .rsplit_once('@')
        .map(|(_, domain)| domain.to_ascii_lowercase())
        .unwrap_or_default();
    host.contains("outlook")
        || host.contains("office365")
        || host.contains("hotmail")
        || ["hotmail.", "outlook.", "live.", "msn."]
            .iter()
            .any(|prefix| domain.starts_with(prefix))
}

/// Ce que la boîte montre : les colis, et les e-mails ignorés.
fn print_report(report: &ha_colis::Report) {
    print_parcels(&report.parcels);
    if report.unsigned > 0 {
        println!();
        println!(
            "attention : {} e-mail(s) « d'Amazon » sur {} ignoré(s) : la signature d'Amazon",
            report.unsigned, report.mails
        );
        println!(
            "n'a pas été vérifiée (un faux, ou un transfert qui l'a cassée : voir docs/colis.md)."
        );
    }
}

/// Les colis des deux dernières semaines.
fn print_parcels(parcels: &[ha_colis::amazon::Parcel]) {
    if parcels.is_empty() {
        println!("aucun colis Amazon ces deux dernières semaines");
        return;
    }
    println!("colis Amazon des deux dernières semaines :");
    for parcel in parcels.iter().rev() {
        let name = ha_colis::amazon::short_name(&parcel.article);
        let when = parcel
            .at
            .to_zoned(jiff::tz::TimeZone::system())
            .strftime("%d/%m %H:%M");
        println!("  {when}  {}", parcel.step.sentence(&name));
    }
}

/// `timetree pair|status|forget` : le compte TimeTree de la maison
/// (étape 74).
pub(crate) async fn timetree_command(args: &[&str]) -> Result<(), Box<dyn Error>> {
    let store = crate::open_store().await?;
    let result = timetree_inner(args, &store).await;
    if result.is_ok() {
        audit_cli(&store, "timetree", args).await;
    }
    store.close().await;
    result
}

fn timetree_client(
    account: ha_storage::TimeTreeAccount,
) -> Result<ha_ics::timetree::Client, Box<dyn Error>> {
    let api = crate::timetree_api();
    // Le faux TimeTree des essais est en HTTP, sur ce PC.
    let http = if api.starts_with("http://127.0.0.1:") {
        ha_tls::web_client(false)?
    } else {
        ha_tls::public_client()?
    };
    Ok(ha_ics::timetree::Client::new(http, &api, account))
}

fn print_timetree_calendars(calendars: &[ha_ics::timetree::CalendarInfo]) {
    println!("agendas visibles par ce compte :");
    for calendar in calendars {
        println!("  - {}", calendar.name);
    }
    println!();
    println!("Pour en lire un, ajoute dans calendriers.yaml :");
    println!("  - nom: Famille");
    println!(
        "    timetree: {}",
        calendars.first().map_or("Famille", |c| c.name.as_str())
    );
}

async fn timetree_inner(args: &[&str], store: &Store) -> Result<(), Box<dyn Error>> {
    match args {
        ["pair"] => {
            println!(
                "TimeTree n'a pas d'accès officiel : le serveur se connecte comme le site web,"
            );
            println!(
                "en lecture seule. Utilise un compte DÉDIÉ à la maison, invité dans ton agenda"
            );
            println!("partagé, jamais ton compte personnel. Le mot de passe est gardé chiffré.");
            println!();
            let email = ask_line("e-mail du compte TimeTree de la maison : ")?;
            if !(3..=254).contains(&email.len())
                || !email.contains('@')
                || email.chars().any(char::is_control)
            {
                return Err("adresse e-mail invalide".into());
            }
            let password = ask_secret("mot de passe : ")?;
            let account = ha_storage::TimeTreeAccount { email, password };
            let client = timetree_client(account.clone())?;
            let calendars = client.calendars().await?;
            store.save_timetree_account(&account).await?;
            println!("associé !");
            print_timetree_calendars(&calendars);
            println!("puis relance « ha-server ».");
            Ok(())
        }
        ["status"] => {
            let Some(account) = store.timetree_account().await? else {
                println!("aucun compte TimeTree associé (ha-server timetree pair)");
                return Ok(());
            };
            println!("compte : {}", account.email);
            let calendars = timetree_client(account)?.calendars().await?;
            print_timetree_calendars(&calendars);
            Ok(())
        }
        ["forget"] => {
            if store.forget_timetree_account().await? {
                println!("compte TimeTree oublié (pense à changer son mot de passe s'il a fuité)");
            } else {
                println!("aucun compte TimeTree associé");
            }
            Ok(())
        }
        _ => Err(USAGE.into()),
    }
}

/// `iglou pair|status|forget` : la liste de courses de l'appli Iglou
/// (étape 81).
pub(crate) async fn iglou_command(args: &[&str]) -> Result<(), Box<dyn Error>> {
    let store = crate::open_store().await?;
    let result = iglou_inner(args, &store).await;
    if result.is_ok() {
        audit_cli(&store, "iglou", args).await;
    }
    store.close().await;
    result
}

fn print_iglou_list(items: &[ha_iglou::list::Item]) {
    let left = items.iter().filter(|i| !i.done).count();
    println!("liste de courses : {left} article(s) à acheter");
    for item in items.iter().take(30) {
        println!(
            "  {} {} ({}, {})",
            if item.done { "[x]" } else { "[ ]" },
            item.summary,
            item.quantity,
            item.group
        );
    }
    if items.len() > 30 {
        println!("  … et {} de plus", items.len() - 30);
    }
}

async fn iglou_inner(args: &[&str], store: &Store) -> Result<(), Box<dyn Error>> {
    match args {
        ["pair"] => {
            println!("Iglou : la maison lit et complète la liste de courses de l'appli.");
            println!("Utilise un compte DÉDIÉ à la maison (créé dans l'appli), jamais ton compte");
            println!("personnel. Le mot de passe est gardé chiffré.");
            println!();
            println!("Le projet et la clé web sont dans la configuration Firebase de l'appli");
            println!("(google-services.json : « project_id » et « current_key »).");
            let project_id = ask_line("projet Firebase : ")?;
            if !ha_iglou::client::is_project_id(&project_id) {
                return Err("projet invalide (minuscules, chiffres et tirets)".into());
            }
            let api_key = ask_line("clé web Firebase : ")?;
            if !ha_iglou::client::is_api_key(&api_key) {
                return Err("clé web invalide".into());
            }
            let email = ask_line("e-mail du compte Iglou de la maison : ")?;
            if !(3..=254).contains(&email.len())
                || !email.contains('@')
                || email.chars().any(char::is_control)
            {
                return Err("adresse e-mail invalide".into());
            }
            let password = ask_secret("mot de passe : ")?;
            let mut account = ha_storage::IglouAccount {
                project_id,
                api_key,
                email,
                password,
                maison_id: String::new(),
            };
            let client = crate::iglou_client(&account)?;
            let maisons = client.maisons().await?;
            let maison = match maisons.as_slice() {
                [] => {
                    println!("Ce compte n'est membre d'aucune maison Iglou.");
                    println!(
                        "Dans l'appli, sur ton téléphone : Réglages → Code d'invitation, puis recopie-le."
                    );
                    let code = ask_line("code d'invitation : ")?;
                    client.join(&code, "Maison").await?
                }
                [one] => one.clone(),
                many => {
                    for (index, maison) in many.iter().enumerate() {
                        println!("  {}. {}", index + 1, maison.nom);
                    }
                    let choice = ask_line("laquelle (numéro) : ")?;
                    choice
                        .trim()
                        .parse::<usize>()
                        .ok()
                        .and_then(|n| many.get(n.checked_sub(1)?))
                        .cloned()
                        .ok_or("choix invalide")?
                }
            };
            account.maison_id = maison.id.clone();
            let items = client.shopping_list(&maison.id).await?;
            store.save_iglou_account(&account).await?;
            println!("associé à la maison « {} » !", maison.nom);
            // Ce qui restait sur la liste locale passe dans Iglou.
            let local = store.todo_items(ha_iglou::ENTITY).await?;
            let mut copied = 0;
            for item in local.iter().filter(|i| !i.done) {
                client.add(&maison.id, &item.summary).await?;
                store.remove_todo_item(ha_iglou::ENTITY, item.id).await?;
                copied += 1;
            }
            if copied > 0 {
                println!("{copied} article(s) de l'ancienne liste locale ajouté(s) dans Iglou");
            }
            print_iglou_list(&items);
            println!("puis relance « ha-server » : todo.courses devient la liste d'Iglou.");
            Ok(())
        }
        ["status"] => {
            let Some(account) = store.iglou_account().await? else {
                println!("aucun compte Iglou associé (ha-server iglou pair)");
                return Ok(());
            };
            println!("compte : {} (projet {})", account.email, account.project_id);
            let client = crate::iglou_client(&account)?;
            let maisons = client.maisons().await?;
            match maisons.iter().find(|m| m.id == account.maison_id) {
                Some(maison) => println!("maison : {}", maison.nom),
                None => return Err("le compte n'est plus membre de sa maison Iglou : refais « ha-server iglou pair »".into()),
            }
            print_iglou_list(&client.shopping_list(&account.maison_id).await?);
            Ok(())
        }
        ["forget"] => {
            if store.forget_iglou_account().await? {
                println!(
                    "compte Iglou oublié : la liste de courses locale reprend au prochain démarrage."
                );
                println!("Pour couper l'accès, retire aussi le membre « Maison » dans l'appli.");
            } else {
                println!("aucun compte Iglou associé");
            }
            Ok(())
        }
        _ => Err(USAGE.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::is_microsoft_mailbox;

    #[test]
    fn microsoft_mailboxes_are_spotted() {
        assert!(is_microsoft_mailbox("imap.gmail.com", "david@hotmail.fr"));
        assert!(is_microsoft_mailbox("imap.gmail.com", "David@Outlook.com"));
        assert!(is_microsoft_mailbox(
            "outlook.office365.com",
            "a@exemple.fr"
        ));
        assert!(!is_microsoft_mailbox("imap.gmail.com", "maison@gmail.com"));
        assert!(!is_microsoft_mailbox("imap.gmail.com", "a@livebox.fr"));
    }
}
