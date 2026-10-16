# Étape 69 : les rappels insistants, expliqué

## Le but

Une notification se rate : on la voit, on se dit « j'irai fermer le garage », et on oublie. Un
**rappel insistant** redit la même chose, à intervalles choisis, **jusqu'à ce que ce soit réglé**.
Si quelqu'un touche « Compris », il se tait.

C'est l'intégration `alert` de Home Assistant, avec les mêmes réglages. Exemples :
- la porte du garage ouverte depuis 10 minutes ;
- le congélateur trop chaud ;
- une fenêtre ouverte alors que le chauffage tourne.

Le garage, par exemple :

| Heure | Ce qui se passe |
| --- | --- |
| 18:00 | le garage s'ouvre ; rien, on vient de l'ouvrir |
| 18:10 | « Le garage est ouvert depuis 10 min. » + bouton **Compris** |
| 18:40 | « Le garage est ouvert depuis 40 min. » (remplace le précédent sur le téléphone) |
| 19:10 | « … depuis 70 min. », et ainsi de suite toutes les 30 min |
| 19:25 | le garage se ferme : « Garage fermé. » remplace le dernier message |

## Mettre en place

Copie `rappels.example.yaml` en `rappels.yaml` (avec Docker : `config/rappels.yaml`), garde ce qui
te sert, puis vérifie :

```bash
ha-server rappels check
```

```yaml
# rappels.yaml
garage_ouvert:                # le nom : alert.garage_ouvert
  name: Garage ouvert
  entity_id: cover.garage     # ce qui est surveillé
  state: open                 # l'état qui déclenche le rappel (« on » par défaut)
  repeat: [10, 30]            # minutes : 10, puis toutes les 30
  skip_first: true            # rien à l'ouverture : d'abord attendre 10 min
  message: >-
    Le garage est ouvert depuis
    {{ ((as_timestamp(now()) - as_timestamp(states.cover.garage.last_changed)) / 60) | round(0) }} min.
  done_message: Garage fermé.
```

Redémarre le serveur : le rappel apparaît dans la section **Rappels** du tableau de bord.

| Réglage | Par défaut | Rôle |
| --- | --- | --- |
| `name` | (obligatoire) | le nom affiché, et le titre de la notification |
| `entity_id` | (obligatoire) | l'entité surveillée : volet, détecteur, capteur modèle… |
| `state` | `on` | l'état qui déclenche le rappel (`open` pour un volet ou une porte de garage) |
| `repeat` | (obligatoire) | les minutes entre deux messages, de 1 à 1440 ; la dernière se répète. `5`, ou `[15, 15, 60]` |
| `skip_first` | `false` | `true` : attendre la première durée avant le premier message |
| `message` | le nom | le texte ; un modèle `{{ … }}` est recalculé à chaque envoi |
| `done_message` | aucun | le message quand c'est réglé ; seulement si un message est parti avant |
| `title` | le nom | le titre de la notification |
| `can_acknowledge` | `true` | `false` : pas de « Compris », il insiste jusqu'au bout |
| `notifiers` | `[appli]` | `appli` (notifications du navigateur, étape 50), `telephone` (ntfy), ou les deux |
| `data` | aucun | `users: [david]` pour ne prévenir que lui, `priority: 5` pour réveiller le téléphone |

**Comment lire `repeat` :**
- `repeat: [10, 30]` sans `skip_first` : un message tout de suite, un autre 10 minutes après, puis
  toutes les 30 minutes ;
- avec `skip_first: true` : rien tout de suite, un message à 10 minutes, puis toutes les 30 minutes ;
- `ha-server rappels check` l'écrit en clair : « après 10 min, puis toutes les 30 min ».

**Pour une condition plus fine,** surveille un capteur modèle (étape 64). Par exemple, « le
congélateur au-dessus de -12 °C », dans `modeles.yaml` :

```yaml
binary_sensor:
  - name: Congélateur chaud
    state: "{{ states('sensor.congelateur') | float(-20) > -12 }}"
```

Puis, dans `rappels.yaml` :

```yaml
congelateur:
  name: Congélateur trop chaud
  entity_id: binary_sensor.congelateur_chaud
  repeat: 5
  can_acknowledge: false        # on ne fait pas taire un congélateur qui décongèle
  message: "Le congélateur est à {{ states('sensor.congelateur') }} °C."
  done_message: Le congélateur est revenu au froid.
  notifiers: [appli, telephone]
  data:
    priority: 5
```

## L'entité `alert.<nom>`

| État | Sens | Sur la tuile |
| --- | --- | --- |
| `idle` | rien à signaler | « Rien à signaler » |
| `on` | en cours : les messages partent | « En cours · 2 messages · prochain à 18:40 », bouton **Compris** |
| `off` | « Compris » : le serveur se tait jusqu'à ce que ce soit réglé | « Compris par david », bouton **Reprendre** |

- **Compris :**
  - le bouton de la notification, le bouton de la tuile ou l'action `alert.turn_off` font taire le
    rappel ;
  - le journal de bord dit qui : « Garage ouvert : compris par david » ;
  - le message de fin part quand même : on sait que c'est réglé.
