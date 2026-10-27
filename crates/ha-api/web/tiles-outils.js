// Tableau de bord — ce que toutes les tuiles partagent : l'en-tête, le
// bouton, le curseur, la classe, le texte de l'état et son accord, puis les
// libellés des états.
//
// Un des fichiers de tiles.js, chargé après tiles-capteurs.js par
// index.html. Les scripts partagent leurs déclarations. Pas d'innerHTML ici
// non plus.
"use strict";

function tileHead(state, icon, subtitle, extra) {
  const head = el(
    "div",
    { class: "tile-head" },
    el("span", { class: "icon" }, svgIcon(icon)),
    el("div", {}, el("div", { class: "name", text: nameOf(state), title: state.entity_id }), el("div", { class: "sub", text: subtitle })),
  );
  if (extra) head.append(extra);
  return head;
}

/** L'interrupteur dessiné à droite d'une tuile qu'on allume d'un toucher. */
const knob = () => el("span", { class: "knob", "aria-hidden": "true" });

/** Un curseur se remplit jusqu'à sa valeur (et suit le doigt). */
function fillSlider(slider) {
  const update = () => {
    const min = Number(slider.min) || 0;
    const max = Number(slider.max) || 100;
    const percent = ((Number(slider.value) - min) / Math.max(1e-9, max - min)) * 100;
    slider.style.setProperty("--fill", `${Math.min(100, Math.max(0, percent))}%`);
  };
  update();
  slider.addEventListener("input", update);
}

function tileClass(state, kind) {
  const classes = ["tile", kind];
  if (state.state === "on" || (kind === "cover" && state.state === "open")) classes.push("on");
  if (state.state === "unavailable" || state.state === "unknown") classes.push("unavailable");
  // Seuls les capteurs ont une famille : une lampe reste une lampe.
  if (kind === "sensor" || kind === "binary") {
    const family = familyOfState(state);
    if (family) classes.push(`fam-${family}`);
  }
  return classes.join(" ");
}

function statusText(state) {
  return agree(state, lookup(STATUS_LABELS, state.state) ?? state.state);
}

// Les noms féminins les plus courants dans une maison : sans eux, on lirait
// « Porte du garage : ouvert ». La même liste existe côté serveur, pour le
// journal de bord (crates/ha-logbook/src/describe.rs) : les garder pareilles.
const FEMININE = new Set([
  "alarme", "allée", "ampoule", "applique", "baie", "barrière", "bouilloire", "box", "buanderie",
  "cafetière", "caméra", "cave", "chambre", "chaudière", "climatisation", "clim", "cour",
  "cuisine", "électrovanne", "enceinte", "entrée", "fenêtre", "friteuse", "guirlande", "hotte",
  "imprimante", "lampe", "lucarne", "lumière", "machine", "multiprise", "piscine", "plaque",
  "pompe", "porte", "prise", "salle", "serrure", "sirène", "sonde", "sonnette", "station",
  "suspension", "télé", "télévision", "terrasse", "trappe", "vanne", "véranda", "ventilation",
  "verrière", "veilleuse", "vmc",
]);
const FEMININE_FORMS = {
  ouvert: "ouverte", fermé: "fermée", allumé: "allumée", éteint: "éteinte", connecté: "connectée",
  déconnecté: "déconnectée", actif: "active", inactif: "inactive", inconnu: "inconnue",
  verrouillé: "verrouillée", déverrouillé: "déverrouillée", bloqué: "bloquée",
};

/** Le nom de l'appareil est-il féminin ? On regarde son premier mot, ou
 *  l'article : « Porte du garage », « La cafetière », binary_sensor.porte_entree. */
function isFeminine(state) {
  const words = nameOf(state).toLowerCase().split(/[\s_'’-]+/u).filter(Boolean);
  const [first, second] = words;
  if (first === "la" || first === "une") return true;
  if (first === "le" || first === "un" || first === "l") return FEMININE.has(second);
  return FEMININE.has(first);
}

/** Accorde le premier mot d'un libellé : « Ouvert à 85 % » → « Ouverte à 85 % ». */
function agree(state, text) {
  if (!isFeminine(state)) return text;
  return text.replace(/^\p{L}+/u, (word) => {
    const feminine = lookup(FEMININE_FORMS, word.toLowerCase());
    if (feminine === undefined) return word;
    return word[0] === word[0].toLowerCase() ? feminine : feminine[0].toUpperCase() + feminine.slice(1);
  });
}

// Modes d'alarme Somfy : [texte affiché, activée ?]
const ALARM_LABELS = {
  off: ["Désactivée", false], disarmed: ["Désactivée", false],
  total: ["Activée (totale)", true], armed: ["Activée", true],
  partial: ["Activée (partielle)", true], partial1: ["Activée (partielle)", true], partial2: ["Activée (partielle)", true],
  detection: ["Intrusion détectée !", true], sos: ["Alerte SOS !", true],
};
const LOCK_LABELS = {
  locked: "Verrouillé", unlocked: "Déverrouillé", locking: "Verrouillage…", unlocking: "Déverrouillage…",
  jammed: "Bloqué !", open: "Ouvert", opening: "Ouverture…",
};
const VACUUM_LABELS = {
  cleaning: "Fait le ménage", docked: "Sur sa base", paused: "En pause", idle: "À l'arrêt",
  returning: "Rentre à sa base", error: "En erreur !",
};
const WATER_HEATER_LABELS = {
  off: "Arrêt", eco: "Éco", electric: "Électrique", gas: "Gaz", heat_pump: "Pompe à chaleur",
  high_demand: "Forte demande", performance: "Performance",
};
const COVER_LABELS = { open: "Ouvert", closed: "Fermé", opening: "Ouverture…", closing: "Fermeture…" };
const STATUS_LABELS = { on: "Allumé", off: "Éteint", unknown: "Inconnu", unavailable: "Indisponible" };
const BINARY_LABELS = {
  door: ["Ouvert", "Fermé"], window: ["Ouvert", "Fermé"], opening: ["Ouvert", "Fermé"], garage_door: ["Ouvert", "Fermé"],
  motion: ["Mouvement", "Calme"], occupancy: ["Présence", "Personne"],
  smoke: ["Fumée !", "Rien à signaler"],
  connectivity: ["Connecté", "Déconnecté"],
};
