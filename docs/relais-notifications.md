# Étape 103 : les notifications du téléphone relayées, expliqué

## Le but

La tondeuse Mammotion n'a pas d'accès ouvert : son appli parle à un serveur en Chine, et
Alexa ne sait que lui donner des ordres. Mais l'appli **prévient le téléphone** : « Tonte
terminée », « La tondeuse est bloquée », « Retour à la station ». Ces notifications disent
déjà presque tout.

Le relais les fait suivre à la maison. Un téléphone Android lit la notification de l'appli
Mammotion et l'envoie au mini-PC, qui en tire l'état de la tondeuse :

```
  appli Mammotion ──notification──▶ téléphone Android (MacroDroid)
                                          │  POST /api/relais
                                          ▼
                                 mini-PC : sensor.tondeuse = « bloquée »
```

Rien n'est demandé au serveur de Mammotion, aucun mot de passe Mammotion n'est donné à la
maison.

## Ce que la maison en fait

L'entité **`sensor.tondeuse`** (« Tondeuse ») prend l'état reconnu dans le titre ou le texte de
la notification :

| État               | Mots reconnus (début de mot, français ou anglais)                            |
| ------------------ | ---------------------------------------------------------------------------- |
| `bloquée`          | bloqué, coincée, soulevée, renversée, stuck, trapped, lifted, tilt           |
| `erreur`           | erreur, échec, anomalie, défaut, panne, error, fault, fail                   |
| `en pause (pluie)` | pluie, pluvieux, rain                                                        |
| `chargée`          | charge + terminée / complète / finie, batterie pleine, fully charged         |
| `tonte terminée`   | terminée, achevée, finie, complète, done                                     |
| `en pause`         | pause, suspendue                                                             |
| `retour à la base` | retour, rentre, return                                                       |
| `en charge`        | charge, charging                                                             |
| `en tonte`         | commence, démarre, lancée, start, mowing, tonte en cours                     |
| `batterie faible`  | batterie faible, low battery                                                 |

La première ligne qui correspond l'emporte : « Tonte terminée, retour à la station » donne
`tonte terminée`. Les mots sont reconnus au **début** d'un mot : « terrain » ne veut pas dire
`en tonte`, « restart » ne veut pas dire `en tonte` non plus.

Une notification qui ne dit aucun de ces mots (« Nouvelle carte disponible ») **garde l'état**
d'avant, mais devient le dernier message. Les attributs de l'entité :

- `dernier_message` : le texte de la dernière notification ;
- `titre` : son titre ;
- `recu` : quand elle est arrivée.

Chaque notification relayée, de la tondeuse ou d'une autre appli, publie aussi l'évènement
**`notification_telephone`** (`app`, `titre`, `texte`, `etat`). Une automatisation peut s'en
servir :

```yaml
- id: tondeuse_bloquee
  triggers:
    - trigger: state
      entity_id: sensor.tondeuse
      to: bloquée
  actions:
    - action: notify.send_message
      target:
        entity_id: notify.telephone
      data:
        title: Tondeuse
        message: La tondeuse est bloquée, va voir dans le jardin.
```

Seules les notifications d'une appli dont le nom contient « Mammotion » changent la tondeuse.

## Mettre en place

### 1. La clé du relais, sur le mini-PC

Le téléphone prouve qui il est avec une **clé** : 32 caractères au moins, tirée au hasard,
gardée dans un fichier **hors du dépôt**, à côté de `compose.override.yaml` :

```sh
openssl rand -hex 32 > secrets/relais_key
chmod 600 secrets/relais_key
```

Dans `compose.override.yaml` (lui aussi hors du dépôt) :

```yaml
services:
  ha-server:
    environment:
      HA_RELAIS_KEY_FILE: /run/secrets/relais_key
    secrets:
      - relais_key
secrets:
  relais_key:
    file: ./secrets/relais_key
```

