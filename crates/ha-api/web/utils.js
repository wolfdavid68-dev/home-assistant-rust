// Tableau de bord — les utilitaires : libellés, icônes, construction des éléments (`el`), noms.
//
// Un des fichiers du tableau de bord, chargés dans l'ordre par index.html
// (utils.js, app.js, tiles.js, dialogs.js, editor.js, account.js, start.js,
// puis canvas.js, house.js, agenda.js). Ils partagent leurs déclarations :
// une fonction d'un fichier peut appeler celle d'un autre, une fois la page
// chargée.
"use strict";

/** Cherche une clé venue du réseau dans un de nos tableaux de libellés.
 *  Sécurité : `Object.hasOwn` ignore les propriétés héritées. Sans lui, un
 *  appareil qui annonce `device_class: "__proto__"` ou un état "constructor"
 *  obtiendrait des objets internes de JavaScript au lieu d'un libellé. */
const lookup = (table, key) => (typeof key === "string" && Object.hasOwn(table, key) ? table[key] : undefined);
const domainOf = (state) => state.entity_id.split(".")[0];
// Une prise Hue est une « lampe » de classe outlet pour le pont : sur le
// tableau de bord, c'est une prise, et « les lumières » ne la comptent pas.
const isOutlet = (state) => domainOf(state) === "light" && state.attributes.device_class === "outlet";
const isLight = (state) => domainOf(state) === "light" && !isOutlet(state);
const nameOf = (state) =>
  customNames.get(state.entity_id) || state.attributes.friendly_name || state.entity_id.split(".")[1].replaceAll("_", " ");
const tileId = (entityId) => `tile-${entityId}`;

const relative = new Intl.RelativeTimeFormat("fr", { numeric: "auto" });
function ago(iso) {
  const seconds = Math.max(0, (Date.now() - Date.parse(iso)) / 1000);
  if (seconds < 60) return "quelques secondes";
  const [value, unit] = seconds < 3600 ? [seconds / 60, "minute"] : seconds < 86400 ? [seconds / 3600, "hour"] : [seconds / 86400, "day"];
  return relative.format(-Math.round(value), unit).replace(/^il y a /, "");
}

/** Crée un élément. Le texte passe TOUJOURS par `textContent` (jamais de HTML). */
function el(tag, attributes = {}, ...children) {
  const node = document.createElement(tag);
  for (const [key, value] of Object.entries(attributes)) {
    if (key === "text") node.textContent = value;
    else node.setAttribute(key, value);
  }
  node.append(...children);
  return node;
}

/** Icône SVG, construite élément par élément (pas de HTML non plus). */
function svgIcon(paths) {
  const NS = "http://www.w3.org/2000/svg";
  const svg = document.createElementNS(NS, "svg");
  for (const [name, value] of Object.entries({
    viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", "stroke-width": "2",
    "stroke-linecap": "round", "stroke-linejoin": "round", "aria-hidden": "true",
  })) svg.setAttribute(name, value);
  for (const d of paths) {
    const path = document.createElementNS(NS, "path");
    path.setAttribute("d", d);
    svg.append(path);
  }
  return svg;
}

