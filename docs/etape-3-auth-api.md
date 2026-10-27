# Étape 3 : comptes, authentification et API, expliqués

## Ce que fait cette étape

```
                 POST /api/auth/login {"username", "password"}
  client  ─────────────────────────────────────────────►  ha-api ──► ha-auth
  (curl,                                                              │ Argon2id
  script,  ◄──── {"access_token": "hat_…"} ──────────────────────────┘ + jeton aléatoire
  appli…)
           ── GET /api/states   +  Authorization: Bearer hat_… ──►  ha-api
                                                                      │ vérifie le jeton
           ◄──────────────── [{"entity_id": "light.salon", …}] ───────┘ puis lit le noyau
```

1. Un compte est créé **en ligne de commande, sur la machine elle-même** :
   `ha-server user add david`. Il n'existe aucune route d'API pour créer un compte : quelqu'un qui
   joint le serveur par le réseau ne peut pas se fabriquer un accès.
2. Le client se connecte avec son nom et son mot de passe et reçoit un **jeton d'accès**
   (`hat_` suivi de 64 caractères), valable 30 jours.
3. Chaque requête suivante présente ce jeton dans l'en-tête `Authorization: Bearer hat_…`.
4. L'API WebSocket permet de recevoir les changements d'état **en direct**, comme celle de
   Home Assistant.

## Les nouvelles crates

| Crate / fichier | Contenu |
|-----------------|---------|
| [ha-auth/password.rs](../crates/ha-auth/src/password.rs) | Règles et hachage Argon2id des mots de passe |
| [ha-auth/token.rs](../crates/ha-auth/src/token.rs) | Création des jetons et leur empreinte SHA-256 |
| [ha-auth/limiter.rs](../crates/ha-auth/src/limiter.rs) | Blocage après trop d'échecs de connexion |
| [ha-auth/lib.rs](../crates/ha-auth/src/lib.rs) | `Auth` : comptes, connexion, vérification des jetons |
| [ha-api/rest.rs](../crates/ha-api/src/rest.rs) | Les routes REST |
| [ha-api/ws.rs](../crates/ha-api/src/ws.rs) | L'API WebSocket |
| [ha-api/auth.rs](../crates/ha-api/src/auth.rs) | `Authenticated` : la vérification du jeton sur chaque route |
| [ha-api/json.rs](../crates/ha-api/src/json.rs) | Conversion des états en JSON, format des dates |
| [ha-server/cli/users.rs](../crates/ha-server/src/cli/users.rs) | Commandes `ha-server user …` |
| [0002_users_and_tokens.sql](../crates/ha-storage/migrations/0002_users_and_tokens.sql) | Tables `users` et `access_tokens` |

## Les routes

| Méthode | Chemin | Rôle |
|---------|--------|------|
| POST | `/api/auth/login` | Connexion, renvoie un jeton. **Seule route sans jeton.** |
| POST | `/api/auth/logout` | Révoque le jeton utilisé |
| GET | `/api/` | Vérifie que l'API répond et que le jeton est bon |
| GET | `/api/states` | Tous les états |
| GET | `/api/states/{entity_id}` | Un état |
| POST | `/api/states/{entity_id}` | Crée ou modifie un état : `{"state": "on", "attributes": {…}}` |
| DELETE | `/api/states/{entity_id}` | Supprime une entité |
| GET | `/api/history/{entity_id}?limit=100` | Historique, du plus récent au plus ancien |
| GET | `/api/websocket` | API WebSocket |

Le WebSocket suit le même déroulé que Home Assistant : le serveur envoie `auth_required`, le
client répond `{"type": "auth", "access_token": "hat_…"}`, puis envoie des commandes
`{"id": 1, "type": "subscribe_events"}`, `get_states` ou `ping`.

## Les concepts Rust rencontrés

**Les extracteurs axum.** Un gestionnaire de route est une simple fonction `async`. Ses
paramètres disent ce dont elle a besoin : `State(...)` pour l'état partagé, `Path(...)` pour un
morceau de l'adresse, `Authenticated` pour un utilisateur connecté. axum remplit ces paramètres
**avant** d'appeler la fonction. Comme `Authenticated` échoue sans jeton valide, une route qui le
demande est protégée par construction : impossible d'oublier la vérification à l'intérieur.

**Implémenter un trait pour son propre type.** `Authenticated` implémente `FromRequestParts` (le
trait des extracteurs) et `ApiError` implémente `IntoResponse` (ce qui peut devenir une réponse
HTTP). C'est comme ça qu'on branche son code dans une bibliothèque.

**`impl From<AuthError> for ApiError`.** Grâce à cette conversion, `?` transforme une erreur
d'authentification en erreur d'API. Les erreurs internes deviennent un simple « erreur
interne » pour le client, et le détail part dans les logs.

