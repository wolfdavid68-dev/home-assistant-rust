// Les fenêtres (dialogues) du tableau de bord : un même en-tête (titre,
// sous-titre, bouton Fermer rond de 44 px en haut à droite), un même pied
// (action principale à droite, secondaires à côté, action dangereuse à
// gauche, en rouge), une même façon de s'ouvrir et de se fermer (Échap,
// toucher le fond pour les fenêtres sans saisie, focus sur le premier champ
// puis rendu à ce qui a ouvert), la confirmation stylée (`confirmer`) et
// l'état vide des listes (`etatVide`).
//
// Chargé juste après utils.js (el, svgIcon). Les fenêtres fixes de
// index.html reprennent les mêmes classes, écrites en entier : Tailwind ne
// trouve que les noms complets.

const FENETRE = {
  // La fenêtre elle-même (celles créées ici ; index.html a les mêmes).
  petite: "fenetre history-dialog [max-height:calc(100dvh_-_2rem)] border-0 rounded-fenetre bg-card text-text shadow-flottant p-5 [width:min(420px,_calc(100vw_-_2rem))] [&::backdrop]:[background:rgb(14_20_32_/_0.45)] [&::backdrop]:[backdrop-filter:blur(3px)]",
  entete: "fenetre-entete flex items-center justify-between gap-4 mb-4",
  titre: "m-0 font-titre text-titre font-semibold normal-case [letter-spacing:-0.02em] leading-tight text-text [overflow-wrap:anywhere]",
  sousTitre: "mt-1 mb-0 mx-0 text-corps text-muted",
  fermer: "fenetre-fermer flex-none grid place-items-center w-11 h-11 p-0 rounded-full border-0 bg-card-2 text-text-2 hover:text-text [&_svg]:w-5 [&_svg]:h-5",
  pied: "fenetre-pied flex flex-wrap items-center justify-end gap-2.5 mt-5",
  principal: "primary min-h-11 px-5 rounded-champ",
  secondaire: "ghost min-h-11 px-5 rounded-champ",
  // Dans une confirmation : le bouton d'action, sur le rouge doux (encre AA dans les deux thèmes).
  danger: "min-h-11 px-5 rounded-champ border-0 bg-alert-soft text-alert-ink font-semibold hover:brightness-95",
  // Dans un pied : l'action dangereuse, à gauche, en texte rouge.
  dangerDiscret: "ghost danger min-h-11 px-5 rounded-champ mr-auto",
};

const FENETRE_CROIX = ["M6 6l12 12", "M18 6L6 18"];

/** L'en-tête d'une fenêtre : `titre` (texte ou nœuds), `sousTitre`
 *  facultatif, et le bouton Fermer si `onClose` est donné. `id` : celui du
 *  titre (pour `aria-labelledby`). */
function fenetreEntete(titre, sousTitre = "", onClose = null, id = "") {
  const h2 = el("h2", { class: FENETRE.titre, ...(id && { id }) });
  h2.append(...[titre].flat());
  const texte = el("div", { class: "min-w-0" }, h2);
  if (sousTitre) texte.append(el("p", { class: FENETRE.sousTitre, text: sousTitre }));
  const entete = el("div", { class: FENETRE.entete }, texte);
  if (onClose) {
    const fermer = el("button", { type: "button", class: FENETRE.fermer, "aria-label": "Fermer" }, svgIcon(FENETRE_CROIX));
    fermer.addEventListener("click", onClose);
    entete.append(fermer);
  }
  return entete;
}

/** Un bouton de fenêtre : `genre` = principal, secondaire, danger ou dangerDiscret. */
function fenetreBouton(texte, genre = "secondaire", onClick = null) {
  const bouton = el("button", { type: "button", class: FENETRE[genre] ?? FENETRE.secondaire, text: texte });
  if (onClick) bouton.addEventListener("click", onClick);
  return bouton;
}

/** Le pied d'une fenêtre : les boutons dans l'ordre (le principal en dernier). */
function fenetrePied(...boutons) {
  return el("div", { class: FENETRE.pied }, ...boutons);
}

/** Le point (x, y) est-il hors du rectangle ? (un toucher sur le fond) */
function fenetreDehors(rect, x, y) {
  return x < rect.left || x > rect.right || y < rect.top || y > rect.bottom;
}

/** Ce qui reçoit le focus à l'ouverture : `[autofocus]`, sinon le premier
 *  champ visible, sinon le bouton principal ; `null` : le navigateur choisit
 *  (le premier bouton, Fermer). */