- **Reprendre** (`alert.turn_on`) : les messages reprennent au suivant prévu, pas tout de suite.
- **L'accueil :** un rappel en cours compte dans « Tout est calme, sauf… », sauf si ce qu'il
  surveille y est déjà, comme la porte du garage : on ne compte pas la même chose deux fois.
- **Dans les automatisations :**
  - la tuile, le déclencheur `state` sur `alert.garage_ouvert` et les actions « Compris » et
    « Reprendre » de l'éditeur marchent comme pour toute entité ;
  - par exemple : « en partant, si un rappel est en cours, prévenir ».

**Ce qui ne déclenche rien :**
- **Un appareil injoignable :** si l'entité surveillée passe « indisponible », le rappel ne change
  pas d'état et n'envoie rien. Un garage qui perd le Wi-Fi n'est pas « fermé ».
- **Un redémarrage du serveur :**
  - un rappel en cours le reste, et « Compris » aussi ;
  - le message suivant attend son intervalle : rien n'est redit tout de suite.

## Sécurité

- **Les invités** ne voient pas les rappels : ils disent ce qui est ouvert dans la maison, comme les
  notifications. Ils ne peuvent pas non plus les faire taire.
- **Les destinataires** sont seulement `notify.appli` et `notify.telephone`, vérifiés au chargement.
  Une faute de frappe (`notifiers: [alexa]`) est refusée avec la raison.
- **Le bouton « Compris »** voyage avec un ticket à usage unique, propre à chaque téléphone
  (étape 51). Seul un compte qui a reçu la notification peut le toucher.
- **Les limites :**
  - 50 rappels ;
  - 10 durées par rappel, de 1 minute à 1 jour ;
  - 500 caractères par message ;
  - un nom de 33 caractères au plus, en minuscules, chiffres et « _ » (il sert d'identifiant au
    bouton) ;
  - `data` n'accepte que `users` et `priority`.
- **Un rappel ne surveille pas un autre rappel :** c'est refusé au chargement, pour éviter une
  boucle.

## Comment c'est construit

- `crates/ha-alert` (nouveau) :
  - `parse_alerts` vérifie `rappels.yaml` ;
  - `Tracker` suit un rappel sans horloge ni réseau : on lui donne l'heure, il dit quoi envoyer
    (`Send::Reminder`, `Send::Done`). Ses tests passent des heures en quelques millisecondes ;
  - `Alerts` le fait tourner. Il écoute :
    - le bus (changements d'état, `alert.turn_off`/`turn_on`/`toggle`, bouton `RAPPEL_<nom>` de
      l'événement `notification_action`) ;
    - une horloge à la seconde.
  - Les messages passent par `notify.send_message`, comme ceux d'une automatisation : même sujet
    (`tag`) pour qu'ils se remplacent, et le bouton « Compris ».
- `crates/ha-server` : le fichier (`HA_RAPPELS_PATH`, par défaut `rappels.yaml`) et
  `ha-server rappels check`.
- `crates/ha-api` :
  - `roles.rs` cache le domaine `alert` aux invités ;
  - `app.js` : la tuile, la section « Rappels », l'accueil, l'éditeur.
- `crates/ha-logbook` : les phrases du journal (« rappel en cours », « compris par david », « c'est
  réglé »).

Essayé sur la démo :
- un rappel sur « Invités à la maison », toutes les 1 puis 2 minutes : les messages sont partis à
  l'heure, avec la durée calculée par le modèle ;
- « Compris » depuis l'API : plus rien pendant deux intervalles ;
- l'interrupteur éteint : « Les invités sont partis. » ;
- après un redémarrage, le rappel est resté en cours sans redire aussitôt ;
- dans Chromium, en clair, en sombre et sur téléphone : la tuile et le bouton « Compris », qui la
  fait passer à « Compris par david » ;
- un invité ne voit pas la section.

### Concepts Rust utilisés

**Une machine à états pure** : `Tracker::update(alert, firing, now)` ne lit pas l'horloge et
n'envoie rien. Il reçoit l'heure et rend une décision (`Option<Send>`). Le service fait le reste :
lire l'état, envoyer, publier. Les tests avancent l'heure à la main (`start + mins(40)`), sans
attendre.

**`Option<bool>` pour « on ne sait pas »** : `Some(true)` veut dire « garage ouvert », `Some(false)`
« fermé », et `None` « injoignable ». `update` commence par `let firing = firing?;`. Le `?` sur une
`Option` rend `None` tout de suite, donc un appareil injoignable ne change rien.

**`#[serde(untagged)]`** : `repeat: 5` et `repeat: [10, 30]` sont lus par la même énumération
`Repeat { One(f64), Many(Vec<f64>) }`. serde essaie chaque forme dans l'ordre.

**`tokio::time::Instant` et `jiff::Timestamp`** : le premier mesure les attentes (il ne recule
jamais, et les tests peuvent le figer avec `start_paused`). Le second donne l'heure affichée
(« prochain à 18:40 »). Elle est calculée une seule fois, au moment où le message est prévu : l'état
ne change donc pas à chaque seconde.
