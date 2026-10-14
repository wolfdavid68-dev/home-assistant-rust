//! Tests de l'éditeur : formes YAML / JSON, vérifications, enregistrement
//! dans la base et rechargement du moteur sans redémarrage.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "dans un test, un échec doit simplement faire échouer le test"
)]

use std::sync::Arc;
use std::time::Duration;

use ha_automation::editor::Source;
use ha_automation::{
    AutomationConfig, AutomationError, Editor, Engine, Setup, parse, parse_configs, reload_channel,
};
use ha_core::{Attributes, Core, Event};
use ha_storage::Store;
use serde_json::json;
use tokio::sync::broadcast::Receiver;
use tokio::time::Instant;

const FILE: &str = r#"
- id: matin
  alias: Volets le matin
  triggers: [{trigger: time, at: "07:30"}]
  actions: [{action: cover.open_cover, target: {entity_id: cover.salon}}]
"#;

fn config(yaml: &str) -> AutomationConfig {
    AutomationConfig::from_yaml(yaml).unwrap()
}

const SOIR: &str = r#"
id: soir
alias: Lampe le soir
triggers:
  - trigger: state
    entity_id: binary_sensor.porte
    to: "on"
conditions:
  - condition: state
    entity_id: sensor.mode
    state: 3
actions:
  - action: light.turn_on
    target: {entity_id: light.salon}
    data: {brightness: 120}
  - delay: 10min
"#;

#[test]
fn yaml_and_json_forms_give_the_same_automation() {
    let soir = config(SOIR);
    // `3` sans guillemets devient du texte, comme dans Home Assistant.
    let json = soir.to_json();
    assert_eq!(json["triggers"][0]["to"], "on");
    assert_eq!(json["conditions"][0]["state"], "3");
    assert_eq!(json["actions"][0]["data"]["brightness"], 120);

    // Aller-retour par chacune des deux formes.
    assert_eq!(AutomationConfig::from_json(json.clone()).unwrap(), soir);
    let yaml = soir.to_yaml();
    assert!(
        yaml.starts_with("id: soir\nalias: Lampe le soir\n"),
        "{yaml}"
    );
    assert_eq!(config(&yaml), soir);

    // Même automatisation vérifiée que par le fichier.
    let from_file = parse(&format!("- {}", SOIR.trim().replace('\n', "\n  "))).unwrap();
    assert_eq!(from_file, [soir.check().unwrap()]);

    // Une liste d'un élément, copiée du fichier, est acceptée ; pas deux.
    assert_eq!(config(FILE).id(), "matin");
    let two = format!("{FILE}{}", FILE.replace("matin", "midi"));
    assert!(AutomationConfig::from_yaml(&two).is_err());

    // `on` sans guillemets est lu par YAML comme « vrai » : refusé, avec
    // un message qui dit quoi faire. Pareil depuis le fichier.
    let unquoted = SOIR.replace("\"on\"", "on");
    let error = AutomationError::to_string(&AutomationConfig::from_yaml(&unquoted).unwrap_err());
    assert!(error.contains("entre guillemets"), "{error}");
    let error = parse(&format!("- {}", unquoted.trim().replace('\n', "\n  "))).unwrap_err();
    assert!(error.to_string().contains("entre guillemets"), "{error}");

    // Les champs inconnus restent refusés, en JSON aussi.
    let mut typo = json;
    typo["alais"] = json!("x");
    assert!(AutomationConfig::from_json(typo).is_err());
}

async fn editor(has_location: bool) -> (Editor, Store) {
    let store = Store::open_in_memory().await.unwrap();
    let file = parse_configs(FILE).unwrap();
    (Editor::new(store.clone(), file, has_location), store)
}

fn ids(automations: &[ha_automation::Automation]) -> Vec<&str> {
    automations.iter().map(|a| a.id.as_str()).collect()
}

#[tokio::test]
async fn save_check_delete_and_versions() {
    let (editor, _) = editor(false).await;
    assert_eq!(ids(&editor.active().await.unwrap()), ["matin"]);

    let saved = editor.save(&config(SOIR), "david").await.unwrap();
    assert!(saved.created);
    let renamed = config(&SOIR.replace("Lampe le soir", "Lampe du soir"));
    assert!(!editor.save(&renamed, "alice").await.unwrap().created);
    assert_eq!(ids(&editor.active().await.unwrap()), ["matin", "soir"]);

    let entries = editor.entries().await.unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].source, Source::File);
    assert_eq!(entries[1].source, Source::Editor);
    assert_eq!(entries[1].config.as_ref().unwrap().alias(), "Lampe du soir");
    assert_eq!(entries[1].updated_by.as_deref(), Some("alice"));

    // Désactivée : gardée, mais ne tourne plus.
    let disabled = config(&SOIR.replace("id: soir", "id: soir\nenabled: false"));
    editor.save(&disabled, "david").await.unwrap();
    assert_eq!(ids(&editor.active().await.unwrap()), ["matin"]);

    // Refusées : identifiant du fichier, soleil sans position, valeur invalide.
    let refused = [
        SOIR.replace("id: soir", "id: matin"),
        SOIR.replace(
            "- trigger: state\n    entity_id: binary_sensor.porte\n    to: \"on\"",
            "- trigger: sun\n    event: sunset",
        ),
        SOIR.replace("10min", "48h"),
    ];
    for yaml in refused {
        let error = editor.save(&config(&yaml), "david").await.unwrap_err();
        assert!(
            matches!(error, AutomationError::Invalid { .. }),
            "{yaml}\n{error}"
        );
    }

    // Le fichier n'est pas modifiable depuis l'éditeur.
    assert!(editor.delete("matin", "david").await.is_err());
    assert!(editor.delete("soir", "david").await.unwrap());
    assert!(!editor.delete("soir", "david").await.unwrap());
    assert_eq!(editor.entries().await.unwrap().len(), 1);

    // Les versions permettent de revenir en arrière, même après suppression.
    let versions = editor.versions("soir").await.unwrap();
    let aliases: Vec<Option<&str>> = versions
        .iter()
        .map(|v| v.config.as_ref().map(AutomationConfig::alias))
        .collect();
    assert_eq!(
        aliases,
        [
            None,
            Some("Lampe le soir"),
            Some("Lampe du soir"),
            Some("Lampe le soir")
        ]
    );
    assert_eq!(versions[2].changed_by, "alice");
}

