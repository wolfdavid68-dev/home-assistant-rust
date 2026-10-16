# Étape 58 : la simulation de présence, expliquée

## Le but

Pendant les vacances, une maison où aucune lumière ne s'allume le soir se repère vite. La
simulation **rejoue la semaine passée** : quand la maison est vide, les lumières (et, si tu veux,
les volets) refont ce qu'elles ont fait **il y a 7 jours, à la même heure**. Les horaires sont
vraisemblables, puisque ce sont les tiens : la cuisine vers 19 h, le salon jusqu'à 23 h, la
chambre un peu après.

## Mettre en place

Crée `simulation.yaml` (ou indique son chemin avec `HA_SIMULATION_PATH`) :

```yaml
appareils:
  - light.salon
  - light.cuisine
  - light.chambre
  - cover.salon       # facultatif : les volets aussi
il_y_a: 7             # de 1 à 9 jours (l'historique est gardé 10 jours)
```

Seulement des lumières, des prises, des ventilateurs, des interrupteurs virtuels et des volets :
jamais une serrure, l'alarme ou le chauffage.

```powershell
ha-server simulation check
```

## L'utiliser

Un interrupteur apparaît : **Simulation de présence** (`switch.simulation_presence`).

- **Armée**, elle rejoue seulement **pendant que personne n'est à la maison** (`zone.home` vaut 0,
  voir les [personnes](personnes.md)). Au retour du premier, elle s'arrête et ne touche plus à
  rien.
- Elle reste armée après un redémarrage du serveur.
- Elle n'agit que quand le passé change : si tu éteins une lampe à distance pendant qu'elle
  rejoue, elle ne la rallume pas avant le prochain changement d'il y a une semaine.
- Le journal de bord dit « allumée par l'automatisation « Simulation de présence » ».

Sans personnes configurées (pas de `zone.home`), la maison est considérée vide : armée, la
simulation rejoue tout le temps.

L'attribut `en_cours` dit si elle rejoue en ce moment.

### L'armer toute seule

Avec l'agenda en ligne des vacances ([étape 57](agendas-en-ligne.md)), ou quand le dernier part :

```yaml
- id: simulation_depart
  alias: Simulation armée quand tout le monde part
  triggers:
    - trigger: state
      entity_id: zone.home
      to: "0"
      for: "00:30:00"
  actions:
    - action: switch.turn_on
      target: {entity_id: switch.simulation_presence}
```

## À savoir

- La simulation rejoue ce qui est **dans l'historique** : il faut que la maison ait été habitée
  normalement il y a 7 jours. Au retour de deux semaines de vacances, les premiers jours
  rejoueraient une maison vide : c'est sans conséquence, puisqu'elle ne rejoue que quand tu es
  absent.
- Un compte « invité » peut désarmer la simulation (comme toute prise). Elle ne sert que maison
  vide : c'est sans risque.

## Comment c'est construit

| Fichier | Rôle |
|---------|------|
| [ha-simulation/src/lib.rs](../crates/ha-simulation/src/lib.rs) (nouveau) | La configuration, l'interrupteur, le rejeu |
| [ha-storage/src/store.rs](../crates/ha-storage/src/store.rs) | `state_at` : l'état d'un appareil à un instant passé |