function fenetrePremierChamp(dialog) {
  const visible = (node) => !node.disabled && node.getClientRects().length > 0;
  const auto = dialog.querySelector("[autofocus]");
  if (auto && visible(auto)) return auto;
  const champs = dialog.querySelectorAll("input:not([type=hidden]):not([type=checkbox]):not([type=radio]):not([type=range]), select, textarea");
  return [...champs].find(visible) ?? [...dialog.querySelectorAll(".primary")].find(visible) ?? null;
}

const fenetreOuvreurs = new WeakMap();
// Fenêtre → fonction « des changements ne sont pas enregistrés ? ».
const fenetreGardes = new WeakMap();
const fenetresPretes = new WeakSet();

/** Une fois par fenêtre : le focus rendu à l'ouvreur, et le toucher sur le
 *  fond qui ferme (attribut `data-fond-ferme`, pour les fenêtres sans saisie). */
function fenetrePrete(dialog) {
  if (fenetresPretes.has(dialog)) return;
  fenetresPretes.add(dialog);
  dialog.addEventListener("close", () => {
    const ouvreur = fenetreOuvreurs.get(dialog);
    fenetreOuvreurs.delete(dialog);
    if (ouvreur?.isConnected) ouvreur.focus();
  });
  let depuisLeFond = false;
  const surLeFond = (event) => event.target === dialog && dialog.hasAttribute("data-fond-ferme")
    && fenetreDehors(dialog.getBoundingClientRect(), event.clientX, event.clientY);
  // Le geste commence et finit sur le fond : un glissé depuis un champ ne ferme pas.
  dialog.addEventListener("pointerdown", (event) => { depuisLeFond = surLeFond(event); });
  dialog.addEventListener("click", (event) => {
    if (depuisLeFond && surLeFond(event)) fenetreFermer(dialog);
    depuisLeFond = false;
  });
  // Échap : avec des changements non enregistrés (`fenetreGarde`), on demande d'abord.
  dialog.addEventListener("cancel", (event) => {
    if (!fenetreGardes.get(dialog)?.()) return;
    event.preventDefault();
    fenetreFermer(dialog);
  });
}

/** Ouvre une fenêtre en modal. `cible` : l'élément à focaliser ; `false` :
 *  laisser le navigateur (Fermer) ; rien : le premier champ ou le bouton principal. */
function ouvrirFenetre(dialog, cible = undefined) {
  fenetrePrete(dialog);
  if (!dialog.open) {
    const avant = document.activeElement;
    if (avant instanceof HTMLElement && avant !== document.body) fenetreOuvreurs.set(dialog, avant);
    dialog.showModal();
  }
  const champ = cible === undefined ? fenetrePremierChamp(dialog) : cible;
  if (champ) champ.focus();
}

// ---------------------------------------------------------------- Confirmation

let confirmerFin = null;

/** Remplace `window.confirm` : une petite fenêtre au style des autres.
 *  Renvoie une promesse : `true` si on confirme ; Annuler, Échap ou le fond :
 *  `false`. Une action dangereuse (`danger`) a son bouton en rouge et le
 *  focus sur Annuler. */
function confirmer(message, { titre = "Confirmer", ok = "Confirmer", annuler = "Annuler", danger = false } = {}) {
  let dialog = document.getElementById("confirmer");
  if (!dialog) {
    dialog = el("dialog", { id: "confirmer", class: FENETRE.petite, "aria-labelledby": "confirmer-titre", "aria-describedby": "confirmer-message", "data-fond-ferme": "" });
    dialog.addEventListener("close", () => {
      const fin = confirmerFin;
      confirmerFin = null;
      fin?.(dialog.returnValue === "oui");
    });
    document.body.append(dialog);
  }
  // Une seule demande à la fois : la précédente vaut « Annuler ».
  confirmerFin?.(false);
  const non = fenetreBouton(annuler, "secondaire", () => dialog.close("non"));
  const oui = fenetreBouton(ok, danger ? "danger" : "principal", () => dialog.close("oui"));
  dialog.replaceChildren(
    fenetreEntete(titre, "", null, "confirmer-titre"),
    el("p", { id: "confirmer-message", class: "m-0 text-courant text-text-2 [overflow-wrap:anywhere]", text: message }),
    fenetrePied(non, oui),
  );
  dialog.returnValue = "";
  return new Promise((resolve) => {
    confirmerFin = resolve;
    ouvrirFenetre(dialog, danger ? non : oui);
  });
}

// ---------------------------------------------------------------- État vide

/** Une liste vide, pleine largeur : une icône dans un rond, le texte
 *  centré, une action facultative `{ texte, onClick }`. `tag` : `li` dans
 *  une liste `ul`/`ol`. */
