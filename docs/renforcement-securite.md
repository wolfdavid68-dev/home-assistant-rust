# Renforcement de la sécurité, expliqué

Trois améliorations, dans l'ordre :

1. [les jetons des appareils chiffrés dans la base](#1-les-jetons-des-appareils-chiffrés) ;
2. [la double authentification](#2-la-double-authentification-totp) ;
3. [le journal d'audit](#3-le-journal-daudit).

## 1. Les jetons des appareils, chiffrés

### Le problème

Pour piloter tes lampes et tes volets, le serveur garde deux secrets dans `ha.db` : la **clé
d'application** du pont Hue et le **jeton** de la box Somfy. Il doit pouvoir les relire à chaque
démarrage, donc on ne peut pas les « hacher » comme les mots de passe.

Jusqu'ici, ils étaient en clair. Une copie de `ha.db` (une sauvegarde oubliée sur une clé USB,
un disque revendu…) suffisait pour piloter ta maison depuis ton réseau.

### La solution

Les secrets sont maintenant **chiffrés**, avec une clé rangée dans **un autre fichier** :

```
ha.db   ──  enc:v1:8f3a…c41e   (jeton Somfy chiffré : illisible)
ha.key  ──  la clé (64 caractères), lisible par toi seul
```

- Sans Docker, `ha.key` est créé à côté de `ha.db` au premier démarrage. La variable
  `HA_KEY_PATH` permet de le mettre ailleurs (sur une clé USB, par exemple).
- Avec Docker, la clé est dans son propre volume (`ha-cle`). L'outil de sauvegarde ne copie
  que la base : **une sauvegarde volée ne permet pas de piloter tes appareils**.
- Les associations faites avant cette mise à jour sont **chiffrées automatiquement** au
  démarrage suivant (le journal affiche « jeton de la box Somfy chiffré dans la base »).
- **Si la clé est perdue**, le serveur démarre quand même. Il signale l'association illisible
  et demande de refaire `hue pair` ou `somfy pair`. Le reste de la maison continue de fonctionner.

Mets la clé de côté une fois (gestionnaire de mots de passe) : la commande est dans le
[guide de l'étape 7](etape-7-deploiement.md#sauvegarder-et-restaurer).

### Ce que ça protège, et ce que ça ne protège pas

| Situation | Protégé ? |
|---|---|
| Quelqu'un récupère une sauvegarde de `ha.db` | **Oui** : les jetons sont illisibles |
| Quelqu'un modifie un jeton chiffré dans la base | **Oui** : la modification est détectée, le jeton est refusé |
| Quelqu'un recopie le jeton Somfy à la place de la clé Hue | **Oui** : chaque secret est lié à son usage |
| Quelqu'un a accès à toute la machine (base **et** clé) | **Non** : aucun chiffrement ne protège de ça. C'est le rôle des droits du système, de Docker et du chiffrement du disque (BitLocker sous Windows) |

### L'algorithme

**XChaCha20-Poly1305**, de la bibliothèque RustCrypto (Rust pur, sans `unsafe` de notre côté) :

- c'est un chiffrement **authentifié** : en plus de cacher le contenu, il détecte toute
  modification ;
- chaque secret a son propre **nonce** aléatoire de 24 octets. Le même jeton chiffré deux fois
  donne deux résultats différents ;
- le **contexte** (`"hue_bridge.app_key"`, `"somfy_box.token"`) est ajouté comme « donnée
  associée » : il n'est pas chiffré, mais il est vérifié au déchiffrement.

### Concepts Rust rencontrés

- **`Arc` pour partager sans copier.** Le `Store` est cloné partout (enregistreur, API, Hue,
  Somfy). La clé est dans un `Arc` : tous les clones pointent vers la même clé en mémoire, au
  lieu d'en faire des copies.
- **Effacer la mémoire (`zeroize`).** La clé et les secrets déchiffrés sont dans des
  `Zeroizing<…>` : ils sont remplis de zéros quand on n'en a plus besoin, au lieu de traîner en
  mémoire. Le chiffreur lui-même efface sa clé (option `zeroize` de la crate).
- **Un `Debug` écrit à la main.** `#[derive(Debug)]` afficherait la clé dans un message de
  débogage. On écrit donc `impl Debug` nous-mêmes, pour afficher `SecretKey(<masquée>)`.
- **`split_first_chunk::<24>()`.** Cette méthode coupe une tranche en « les 24 premiers octets »
  (un tableau de taille connue à la compilation) et « le reste ». Elle renvoie `None` si la
  tranche est trop courte : pas d'indice qui déborde, pas de `panic`.

### Testé

- Chiffrer puis déchiffrer redonne le secret ; deux chiffrements du même secret diffèrent.
- Mauvaise clé, mauvais contexte, secret modifié ou tronqué : tous refusés.
- Le fichier de clé est créé avec les droits 600, puis réutilisé ; un fichier de clé abîmé
  est refusé avec un message clair.
- Base relue après redémarrage : le secret se déchiffre. Base **sans sa clé** : impossible.
- Hue et Somfy : la valeur en base ne contient plus le jeton, et une ancienne association en
  clair est rechiffrée à la lecture.
- Clé perdue : le serveur démarre, signale l'association illisible et passe aux appareils
  restants.
- Docker : clé créée dans `/cle/ha.key` (droits 600, utilisateur sans privilège), conteneur
  « healthy ».

## 2. La double authentification (TOTP)

### Le principe

Un mot de passe peut fuiter : réutilisé sur un site piraté, tapé sur un faux site, vu par-dessus
ton épaule. Avec la double authentification, il faut **en plus** un code à 6 chiffres qui change
toutes les 30 secondes, affiché par une appli de ton téléphone. Un mot de passe volé ne suffit
plus.

C'est le même système (TOTP, RFC 6238) que pour ta banque ou ton compte Google. N'importe quelle
appli d'authentification convient : Aegis, 2FAS, Google Authenticator, Microsoft Authenticator,
ou ton gestionnaire de mots de passe.

### L'activer

Sur la machine du serveur (personne ne peut l'activer, ni la désactiver, par le réseau) :

```powershell
cargo run -p ha-server -- user totp david
# avec Docker :
docker compose run --rm ha-server user totp david
```

1. Un **QR code** s'affiche dans le terminal : scanne-le avec l'appli. (Si elle ne le lit pas,
   saisis à la main le code affiché en dessous.)
2. Tape le code à 6 chiffres que l'appli affiche. La double authentification n'est activée
   que si ce code est bon : impossible de s'enfermer dehors avec un QR code mal scanné.
3. Toutes les sessions ouvertes sont fermées. À la prochaine connexion, la page demande le code :

![Page de connexion avec le code](images/connexion.png)

Le code qui a servi à l'activation ne peut pas resservir : pour te connecter juste après,
attends le code suivant (au plus 30 secondes).

Autres commandes : `user list` indique les comptes protégés, `user totp-off david` désactive.

**Si tu perds ton téléphone** : sur la machine du serveur, `user totp-off david`, puis réactive
avec le nouveau téléphone. Pense à noter le code de secours (le code « à la main ») dans ton
gestionnaire de mots de passe au moment de l'activation.

### Les protections

- **Le secret est chiffré** dans la base, avec la clé de la partie 1.
- **Un code ne sert qu'une fois.** Le serveur retient le dernier créneau accepté : un code
  intercepté (par-dessus ton épaule, par exemple) est refusé, même dans les 30 secondes. Deux
  connexions simultanées avec le même code ? Une seule passe : la vérification et
  l'enregistrement se font en une seule opération SQL.
- **Le message d'erreur ne dit rien.** Mauvais nom, mauvais mot de passe, code oublié ou faux :
  toujours « nom d'utilisateur, mot de passe ou code incorrect ». Un attaquant n'apprend même pas
  si le mot de passe était bon.
- **Les codes faux comptent** dans la limite de 5 essais par quart d'heure : impossible
  d'essayer le million de codes possibles.
- **Un créneau d'écart est toléré** (horloge du téléphone en avance ou en retard de
  30 secondes), pas plus.
- **Clé de chiffrement perdue** : la connexion est refusée (sans planter le serveur), et le
  journal explique comment désactiver puis réactiver.

### Comment ça marche

Le téléphone et le serveur partagent un **secret** de 160 bits (dans le QR code). Toutes les 30
secondes, chacun calcule :

```
créneau = secondes depuis 1970 / 30
code    = HMAC-SHA1(secret, créneau) → on garde 6 chiffres
```

Ils trouvent le même code sans jamais se parler. Le calcul est dans
[totp.rs](../crates/ha-auth/src/totp.rs) : 20 lignes, vérifiées avec les exemples officiels de
la RFC 6238.

### Concepts Rust rencontrés

- **`Option<&str>` pour un paramètre facultatif.** `login(nom, mot_de_passe, code, ip)` reçoit
  `Some("123456")` ou `None`. Côté JSON, `#[serde(default)]` rend le champ `code` facultatif,
  tout en refusant toujours les champs inconnus.
- **Les motifs sur les tableaux.** `let Some(&[a, b, c, d]) = … else { … }` lit exactement 4
  octets, ou passe dans le `else`. Pas d'indice qui pourrait déborder.
- **`is_none_or`.** `last_step.is_none_or(|last| step > last)` se lit « pas encore de code
  utilisé, ou ce créneau est plus récent ».
- **Le calcul en temps constant.** La vérification calcule toujours les trois codes possibles
  (créneau précédent, actuel, suivant), même après avoir trouvé le bon : le temps de réponse ne
  révèle rien.

### Testé

- Les exemples officiels de la RFC 6238 donnent les bons codes ; l'encodage base 32 suit la
  RFC 4648.
- Activation refusée avec un mauvais code ; acceptée avec le bon, sessions fermées, secret
  chiffré dans la base.
- Connexion refusée sans code, avec un faux code, avec un bon code et un mauvais mot de passe,
  et en rejouant un code déjà utilisé ; acceptée avec le code suivant.
- Codes mal formés refusés (5 ou 7 chiffres, lettres, chiffres arabes…).
- Secret illisible : connexion refusée sans planter, `totp-off` répare.
- De bout en bout : activation en ligne de commande, QR code **décodé** comme le ferait un
  téléphone (sur fond sombre, comme PowerShell), puis connexion dans Chromium avec le code.

## 3. Le journal d'audit

### À quoi ça sert

Savoir **qui a fait quoi, quand, et d'où**. Qui a ouvert les volets à 3 h du matin ? Y a-t-il
eu des tentatives de connexion depuis une adresse inconnue ? Quand la double authentification
a-t-elle été désactivée ? C'est aussi indispensable avant de pouvoir piloter l'alarme.

### Ce qui est enregistré

| Action | Qui | D'où |
|---|---|---|
| Connexion, **échec de connexion**, déconnexion | le compte (ou le nom tapé) | adresse IP |
| Commande (allumer, ouvrir un volet…), état modifié ou supprimé par l'API | le compte | adresse IP |
| Compte créé, supprimé, mot de passe changé, double authentification activée ou désactivée | « ligne de commande » | local |
| Pont Hue ou box Somfy associé ou oublié | « ligne de commande » | local |
| Démarrage et arrêt du serveur | « serveur » | local |

Jamais de mot de passe, de code ni de jeton. Les tentatives **bloquées** (après 5 échecs) ne sont
pas enregistrées, sinon un attaquant pourrait remplir le journal à volonté. Les actions des
automatisations ne sont pas encore enregistrées.

Le journal est gardé **365 jours** (variable `HA_AUDIT_DAYS` pour changer).

### Le consulter

Sur le tableau de bord, en bas : **Journal d'activité** (les échecs de connexion sont en rouge).

![Journal d'activité](images/journal.png)

En ligne de commande :

```powershell
cargo run -p ha-server -- audit          # les 50 dernières actions
cargo run -p ha-server -- audit 200      # les 200 dernières
cargo run -p ha-server -- audit verify   # le journal a-t-il été modifié ?
```

```
14/10/2026 21:37:27  ligne de commande  compte créé                        david  (local)
14/10/2026 21:37:30  david              ÉCHEC de connexion                   (127.0.0.1)
14/10/2026 21:37:30  david              connexion                            (127.0.0.1)
14/10/2026 21:38:21  david              commande                           switch.prise_cafe switch.toggle  (127.0.0.1)
```

### Un journal qu'on ne peut pas falsifier en douce

Un journal n'a de valeur que si un intrus ne peut pas effacer ses traces. Chaque ligne est donc :

- **signée** : HMAC-SHA256 de tout son contenu, avec une clé dérivée de `ha.key`. Sans la clé,
  impossible de modifier une ligne ou d'en fabriquer une fausse avec une signature valide ;
- **chaînée** : elle contient la signature de la ligne précédente. Supprimer ou remplacer une
  ligne au milieu casse la chaîne.

```
ligne 1 : …, signature S1 = HMAC(clé, contenu 1 + 000…0)
ligne 2 : …, signature S2 = HMAC(clé, contenu 2 + S1)
ligne 3 : …, signature S3 = HMAC(clé, contenu 3 + S2)
```

`ha-server audit verify` recalcule tout. **Le serveur le fait aussi à chaque démarrage**, et
prévient dans les logs si le journal a été touché pendant qu'il était arrêté.

Les limites, honnêtement :

- Supprimer les **dernières** lignes ne casse pas la chaîne. On peut seulement remarquer qu'il
  en manque (les numéros de ligne).
- Quelqu'un qui a la base **et** la clé peut tout réécrire. Comme pour la partie 1, c'est le rôle
  des droits du système et du chiffrement du disque.

### Les autres protections

- **Les champs sont nettoyés** avant d'être écrits : longueur limitée, et aucun caractère de
  contrôle. Sinon, un attaquant pourrait taper comme nom d'utilisateur une séquence
  d'échappement qui efface ou maquille l'affichage de ton terminal quand tu lis le journal.
- **Deux écritures en même temps** (le serveur, et une commande `ha-server user …` lancée à côté)
  ne cassent pas la chaîne : chaque écriture réserve la base (`BEGIN IMMEDIATE`) le temps de lire
  la dernière ligne et d'ajouter la sienne. Un test lance 20 écritures simultanées depuis deux
  connexions.
- **Si le journal ne peut pas être écrit** (disque plein…), l'action se fait quand même et
  l'erreur est signalée dans les logs. Mieux vaut pouvoir encore fermer ses volets.

### Concepts Rust rencontrés

- **Les motifs « ou » avec liaison.** `["add" | "passwd" | "delete", name]` reconnaît plusieurs
  commandes d'un coup, et `command @ (…)` garde au passage le mot qui a correspondu.
- **Une clé par usage.** La clé du journal est *dérivée* de la clé principale
  (`HMAC(clé, "ha-rust audit log v1")`), au lieu de réutiliser la même clé pour deux choses
  différentes (chiffrer et signer). C'est une bonne pratique de cryptographie.
- **La longueur avant chaque champ.** Si on signait `"ab" + "c"` en collant simplement les
  champs, la signature serait la même que pour `"a" + "bc"`. Chaque champ est donc précédé de sa
  longueur.
- **`pub(crate)`.** La fonction de signature n'est visible que dans la crate `ha-storage` : aucun
  autre code ne peut s'en servir pour signer n'importe quoi.

### Testé

- Les lignes se relisent dans l'ordre, avec acteur, cible, détail et adresse.
- Ligne modifiée, ligne supprimée au milieu, ligne ajoutée sans la clé : **détectées**. Purge des
  lignes anciennes : la chaîne reste valide.
- 20 écritures simultanées depuis deux connexions : chaîne intacte.
- API : échec de connexion, connexion et commande enregistrés, avec l'adresse IP ; nom piégé
  (séquence d'échappement) neutralisé ; mot de passe absent ; tentatives bloquées non
  enregistrées ; `/api/audit` exige d'être connecté.
- De bout en bout : compte créé en ligne de commande, échec puis connexion et commande dans
  Chromium, journal affiché sur le tableau de bord, `audit` et `audit verify` ; base falsifiée à
  la main → `audit verify` et le démarrage du serveur le signalent.
