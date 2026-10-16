# Étape 56 : jours ouvrés et jours fériés, expliqués

## Le but

Ouvrir les volets à 7 h **les jours de travail**, et laisser dormir le week-end et les jours
fériés. C'est l'intégration « Workday » de Home Assistant, pour la France : tout est calculé sur
ton serveur, sans réseau.

## Les entités

| Entité | État |
|--------|------|
| `binary_sensor.jour_ouvre` | `on` un jour de travail, `off` un week-end ou un jour férié (attribut `jour_ferie` : son nom) |
| `binary_sensor.demain_jour_ouvre` | pareil pour demain : pour décider la veille au soir |
| `sensor.prochain_jour_ferie` | la date du prochain jour férié ; attributs `nom`, `date` (« 01/11/2026 »), `dans_jours` |

Dans le tableau de bord : « Jour ouvré : Non (Ascension) », « Prochain jour férié : Toussaint,
dim. 1 nov., dans 16 jours ». Assist répond à « est-ce que demain est un jour ouvré ? ».

## Les jours fériés

Les onze jours fériés du Code du travail : 1er janvier, lundi de Pâques, 1er mai, 8 mai,
Ascension, lundi de Pentecôte, 14 juillet, 15 août, 1er novembre, 11 novembre, 25 décembre.
Pâques est calculée pour chaque année.

En **Alsace-Moselle** (Bas-Rhin, Haut-Rhin, Moselle), deux de plus : le **Vendredi saint** et la
**Saint-Étienne** (26 décembre).

Le lundi de Pentecôte compte comme férié, même si ton entreprise en fait la journée de
solidarité : dans ce cas, mets une condition de plus dans ton automatisation.

## Réglages

| Variable | Rôle |
|----------|------|
| `HA_JOURS_FERIES` | `france` ou `alsace-moselle`. Sans réglage : Alsace-Moselle si `HA_VIGILANCE_DEPARTEMENT` vaut 57, 67 ou 68, sinon France |
| `HA_JOURS_OUVRES` | les jours de travail, par défaut `lun,mar,mer,jeu,ven` (par exemple `lun,mar,mer,jeu,ven,sam`) |

## Exemples

Les volets de la chambre, seulement les jours ouvrés :

```yaml
- id: volets_matin
  alias: Volets du matin
  triggers:
    - trigger: time
      at: "07:00:00"
  conditions:
    - condition: state
      entity_id: binary_sensor.jour_ouvre
      state: "on"
  actions:
    - action: cover.open_cover
      target: {entity_id: cover.chambre}
```

La veille d'un jour de repos, un rappel :

```yaml
- id: demain_repos
  alias: Demain, repos
  triggers:
    - trigger: time
      at: "21:00:00"
  conditions:
    - condition: state
      entity_id: binary_sensor.demain_jour_ouvre
      state: "off"
  actions:
    - action: notify.send_message
      target: {entity_id: notify.appli}
      data:
        title: Demain, repos
        message: "Pas de réveil demain. {{ state_attr('binary_sensor.demain_jour_ouvre', 'jour_ferie') | default('') }}"
```

## Comment c'est construit

| Fichier | Rôle |
|---------|------|
| [ha-workday/src/lib.rs](../crates/ha-workday/src/lib.rs) (nouveau) | Pâques, les jours fériés, les jours ouvrés, et les trois entités, mises à jour chaque minute |
| [web/app.js](../crates/ha-api/web/app.js) | Les tuiles « Oui / Non (jour férié) » et « prochain jour férié » |
