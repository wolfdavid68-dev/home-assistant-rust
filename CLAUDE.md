# Consignes projet — home-assistant-rust

## Pull requests

Le dépôt est **public** depuis le 19 octobre 2026 : la CI ne consomme plus de
minutes facturées. Les règles ci-dessous gardent la CI utile et les PR lisibles.

**Ouvrir une PR en brouillon pendant le travail.** La CI ne tourne pas sur un
brouillon (voir l'en-tête de `.github/workflows/ci.yml`) : une session pousse
une dizaine de commits, et seul le dernier verdict compte.

**Passer la PR en « prête » soi-même, une fois le travail terminé et vérifié
en local** : c'est ce qui lance la CI. Pas besoin d'attendre l'accord de David
pour cela. Avant, lancer en local ce que la CI vérifiera :

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
```

```bash
gh pr create --draft …
gh pr ready <numéro>   # après les vérifications locales
```

On peut pousser sur une PR déjà « prête » (la CI repart, sans coût). Pour une
longue reprise de travail, la repasser d'abord en brouillon
(`POST /repos/{owner}/{repo}/pulls/{n}/ccr/convert_to_draft`), pour ne pas
lancer la CI à chaque commit.

**Ne jamais fusionner sans le « Fusion » explicite de David.** C'est lui qui
décide de ce qui entre dans `main`.

**Le dépôt est public** : jamais de mot de passe, de jeton, d'adresse de la
maison ni de photo privée (comme les images de la maison de David) dans un
commit, une PR ou un commentaire. Ses fichiers personnels restent hors du dépôt.

Ajouter l'étiquette `windows` à une PR qui touche aux chemins de fichiers, au
port série ou aux permissions : c'est elle qui lance la vérification Windows.
