# Étape 27 : serrures, sirènes, vannes, aspirateurs, humidificateurs, chauffe-eau, expliqué

![Les nouveaux appareils sur le tableau de bord](images/appareils-mqtt.png)

## Le but

Jusqu'ici, la découverte MQTT (étape 4) comprenait les capteurs, les détecteurs, les prises et les
lumières. Elle comprend maintenant six nouveaux types d'appareils, annoncés exactement comme dans Home
Assistant. Zigbee2MQTT, ESPHome, Tasmota ou un montage maison les déclarent donc tout seuls :

| Type MQTT       | Exemple                        | Sur le tableau de bord                            |
| --------------- | ------------------------------ | ------------------------------------------------- |
| `lock`          | serrure connectée              | Verrouiller / Déverrouiller (mot de passe), Ouvrir |
| `siren`         | sirène                         | bouton glissant, « Sonne ! »                       |
| `valve`         | vanne d'arrosage, d'eau        | Ouvrir / Fermer                                    |
| `vacuum`        | robot aspirateur               | Lancer, Pause, Arrêter, Retour à la base, batterie |
| `humidifier`    | humidificateur, déshumidificateur | marche / arrêt, humidité voulue (− / +)         |
| `water_heater`  | chauffe-eau, ballon            | modes (Éco, Électrique…), température (− / +)      |

Rien à configurer : dès que l'appareil publie sa description sur `homeassistant/<type>/…/config`,
la tuile apparaît, et le journal de bord raconte ce qu'il fait (« Porte d'entrée s'est
déverrouillée par david », « Robot est rentré à sa base »).

Les commandes sont aussi proposées dans l'éditeur d'automatisations (« Renvoyer à sa base »,
« Régler l'humidité voulue »…).

## La sécurité d'abord

### Déverrouiller demande le mot de passe

Verrouiller est immédiat. **Déverrouiller ou ouvrir** une porte redemande ton mot de passe (et ton
code de double authentification si tu l'as activée), même si tu es administrateur. La confirmation
reste valable 15 minutes, comme pour modifier une automatisation (étape 13).

Pourquoi ? Un téléphone oublié déverrouillé sur une table, ou une session restée ouverte sur la
tablette du salon, ne doit pas suffire à ouvrir la maison. Le contrôle est fait **par le serveur** :
appeler l'API directement ne permet pas de le contourner.

```rust
pub(crate) fn needs_confirmation(domain: &str, service: &str) -> bool {
    matches!((domain, service), ("lock", "unlock" | "open"))
}
```

`matches!` compare une valeur à un **motif** : ici un couple `(domaine, service)` dont le second
élément est `"unlock"` **ou** `"open"`. C'est une liste de ce qui est sensible : un nouveau service
de verrou serait d'abord refusé à l'invité (règle des rôles), et on ajouterait ici ceux qui ouvrent.

« Basculer » une serrure (`lock.toggle`) n'existe volontairement pas : appuyer deux fois par erreur
ne doit jamais laisser la porte ouverte.

### Les invités ne voient ni les serrures ni les sirènes

Savoir si la porte est verrouillée, c'est savoir si la maison est ouverte. Comme la présence de la
famille et l'alarme (étape 23), les serrures et les sirènes sont cachées aux invités. Ils ne peuvent
pas non plus commander les vannes, l'aspirateur, l'humidificateur ou le chauffe-eau.

### Ce qu'envoie un appareil est vérifié

Comme pour les autres appareils MQTT, tout ce qui arrive du réseau est contrôlé :

- un **état** inconnu (`"???"` au lieu de `LOCKED`) est ignoré, l'état reste le dernier connu ;
- les **modes** d'un chauffe-eau doivent être une courte liste de textes simples ;
- les **bornes** (humidité de 30 à 70 %, eau de 40 à 65 °C) doivent avoir du sens.

Et dans l'autre sens, ce qu'on publie ne reprend **jamais** un texte de l'appelant tel quel :

```rust
Payload::Number { field, min, max } => {
    let value = data
        .get(*field)
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite())
        .ok_or(CommandError::Invalid(field))?;
    if value < *min || value > *max {
        return Err(CommandError::OutOfRange { … });
    }
    Ok(format_number(value))
}
```

- `and_then` enchaîne des étapes qui peuvent chacune échouer (`None`) : le champ existe-t-il ?
  est-ce un nombre ?
- `filter` garde la valeur seulement si elle est **finie** (pas l'infini, pas « NaN »).
- `ok_or(…)?` transforme « rien » en erreur, et `?` arrête la fonction avec cette erreur.

Un mode (`set_operation_mode`) doit faire partie de la liste **annoncée par l'appareil** : on renvoie
le texte de cette liste, pas celui reçu.

## Comment c'est construit

Un capteur ou une prise n'ont qu'un sujet d'état. Un humidificateur en a trois : son état
(`ON`/`OFF`), l'humidité voulue, et l'humidité mesurée. Chaque entité MQTT a donc maintenant un
**profil** (`crates/ha-mqtt/src/profile.rs`) qui dit :

- les **états** possibles (`LOCKED` reçu → `locked`) ;
- les **sujets d'attributs** (`h/consigne` → l'attribut `humidity`) ;
- ce qu'il faut **publier** pour chaque commande (`lock.unlock` → `UNLOCK` sur le sujet de commande).

Quand un message arrive, le pont met à jour l'état **sans effacer** les attributs reçus avant :
l'humidificateur qui envoie « ON » garde sa consigne de 45 %.

Un aspirateur envoie tout dans un seul message (`{"state": "cleaning", "battery_level": 61}`) : le
pont s'abonne **une seule fois** à ce sujet, et en tire l'état et la batterie.

## Tester sans appareil

Avec `mosquitto_pub`, tu peux annoncer un faux verrou :

```bash
mosquitto_pub -h <broker> -u <compte> -P <mot de passe> -r \
  -t homeassistant/lock/entree/config \
  -m '{"name": "Porte d'"'"'entrée", "stat_t": "maison/serrure", "cmd_t": "maison/serrure/set"}'
mosquitto_pub -h <broker> -u <compte> -P <mot de passe> -t maison/serrure -m LOCKED
```

et regarder les ordres envoyés : `mosquitto_sub -t maison/serrure/set -v`.