const ICONS = {
  back: ["M15 18l-6-6 6-6"],
  lock: ["M5 11h14v10H5z", "M8 11V7a4 4 0 0 1 8 0v4"],
  unlock: ["M5 11h14v10H5z", "M8 11V7a4 4 0 0 1 7.8-1"],
  siren: ["M7 18v-6a5 5 0 0 1 10 0v6", "M5 21h14v-3H5z", "M12 3v2", "M4.2 7.2l1.4 1.4", "M19.8 7.2l-1.4 1.4"],
  valve: ["M4 12h16", "M8 8v8", "M16 8v8", "M12 12V5", "M9 5h6"],
  vacuum: ["M12 3a9 9 0 1 0 0 18 9 9 0 1 0 0-18z", "M12 8a4 4 0 1 0 0 8 4 4 0 1 0 0-8z", "M12 3v2"],
  pause: ["M9 5v14", "M15 5v14"],
  calendar: ["M4 6h16v15H4z", "M4 10h16", "M8 3v4", "M16 3v4"],
  fan: ["M12 12a2 2 0 1 0 0-.01", "M12 10c0-4 1-7 4-7 2 0 3 2 1 4l-5 3", "M14 12c4 0 7 1 7 4 0 2-2 3-4 1l-3-5", "M12 14c0 4-1 7-4 7-2 0-3-2-1-4l5-3", "M10 12c-4 0-7-1-7-4 0-2 2-3 4-1l3 5"],
  bulb: ["M9 18h6", "M10 22h4", "M12 2a7 7 0 0 0-4 12.7V17h8v-2.3A7 7 0 0 0 12 2z"],
  plug: ["M9 2v6", "M15 2v6", "M6 8h12v3a6 6 0 0 1-12 0z", "M12 17v5"],
  door: ["M5 22V3a1 1 0 0 1 1-1h12a1 1 0 0 1 1 1v19", "M3 22h18", "M15 12h.01"],
  motion: ["M13 4a1 1 0 1 0 2 0 1 1 0 1 0-2 0", "M7 21l3-6 3 2v4", "M10 15l1-5 4 3 3 1", "M6 12l3-3 3 1"],
  temperature: ["M14 14.8V4a2 2 0 0 0-4 0v10.8a4 4 0 1 0 4 0z"],
  humidity: ["M12 3s6 6.5 6 11a6 6 0 0 1-12 0c0-4.5 6-11 6-11z"],
  battery: ["M3 7h15a1 1 0 0 1 1 1v8a1 1 0 0 1-1 1H3a1 1 0 0 1-1-1V8a1 1 0 0 1 1-1z", "M22 11v2"],
  gauge: ["M12 14l4-4", "M3.3 19a10 10 0 1 1 17.4 0"],
  shield: ["M12 3l8 3v6c0 4.5-3.4 8.3-8 9-4.6-.7-8-4.5-8-9V6z"],
  blinds: ["M3 3h18", "M4 3v13h16V3", "M4 7h16", "M4 11h16", "M12 16v4", "M10 20h4"],
  up: ["M6 15l6-6 6 6"],
  down: ["M6 9l6 6 6-6"],
  stop: ["M8 8h8v8H8z"],
  illuminance: ["M12 3v2", "M12 19v2", "M5 12H3", "M21 12h-2", "M12 8a4 4 0 1 0 0 8 4 4 0 1 0 0-8z"],
  occupancy: ["M12 4a2 2 0 1 0 0 4 2 2 0 1 0 0-4z", "M8 21v-6l-2-1 2-5h8l2 5-2 1v6"],
  bell: ["M6 8a6 6 0 0 1 12 0c0 7 3 9 3 9H3s3-2 3-9", "M10 21a2 2 0 0 0 4 0"],
  play: ["M8 5v14l11-7z"],
  previous: ["M18 6l-9 6 9 6z", "M6 6v12"],
  next: ["M6 6l9 6-9 6z", "M18 6v12"],
  speaker: ["M4 9h4l5-4v14l-5-4H4z", "M16.5 9a4 4 0 0 1 0 6", "M19 6.5a8 8 0 0 1 0 11"],
  muted: ["M4 9h4l5-4v14l-5-4H4z", "M17 9.5l5 5", "M22 9.5l-5 5"],
  megaphone: ["M3 10v4h3l10 5V5L6 10z", "M19 9.5a3 3 0 0 1 0 5", "M6 14l1.5 5h3L9 15"],
  sparkle: ["M12 3v4", "M12 17v4", "M3 12h4", "M17 12h4", "M6 6l2.5 2.5", "M15.5 15.5L18 18", "M6 18l2.5-2.5", "M15.5 8.5L18 6"],
  chart: ["M4 19h16", "M4 15l4-5 4 3 6-7"],
  moon: ["M21 12.8A9 9 0 1 1 11.2 3a7 7 0 0 0 9.8 9.8z"],
  person: ["M12 3a4 4 0 1 0 0 8 4 4 0 1 0 0-8z", "M4 21a8 8 0 0 1 16 0"],
  globe: ["M12 3a9 9 0 1 0 0 18 9 9 0 1 0 0-18z", "M3 12h18", "M12 3a14 14 0 0 1 0 18", "M12 3a14 14 0 0 0 0 18"],
  data_rate: ["M4 17l5-5 4 4 7-8", "M15 8h5v5"],
  cloud: ["M17.5 19H9a7 7 0 1 1 6.71-9h1.79a4.5 4.5 0 1 1 0 9z"],
  archive: ["M3 4h18v4H3z", "M5 8v12h14V8", "M10 12h4"],
  home: ["M3 11l9-8 9 8", "M5 10v10h14V10", "M10 20v-6h4v6"],
  toggle: ["M7 6h10a6 6 0 0 1 0 12H7A6 6 0 0 1 7 6z", "M16 9a3 3 0 1 0 0 6 3 3 0 1 0 0-6z"],
  sliders: ["M4 6h16", "M4 12h16", "M4 18h16", "M9 4v4", "M15 10v4", "M7 16v4"],
  list: ["M9 6h11", "M9 12h11", "M9 18h11", "M4 6h.01", "M4 12h.01", "M4 18h.01"],
  button: ["M12 3a9 9 0 1 0 0 18 9 9 0 1 0 0-18z", "M12 8a4 4 0 1 0 0 8 4 4 0 1 0 0-8z"],
  timer: ["M10 2h4", "M12 14l3-3", "M12 6a8 8 0 1 0 0 16 8 8 0 1 0 0-16z"],
  flame: ["M8.5 14.5A2.5 2.5 0 0 0 11 12c0-1.38-.5-2-1-3-1.07-2.14-.22-4.05 2-6 .5 2.5 2 4.9 4 6.5 2 1.6 3 3.5 3 5.5a7 7 0 1 1-14 0c0-1.15.43-2.29 1-3a2.5 2.5 0 0 0 2.5 2.5z"],
  partly: ["M12 2v2", "M4.9 4.9l1.4 1.4", "M20 12h2", "M19.1 4.9l-1.4 1.4", "M15.9 13A4 4 0 0 0 8.1 9.2", "M13 22H7a5 5 0 1 1 4.9-6H13a3 3 0 0 1 0 6z"],
  rain: ["M4 14.9A7 7 0 1 1 15.7 8h1.8a4.5 4.5 0 0 1 2.5 8.2", "M16 14v6", "M8 14v6", "M12 16v6"],
  snow: ["M4 14.9A7 7 0 1 1 15.7 8h1.8a4.5 4.5 0 0 1 2.5 8.2", "M8 15h.01", "M8 19h.01", "M12 17h.01", "M12 21h.01", "M16 15h.01", "M16 19h.01"],
  storm: ["M6 16.3A7 7 0 1 1 15.7 8h1.8a4.5 4.5 0 0 1 .5 8.97", "M13 12l-3 5h4l-3 5"],
  fog: ["M4 14.9A7 7 0 1 1 15.7 8h1.8a4.5 4.5 0 0 1 2.5 8.2", "M16 17H7", "M17 21H9"],
  wind_speed: ["M17.7 7.7A2.5 2.5 0 1 1 19.5 12H2", "M9.6 4.6A2 2 0 1 1 11 8H2", "M12.6 19.4A2 2 0 1 0 14 16H2"],
  precipitation: ["M4 14.9A7 7 0 1 1 15.7 8h1.8a4.5 4.5 0 0 1 2.5 8.2", "M16 14v6", "M8 14v6", "M12 16v6"],
};
// Les pièces (mise en page « tableau »).
Object.assign(ICONS, {
  bed: ["M2 18V6", "M2 13h20v5", "M22 13v-1a3 3 0 0 0-3-3h-8v4", "M6.5 9a1.5 1.5 0 1 0 0 3 1.5 1.5 0 1 0 0-3z"],
  sofa: ["M4 11V8a2 2 0 0 1 2-2h12a2 2 0 0 1 2 2v3", "M2 13a2 2 0 0 1 4 0v2h12v-2a2 2 0 0 1 4 0v5H2z", "M5 18v2", "M19 18v2"],
  pot: ["M4 10h16v6a4 4 0 0 1-4 4H8a4 4 0 0 1-4-4z", "M2 10h20", "M9 7c0-1 1-1.5 1-3", "M14 7c0-1 1-1.5 1-3"],
  bath: ["M3 12h18v3a4 4 0 0 1-4 4H7a4 4 0 0 1-4-4z", "M6 12V5a2 2 0 0 1 4 0", "M7 19l-1 2", "M17 19l1 2"],
  stairs: ["M3 20h5v-5h5v-5h5V5h3"],
  toilet: ["M7 3h6v7H7z", "M5 10h14a7 7 0 0 1-14 0z", "M9 16.5L8 21h8l-1-4.5"],
  car: ["M5 15l1.6-5h10.8l1.6 5", "M3 15h18v4H3z", "M6 19v2", "M18 19v2", "M7 17h.01", "M17 17h.01"],
  desk: ["M3 8h18", "M5 8v12", "M19 8v12", "M13 8v6h6", "M15 11h2"],
  washer: ["M5 3h14v18H5z", "M5 7h14", "M12 10a4 4 0 1 0 0 8 4 4 0 1 0 0-8z"],
  tree: ["M12 22v-5", "M12 3l6 8h-3l4 6H5l4-6H6z"],
  // Les boutons des volets (mise en page « tableau »).
  arrowUp: ["M12 19V5", "M6 11l6-6 6 6"],
  arrowDown: ["M12 5v14", "M18 13l-6 6-6-6"],
  square: ["M8 8h8v8H8z"],
  bolt: ["M13 2L4 14h7l-1 8 9-12h-7z"],
});
const BINARY_ICONS = { motion: ICONS.motion, connectivity: ICONS.globe };
