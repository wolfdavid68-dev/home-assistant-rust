# Philips Hue : piloter tes lampes, expliqué

## Le principe

Le **pont Hue** (le boîtier rond relié à ta box) parle à tes ampoules en Zigbee, et propose une
**API locale** sur ton réseau. `ha-server` l'utilise directement : pas d'Internet, pas de compte
Philips, pas de cloud.

```
 ampoules Hue ◄─Zigbee─► pont Hue ◄──HTTPS (réseau local)──► ha-server (ha-hue)
                            ▲
       appli Hue, Alexa ────┘   (continuent de marcher comme avant)
```

1. **Association** (une seule fois) : `ha-server hue pair <adresse-du-pont>`. Tu appuies sur le
   bouton du pont, qui remet une **clé d'application** au serveur.
2. Au démarrage, le serveur lit la liste des lampes et crée les entités (`light.lampe_salon`…).
3. Il reste connecté au **flux d'événements** du pont : quand une lampe change, que ce soit par
   l'appli Hue, par Alexa ou par un interrupteur, l'entité est mise à jour en direct.
4. `POST /api/services/light/turn_on` (avec `brightness` de 0 à 255 si tu veux) envoie l'ordre
   au pont, exactement comme pour les lampes MQTT.

**Et Alexa ?** Alexa pilote déjà tes lampes Hue directement, et ça continue de marcher. Comme le
serveur suit le flux d'événements du pont, il voit aussi les changements faits par Alexa. Les
deux cohabitent sans rien configurer.

## Les fichiers

| Fichier | Contenu |
|---------|---------|
| [ha-tls](../crates/ha-tls/src/lib.rs) | Connexion HTTPS avec épinglage du certificat (partagée avec Somfy) |
| [pairing.rs](../crates/ha-hue/src/pairing.rs) | Association, sauvegarde de la clé |
| [api.rs](../crates/ha-hue/src/api.rs) | Appels à l'API v2 du pont |
| [lights.rs](../crates/ha-hue/src/lights.rs) | Lampes ⇄ entités, commandes |
| [sse.rs](../crates/ha-hue/src/sse.rs) | Lecture du flux d'événements |
| [client.rs](../crates/ha-hue/src/client.rs) | Boucle principale, reconnexion |
| [tests/fake_bridge.rs](../crates/ha-hue/tests/fake_bridge.rs) | Un **faux pont Hue** en HTTPS pour les tests |
| [0003_hue_bridge.sql](../crates/ha-storage/migrations/0003_hue_bridge.sql) | Table de l'association |

## La sécurité de la connexion : l'épinglage de certificat

Le pont Hue chiffre ses échanges (HTTPS), mais son certificat n'est pas signé par une autorité
publique comme ceux des sites web. Beaucoup de programmes **désactivent alors la vérification**…
et n'importe quel appareil de ton réseau pourrait se faire passer pour le pont, puis voler la clé.

On fait comme SSH (« confiance au premier usage ») :

- à l'association, le serveur affiche l'**empreinte** du certificat du pont et la mémorise ;
- ensuite, **seul ce certificat précis est accepté**. Le serveur vérifie aussi que le pont
  possède bien la clé privée correspondante : présenter une copie du certificat ne suffit pas.

Un test le prouve : le faux pont des tests, avec un autre certificat, est refusé.

Autres protections :

- La **clé d'application** est rangée dans `ha.db`, lisible par toi seul (droits `600`). Elle
  n'apparaît jamais dans les logs ni dans un affichage de débogage, et elle est effacée de la
  mémoire après usage.
- Tout ce qui vient du pont est vérifié : identifiants de lampes au format UUID (ils finissent dans
  des adresses), noms limités et sans caractères de contrôle, réponses limitées à 1 Mio,
  événements à 1 Mio, 500 lampes au plus.
- L'adresse du pont est vérifiée (ni `/`, ni `@`…) pour qu'elle ne puisse pas détourner l'URL.
- `ha-server hue forget` efface la clé. Retire aussi « ha_rust » des applications autorisées dans
  l'appli Hue pour la révoquer complètement.

## Les concepts Rust rencontrés

**Implémenter le trait d'une bibliothèque pour changer son comportement.** `PinnedVerifier`
implémente `ServerCertVerifier` de rustls. rustls appelle nos méthodes pendant la poignée de main
TLS, et notre code décide d'accepter ou non le certificat.

**`Arc<dyn Trait>`.** rustls attend « n'importe quel vérificateur » : un `Arc<dyn
ServerCertVerifier>`. `dyn` veut dire que le type exact n'est connu qu'à l'exécution ; `Arc`
permet de garder notre propre copie pour lire l'empreinte vue.

**Étiquettes de boucle.** `'connection: loop { … continue 'connection; … }` : depuis la boucle
de lecture du flux, on recommence directement la connexion complète (relecture des lampes, nouveau
flux). Sans étiquette, `continue` ne viserait que la boucle intérieure.

**`if let … && let …` (« let chains »).** Dans `client.rs`, une seule condition enchaîne
« c'est un appel de service » **et** « il concerne une de nos lampes ».

**Un tampon pour un flux.** Le réseau livre les données par morceaux de taille quelconque.
`SseParser` accumule et ne rend que les événements complets.

**Tester contre un faux serveur.** Le test démarre un vrai serveur HTTPS local (certificat
généré par `rcgen`), qui répond comme le pont. Le vrai code client est testé de bout en bout,
sans matériel.

## Mettre en place

**1. Trouver l'adresse du pont** : dans l'appli Hue, Paramètres > Ponts Hue > ⓘ > Adresse IP.
Idéalement, réserve cette adresse dans ta box (bail DHCP fixe) pour qu'elle ne change pas.

**2. Associer** (depuis le dossier du projet) :

```powershell
cargo run -p ha-server -- hue pair 192.168.1.20
```

Le programme affiche l'identifiant et l'empreinte du pont, puis te laisse 60 secondes pour
appuyer sur le gros bouton rond du pont.

**3. Lancer le serveur** : `cargo run -p ha-server`. Les lampes apparaissent dans les logs
(`lampe Hue trouvée : light.salon`) et dans `GET /api/states`.

**4. Piloter** (avec le jeton `$h`, voir le [guide de l'étape 3](etape-3-auth-api.md#essayer)) :

```powershell
Invoke-RestMethod -Method Post -Uri http://127.0.0.1:8123/api/services/light/turn_on `
     -Headers $h -Body '{"entity_id":"light.salon","brightness":128}'
Invoke-RestMethod -Method Post -Uri http://127.0.0.1:8123/api/services/light/toggle `
     -Headers $h -Body '{"entity_id":"light.salon"}'
```

Autres commandes : `ha-server hue status` (pont associé et empreinte), `ha-server hue forget`.

Si l'adresse du pont change, relance simplement `hue pair` avec la nouvelle adresse.

## Pas encore pris en charge

Couleurs et température de blanc, pièces et zones (`grouped_light`), scènes, capteurs de
mouvement et interrupteurs Hue. Les lampes elles-mêmes (marche/arrêt, luminosité) sont gérées.

```bash
cargo test -p ha-hue
```
