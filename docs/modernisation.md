# Modernisation du code, expliquée

Cette mise à jour n'ajoute **aucune fonctionnalité** : le serveur fait exactement la même chose
qu'avant. Elle rend le code plus solide, plus lisible et mieux vérifié, avec les outils et les
habitudes les plus récents de Rust.

## 1. Des règles de qualité plus strictes (Clippy « pedantic »)

Clippy, l'outil qui relit le code, a plusieurs niveaux d'exigence. On est passé au niveau
**pedantic** pour tout le projet (dans [Cargo.toml](../Cargo.toml), section
`[workspace.lints.clippy]`). Il a trouvé environ 140 points, presque tous corrigés
automatiquement (`cargo clippy --fix`), puis relus à la main :

| Correction | Combien | Pourquoi |
|---|---|---|
| `#[must_use]` ajouté | 34 | Si on appelle `Auth::with_token_lifetime(…)` sans utiliser le résultat, c'est sûrement un oubli : le compilateur le signale maintenant. |
| `Duration::from_mins(15)` au lieu de `from_secs(15 * 60)` | 18 | Plus lisible, et plus d'erreur de calcul possible. |
| Paramètres passés par référence (`&Action`) | 4 | Évite une copie inutile quand la fonction ne fait que lire. |
| Petites simplifications | une dizaine | `match` à une seule branche → `if let`, branches identiques fusionnées… |

Trois règles restent désactivées, avec la raison écrite à côté : elles se trompent sur les
textes en français ou demandent de la documentation sans valeur ajoutée ici.

Une correction automatique a été **refusée** après relecture : Clippy voulait marquer
`EventBus::publish` comme « résultat à utiliser obligatoirement ». Or ignorer le nombre
d'abonnés est parfaitement normal. On l'a donc dit explicitement (voir le point suivant).

## 2. Un seul outil pour arrêter les tâches de fond

Avant, l'enregistreur, MQTT, Hue et les automatisations contenaient chacun **la même dizaine de
lignes** pour s'arrêter proprement (un canal `oneshot` et une `JoinHandle`). L'API, elle,
utilisait un autre mécanisme (`watch`).

Maintenant, tout le monde utilise [`Background`](../crates/ha-core/src/task.rs), dans le noyau,
construit sur l'outil standard de l'écosystème tokio : **`CancellationToken`**.

```rust
pub struct Recorder(Background);

impl Recorder {
    pub fn start(store: Store, core: Arc<Core>) -> Self {
        let events = core.bus.subscribe();
        Self(Background::spawn("l'enregistreur", |token| run(store, core, events, token)))
    }
}

// Dans la boucle de la tâche :
tokio::select! {
    received = events.recv() => { /* … */ }
    () = token.cancelled() => break,   // arrêt demandé
}
```

**Concept Rust : le « drop guard ».** `Background` ne garde pas le jeton lui-même, mais un
`DropGuard` qui annule le jeton **quand il est détruit**. Si une poignée est oubliée (ou si une
erreur fait sortir de la fonction plus tôt), Rust la détruit automatiquement… et la tâche
s'arrête quand même. Impossible de laisser une tâche orpheline tourner pour rien. Deux tests le
vérifient.

**Concept Rust : un « newtype » qui enveloppe un autre type.** `pub struct Recorder(Background);`
réutilise tout le comportement de `Background` tout en gardant un nom clair et sa propre
méthode `start`.

## 3. `#[expect]` avec une raison, au lieu de `#[allow]`

Quand on désactive une règle à un endroit précis, on utilise maintenant la forme moderne
(Rust 1.81+) :

```rust
#[expect(
    clippy::cast_precision_loss,
    reason = "valeur entre 1 et 255 : exacte en flottant"
)]
```

Deux avantages :

- la **raison** est écrite à côté, pour la personne qui relira (toi, plus tard) ;
- `expect` signale une erreur si la règle n'est plus déclenchée, par exemple après une
  modification. Une exception devenue inutile ne peut donc pas traîner indéfiniment, ce qui
  arrivait avec `allow`.

## 4. Une seule règle pour les noms simples

La vérification « 1 à 64 caractères parmi `a-z`, `0-9` et `_` » (noms de domaine, de service,
d'automatisation) était copiée à plusieurs endroits. Elle est maintenant dans une seule fonction,
`ha_core::is_valid_name`. Si la règle change un jour, elle change partout d'un coup.

## 5. La CI teste aussi sous Windows, et la version minimale de Rust

- **Windows** : jusqu'ici, GitHub ne testait que sous Linux, alors que tu développes sous
  Windows. Les tests tournent maintenant sur les deux systèmes, à chaque PR. Une différence
  (chemins, droits de fichiers, compilation d'une bibliothèque en C comme SQLite ou `ring`) sera
  vue tout de suite.
- **Version minimale** : `Cargo.toml` annonce `rust-version = "1.94"`. Une nouvelle tâche vérifie
  que le projet compile vraiment avec Rust 1.94 et pas seulement avec la dernière version.
  Sinon, la promesse ne veut rien dire.

## Ce qui était déjà moderne

- **Édition Rust 2024** (la plus récente), résolveur de dépendances v3.
- Toutes les dépendances sont à leur **dernière version** (vérifié avec `cargo update` : rien à
  mettre à jour).
- Syntaxes récentes déjà utilisées : `let … else`, `if let … && let …` (« let chains »),
  `Option::is_none_or`, `async fn` dans les traits (extracteurs axum).

## Vérifié

Format, Clippy niveau pedantic sans aucun avertissement, 103 tests, `cargo deny`, et compilation
avec Rust 1.94.