**`spawn_blocking`.** Argon2 occupe le processeur pendant des dizaines de millisecondes. Les
tâches `async` partagent quelques fils d'exécution : un calcul long les bloquerait toutes.
`spawn_blocking` l'envoie sur un fil séparé prévu pour ça.

**`Semaphore`.** Un sémaphore distribue un nombre limité de « jetons de passage ». Ici, au plus
2 calculs Argon2 en même temps : une rafale de connexions ne peut pas saturer la mémoire.

**`Zeroizing<String>` et `Debug` maîtrisé.** Les mots de passe et jetons sont dans des
`Zeroizing<String>`, effacées de la mémoire quand on n'en a plus besoin. Les structures qui
contiennent un secret n'ont pas de `#[derive(Debug)]`, ou un `Debug` écrit à la main qui affiche
`<masqué>` : un `{:?}` oublié dans un log ne peut pas les révéler.

**Un signal d'arrêt partagé.** `ApiState` garde un `CancellationToken`, copié dans chaque
connexion WebSocket. À l'arrêt, on l'annule : le serveur HTTP cesse d'accepter des connexions et
chaque WebSocket ouvert se ferme proprement.

**Filtrage par motif sur des tranches.** Dans `main`, `match args.as_slice()` avec des motifs
comme `["user", rest @ ..]` analyse la ligne de commande sans bibliothèque supplémentaire.

## Choix de sécurité de cette étape

- **Mots de passe** : hachés avec Argon2id (paramètres recommandés par l'OWASP), avec un sel
  aléatoire. Minimum 12 caractères : une phrase de passe est idéale. La saisie ne s'affiche pas
  à l'écran.
- **Jetons** : 256 bits du générateur aléatoire du système. Seule leur empreinte SHA-256 est en
  base : une copie volée de `ha.db` ne permet pas de se connecter. Validité de 30 jours ;
  révoqués à la déconnexion, au changement de mot de passe et à la suppression du compte.
- **Comparaison des jetons** : on ne compare pas le jeton reçu à un jeton stocké, on cherche son
  empreinte dans la base. Le temps de réponse ne dépend que de l'empreinte, qui ne révèle rien du
  jeton : aucune attaque par mesure du temps n'est possible.
- **Comptes impossibles à deviner** : un nom inconnu et un mauvais mot de passe donnent
  **exactement** la même erreur, et le serveur fait le même calcul Argon2 dans les deux cas pour
  que le temps de réponse ne trahisse rien.
- **Force brute** : 5 échecs en 15 minutes depuis une même adresse IP bloquent cette adresse
  15 minutes (même avec le bon mot de passe).
- **Réseau** : par défaut, l'API écoute seulement sur `127.0.0.1` (la machine elle-même). Un
  avertissement s'affiche si `HA_BIND` l'ouvre au réseau. Jamais d'exposition directe sur
  Internet : uniquement via VPN (étape 7).
- **Entrées** : corps limité à 64 Kio, requêtes limitées à 10 secondes, JSON strict
  (`deny_unknown_fields`), identifiants et états revalidés par le noyau. Messages WebSocket
  limités à 64 Kio, authentification exigée dans les 10 secondes.
- **WebSocket** : le jeton passe dans le premier message, jamais dans l'adresse (les adresses
  finissent souvent dans des logs). Le jeton est revérifié chaque minute : un jeton révoqué
  ferme la connexion.
- **Logs** : ni mot de passe ni jeton n'y apparaissent. Les connexions réussies et les échecs
  sont notés, avec l'adresse IP.

## Essayer

Créer un compte (le mot de passe est demandé deux fois, sans s'afficher) :

```bash
cargo run -p ha-server -- user add david
```

Démarrer le serveur :

```bash
cargo run -p ha-server
```

Dans un autre terminal PowerShell :

```powershell
# Connexion : on récupère un jeton
$r = Invoke-RestMethod -Method Post -Uri http://127.0.0.1:8123/api/auth/login `
     -Body '{"username":"david","password":"ta phrase de passe"}'
$h = @{ Authorization = "Bearer $($r.access_token)" }

# Lire les états
Invoke-RestMethod -Uri http://127.0.0.1:8123/api/states -Headers $h

# Allumer une lampe
Invoke-RestMethod -Method Post -Uri http://127.0.0.1:8123/api/states/light.cuisine -Headers $h `
     -Body '{"state":"on"}'

# Historique
Invoke-RestMethod -Uri "http://127.0.0.1:8123/api/history/light.salon?limit=5" -Headers $h

# Déconnexion (le jeton ne marche plus ensuite)
Invoke-RestMethod -Method Post -Uri http://127.0.0.1:8123/api/auth/logout -Headers $h
```

Gérer les comptes :

```bash
cargo run -p ha-server -- user list
cargo run -p ha-server -- user passwd david
cargo run -p ha-server -- user delete david
```

```bash
cargo test
```