function etatVide(icone, texte, action = null, tag = "div") {
  const box = el(tag, { class: "etat-vide grid justify-items-center gap-3 py-8 px-4 text-center list-none" },
    el("span", { class: "grid place-items-center w-14 h-14 rounded-full bg-card-2 text-muted [&_svg]:w-6 [&_svg]:h-6", "aria-hidden": "true" }, svgIcon(icone)),
    el("p", { class: "m-0 max-w-sm text-corps text-muted", text: texte }));
  if (action) box.append(fenetreBouton(action.texte, "secondaire", action.onClick));
  return box;
}

// ---------------------------------------------------------------- Changements non enregistrés

/** Rien de perdu par erreur : `modifie()` dit si la fenêtre a des
 *  changements non enregistrés ; Échap, le fond ou `fenetreFermer` demandent
 *  alors « Abandonner les changements ? » avant de fermer. */
function fenetreGarde(dialog, modifie) {
  fenetreGardes.set(dialog, modifie);
  fenetrePrete(dialog);
}

/** « Abandonner les changements ? » : `true` si on abandonne. Le focus va
 *  sur « Continuer » : Entrée ne perd rien. */
function abandonnerChangements(message = "Ce qui n'est pas enregistré sera perdu.") {
  return confirmer(message, { titre: "Abandonner les changements ?", ok: "Abandonner", annuler: "Continuer", danger: true });
}

/** Ferme la fenêtre (bouton Fermer, Annuler) ; avec des changements non
 *  enregistrés, après confirmation. Renvoie `true` si elle est fermée. */
async function fenetreFermer(dialog) {
  if (fenetreGardes.get(dialog)?.() && !await abandonnerChangements()) return false;
  dialog.close();
  return true;
}

// ---------------------------------------------------------------- Formulaires

let champNumero = 0;

/** L'erreur d'un champ, écrite juste sous lui : le texte, `aria-invalid` et
 *  `aria-describedby`. Un message vide l'efface ; taper dans le champ aussi.
 *  Dans un `<label>`, le texte y est ajouté en dernier, caché aux lecteurs
 *  d'écran (sinon il s'ajouterait au nom du champ) : ils le lisent par
 *  `aria-describedby`. Renvoie le champ. */
function champErreur(champ, message = "") {
  if (!champ.id) champ.id = `champ-${++champNumero}`;
  const id = `${champ.id}-erreur`;
  const decrit = (champ.getAttribute("aria-describedby") ?? "").split(" ").filter((x) => x && x !== id);
  document.getElementById(id)?.remove();
  if (!message) {
    champ.removeAttribute("aria-invalid");
    if (decrit.length) champ.setAttribute("aria-describedby", decrit.join(" "));
    else champ.removeAttribute("aria-describedby");
    return champ;
  }
  const texte = el("p", { id, class: "champ-erreur m-0 text-petit font-semibold text-alert-ink [overflow-wrap:anywhere]", text: message });
  const label = champ.closest("label");
  if (label) {
    texte.setAttribute("aria-hidden", "true");
    label.append(texte);
  } else {
    champ.after(texte);
  }
  champ.setAttribute("aria-invalid", "true");
  champ.setAttribute("aria-describedby", [...decrit, id].join(" "));
  champ.addEventListener("input", () => champErreur(champ), { once: true });
  return champ;
}

/** Le bouton principal pendant un envoi : désactivé, `aria-busy`, son texte
 *  remplacé par `texte` (« Ajout… »). Renvoie la fonction qui le rétablit. */
function boutonEnCours(bouton, texte = "Envoi…") {
  const avant = bouton.textContent;
  const seulTexte = bouton.childElementCount === 0;
  bouton.disabled = true;
  bouton.setAttribute("aria-busy", "true");
  if (seulTexte) bouton.textContent = texte;
  return () => {
    bouton.disabled = false;
    bouton.removeAttribute("aria-busy");
    if (seulTexte) bouton.textContent = avant;
  };
}

/** « 14:07 » → « 14:15 » : le prochain quart d'heure (au plus 23:45). */
function formProchainQuart(date) {
  const minutes = Math.min(Math.ceil((date.getHours() * 60 + date.getMinutes() + 1) / 15) * 15, 23 * 60 + 45);
  return `${String(Math.floor(minutes / 60)).padStart(2, "0")}:${String(minutes % 60).padStart(2, "0")}`;
}

/** « 14:15 » + 60 → « 15:15 », sans passer minuit (au plus 23:59). */
function formHeurePlus(heure, plus) {
  const [h, m] = heure.split(":").map(Number);
  const minutes = Math.max(0, Math.min(h * 60 + m + plus, 23 * 60 + 59));
  return `${String(Math.floor(minutes / 60)).padStart(2, "0")}:${String(minutes % 60).padStart(2, "0")}`;
}

/** Minutes de « 14:15 » (pour comparer et garder une durée). */
const formMinutes = (heure) => {
  const [h, m] = heure.split(":").map(Number);
  return h * 60 + m;
};