Relance le serveur. Le journal dit `relais des notifications du téléphone : ouvert`. Sans clé,
`/api/relais` répond « introuvable » : le relais est fermé.

Sans Docker : `HA_RELAIS_KEY_FILE` (chemin du fichier) dans l'environnement.

### 2. MacroDroid, sur le téléphone Android

[MacroDroid](https://play.google.com/store/apps/details?id=com.arlosoft.macrodroid) est
gratuit jusqu'à 5 macros ; une seule suffit.

1. Ouvre MacroDroid, **Ajouter une macro**.
2. **Déclencheur** : *Évènements d'appareil* → **Notification reçue** → *Notification
   reçue* → choisis l'appli **Mammotion** (ou *Toute application* pour relayer aussi les
   autres). Accorde à MacroDroid l'**accès aux notifications** quand il le demande.
3. **Action** : *Connectivité* → **Requête HTTP** :
   - Méthode : **POST** ;
   - Adresse : `http://<adresse-du-mini-pc>:8123/api/relais` ;
   - En-tête : `Authorization` = `Bearer <la clé>` (le contenu de `secrets/relais_key`) ;
   - Corps : **formulaire** (`application/x-www-form-urlencoded`) avec trois champs :

     | Nom     | Valeur (bouton « … » de MacroDroid) |
     | ------- | ----------------------------------- |
     | `app`   | `[app_name]`                        |
     | `titre` | `[notification_title]`              |
     | `texte` | `[notification]`                    |

4. Nomme la macro « Relais tondeuse » et enregistre.
5. Dans les réglages Android, **Applications → MacroDroid → Batterie → Sans restriction**
   (sinon Android l'endort et des notifications se perdent).

Pour tester : *Tester l'action* dans MacroDroid. La réponse doit être `{"result":"ok", …}`, et
`sensor.tondeuse` apparaît dans le tableau de bord.

Un corps JSON marche aussi (`{"app": "…", "titre": "…", "texte": "…"}`, avec
`Content-Type: application/json`), pour Tasker ou un autre outil.

### 3. Hors de la maison

Le téléphone doit joindre le mini-PC. À la maison, par le Wi-Fi. Dehors, par **Tailscale**
(déjà utilisé pour la tablette) : l'adresse Tailscale du mini-PC dans la requête HTTP, et
Tailscale allumé sur le téléphone. Il coûte en général 1 à 3 % de batterie par jour ;
MacroDroid seul, qui attend une notification, presque rien.

Le relais n'est **jamais** ouvert sur Internet : pas de redirection de port, pas de Funnel.

## La sécurité

- **La clé** est lue dans un fichier, jamais écrite dans le journal. Seule son empreinte
  (SHA-256) reste en mémoire, et la comparaison se fait sur les empreintes. Une clé trop
  courte (moins de 32 caractères) ou avec des espaces est refusée au démarrage.
- **Une mauvaise clé** donne `401` et une ligne d'avertissement dans le journal.
- **30 notifications par minute** au plus ; au-delà, `429` avec le délai à attendre.
- **Le texte est nettoyé** : caractères de contrôle remplacés, titre coupé à 200 caractères,
  texte à 1 000, nom de l'appli à 100. Une notification sans titre ni texte est refusée.
- La clé ne donne **aucun autre droit** : elle ne sert qu'à `/api/relais`, qui ne peut changer
  que `sensor.tondeuse` et publier l'évènement `notification_telephone`.

## Les limites

- L'état ne change que quand l'appli Mammotion envoie une notification. Si elle se tait (tonte
  normale sans alerte), la maison ne sait rien de plus. Le niveau de batterie exact, la
  position ou la carte ne sont pas dans les notifications.
- Les mots de l'appli peuvent changer avec ses mises à jour : le texte brut reste toujours
  dans `dernier_message`, et le tableau des mots se complète facilement.
- Il faut un téléphone **Android** : iOS ne laisse pas une appli lire les notifications des
  autres.
