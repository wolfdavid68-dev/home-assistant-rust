//! Les commandes en ligne de commande de `ha-server` (`user`, `hue`,
//! `automations check`…), rangées par sujet :
//!
//! - `users` : les comptes (`user …`) ;
//! - `devices` : les appareils du réseau local (`hue`, `somfy`, `freebox`) ;
//! - `accounts` : les comptes en ligne (`smartthings`, `colis`, `timetree`,
//!   `iglou`) ;
//! - `automations` : `automations check | list | delete` ;
//! - `checks` : les vérifications de fichiers (`areas check`…) ;
//! - `audit` : le journal d'audit ;
//! - `tools` : relais Alexa et essais (vigilance, Sonos, voix, écoute) ;
//! - `terminal` : la saisie au clavier et les dates, partagées.

mod accounts;
mod audit;
mod automations;
mod checks;
mod devices;
mod terminal;
mod tools;
mod users;

pub(crate) use accounts::{colis_command, iglou_command, smartthings_command, timetree_command};
pub use audit::{audit_command, audit_local};
pub use automations::automations_command;
pub use checks::{
    check_alerts, check_alexa, check_areas, check_calendars, check_derived, check_energy,
    check_guard, check_inputs, check_persons, check_shade, check_shedding, check_simulation,
    check_templates, check_thermostats,
};
pub use devices::{freebox_command, hue_command, somfy_command};
pub(crate) use tools::{list_sonos, test_listening, test_voice};
pub use tools::{run_alexa_relay, test_vigilance};
pub use users::user_command;

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
