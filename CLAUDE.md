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

**Publier une version** (étiquette `vX.Y.Z`, workflow `release.yml`) seulement avec l'accord
explicite de David : la version est publique et téléchargée telle quelle. Voir
`docs/installation.md`.

La vérification Windows (`windows.yml`) tourne sur chaque PR prête, comme la
CI Linux : plus besoin d'étiquette.

## Tableau de bord : le style en Tailwind CSS

- Le style s'écrit en classes **Tailwind v4** dans le HTML et le JS (`crates/ha-api/web/`). La source est `web/css/app.css` ; `web/style.css`, servi par le hub, en est **produit** par `scripts/tailwind.sh` (outil autonome, sans Node) : ne pas le modifier à la main, le reconstruire et le committer avec le changement. La CI (`web.yml`) vérifie qu'il est à jour.
- Couleurs, polices, ombres : les jetons des thèmes (`bg-card`, `text-muted`, `bg-accent`, `font-titre`, `shadow-carte`…), jamais une couleur en dur, pour que clair, sombre, plan et maquette suivent.
- L'ancien CSS a été converti en utilitaires dans le HTML et le JS. Ce qui reste dans `web/css/legacy.css` (la base des éléments, les règles de contexte ou d'état que l'on ne peut pas poser sur une classe, les classes testées par `[class*=…]`) est une couche que les classes Tailwind remplacent toujours : ne pas y ajouter de règles nouvelles.
- Les variantes `[:not(:where(.oNNNNNN))&]:…` viennent de la conversion automatique : elles ne changent pas la spécificité et reproduisent l'ordre de l'ancien fichier (Tailwind trie les variantes arbitraires à sa façon). On peut les retirer quand on retouche une classe et qu'aucune autre règle ne vise la même propriété.
- Une classe construite dans le JS doit y être écrite en entier (`"bg-ok"`, pas `"bg-" + état`) : Tailwind ne trouve que les noms complets.