#[tokio::test]
async fn the_sun_is_accepted_with_a_location() {
    let (editor, _) = editor(true).await;
    let sun = SOIR.replace(
        "- trigger: state\n    entity_id: binary_sensor.porte\n    to: \"on\"",
        "- trigger: sun\n    event: sunset\n    offset: \"-00:30:00\"",
    );
    editor.save(&config(&sun), "david").await.unwrap();
}

#[tokio::test]
async fn a_broken_stored_automation_does_not_block_the_others() {
    let (editor, store) = editor(false).await;
    editor.save(&config(SOIR), "david").await.unwrap();
    // Une ligne abîmée (base modifiée à la main, ancienne version…).
    store
        .save_automation("abimee", r#"{"id":"abimee","triggers":"?"}"#, "x")
        .await
        .unwrap();

    assert_eq!(ids(&editor.active().await.unwrap()), ["matin", "soir"]);
    let entries = editor.entries().await.unwrap();
    let broken = entries.iter().find(|e| e.id == "abimee").unwrap();
    assert!(broken.config.is_none());
    assert!(broken.error.is_some());
    // On peut la supprimer pour repartir proprement.
    assert!(editor.delete("abimee", "david").await.unwrap());
}

#[tokio::test]
async fn too_many_automations_are_refused() {
    let (editor, _) = editor(false).await;
    for n in 1..200 {
        let yaml = SOIR.replace("id: soir", &format!("id: a{n}"));
        editor.save(&config(&yaml), "david").await.unwrap();
    }
    let error = editor.save(&config(SOIR), "david").await.unwrap_err();
    assert!(matches!(error, AutomationError::TooMany(200)), "{error}");
    // Modifier une automatisation existante reste possible.
    let yaml = SOIR.replace("id: soir", "id: a1");
    editor.save(&config(&yaml), "david").await.unwrap();
}

/// Prochain appel de service publié sur le bus (3 s au plus) : (service, entité).
async fn next_call(rx: &mut Receiver<Arc<Event>>) -> (String, String) {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let event = tokio::time::timeout_at(deadline, rx.recv())
            .await
            .expect("aucun appel de service")
            .unwrap();
        if let Event::CallService {
            domain,
            service,
            entity_id,
            ..
        } = event.as_ref()
        {
            return (format!("{domain}.{service}"), entity_id.to_string());
        }
    }
}

fn set(core: &Core, entity: &str, value: &str) {
    core.states
        .set(entity.parse().unwrap(), value, Attributes::new())
        .unwrap();
}

#[tokio::test]
async fn saving_reloads_the_running_engine() {
    // « attente » : allume, attend 1 s, éteint. Elle ne sera pas modifiée :
    // son action en cours doit continuer malgré le rechargement.
    const WAIT: &str = r#"
id: attente
triggers: [{trigger: state, entity_id: input.attente, to: "go"}]
actions:
  - {action: switch.turn_on, target: {entity_id: switch.attente}}
  - delay: 1s
  - {action: switch.turn_off, target: {entity_id: switch.attente}}
"#;
    let store = Store::open_in_memory().await.unwrap();
    let (reloader, reloads) = reload_channel();
    let editor = Editor::new(store, Vec::new(), false).with_reloader(reloader);
    editor.save(&config(WAIT), "david").await.unwrap();

    let core = Arc::new(Core::new());
    let mut rx = core.bus.subscribe();
    let engine = Engine::start_with(
        Setup {
            automations: editor.active().await.unwrap(),
            reloads: Some(reloads),
            ..Setup::default()
        },
        Arc::clone(&core),
    );

    set(&core, "input.attente", "go");
    assert_eq!(
        next_call(&mut rx).await,
        ("switch.turn_on".into(), "switch.attente".into())
    );

    // Pendant l'attente, on crée une automatisation depuis l'éditeur.
    editor.save(&config(SOIR), "david").await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    set(&core, "binary_sensor.porte", "on");
    set(&core, "sensor.mode", "3");
    set(&core, "binary_sensor.porte", "off");
    set(&core, "binary_sensor.porte", "on");
    assert_eq!(
        next_call(&mut rx).await,
        ("light.turn_on".into(), "light.salon".into()),
        "la nouvelle automatisation tourne, sans redémarrage"
    );
    assert_eq!(
        next_call(&mut rx).await,
        ("switch.turn_off".into(), "switch.attente".into()),
        "l'action en cours de l'automatisation inchangée a continué"
    );

    // Supprimée : elle ne se déclenche plus.
    assert!(editor.delete("soir", "david").await.unwrap());
    tokio::time::sleep(Duration::from_millis(100)).await;
    set(&core, "binary_sensor.porte", "off");
    set(&core, "binary_sensor.porte", "on");
    set(&core, "input.attente", "stop");
    set(&core, "input.attente", "go");
    assert_eq!(
        next_call(&mut rx).await,
        ("switch.turn_on".into(), "switch.attente".into()),
        "plus de light.turn_on : seule « attente » réagit"
    );

    engine.stop().await;
}
