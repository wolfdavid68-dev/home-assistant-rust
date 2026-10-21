# Étape 74 : l'agenda TimeTree de la famille, expliqué

## Le but

Le planning de la famille est dans **TimeTree**. La maison le lit, en **lecture seule** :
- il s'affiche dans le tableau de bord, à côté de l'agenda de la maison ;
- les automatisations s'en servent : « 30 minutes avant un rendez-vous, l'annoncer sur les
  enceintes », « pendant “Vacances”, armer la simulation de présence ».

La maison ne modifie jamais rien dans TimeTree.

## Pourquoi un compte dédié

TimeTree ne propose ni lien d'abonnement (ICS) ni accès officiel pour les autres logiciels. Le serveur
se connecte donc **comme le site web de TimeTree**, avec une adresse e-mail et un mot de passe, et il
doit garder ce mot de passe pour se reconnecter quand la session expire.

D'où la règle : **un compte TimeTree pour la maison**, jamais ton compte personnel.
- Il ne voit que les agendas où tu l'invites, en lecture.
- Si le PC était volé, seul ce compte serait en jeu, pas le tien : il suffit de le retirer de l'agenda
  partagé, depuis ton téléphone.
- Le mot de passe est **chiffré** dans la base, avec la clé `ha.key`, comme les jetons Hue et Somfy.

## Mettre en place

1. **Crée le compte de la maison** dans l'appli TimeTree (une adresse e-mail à part, par exemple
   `maison.martin@…`), avec un mot de passe que tu n'utilises nulle part ailleurs.
2. **Invite-le** dans l'agenda de la famille : dans TimeTree, l'agenda → Membres → Inviter.
   Accepte l'invitation avec le compte de la maison.
3. **Associe-le** au serveur :
   ```bat
   ha-server.exe timetree pair
   ```
   ```text
   e-mail du compte TimeTree de la maison : maison@exemple.fr
   mot de passe :
   associé !
   agendas visibles par ce compte :
     - Famille Martin
   ```
4. **Ajoute l'agenda** dans `calendriers.yaml`, avec son nom tel qu'il apparaît dans TimeTree (les
   majuscules ne comptent pas) :
   ```yaml
   - nom: Famille
     timetree: Famille Martin
   ```
   Il devient `calendar.famille`. Vérifie avec `ha-server calendriers check`, puis redémarre le serveur.

Les autres commandes :
- `ha-server timetree status` : le compte associé, et les agendas qu'il voit ;
- `ha-server timetree forget` : oublie le compte.

## Dans les automatisations

Le déclencheur `calendar` marche comme avec l'agenda de la maison. En plus, le rendez-vous en cause
est donné aux modèles : `trigger.calendar_event.summary` (le titre), `.description`, `.start`, `.end`
et `.all_day`, comme dans Home Assistant.

Annoncer chaque rendez-vous 30 minutes avant, sur les enceintes (étape 73) :

```yaml
- id: rendez_vous_annonce
  alias: Rendez-vous dans 30 minutes
  triggers:
    - trigger: calendar
      event: start
      entity_id: calendar.famille
      offset: "-00:30:00"
  conditions:
    - condition: template
      value_template: "{{ not trigger.calendar_event.all_day }}"
  actions:
    - action: tts.speak
      target: { entity_id: tts.annonces }
      data:
        media_player_entity_id: media_player.salon
        message: "Dans 30 minutes : {{ trigger.calendar_event.summary }}."
```

La veille des vacances, armer la simulation de présence : un rendez-vous « Vacances » dans TimeTree,
puis un déclencheur `calendar` avec `offset: "-12:00:00"` et la condition
`{{ 'vacances' in trigger.calendar_event.summary | lower }}`.

## Ce que l'agenda montre

- Les rendez-vous de la semaine passée et des deux mois à venir, répétitions comprises (« Piscine tous
  les mercredis », sauf les dates annulées), avec leurs fuseaux horaires.
- Pas les **anniversaires** que TimeTree ajoute tout seul d'après les profils, ni les **mémos** (des
  notes sans date), ni les rendez-vous supprimés.
- Il est relu toutes les **15 minutes**. Après un mot de passe refusé ou trop de connexions, le serveur
  attend une heure avant de réessayer, pour ne pas faire bloquer le compte.

Si TimeTree change son site, l'agenda passe « indisponible » avec la raison (« réponse inattendue de
TimeTree »), et le reste de la maison continue. Envoie-moi alors ce message.

## Sécurité

- Un compte **dédié à la maison**, invité en lecture : jamais ton compte personnel.
- Le mot de passe est chiffré dans la base (XChaCha20-Poly1305, clé `ha.key`). Il n'apparaît jamais dans
  les journaux, ni dans les messages d'erreur, ni dans l'état de l'agenda, ni dans `Debug`.
- La connexion passe en HTTPS vers `timetreeapp.com` seulement, avec le certificat vérifié et aucune
  redirection suivie. Réponses limitées à 2 Mio, 5 000 rendez-vous au plus.
- Lecture seule : le serveur ne fait que lire la liste des agendas et leurs rendez-vous.
- `ha-server timetree pair` et `forget` sont notés dans le journal d'audit.

## Comment c'est construit

- **`crates/ha-ics/src/timetree.rs`** (nouveau) :
  - `Client` se connecte (`/auth/email/signin`, cookie `_session_id`), garde la session et la refait
    une fois si elle a expiré ;
  - `calendars`, `find` et `events` lisent les agendas et leurs rendez-vous, par morceaux ;
  - `to_ics` en fait un petit fichier ICS.
- **`crates/ha-ics`** : une source `timetree:` à côté de `url:`. Le fichier ICS produit est lu par le
  même code que les agendas en ligne de l'étape 57 : répétitions, dates annulées, fuseaux horaires.
- **`crates/ha-storage`** : la table `timetree_account`, avec le mot de passe chiffré.
- **`crates/ha-automation`** : `trigger.calendar_event.…` dans les modèles.
- **`crates/ha-server`** : `ha-server timetree pair|status|forget`.

Essayé avec un faux site TimeTree, qui répond comme le vrai (d'après l'outil libre TimeTree-Exporter) :
- connexion, puis session expirée refaite toute seule ;
- rendez-vous lus en deux morceaux ;
- « Piscine » chaque semaine, dentiste, vacances sur plusieurs jours, sans anniversaire ni mémo ;
- mot de passe refusé : l'agenda le dit ;
- compte non associé : l'agenda dit quelle commande lancer.

Pas encore essayé sur ton vrai compte TimeTree : si `ha-server timetree pair` échoue, envoie-moi le
message.

### Concepts Rust utilisés

**Réutiliser plutôt que réécrire** : les répétitions d'agenda (`RRULE`) sont délicates (fuseaux,
heure d'été, dates annulées). TimeTree les donne déjà au format ICS : on fabrique un petit fichier
ICS et on le passe à `ics::parse`, déjà testé à l'étape 57.

**`Mutex<Option<Zeroizing<String>>>`** : la session est partagée entre les lectures (`Mutex`), peut
manquer (`Option`), et son texte est effacé de la mémoire quand on la jette (`Zeroizing`).

**`#[derive(PartialEq)]` sur les erreurs** : les tests comparent l'erreur entière
(`TimeTreeError::NotFound("Vacances", "Travail, Famille Martin")`) au lieu de chercher un mot dans un
texte.
