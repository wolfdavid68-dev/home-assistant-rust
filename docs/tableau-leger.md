# Étape 76 : le tableau de bord plus léger, expliqué

## Le but

Le tableau de bord s'ouvre souvent sur un téléphone en 4G ou sur une vieille tablette au mur. À
chaque ouverture, le navigateur rechargeait **590 Kio** de JavaScript, de CSS et de HTML, même
quand rien n'avait changé.

Deux économies, sans rien changer à l'écran :
- **la compression** : les fichiers partent compressés ;
- **l'empreinte** : un fichier déjà reçu n'est pas renvoyé.

Le gros fichier `app.js` (4 800 lignes) est aussi **découpé par thème**, pour s'y retrouver.

## Les mesures

Sur la maison de démonstration :

| | Avant | Maintenant |
| - | ----- | ---------- |
| Première ouverture (HTML, JavaScript, CSS) | 593 Kio | **162 Kio** (−73 %) |
| Ouvertures suivantes, serveur pas mis à jour | 593 Kio | **0** : seulement des réponses « pas changé » (`304`) |

Les polices et les icônes (déjà compressées) gardent leur règle : le navigateur les garde une
semaine.

## Comment ça marche

### La compression (gzip)

Le navigateur annonce ce qu'il sait lire : `Accept-Encoding: gzip, deflate, br`. Le serveur répond
alors avec la version compressée et le dit : `Content-Encoding: gzip`. Tous les navigateurs
savent la lire depuis plus de vingt ans.

- Un fichier est compressé **une seule fois**, à la première demande, puis gardé en mémoire
  (160 Kio en tout). Les demandes suivantes ne coûtent rien au processeur.
- Seuls le HTML, le JavaScript, le CSS, le SVG et le manifeste sont compressés. Les images PNG et
  les polices WOFF2 le sont déjà : les recompresser ne ferait rien gagner.
- `Vary: Accept-Encoding` prévient les caches (un relais, le navigateur) qu'il existe deux versions
  du même fichier.

### L'empreinte (`ETag`)

Chaque fichier a une empreinte : le début de son SHA-256, par exemple
`"9f40f79eaf237fc5f38bfe073503c1e7-gz"`. Elle change dès qu'une lettre du fichier change, donc à
chaque mise à jour du serveur.

1. Le navigateur reçoit le fichier et son empreinte.
2. À l'ouverture suivante, il demande : « j'ai la version `"9f40…"`, a-t-elle changé ? »
   (`If-None-Match`).
3. Si non, le serveur répond `304 Not Modified`, sans le fichier.

La règle `Cache-Control: no-cache` reste : le navigateur **demande toujours**, mais une réponse
`304` ne pèse que quelques centaines d'octets. Après une mise à jour du serveur, l'empreinte
change et le nouveau fichier arrive tout de suite.

## Le JavaScript, rangé par thème

Tous les fichiers sont des scripts classiques, chargés dans l'ordre par `index.html`. Ils partagent
leurs déclarations : une fonction de `tiles.js` peut appeler `api()` de `app.js`.

| Fichier | Ce qu'il contient |
| ------- | ----------------- |
| `utils.js` | Les petits outils : `el()` pour créer un élément, le nom et la sorte d'un appareil, « il y a 5 min », les icônes |
| `app.js` | Le cœur : l'apparence, la connexion, le temps réel, les commandes, les pièces, `api()` |
| `tiles.js` | L'accueil et les tuiles de chaque sorte d'appareil (lumière, volet, enceinte, agenda…) |
| `dialogs.js` | Les fenêtres : historique, journal, journal de bord, notifications, énergie |
| `editor.js` | Les automatisations : la liste, le formulaire, les versions, les exécutions |
| `account.js` | Mon compte, Assist, les notifications du navigateur, les intégrations, les piles, le système, le mode tablette |
| `start.js` | L'écran de veille, puis le démarrage de la page (en dernier) |
| `canvas.js` | Le schéma des automatisations (étape 35) |
| `house.js` | Le dessin de la maison et la mise en page « tableau » |
| `agenda.js` | L'agenda (étape 75) |

Les guides des étapes précédentes citent `app.js` : la fonction citée est maintenant dans le
fichier de son thème (par exemple `showSaver` dans `start.js`, `openEnergy` dans `dialogs.js`).

**Une règle à garder :** un fichier ne doit pas appeler, **pendant son chargement**, une fonction
d'un fichier chargé après lui. Il peut le faire dans une fonction appelée plus tard (un clic, un
message du serveur). C'est pour cela que le démarrage (`start.js`) passe en dernier.

## Sécurité

- **Rien ne change pour la page** : mêmes en-têtes de sécurité (CSP, `nosniff`, `DENY`), y compris
  sur les réponses `304`.
- **Toujours pas de `innerHTML` ni d'`eval`** : le test qui le vérifie lit maintenant chaque script
  chargé par `index.html`, et échoue si un nouveau fichier est oublié.
- **La compression ne touche que des fichiers fixes**, intégrés au programme. Les réponses de l'API,
  qui contiennent des données privées, ne sont pas compressées : pas de risque de type BREACH
  (deviner un secret à partir de la taille compressée).
- **Une seule dépendance en plus** : `flate2`, en Rust pur (`miniz_oxide`), sans bibliothèque C.

## Comment c'est construit

- **`crates/ha-api/src/web.rs`** :
  - un tableau `ASSETS` : chaque fichier, son type, sa règle de cache, son contenu
    (`include_bytes!`) ;
  - une route par fichier, avec une seule fonction, `serve` ;
  - `OnceLock<Prepared>` : l'empreinte et la version compressée, calculées à la première demande ;
  - `accepts_gzip` lit `Accept-Encoding` (`gzip;q=0` veut dire « non ») ; `still_fresh` lit
    `If-None-Match` (une liste, ou une empreinte « affaiblie » `W/"…"` par un relais).
- **`crates/ha-api/web/`** : `app.js` découpé en sept fichiers, `index.html` les charge dans
  l'ordre.
- **Tests** : la version compressée redonne exactement le fichier, les polices ne sont jamais
  recompressées, le `304` arrive avec la bonne empreinte et une ancienne empreinte fait renvoyer
  le fichier.

Essayé dans un vrai navigateur (Chromium) : connexion, accueil, chaque fenêtre, l'éditeur
d'automatisations et son schéma, une pièce, l'écran de veille, l'agenda et un rendez-vous, en
maquette, en clair et sur téléphone, sans erreur JavaScript. Tous les fichiers arrivent
compressés, puis en `304` au rechargement.

### Concepts utilisés

**Rust : `OnceLock`, un calcul fait une seule fois** : `get_or_init` lance le calcul à la première
demande, et les demandes suivantes reçoivent le résultat. Si deux demandes arrivent en même temps,
une seule calcule, l'autre attend. Il se met dans une `static`, comme le tableau des fichiers.

**Rust : une `macro_rules!` pour un tableau sans répétition** : `asset!("/app.js", "app.js", JS,
REVALIDATE)` écrit l'entrée complète, avec `include_bytes!(concat!("../web/", …))`. Le chemin du
fichier est assemblé pendant la compilation.

**JavaScript : les scripts classiques partagent leur portée** : contrairement aux modules
(`type="module"`), plusieurs `<script defer>` voient les mêmes `const` et `function`, dans l'ordre
où ils sont chargés. On évite une étape de construction (pas de bundler), et le code reste celui qui
est servi.
