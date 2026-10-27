//! Les commandes `… check` : chacune vérifie un fichier de configuration
//! sans démarrer le serveur, et résume ce qu'il contient.

use std::error::Error;

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
    // Un autre nom porté par deux appareils : permis, mais Assist demandera
    // « Lequel ? » (étape 94).
    for (alias, owners) in ha_areas::shared_aliases(&areas) {
        let owners: Vec<&str> = owners.iter().map(ha_core::EntityId::as_str).collect();
        println!(
            "  attention : « {alias} » est l'autre nom de {} ; Assist demandera lequel",
            owners.join(", ")
        );
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
