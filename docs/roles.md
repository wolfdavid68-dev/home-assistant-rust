# Étape 23 : les comptes avec rôles, expliqué

## Le but

Jusqu'ici, tout compte pouvait tout faire : modifier les automatisations, lire l'historique de
présence de chacun, ouvrir le portail. C'est bien pour toi, moins pour un enfant, une baby-sitter ou
un ami de passage. Comme dans Home Assistant, chaque compte a maintenant un **rôle** :

| | Administrateur | Famille | Invité |
|---|:---:|:---:|:---:|
| Voir les appareils | tous | tous | sauf la présence (personnes, téléphones, zone Maison), l'alarme et les notifications |
| Commander les appareils | tous | tous | lumières, prises, ventilateurs, scènes, volets (**jamais** un portail, une porte de garage, une porte ou une serrure) |
| Historique et journal de bord | ✓ | ✓ | ✗ |
| Voir les automatisations | ✓ | ✓ (lecture seule) | ✗ |
| Modifier les automatisations | ✓ | ✗ | ✗ |
| Écrire un état à la main (API), journal d'activité | ✓ | ✗ | ✗ |

## Gérer les comptes

Tout se fait **sur la machine**, en ligne de commande (avec Docker :
`docker compose run --rm ha-server user …`) :

```powershell
cargo run -p ha-server -- user add alice                # « famille » par défaut
cargo run -p ha-server -- user add babysitter invité    # un invité
cargo run -p ha-server -- user add bob admin            # un deuxième administrateur
cargo run -p ha-server -- user role alice admin         # changer un rôle
cargo run -p ha-server -- user list                     # les comptes et leur rôle
```

- Le **tout premier** compte créé est administrateur ; les suivants sont « famille », sauf si tu
  précises le rôle. C'est le principe du **moindre privilège** : on ne donne que ce qui sert.
- Les comptes créés avant cette étape sont **administrateurs** : rien ne change pour toi.
- Un changement de rôle s'applique **tout de suite**, même à quelqu'un de connecté (l'API relit le
  rôle à chaque demande ; le direct du tableau de bord, en moins d'une minute).
- Il reste **toujours au moins un administrateur** : le dernier ne peut être ni rétrogradé ni
  supprimé.
- Pour retirer l'accès à un invité : `user delete babysitter`. Ses sessions sont fermées aussitôt.

## Dans le tableau de bord

En haut, à côté de « En direct », le nom et le rôle du compte : « alice · famille ».

- **Invité** : pas de bouton « Journal de bord », ni « Automatisations », ni historique sur les
  tuiles, ni journal d'activité. Les tuiles des personnes n'apparaissent pas. Les appareils qu'il
  ne peut pas commander (la porte du garage, le chauffage) restent lisibles mais **sans leurs
  boutons** ([étape 25](mise-en-forme.md#les-invités-ne-voient-plus-de-boutons-inutiles)). Si une
  commande est quand même refusée, un message bref le dit : « Ton compte (invité) ne permet pas de
  commander cet appareil. »
- **Famille** : tout, mais les automatisations sont en **lecture seule** (pas de « Nouvelle
  automatisation », pas de modification).
- **Administrateur** : tout, comme avant.

## La sécurité, c'est le serveur

Cacher un bouton ne protège rien : quelqu'un pourrait appeler l'API directement. **Chaque demande
est vérifiée par le serveur**, et le tableau de bord se contente de cacher ce qui ne servirait à
rien.

- **Tout ce qui n'est pas accordé est refusé.** Les droits sont une liste de ce qui est permis ; un
  droit ajouté plus tard sera refusé par défaut, jamais donné par erreur.
- **Le direct (WebSocket) est filtré aussi.** Un invité ne reçoit ni le départ de David, ni les
  commandes des autres, ni les événements des automatisations : rien qui révèle qui est là.
- Un appareil caché répond **« introuvable »** (404) plutôt que « interdit » : un invité n'apprend
  même pas qu'il existe.
- Un rôle inconnu dans la base (modifiée à la main) est refusé par la base elle-même (contrainte
  `CHECK`), et s'il passait, il donnerait les droits **les plus faibles**.
- Chaque refus de commande est écrit dans le journal du serveur (« cover.open_cover sur cover.garage
  refusé à babysitter (invité) »).

**Les limites à connaître** : un invité peut commander toutes les **prises** (`switch`). Si une prise
alimente quelque chose de sensible (une pompe, un portail commandé par un relais), ne donne pas de
compte à un invité, ou range cet appareil sous un autre type. Les règles d'un invité sont les mêmes
pour tous les invités ; des droits appareil par appareil viendront peut-être plus tard.

## Comment c'est construit

| Fichier | Contenu |
|---------|---------|
| [0012_roles.sql](../crates/ha-storage/migrations/0012_roles.sql) | La colonne `role` (administrateur par défaut pour les comptes existants) |
| [ha-auth/src/role.rs](../crates/ha-auth/src/role.rs) | `Role`, `Permission`, et qui a droit à quoi |
| [ha-auth/src/lib.rs](../crates/ha-auth/src/lib.rs) | Création avec un rôle, changement de rôle, garde du dernier administrateur |
| [ha-api/src/roles.rs](../crates/ha-api/src/roles.rs) | Ce qu'un invité voit et commande |
| [rest.rs](../crates/ha-api/src/rest.rs), [ws.rs](../crates/ha-api/src/ws.rs), [automations.rs](../crates/ha-api/src/automations.rs) | La vérification sur chaque route |

**Concepts Rust rencontrés :**

- **`matches!` sur un couple** : `matches!((role, permission), (Admin, _) | (Family, ReadHistory | …))`.
  Tout ce qui n'est pas dans le motif donne `false`.
- **Un `enum` `Copy`** : `Role` se passe par valeur, comme un nombre, sans `&` ni `.clone()`.
- **Une transaction pour une règle** : « est-ce le dernier administrateur ? » et le changement se
  font dans la même transaction SQLite, sans trou entre la vérification et l'écriture.
- **`Option::filter`** : `states.get(id).filter(|s| can_see(user, id, Some(s)))` : un appareil caché
  devient « absent », et la suite du code n'a pas à y penser.

## Testé

- Les droits de chaque rôle, un par un ; les noms de rôle (`famille`, `invité`, `admin`).
- Les comptes : rôle porté par la session, changement immédiat avec le même jeton, dernier
  administrateur protégé (rétrogradation et suppression), rôle inconnu refusé par la base, comptes
  d'avant cette étape administrateurs.
- L'API : un invité ne voit ni la personne ni l'alarme (même en les demandant directement : 404),
  commande la lumière et le volet du salon, mais pas la porte du garage, la serrure, l'alarme ni les
  notifications (403) ; n'a accès ni à l'historique, ni au journal de bord, ni aux automatisations,
  ni à l'audit. La famille voit tout et commande le garage, lit les automatisations sans pouvoir les
  modifier ni les supprimer, n'a ni l'audit ni l'écriture directe d'un état.
- Le direct : un invité ne reçoit ni le départ de David, ni la commande, seulement la lumière.
- **En vrai** : `user add` (famille par défaut), `user add … invité`, la rétrogradation du dernier
  administrateur refusée, `user list` ; puis dans Chromium avec les trois comptes : boutons et tuiles
  affichés selon le rôle, la porte du garage refusée à l'invité avec un message clair, le volet du
  salon accepté ; aucune erreur, pas de débordement sur téléphone.
