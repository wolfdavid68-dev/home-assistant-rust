# Étape 50 : les notifications du navigateur, expliquées

![Mon compte → Notifications sur cet appareil : activées, avec le choix de ce que l'appareil reçoit](images/notifications-navigateur.png)

## Le but

Un ordre retenu par la [garde](garde.md) (« Ordre à valider ») n'était vu que si quelqu'un ouvrait
la cloche du tableau de bord. Désormais, **le téléphone sonne**, même page fermée, et un toucher
sur la notification ouvre la cloche, avec « Valider » et « Refuser ».

C'est le **Web Push** des navigateurs : rien à installer, pas de compte à créer. Ça complète
[ntfy](notifications.md) (étape 12), qui reste utile pour les messages des automatisations vers
un téléphone sans tableau de bord.

## Ce qui arrive sur le téléphone

| Catégorie | Quand | Exemple |
|-----------|-------|---------|
| **Les ordres à valider** | un ordre à la voix ou d'un invité attend (la garde) | « Ordre à valider : fermer Volet chambre » |
| **Les alertes du serveur** | une alerte grave apparaît dans la cloche | intégration en panne, automatisation en erreur, clé Météo-France refusée |
| **Les messages des automatisations** | une automatisation écrit à `notify.appli` | « Lessive : la machine a fini » |

Chaque appareil choisit ses catégories (le téléphone tout, la tablette du salon rien, par
exemple). Les simples avertissements (« lampe injoignable ») restent dans la cloche, sans faire
sonner le téléphone.

## Mettre en place

Rien à configurer sur le serveur : la clé est créée au premier démarrage.

1. Le tableau de bord doit être ouvert **en HTTPS** (voir l'[étape 7](etape-7-deploiement.md)) :
   sans connexion sécurisée, les navigateurs refusent les notifications.
2. **Sur iPhone** (iOS 16.4 ou plus récent) : dans Safari, touche Partager → « Sur l'écran
   d'accueil », puis ouvre **Maison** depuis l'écran d'accueil. Apple ne permet les notifications
   qu'aux sites ajoutés à l'écran d'accueil. Sur Android, Windows et Mac, rien à faire.
3. **Mon compte → Notifications sur cet appareil → Activer**, et accepte la demande du
   navigateur.
4. **Envoyer un essai** : la notification arrive en quelques secondes.

Tes autres appareils sont listés en dessous, avec la date de la dernière notification reçue ; un
téléphone perdu se retire d'un clic. Un appareil désinstallé est oublié tout seul : le service de
notifications répond qu'il n'existe plus.

### Depuis une automatisation

```yaml
- action: notify.send_message
  target:
    entity_id: notify.appli
  data:
    title: Lessive
    message: La machine a fini.
    users: [david]        # facultatif : sinon, toute la famille
    priority: 4           # 4 ou 5 : réveille le téléphone même en économie d'énergie
```

La vigilance météo orange ou rouge, par exemple (voir l'[étape 33](vigilance.md)) :

```yaml
- id: vigilance_appli
  alias: Vigilance orange ou rouge
  triggers:
    - trigger: state
      entity_id: sensor.vigilance_meteo
      to: [Orange, Rouge]
  actions:
    - action: notify.send_message
      target: {entity_id: notify.appli}
      data:
        title: "Vigilance {{ trigger.to_state.state | lower }}"
        message: >-
          En ce moment : {{ state_attr('sensor.vigilance_meteo', 'niveau_actuel') | lower }}.
        priority: 5
```

## Étape 51 : des boutons dans la notification

Un **ordre à valider** arrive avec deux boutons, **Valider** et **Refuser** : un toucher suffit,
sans ouvrir le tableau de bord ni se reconnecter. La notification est alors remplacée, sans
sonner, par la réponse de la maison (« Validé : fermer Volet chambre. »). Si quelqu'un d'autre a
déjà répondu, depuis son téléphone ou depuis la cloche : « Déjà traité, ou trop tard : rien n'a
été fait. »

Une automatisation peut aussi ajouter **deux boutons** à son message. Le bouton touché déclenche
l'événement `notification_action`, avec son identifiant et le compte de la personne :

```yaml
- id: lessive_finie
  alias: Lessive finie
  triggers:
    - trigger: state
      entity_id: input_boolean.lessive
      to: "on"
  actions:
    - action: notify.send_message
      target: {entity_id: notify.appli}
      data:
        title: Lessive
        message: La machine a fini.
        actions:
          - action: LESSIVE_VUE        # lettres, chiffres, « _ » et « - »
            title: Je m'en occupe      # 24 caractères au plus
          - action: LESSIVE_PLUS_TARD
            title: Rappelle-moi

- id: lessive_vue
  alias: Quelqu'un s'occupe de la lessive
  triggers:
    - trigger: event
      event_type: notification_action
      event_data:
        action: LESSIVE_VUE
  actions:
    - action: input_boolean.turn_off
      target: {entity_id: input_boolean.lessive}
```

`trigger.event.data.user` donne qui a touché le bouton.

Sur **iPhone**, Safari n'affiche pas les boutons des notifications : toucher la notification
ouvre la cloche, comme avant. Sur Android, Windows et Mac, ils apparaissent sous le message
(parfois après l'avoir déplié).

### Ce qui protège les boutons

Le service worker n'a pas le jeton de connexion de la page. Chaque notification avec des boutons
emporte donc un **ticket** :

- tiré au hasard (256 bits), **à usage unique**, pour **ce** message, **cet** appareil et **ce**
  compte ; il ne permet que les boutons de sa notification (valider **cet** ordre, pas un autre) ;
- valable le temps que la notification reste utile : 10 minutes pour un ordre à valider (comme
  l'ordre lui-même), 24 heures pour un message d'automatisation ;
- il voyage dans la notification **chiffrée de bout en bout** : Google ou Apple ne le voient pas ;
- le serveur n'en garde que l'**empreinte**, en mémoire (un redémarrage les oublie tous) ;
- au toucher, le serveur revérifie que l'appareil est **toujours abonné** à cette catégorie et que
  le compte est **toujours famille ou administrateur** : retirer un téléphone perdu annule ses
  boutons, comme passer un compte en invité ;
- un ordre traité annule les boutons envoyés aux autres appareils ; le journal d'audit note qui a
  validé, comme depuis la cloche.

À savoir : sur Android, les boutons peuvent être touchés **depuis l'écran verrouillé**, si le
téléphone affiche le contenu des notifications sur cet écran. Quelqu'un qui a ton téléphone en
main peut donc valider un ordre retenu sans le déverrouiller. Pour l'éviter : réglages
d'Android → Notifications → « Notifications sensibles » ou « Sur l'écran de verrouillage » →
masquer le contenu. Les ordres que la garde retient sont ceux d'un invité ou de la voix
(lumières, prises, volets) : jamais une serrure, l'alarme, un portail ni la porte du garage.

## Étape 52 : les notifications se rangent toutes seules

Une notification qui ne sert plus est **remplacée sur tous les téléphones**, sans les faire
sonner :

| Avant | Après | Quand |
|-------|-------|-------|
| Ordre à valider | **Ordre validé** : « alice a validé : fermer Volet chambre. » | quelqu'un valide, depuis son téléphone ou la cloche |
| Ordre à valider | **Ordre refusé** : « alice a refusé : … » | quelqu'un refuse |
| Ordre à valider | **Ordre oublié** : « Personne n'a répondu à temps : rien n'a été fait (…). » | 10 minutes sans réponse |
| MQTT en panne | **Résolu** : « MQTT en panne : c'est réglé. » | le problème disparaît tout seul |

Plus de bouton « Valider » sur un ordre déjà traité par quelqu'un d'autre, et plus d'alerte en
panne sur l'écran verrouillé une heure après que tout est rentré dans l'ordre. Une alerte que tu
as fermée dans la cloche ne donne pas de « Résolu ».

Les réponses sont aussi des événements que les automatisations peuvent écouter :
`guard.approval_answered` (`id`, `entity_id`, `approved`, `by`) et `guard.approval_expired`.
Seul le serveur les publie : ni l'API, ni MQTT, ni une automatisation ne peuvent en fabriquer un
faux.

### Réglages (facultatifs)

| Variable | Rôle |
|----------|------|
| `HA_PUSH_CONTACT` | ton contact pour les services de notifications (`mailto:toi@exemple.fr`), qui l'utilisent en cas de problème |
| `HA_PUSH=off` | coupe les notifications du navigateur |

## Comment ça marche

1. Le navigateur s'abonne auprès du **service de notifications de son éditeur** (Google pour
   Chrome et Android, Mozilla pour Firefox, Apple pour Safari, Microsoft pour Edge), avec la
   **clé publique** du serveur. Il donne au serveur une adresse chez ce service, et deux clés.
2. Pour notifier, le serveur **chiffre** le message avec les clés du navigateur
   ([RFC 8291](https://www.rfc-editor.org/rfc/rfc8291)), le **signe** avec sa clé privée
   ([VAPID, RFC 8292](https://www.rfc-editor.org/rfc/rfc8292)), et l'envoie à cette adresse.
3. Le service le remet au téléphone ; le **service worker** (`sw.js`) le déchiffre et l'affiche.

## Sécurité

- **Le contenu est chiffré de bout en bout** : Google, Apple ou Mozilla ne voient qu'un bloc
  illisible, et la taille. Seul le navigateur abonné peut le lire.
- **Personne d'autre ne peut écrire à tes appareils** : le service refuse tout envoi qui n'est
  pas signé par la clé privée du serveur. Cette clé est **chiffrée dans la base**, comme les
  jetons Hue et Somfy ; le secret `auth` de chaque appareil aussi.
- **Le serveur n'appelle que les services de notifications** : une adresse d'abonnement doit être
  en HTTPS, sur `fcm.googleapis.com`, `push.services.mozilla.com`, `push.apple.com` ou
  `notify.windows.com` (ou un de leurs sous-domaines), sur le port standard. Une page piégée ne
  peut donc pas faire appeler par ton serveur une adresse de ton réseau (`https://192.168.1.1/…`).
  L'adresse est revérifiée à chaque envoi, et les redirections ne sont jamais suivies.
- **Famille et administrateur seulement** : un invité ne peut pas s'abonner, et un compte devenu
  invité ne reçoit plus rien. Chacun ne voit et ne retire que ses propres appareils.
- **Des limites** : 10 appareils par compte, 50 pour la maison ; 10 messages d'automatisation en
  10 minutes (une automatisation qui boucle ne fait pas vibrer le téléphone sans fin), 60
  notifications en tout. Un message fait 600 caractères au plus.
- **Toucher une notification n'ouvre que le tableau de bord** : le service worker refuse toute
  autre adresse. Il ne met rien en cache et ne voit aucune donnée de la maison.
- **Pas de contenu dans les logs** : le serveur note qu'une notification est partie, pas ce
  qu'elle dit. Abonnements et désabonnements vont au journal d'audit.
- Écris tout de même des messages sans détail sensible : le téléphone les affiche sur l'écran
  verrouillé.

## Comment c'est construit

| Fichier | Rôle |
|---------|------|
| [ha-push/src/crypto.rs](../crates/ha-push/src/crypto.rs) (nouveau) | Chiffrement RFC 8291 et jeton VAPID, avec `ring` (déjà là pour le HTTPS) ; testé avec l'exemple de la RFC |
| [ha-push/src/lib.rs](../crates/ha-push/src/lib.rs) (nouveau) | Vérification des abonnements, quels événements notifient, envoi, limites |
| [ha-storage/src/push.rs](../crates/ha-storage/src/push.rs) (nouveau) | La clé du serveur et les abonnements (migration `0018_push.sql`) |
| [ha-api/src/push.rs](../crates/ha-api/src/push.rs) (nouveau) | `GET /api/push`, `POST /api/push/subscriptions`, `PATCH` et `DELETE /api/push/subscriptions/{id}`, `POST /api/push/test` |
| [web/sw.js](../crates/ha-api/web/sw.js) (nouveau) | Le service worker : affiche la notification et ses boutons, ouvre la cloche au toucher, remplace sans sonner (étape 52) |
| [ha-core/src/guard.rs](../crates/ha-core/src/guard.rs) (étape 52) | Annonce les ordres validés, refusés et oubliés |
| [ha-push/src/tickets.rs](../crates/ha-push/src/tickets.rs) (étape 51) | Les tickets des boutons : émis, servis une fois, oubliés |
| [ha-api/src/push.rs](../crates/ha-api/src/push.rs) (étape 51) | `POST /api/push/action` : le bouton touché, revérifié, puis la même validation que la cloche |
| [web/app.js](../crates/ha-api/web/app.js) | La section « Notifications sur cet appareil » de Mon compte |
| `web/icon-192.png`, `icon-512.png`, `apple-touch-icon.png` | Les icônes de l'appli installée (Android et iPhone ne prennent pas le SVG) |

La politique de sécurité du contenu (CSP) autorise désormais `worker-src 'self'` : le service
worker vient du serveur, et de lui seul.
