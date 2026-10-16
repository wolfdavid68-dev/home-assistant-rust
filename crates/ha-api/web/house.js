// La mise en page « tableau » (styles plan et maquette) : le dessin de la
// maison, ses calques et ses valeurs, les repères, les boutons rapides, les
// volets en lignes, et le cartouche des styles plan.
//
// Comme les tableaux de bord isométriques de Home Assistant : un fond (de
// jour, et de nuit si on veut), des calques transparents de la même taille
// qui n'apparaissent que si un appareil est dans un certain état (le garage
// ouvert, le porche allumé), et des valeurs posées sur le dessin.
//
// Partage la portée globale de app.js (script classique) : `el`, `api`,
// `entities`, `nameOf`… viennent de là. Pas d'innerHTML ici non plus.
"use strict";

const house = {
  // Les onglets (Maison, Garage…), et celui qui est affiché.
  pages: [homePage()],
  config: null,
  images: [],
  editable: false,
  // Images lues avec le jeton, gardées en mémoire : « nom@date » → blob:…
  urls: new Map(),
  loading: null,
  // Ce qui est dessiné, pour le mettre à jour sans tout refaire.
  view: null,
};
const HOUSE_DEFAULT = "/house.svg";
const MAX_HOUSE_IMAGE = 3 * 1024 * 1024;
const WHEN_LABELS = [["always", "Toujours"], ["day", "Le jour"], ["night", "La nuit"]];
// Les ambiances : le moment, puis le temps qu'il fait. « jour » et « nuit »
// (soleil) sont le fond de jour et le fond de nuit.
const MOMENTS = [["jour", "Jour"], ["nuit", "Nuit"]];
// Tous les temps de la météo, avec le temps « parent » qu'on prend s'il
// manque un fond (orage → pluie → nuageux → soleil).
const WEATHERS = [
  ["soleil", "Soleil", null], ["eclaircies", "Éclaircies", "soleil"], ["nuageux", "Nuageux", "soleil"],
  ["brouillard", "Brouillard", "nuageux"], ["vent", "Vent", "nuageux"], ["pluie", "Pluie", "nuageux"],
  ["averses", "Averses", "pluie"], ["orage", "Orage", "pluie"], ["grele", "Grêle", "orage"],
  ["neige", "Neige", "nuageux"], ["pluie-neige", "Pluie et neige", "neige"],
];
const WEATHER_PARENT = Object.fromEntries(WEATHERS.map(([key, , parent]) => [key, parent]));
const AMBIANCE_KEYS = MOMENTS.flatMap(([moment]) => WEATHERS.filter(([key]) => key !== "soleil").map(([key]) => `${moment}-${key}`));
/** La condition de la météo (Home Assistant) → le temps. */
const CONDITION_WEATHER = {
  "sunny": "soleil", "clear-night": "soleil", "partlycloudy": "eclaircies", "cloudy": "nuageux",
  "fog": "brouillard", "windy": "vent", "windy-variant": "vent", "rainy": "pluie", "pouring": "averses",
  "lightning": "orage", "lightning-rainy": "orage", "hail": "grele", "snowy": "neige",
  "snowy-rainy": "pluie-neige", "exceptional": "nuageux",
};

// ---------------------------------------------------------------- Chargement

function emptyDrawing() {
  return { layers: [], labels: [], actions: [], charts: [], zones: [] };
}

function homePage() {
  return { id: "maison", name: "Maison", areas: [], ...emptyDrawing() };
}

// Les courbes : les couleurs (celles du thème) et les durées.
const CHART_COLORS = [["ambre", "Ambre"], ["orange", "Orange"], ["rouge", "Rouge"], ["vert", "Vert"], ["cyan", "Cyan"], ["bleu", "Bleu"], ["violet", "Violet"], ["rose", "Rose"]];
const CHART_HOURS = [[6, "6 heures"], [24, "24 heures"], [168, "7 jours"]];

/** Les onglets reçus (ou, avant les onglets, un seul dessin). */
function cleanPages(config) {
  const pages = Array.isArray(config?.pages) ? config.pages.map(cleanDrawing) : [cleanDrawing({ id: "maison", name: "Maison", ...config })];
  return pages.length > 0 ? pages : [homePage()];
}

/** Recopie un onglet reçu en ne gardant que ce qu'on attend. */
function cleanDrawing(config) {
  const text = (value) => (typeof value === "string" && value !== "" ? value : undefined);
  const out = { id: text(config?.id) ?? "maison", name: text(config?.name) ?? "Maison", areas: [], ...emptyDrawing() };
  for (const area of Array.isArray(config?.areas) ? config.areas : []) if (text(area)) out.areas.push(area);
  if (text(config?.background)) out.background = config.background;
  if (text(config?.night)) out.night = config.night;
  out.ambiances = {};
  for (const key of AMBIANCE_KEYS) if (text(config?.ambiances?.[key])) out.ambiances[key] = config.ambiances[key];
  if (["jour", "nuit", ...AMBIANCE_KEYS].includes(config?.forced)) out.forced = config.forced;
  for (const layer of Array.isArray(config?.layers) ? config.layers : []) {
    out.layers.push({
      image: text(layer.image) ?? "",
      entity_id: text(layer.entity_id) ?? "",
      states: Array.isArray(layer.states) ? layer.states.filter((s) => typeof s === "string") : [],
      not: layer.not === true,
      when: ["day", "night"].includes(layer.when) ? layer.when : "always",
    });
  }
  for (const label of Array.isArray(config?.labels) ? config.labels : []) {
    const entry = { entity_id: text(label.entity_id) ?? "", x: Number(label.x) || 0, y: Number(label.y) || 0 };
    if (text(label.name)) entry.name = label.name;
    out.labels.push(entry);
  }
  for (const id of Array.isArray(config?.actions) ? config.actions : []) {
    if (text(id)) out.actions.push(id);
  }
  out.charts = [];
  for (const chart of Array.isArray(config?.charts) ? config.charts : []) {
    if (!text(chart.entity_id)) continue;
    const entry = {
      entity_id: chart.entity_id,
      color: CHART_COLORS.some(([key]) => key === chart.color) ? chart.color : "bleu",
      hours: [6, 24, 168].includes(chart.hours) ? chart.hours : 24,
    };
    if (text(chart.name)) entry.name = chart.name;
    out.charts.push(entry);
  }
  for (const zone of Array.isArray(config?.zones) ? config.zones : []) {
    if (!text(zone.entity_id)) continue;
    const entry = { entity_id: zone.entity_id, x: Number(zone.x) || 0, y: Number(zone.y) || 0, w: Number(zone.w) || 1, h: Number(zone.h) || 1 };
    if (text(zone.name)) entry.name = zone.name;
    out.zones.push(entry);
  }
  return out;
}

async function loadHouse(force = false) {
  if (house.loading && !force) return house.loading;
  house.loading = (async () => {
    const { status, data } = await api("GET", "/api/home/drawing");
    if (status !== 200) return;
    house.pages = cleanPages(data.config);
    house.images = Array.isArray(data.images) ? data.images : [];
    house.editable = data.editable === true;
    const used = house.pages.flatMap((page) => [page.background, page.night, ...Object.values(page.ambiances), ...page.layers.map((l) => l.image)]);
    await Promise.all(used.filter(Boolean).map(houseImage));
    selectPage(house.config?.id ?? pageWanted(), false);
  })().catch(() => {});
  return house.loading;
}

/** L'adresse locale (blob:) d'une image : une balise <img> ne sait pas
 *  envoyer le jeton, on la lit donc avec fetch. */
async function houseImage(name) {
  const info = house.images.find((image) => image.name === name);
  if (!info) return null;
  const key = `${name}@${info.updated_at}`;
  if (house.urls.has(key)) return house.urls.get(key);
  const response = await fetch(`/api/home/images/${encodeURIComponent(name)}`, {
    headers: { Authorization: `Bearer ${token.get()}` },
  });
  if (!response.ok) return null;
  const url = URL.createObjectURL(await response.blob());
  // Une ancienne version de la même image n'a plus d'usage.
  for (const [old, oldUrl] of house.urls) {
    if (old.startsWith(`${name}@`)) {
      URL.revokeObjectURL(oldUrl);
      house.urls.delete(old);
    }
  }
  house.urls.set(key, url);
  return url;
}

function houseUrl(name) {
  const info = house.images.find((image) => image.name === name);
  return info ? house.urls.get(`${name}@${info.updated_at}`) ?? null : null;
}

// ---------------------------------------------------------------- Le dessin

/** Fait-il nuit ? (Le soleil, s'il est connu ; sinon, jamais.) */
function houseNight() {
  return entities.get("sun.sun")?.state === "below_horizon";
}

/** Le temps qu'il fait, d'après la condition de la météo. */
function weatherKind() {
  const weather = [...entities.values()].find((s) => domainOf(s) === "weather");
  return lookup(CONDITION_WEATHER, weather?.state) ?? "soleil";
}

/** L'ambiance du moment (« nuit-pluie »…), ou celle imposée par le réglage. */
function ambianceOf(page) {
  if (page.forced) return page.forced;
  const moment = houseNight() ? "nuit" : "jour";
  const weather = weatherKind();
  return weather === "soleil" ? moment : `${moment}-${weather}`;
}

/** Le fond à montrer pour une ambiance : le sien, sinon le plus proche. On
 *  remonte les temps (orage → pluie → nuageux → soleil) au même moment, puis,
 *  la nuit, la même chose de jour. */
function backgroundFor(page, ambiance) {
  const cut = ambiance.indexOf("-");
  const moment = cut < 0 ? ambiance : ambiance.slice(0, cut);
  const weather = cut < 0 ? "soleil" : ambiance.slice(cut + 1);
  const chain = (when) => {
    const names = [];
    for (let w = weather; w; w = WEATHER_PARENT[w]) {
      names.push(w === "soleil" ? (when === "jour" ? page.background : page.night) : page.ambiances[`${when}-${w}`]);
    }
    return names;
  };
  const candidates = moment === "nuit" ? [...chain("nuit"), ...chain("jour")] : chain("jour");
  return candidates.find((name) => name && houseUrl(name)) ?? page.background;
}

function layerShown(layer, night) {
  const state = entities.get(layer.entity_id);
  if (!state) return false;
  if (layer.states.includes(state.state) === layer.not) return false;
  return layer.when === "always" || (layer.when === "night") === night;
}

/** « 21,4 °C », « 1 513 VA », « Ouvert »… */
function labelValue(state) {
  if (!state) return "—";
  if (/^-?\d+(\.\d+)?$/.test(state.state)) {
    const unit = state.attributes.unit_of_measurement;
    return `${number.format(Number(state.state))}${typeof unit === "string" ? ` ${unit}` : ""}`;
  }
  if (domainOf(state) === "climate" && finite(state.attributes.current_temperature)) {
    return `${number.format(state.attributes.current_temperature)} °C`;
  }
  if (domainOf(state) === "cover") return agree(state, lookup(COVER_LABELS, state.state) ?? state.state);
  return statusText(state);
}

/**
 * Dessine l'assemblage `config` dans `stage` : le fond, la nuit, les
 * calques, les valeurs. Renvoie de quoi les mettre à jour.
 */
function drawHouse(stage, config, { interactive }) {
  const base = el("img", { class: "house-base", alt: "La maison", draggable: "false" });
  const custom = config.background ? houseUrl(config.background) : null;
  base.src = custom ?? HOUSE_DEFAULT;
  stage.classList.toggle("default-drawing", custom === null);
  const parts = [base];
  // L'ambiance (nuit, pluie, neige…) : une image de la même taille, posée
  // sur le fond, qui change en fondu.
  let ambiance = null;
  if (custom && (config.night || Object.keys(config.ambiances).length > 0)) {
    ambiance = el("img", { class: "house-layer house-ambiance off", alt: "", draggable: "false" });
    parts.push(ambiance);
  }
  const layers = config.layers.map((layer) => {
    const url = houseUrl(layer.image);
    const img = el("img", { class: "house-layer", alt: "", draggable: "false" });
    if (url) img.src = url;
    parts.push(img);
    return { layer, img };
  });
  // Les zones à toucher : sous les valeurs, pour que celles-ci restent
  // cliquables. Dans l'aperçu du réglage, elles sont dessinées en pointillé.
  const zones = config.zones.map((zone, index) => {
    const node = interactive
      ? el("button", { type: "button", class: "house-zone" })
      : el("span", { class: "house-zone edit", text: String(index + 1) });
    placeZone(node, zone);
    if (interactive) zoneTouch(zone, node);
    parts.push(node);
    return { zone, node };
  });
  const labels = config.labels.map((label) => {
    const value = el("strong");
    const name = el("span", { class: "house-label-name" });
    const tag = el("span", { class: "house-tag" }, name, value);
    const node = el(interactive ? "button" : "span", { class: "house-label" }, tag);
    if (interactive) node.type = "button";
    node.style.left = `${label.x}%`;
    node.style.top = `${label.y}%`;
    if (interactive) {
      node.addEventListener("click", () => {
        if (account.permissions.history !== false && entities.has(label.entity_id)) openHistory(label.entity_id);
      });
    }
    parts.push(node);
    return { label, name, value, node };
  });
  stage.replaceChildren(...parts);
  const view = { stage, config, ambiance, layers, zones, labels };
  // La taille du dessin n'est connue qu'une fois l'image chargée : ses
  // proportions servent à le faire tenir dans la hauteur de l'écran.
  base.addEventListener("load", () => {
    if (base.naturalWidth > 0 && base.naturalHeight > 0) stage.style.setProperty("--ratio", String(base.naturalWidth / base.naturalHeight));
    fitLabels(view);
  });
  return view;
}

function updateHouse(view) {
  if (!view) return;
  const isNight = houseNight();
  view.stage.classList.toggle("is-night", isNight);
  if (view.ambiance) {
    const chosen = backgroundFor(view.config, ambianceOf(view.config));
    const url = chosen && chosen !== view.config.background ? houseUrl(chosen) : null;
    if (url && view.ambiance.getAttribute("src") !== url) view.ambiance.src = url;
    view.ambiance.classList.toggle("off", url === null);
  }
  for (const { layer, img } of view.layers) img.classList.toggle("off", !layerShown(layer, isNight));
  for (const { zone, node } of view.zones) {
    if (node.tagName !== "BUTTON") continue;
    const state = entities.get(zone.entity_id);
    const name = zone.name ?? (state ? nameOf(state) : zone.entity_id);
    const command = state ? actionCommand(state) : null;
    const readonly = command === null || !canControl(state);
    node.classList.toggle("readonly", readonly);
    node.classList.toggle("on", Boolean(state) && ["on", "open", "opening", "unlocked"].includes(state.state));
    const what = readonly ? "" : command[1] === "turn_off" ? " Toucher pour éteindre." : command[1] === "turn_on" ? (["scene", "script"].includes(command[0]) ? " Toucher pour lancer." : " Toucher pour allumer.")
      : command[1] === "close_cover" ? " Toucher pour fermer." : command[1] === "open_cover" ? " Toucher pour ouvrir." : " Toucher pour appuyer.";
    node.setAttribute("aria-label", `${name} : ${labelValue(state)}.${what}`);
    node.title = name;
    if (node.dataset.state !== state?.state) node.classList.remove("pending");
    node.dataset.state = state?.state ?? "";
  }
  for (const { label, name, value, node } of view.labels) {
    const state = entities.get(label.entity_id);
    name.textContent = label.name ?? (state ? nameOf(state) : label.entity_id);
    value.textContent = labelValue(state);
    node.setAttribute("aria-label", `${name.textContent} : ${value.textContent}`);
  }
  fitLabels(view);
}

function placeZone(node, zone) {
  node.style.left = `${zone.x}%`;
  node.style.top = `${zone.y}%`;
  node.style.width = `${zone.w}%`;
  node.style.height = `${zone.h}%`;
}

// Un toucher commande l'appareil (comme les boutons rapides) ; un appui long,
// ou le clic droit, ouvre son historique.
const LONG_PRESS = 550;

function zoneHistory(zone) {
  if (account.permissions.history !== false && entities.has(zone.entity_id)) openHistory(zone.entity_id);
}

function zoneTouch(zone, node) {
  let timer = 0;
  let long = false;
  node.addEventListener("pointerdown", (event) => {
    if (event.button !== 0) return;
    long = false;
    clearTimeout(timer);
    timer = setTimeout(() => { long = true; zoneHistory(zone); }, LONG_PRESS);
  });
  for (const type of ["pointerup", "pointerleave", "pointercancel"]) node.addEventListener(type, () => clearTimeout(timer));
  node.addEventListener("contextmenu", (event) => {
    event.preventDefault();
    clearTimeout(timer);
    if (!long) { long = true; zoneHistory(zone); }
  });
  node.addEventListener("click", () => {
    if (long) { long = false; return; }
    const state = entities.get(zone.entity_id);
    if (!state) return;
    const command = actionCommand(state);
    // Un capteur, ou un appareil que ce compte ne commande pas : l'historique.
    if (command === null || !canControl(state)) return zoneHistory(zone);
    node.classList.remove("hit");
    void node.offsetWidth; // relance l'animation
    node.classList.add("hit", "pending");
    setTimeout(() => node.classList.remove("hit"), 700);
    setTimeout(() => node.classList.remove("pending"), 5000);
    callService(command[0], command[1], { entity_id: state.entity_id });
  });
}

/** Une étiquette qui dépasserait du dessin passe de l'autre côté de son point. */
function fitLabels(view) {
  const frame = view.stage.getBoundingClientRect();
  if (frame.width === 0) return; // pas encore affiché
  for (const { label, node } of view.labels) {
    node.classList.toggle("left", label.x > 50);
    node.classList.toggle("below", label.y < 18);
    const tag = node.firstElementChild.getBoundingClientRect();
    if (tag.right > frame.right) node.classList.add("left");
    else if (tag.left < frame.left) node.classList.remove("left");
    if (tag.top < frame.top) node.classList.add("below");
    else if (tag.bottom > frame.bottom) node.classList.remove("below");
  }
}

// ---------------------------------------------------------------- Onglets
// Maison, Garage, Piscine… : chacun son dessin, ses boutons, et les pièces
// qu'il montre. L'onglet ouvert est retenu sur cet appareil (localStorage) :
// la tablette du garage reste sur « Garage ».
const PAGE_KEY = "ha-page";

function pageWanted() {
  try {
    return localStorage.getItem(PAGE_KEY) ?? "";
  } catch {
    return "";
  }
}

/** Affiche un onglet : son dessin, et seulement ses pièces (pageAreas, lu
 *  par app.js). */
function selectPage(id, remember = true) {
  house.config = house.pages.find((page) => page.id === id) ?? house.pages[0];
  pageAreas = house.config.areas.length > 0 ? new Set(house.config.areas) : null;
  if (remember) {
    try {
      localStorage.setItem(PAGE_KEY, house.config.id);
    } catch {
      // Stockage bloqué : l'onglet ne sera pas retenu.
    }
  }
  buildHouse();
  renderTabs();
  if (!$("#dashboard-view").hidden) renderAll();
}

function renderTabs() {
  const nav = $("#board-tabs");
  nav.replaceChildren(...house.pages.map((page) => {
    const current = page.id === house.config?.id;
    const tab = el("button", { type: "button", class: current ? "board-tab-item active" : "board-tab-item", "aria-current": current ? "page" : "false", text: page.name });
    tab.addEventListener("click", () => {
      if (page.id !== house.config?.id) selectPage(page.id);
    });
    return tab;
  }));
  // Un seul onglet : rien à choisir sur téléphone.
  nav.classList.toggle("single", house.pages.length === 1);
}

function buildHouse() {
  house.config ??= house.pages[0];
  house.view = drawHouse($("#house-stage"), house.config, { interactive: true });
  updateHouse(house.view);
  houseCaption();
  if (isBoard()) {
    renderActions();
    renderCovers();
  }
}

function houseCaption() {
  const caption = $("#house-caption");
  const left = el("span", {}, el("b", { text: "Vue isométrique" }), house.config.background ? "" : " · dessin d'exemple");
  const parts = [left];
  if (house.editable) {
    const edit = el("button", { type: "button", class: "ghost small", text: house.config.background ? "Régler" : "Mettre ma maison" });
    edit.addEventListener("click", openDrawingEditor);
    parts.push(edit);
  }
  caption.replaceChildren(...parts);
}

// ---------------------------------------------------------------- Repères

function gauge(icon, label, value, kind) {
  return el("span", { class: `gauge g-${kind}`, title: label }, svgIcon(icon), el("span", { text: label }), el("strong", { text: value }));
}

function renderGauges() {
  const box = $("#gauges");
  const items = [];
  const weather = [...entities.values()].find((s) => domainOf(s) === "weather");
  const outside = entities.get("sensor.meteo_temperature");
  const outsideTemp = finite(weather?.attributes.temperature) ? weather.attributes.temperature
    : outside && Number.isFinite(Number(outside.state)) ? Number(outside.state) : NaN;
  if (Number.isFinite(outsideTemp)) items.push(gauge(ICONS.temperature, "Dehors", `${number.format(outsideTemp)} °C`, "outside"));
  const temps = areas.map(roomInfo).map((info) => info.temperature).filter((t) => Number.isFinite(t));
  if (temps.length > 0) {
    items.push(gauge(ICONS.home, "Dedans", `${number.format(temps.reduce((a, b) => a + b, 0) / temps.length)} °C`, "inside"));
  }
  const power = entities.get("sensor.linky_puissance");
  if (power && Number.isFinite(Number(power.state))) {
    items.push(gauge(ICONS.flame, "Puissance", `${number.format(Number(power.state))} ${power.attributes.unit_of_measurement ?? "VA"}`, "power"));
  }
  const lights = [...entities.values()].filter((s) => domainOf(s) === "light" && s.state === "on").length;
  items.push(gauge(ICONS.bulb, "Lumières", lights === 0 ? "éteintes" : `${lights} allumée${lights > 1 ? "s" : ""}`, "lights"));
  const zone = entities.get("zone.home");
  if (Array.isArray(zone?.attributes.persons)) {
    const count = zone.attributes.persons.length;
    items.push(gauge(ICONS.person, "Présents", String(count), "people"));
  }
  box.hidden = items.length === 0;
  box.replaceChildren(...items);
}

// ---------------------------------------------------------------- Cartouche

function renderTitleBlock() {
  const who = account.user ? `${account.user.charAt(0).toUpperCase()}${account.user.slice(1)}` : "";
  const cell = (term, value, wide = false) =>
    el("div", { class: wide ? "wide" : "" }, el("dt", { text: term }), el("dd", { text: value }));
  const devices = [...entities.values()].filter((s) => !["persistent_notification", "zone", "sun"].includes(domainOf(s))).length;
  $("#title-block").replaceChildren(
    el("dl", {},
      cell("Projet", who ? `Maison de ${who}` : "Maison", true),
      cell("Plan", "Vue d'ensemble, en temps réel"),
      cell("Pièces · appareils", `${areas.length} · ${devices}`),
      cell("Date", fullDate.format(new Date())),
      cell("Échelle", "N.D. — conceptuel"),
      cell("Feuille", "1 / 1")),
  );
}

// ---------------------------------------------------------------- Boutons rapides
// Sous le dessin : le porche, le parking, les portes de garage… choisis dans
// le réglage. Un toucher allume, éteint, ouvre ou ferme.

const ACTION_ICONS = {
  light: ICONS.bulb, switch: ICONS.plug, fan: ICONS.fan, cover: ICONS.blinds, lock: ICONS.lock,
  scene: ICONS.sparkle, script: ICONS.play, input_boolean: ICONS.toggle, button: ICONS.button,
  input_button: ICONS.button, siren: ICONS.siren,
};

/** Le service d'un toucher : [domaine, service], ou null (lecture seule). */
function actionCommand(state) {
  const domain = domainOf(state);
  const on = state.state === "on";
  if (["light", "switch", "fan", "input_boolean", "siren"].includes(domain)) return [domain, on ? "turn_off" : "turn_on"];
  if (domain === "cover") return ["cover", ["open", "opening"].includes(state.state) ? "close_cover" : "open_cover"];
  if (domain === "scene" || domain === "script") return [domain, "turn_on"];
  if (domain === "button" || domain === "input_button") return [domain, "press"];
  return null;
}

function coverText(state) {
  const position = state.attributes.current_position;
  if (typeof position === "number") {
    return agree(state, position >= 100 ? "Ouvert" : position <= 0 ? "Fermé" : `Ouvert à ${Math.round(position)}\u202f%`);
  }
  return agree(state, lookup(COVER_LABELS, state.state) ?? statusText(state));
}

function actionChip(state) {
  const domain = domainOf(state);
  const garage = domain === "cover" && ALERT_COVERS.includes(state.attributes.device_class);
  const icon = garage ? ICONS.car : lookup(ACTION_ICONS, domain) ?? ICONS.toggle;
  const status = domain === "cover" ? coverText(state) : statusText(state);
  const active = ["on", "open", "opening", "unlocked"].includes(state.state);
  const command = actionCommand(state);
  const chip = el("button", { type: "button", class: `house-action d-${garage ? "garage" : domain}${active ? " on" : ""}` },
    el("span", { class: "house-action-icon" }, svgIcon(icon)),
    el("span", { class: "house-action-text" }, el("b", { text: nameOf(state) }), el("span", { text: status })));
  chip.setAttribute("aria-label", `${nameOf(state)} : ${status}`);
  if (command === null || !canControl(state)) {
    chip.disabled = true;
    chip.classList.add("readonly");
    return chip;
  }
  chip.addEventListener("click", () => {
    chip.classList.add("pending");
    setTimeout(() => chip.classList.remove("pending"), 5000);
    callService(command[0], command[1], { entity_id: state.entity_id });
  });
  return chip;
}

function renderActions() {
  const box = $("#house-actions");
  const states = house.config.actions.map((id) => entities.get(id)).filter((state) => state !== undefined);
  box.hidden = states.length === 0;
  box.replaceChildren(...states.map(actionChip));
}

// ---------------------------------------------------------------- Volets en lignes

/** Le nom d'un volet dans sa ligne : celui de sa pièce (« Salon »), ou
 *  « Salon · Baie » s'il y en a plusieurs dans la pièce. */
function coverTitle(state) {
  const area = areas.find((a) => a.entities.some((e) => e.entity_id === state.entity_id));
  if (!area) return nameOf(state);
  const covers = area.entities.filter((e) => e.entity_id.startsWith("cover."));
  return covers.length > 1 ? `${area.name} · ${nameOf(state)}` : area.name;
}

function coverRow(state) {
  const buttons = [["open_cover", ICONS.arrowUp, "Ouvrir"], ["stop_cover", ICONS.square, "Arrêter"], ["close_cover", ICONS.arrowDown, "Fermer"]]
    .map(([service, icon, label]) => {
      const button = el("button", { type: "button", class: "round", "aria-label": `${label} ${coverTitle(state)}` }, svgIcon(icon));
      button.addEventListener("click", () => {
        row.classList.add("pending");
        setTimeout(() => row.classList.remove("pending"), 5000);
        callService("cover", service, { entity_id: state.entity_id });
      });
      return button;
    });
  const row = el("div", { class: `cover-row${state.state === "open" ? " on" : ""}` },
    el("span", { class: "house-action-icon" }, svgIcon(ICONS.blinds)),
    el("span", { class: "house-action-text" }, el("b", { text: coverTitle(state) }), el("span", { text: coverText(state) })),
    canControl(state) ? el("span", { class: "cover-row-buttons" }, ...buttons) : "");
  return row;
}

/** Les volets (pas les portes de garage ni les portails, ni ceux déjà en
 *  boutons rapides), en lignes sous le dessin. */
function renderCovers() {
  const box = $("#board-covers");
  const inPage = pageAreas === null ? null
    : new Set(areas.filter((a) => pageAreas.has(a.name)).flatMap((a) => a.entities.map((e) => e.entity_id)));
  const covers = [...entities.values()]
    .filter((s) => domainOf(s) === "cover" && !ALERT_COVERS.includes(s.attributes.device_class)
      && !house.config.actions.includes(s.entity_id) && (inPage === null || inPage.has(s.entity_id)))
    .sort((a, b) => nameOf(a).localeCompare(nameOf(b), "fr"));
  box.hidden = covers.length === 0;
  box.replaceChildren(el("h2", { text: "Volets" }), el("div", { class: "cover-rows" }, ...covers.map(coverRow)));
}

// ---------------------------------------------------------------- Courbes
// Dans la colonne de gauche : la puissance du NAS sur 24 h, la température
// de la cave sur 7 jours… L'historique est relu toutes les 5 minutes ; la
// valeur du moment, elle, suit l'état en direct.
const SERIES_TTL = 5 * 60_000;
const series = new Map(); // « entité|heures » → { at, points, loading }

function seriesFor(chart) {
  const key = `${chart.entity_id}|${chart.hours}`;
  const cached = series.get(key);
  if (account.permissions.history !== false && (!cached || (!cached.loading && Date.now() - cached.at > SERIES_TTL))) {
    const entry = { at: Date.now(), points: cached?.points ?? null, loading: true };
    series.set(key, entry);
    api("GET", `/api/history/${encodeURIComponent(chart.entity_id)}/series?hours=${chart.hours}`)
      .then(({ status, data }) => {
        entry.loading = false;
        if (status !== 200 || !Array.isArray(data.numeric)) return;
        entry.points = data.numeric
          .map((p) => [Date.parse(p.at), Number(p.mean)])
          .filter(([t, v]) => Number.isFinite(t) && Number.isFinite(v));
        if (isBoard()) renderCharts();
      })
      .catch(() => { entry.loading = false; });
  }
  return series.get(key)?.points ?? null;
}

/** Une courbe en SVG : une ligne, et un dégradé dessous. */
function sparkline(chart, points) {
  const W = 300;
  const H = 64;
  const box = svg("svg", { viewBox: `0 0 ${W} ${H}`, preserveAspectRatio: "none", class: "chart-line", "aria-hidden": "true" });
  const state = entities.get(chart.entity_id);
  const now = Date.now();
  const all = [...points];
  if (state && Number.isFinite(Number(state.state))) all.push([now, Number(state.state)]);
  if (all.length < 2) return box;
  const since = now - chart.hours * 3_600_000;
  const values = all.map(([, v]) => v);
  let min = Math.min(...values);
  let max = Math.max(...values);
  if (max - min < 1e-9) { min -= 1; max += 1; }
  const x = (t) => ((Math.max(t, since) - since) / (now - since)) * W;
  const y = (v) => H - 4 - ((v - min) / (max - min)) * (H - 10);
  const line = all.map(([t, v], i) => `${i === 0 ? "M" : "L"}${x(t).toFixed(1)},${y(v).toFixed(1)}`).join(" ");
  const id = `chart-fill-${chart.entity_id.replace(/[^a-z0-9]/gi, "-")}-${chart.hours}`;
  const gradient = svg("linearGradient", { id, x1: 0, y1: 0, x2: 0, y2: 1 });
  gradient.append(svg("stop", { offset: "0%", "stop-color": "currentColor", "stop-opacity": "0.35" }), svg("stop", { offset: "100%", "stop-color": "currentColor", "stop-opacity": "0" }));
  const defs = svg("defs");
  defs.append(gradient);
  box.append(
    defs,
    svg("path", { d: `${line} L${x(all.at(-1)[0]).toFixed(1)},${H} L${x(all[0][0]).toFixed(1)},${H} Z`, fill: `url(#${id})`, stroke: "none" }),
    svg("path", { d: line, fill: "none", stroke: "currentColor", "stroke-width": "1.8", "vector-effect": "non-scaling-stroke", "stroke-linejoin": "round" }),
  );
  return box;
}

function renderCharts() {
  const box = $("#board-charts");
  const charts = house.config?.charts ?? [];
  box.hidden = charts.length === 0;
  box.replaceChildren(...charts.map((chart) => {
    const state = entities.get(chart.entity_id);
    const name = chart.name ?? (state ? nameOf(state) : chart.entity_id);
    const points = seriesFor(chart);
    const period = CHART_HOURS.find(([hours]) => hours === chart.hours)?.[1] ?? "";
    const card = el("figure", { class: `chart-card chart-${chart.color}` },
      el("figcaption", {},
        el("span", { class: "house-action-icon chart-icon" }, svgIcon(ICONS.chart)),
        el("span", { class: "house-action-text" }, el("b", { text: name }), el("span", { text: labelValue(state) })),
        el("span", { class: "chart-period", text: period })),
      points === null && account.permissions.history === false
        ? el("p", { class: "auto-meta", text: "Historique réservé" })
        : sparkline(chart, points ?? []));
    if (account.permissions.history !== false && state) {
      card.tabIndex = 0;
      card.setAttribute("role", "button");
      card.setAttribute("aria-label", `${name} : ${labelValue(state)}, ${period}. Ouvrir l'historique.`);
      const open = () => openHistory(chart.entity_id);
      card.addEventListener("click", open);
      card.addEventListener("keydown", (event) => { if (event.key === "Enter") open(); });
    }
    return card;
  }));
}

// ---------------------------------------------------------------- Personnes
// En tête de la colonne des pièces : chacun, avec l'initiale de son nom et où
// il est (« Maison », « Absent », ou le nom de la zone).

function renderPeople() {
  const box = $("#board-people");
  const people = [...entities.values()]
    .filter((s) => domainOf(s) === "person")
    .sort((a, b) => nameOf(a).localeCompare(nameOf(b), "fr"));
  box.hidden = people.length === 0;
  box.replaceChildren(...people.map((state) => {
    const home = state.state === "home";
    const where = home ? "Maison" : state.state === "not_home" ? "Absent" : lookup(STATUS_LABELS, state.state) ?? state.state;
    const name = nameOf(state);
    return el("div", { class: `person-chip${home ? " home" : ""}` },
      el("span", { class: "person-avatar", text: name.charAt(0).toUpperCase(), "aria-hidden": "true" }),
      el("span", { class: "house-action-text" }, el("b", { text: name }), el("span", { text: where })));
  }));
}

/** Appelé par app.js à chaque changement (au plus une fois par image). */
function renderHouse() {
  const board = isBoard();
  $("#house").hidden = !board;
  $("#title-block").hidden = !isPlan();
  if (!board) {
    $("#board-covers").hidden = true;
    $("#board-people").hidden = true;
    $("#board-charts").hidden = true;
    return;
  }
  if (house.loading === null && token.get()) loadHouse();
  if (house.config === null) return;
  updateHouse(house.view);
  renderGauges();
  renderActions();
  renderCovers();
  renderPeople();
  renderCharts();
  if (isPlan()) renderTitleBlock();
}

// ---------------------------------------------------------------- Apparence

$("#theme").value = themeWanted();
$("#theme").addEventListener("change", (event) => {
  const choice = THEMES.includes(event.target.value) ? event.target.value : "auto";
  try {
    if (choice === "auto") localStorage.removeItem(THEME_KEY);
    else localStorage.setItem(THEME_KEY, choice);
  } catch {
    // Stockage bloqué : l'apparence ne durera que le temps de la page.
  }
  applyTheme(choice);
  if (!$("#dashboard-view").hidden) renderHome();
});

// ---------------------------------------------------------------- Réglage
// Pour l'administrateur : les images, les fonds, les calques et les valeurs.
// Rien n'est enregistré avant « Enregistrer », sauf l'envoi d'une image.

// `pages` : la copie de tous les onglets ; `draft` : celui qu'on règle.
const drawingEdit = { pages: null, draft: null, placing: -1, tracing: -1, view: null };

async function openDrawingEditor() {
  $("#account-dialog").close();
  await loadHouse(true);
  await Promise.all(house.images.map((image) => houseImage(image.name)));
  drawingEdit.pages = structuredClone(house.pages);
  drawingEdit.draft = drawingEdit.pages.find((page) => page.id === house.config?.id) ?? drawingEdit.pages[0];
  drawingEdit.placing = -1;
  drawingEdit.tracing = -1;
  drawingStatus("");
  renderDrawingEditor();
  $("#drawing").showModal();
}

function drawingStatus(text, kind = "") {
  const status = $("#drawing-status");
  status.textContent = text;
  status.className = `auto-status ${kind}`;
}

function renderDrawingPreview() {
  drawingEdit.view = drawHouse($("#drawing-preview"), drawingEdit.draft, { interactive: false });
  updateHouse(drawingEdit.view);
  drawingEdit.view.labels[drawingEdit.placing]?.node.classList.add("placing");
  drawingEdit.view.zones[drawingEdit.tracing]?.node.classList.add("placing");
  const hint = $("#drawing-place-hint");
  hint.hidden = drawingEdit.placing < 0 && drawingEdit.tracing < 0;
  hint.textContent = drawingEdit.tracing >= 0 ? "Fais glisser sur le dessin pour tracer la zone (du coin en haut à gauche au coin opposé)." : "Touche le dessin là où poser la valeur.";
  $("#drawing-preview").classList.toggle("placing", drawingEdit.placing >= 0 || drawingEdit.tracing >= 0);
}

/** En mode « placer », l'aperçu remonte en vue pour qu'on puisse le toucher. */
function showPreview() {
  if (drawingEdit.placing < 0 && drawingEdit.tracing < 0) return;
  const smooth = !matchMedia("(prefers-reduced-motion: reduce)").matches;
  $("#drawing-preview").scrollIntoView({ block: "nearest", behavior: smooth ? "smooth" : "auto" });
}

function imageOptions(selected, empty) {
  const select = el("select");
  if (empty) select.append(el("option", { value: "", text: empty }));
  for (const image of house.images) select.append(el("option", { value: image.name, text: image.name }));
  select.value = selected ?? "";
  return select;
}

function drawingFieldset(title, hint, ...children) {
  return el("fieldset", { class: "drawing-part" }, el("legend", { text: title }), hint ? el("p", { class: "auto-meta", text: hint }) : "", ...children);
}

function renderDrawingEditor() {
  const draft = drawingEdit.draft;
  renderDrawingPreview();
  const refresh = () => renderDrawingEditor();

  // Les images.
  const list = el("ul", { class: "drawing-images" }, ...house.images.map((image) => {
    const remove = el("button", { type: "button", class: "ghost small danger", text: "Retirer" });
    remove.addEventListener("click", async () => {
      const response = await fetch(`/api/home/images/${encodeURIComponent(image.name)}`, {
        method: "DELETE", headers: { Authorization: `Bearer ${token.get()}` },
      });
      const data = await response.json().catch(() => ({}));
      if (!response.ok) return drawingStatus(data.error ?? "Retrait impossible.", "error");
      await loadHouse(true);
      drawingStatus(`Image « ${image.name} » retirée.`, "ok");
      refresh();
    });
    const size = `${Math.max(1, Math.round(image.size / 1024))} Kio`;
    return el("li", {}, el("code", { text: image.name }), el("span", { class: "auto-meta", text: size }), remove);
  }));
  const nameInput = el("input", { type: "text", placeholder: "ex. jour, nuit, garage_ouvert", maxlength: "40", autocapitalize: "none", spellcheck: "false", "aria-label": "Nom de l'image" });
  const fileInput = el("input", { type: "file", accept: "image/png,image/jpeg,image/webp", "aria-label": "Fichier de l'image" });
  fileInput.addEventListener("change", () => {
    if (nameInput.value === "" && fileInput.files[0]) nameInput.value = slug(fileInput.files[0].name.replace(/\.[a-z0-9]+$/i, "")).replaceAll(".", "_").slice(0, 40);
  });
  const send = el("button", { type: "button", class: "ghost small", text: "Envoyer" });
  send.addEventListener("click", async () => {
    const file = fileInput.files[0];
    const name = nameInput.value.trim();
    if (!file) return drawingStatus("Choisis d'abord une image (PNG, JPEG ou WebP).", "error");
    if (!/^[a-z0-9_-]{1,40}$/.test(name)) return drawingStatus("Nom : lettres minuscules, chiffres, _ et - (40 au plus).", "error");
    if (file.size > MAX_HOUSE_IMAGE) return drawingStatus("Image trop grosse : 3 Mio au plus (exporte-la en WebP, qualité 80).", "error");
    send.disabled = true;
    drawingStatus("Envoi…");
    const response = await fetch(`/api/home/images/${encodeURIComponent(name)}`, {
      method: "PUT",
      headers: { Authorization: `Bearer ${token.get()}`, "Content-Type": file.type || "application/octet-stream" },
      body: file,
    }).catch(() => null);
    send.disabled = false;
    const data = await response?.json().catch(() => ({}));
    if (!response?.ok) return drawingStatus(data?.error ?? "Envoi impossible.", "error");
    await loadHouse(true);
    await houseImage(name);
    // Première image : elle devient le fond.
    if (!draft.background) draft.background = name;
    drawingStatus(`Image « ${name} » enregistrée.`, "ok");
    refresh();
  });
  const images = drawingFieldset(
    "Images",
    "PNG, JPEG ou WebP, 3 Mio au plus. Les calques doivent avoir exactement la taille du fond, avec de la transparence (PNG ou WebP).",
    list,
    el("div", { class: "drawing-row" }, nameInput, fileInput, send),
  );

  // Les fonds : un par ambiance (moment × temps). Le jour au soleil est le
  // fond principal ; les autres sont facultatifs.
  const groups = [];
  for (const [moment, momentLabel] of MOMENTS) {
    const cells = [];
    for (const [weather, weatherLabel] of WEATHERS) {
      const key = weather === "soleil" ? moment : `${moment}-${weather}`;
      const main = key === "jour";
      const current = main ? draft.background : key === "nuit" ? draft.night : draft.ambiances[key];
      const select = imageOptions(current, main ? "Dessin d'exemple" : "Le plus proche");
      select.disabled = !main && !draft.background;
      select.addEventListener("change", () => {
        const value = select.value || undefined;
        if (main) {
          if (value) draft.background = value;
          else { delete draft.background; delete draft.night; draft.ambiances = {}; delete draft.forced; }
        } else if (key === "nuit") {
          if (value) draft.night = value; else delete draft.night;
        } else if (value) {
          draft.ambiances[key] = value;
        } else {
          delete draft.ambiances[key];
        }
        refresh();
      });
      cells.push(field(weatherLabel, select));
    }
    groups.push(el("div", { class: "drawing-ambiance-group" }, el("b", { text: momentLabel }), el("div", { class: "drawing-ambiances" }, ...cells)));
  }
  const forced = el("select", { "aria-label": "Ambiance affichée" },
    el("option", { value: "", text: "Automatique (météo et soleil)" }),
    ...MOMENTS.flatMap(([moment, momentLabel]) => WEATHERS.map(([weather, weatherLabel]) =>
      el("option", { value: weather === "soleil" ? moment : `${moment}-${weather}`, text: `Toujours : ${momentLabel.toLowerCase()}, ${weatherLabel.toLowerCase()}` }))));
  forced.value = draft.forced ?? "";
  forced.disabled = !draft.background;
  forced.addEventListener("change", () => {
    if (forced.value) draft.forced = forced.value; else delete draft.forced;
    renderDrawingPreview();
  });
  const nowKey = ambianceOf({ ...draft, forced: undefined });
  const nowWeather = nowKey.includes("-") ? nowKey.slice(nowKey.indexOf("-") + 1) : "soleil";
  const nowText = `Maintenant : ${nowKey.startsWith("nuit") ? "nuit" : "jour"}, ${(WEATHERS.find(([key]) => key === nowWeather)?.[1] ?? nowWeather).toLowerCase()}`;
  const backgrounds = drawingFieldset(
    "Fonds et ambiances",
    "Comme sur les tableaux de domo-blog : un fond par ambiance, choisi d'après le soleil et la météo. Il en manque un ? Le plus proche s'affiche (orage → pluie → nuageux → soleil ; la nuit, puis le même temps de jour).",
    ...groups,
    el("div", { class: "drawing-grid" }, field("Ambiance affichée", forced)),
    el("p", { class: "auto-meta", text: nowText }),
  );

  // Les calques.
  const layerRows = draft.layers.map((layer, index) => {
    const image = imageOptions(layer.image, null);
    image.addEventListener("change", () => { layer.image = image.value; renderDrawingPreview(); });
    const entity = el("input", { type: "text", list: "entity-list", value: layer.entity_id, placeholder: "cover.garage", autocapitalize: "none", spellcheck: "false" });
    entity.addEventListener("change", () => { layer.entity_id = entity.value.trim(); renderDrawingPreview(); });
    const states = el("input", { type: "text", value: layer.states.join(", "), placeholder: "open, opening" });
    states.addEventListener("change", () => {
      layer.states = states.value.split(",").map((s) => s.trim()).filter(Boolean);
      renderDrawingPreview();
    });
    const not = el("input", { type: "checkbox" });
    not.checked = layer.not;
    not.addEventListener("change", () => { layer.not = not.checked; renderDrawingPreview(); });
    const when = el("select", {}, ...WHEN_LABELS.map(([value, text]) => el("option", { value, text })));
    when.value = layer.when;
    when.addEventListener("change", () => { layer.when = when.value; renderDrawingPreview(); });
    const remove = el("button", { type: "button", class: "ghost small danger", text: "Retirer" });
    remove.addEventListener("click", () => { draft.layers.splice(index, 1); refresh(); });
    const shown = layerShown(layer, houseNight());
    return el("li", { class: "drawing-item" },
      el("div", { class: "drawing-item-head" }, el("b", { text: `Calque ${index + 1}` }),
        el("span", { class: shown ? "badge ok" : "badge", text: shown ? "visible maintenant" : "caché maintenant" }), remove),
      el("div", { class: "drawing-grid" },
        field("Image", image), field("Appareil", entity),
        field("Quand son état est (séparés par des virgules)", states), field("Moment", when)),
      el("label", { class: "check" }, not, " Sauf ces états (visible pour tous les autres)"));
  });
  const addLayer = el("button", { type: "button", class: "ghost small", text: "+ Ajouter un calque" });
  addLayer.disabled = house.images.length === 0;
  addLayer.addEventListener("click", () => {
    draft.layers.push({ image: house.images.at(-1)?.name ?? "", entity_id: "", states: ["on"], not: false, when: "always" });
    refresh();
  });
  const layers = drawingFieldset(
    "Calques",
    "Ex. : l'image garage_ouvert quand cover.garage est open ou opening ; porche_allume quand light.porche est on, la nuit.",
    el("ol", { class: "drawing-items" }, ...layerRows),
    addLayer,
  );

  // Les valeurs posées sur le dessin.
  const labelRows = draft.labels.map((label, index) => {
    const entity = el("input", { type: "text", list: "entity-list", value: label.entity_id, placeholder: "sensor.temperature_salon", autocapitalize: "none", spellcheck: "false" });
    entity.addEventListener("change", () => { label.entity_id = entity.value.trim(); renderDrawingPreview(); });
    const name = el("input", { type: "text", value: label.name ?? "", placeholder: "(le nom de l'appareil)", maxlength: "60" });
    name.addEventListener("change", () => {
      if (name.value.trim()) label.name = name.value.trim();
      else delete label.name;
      renderDrawingPreview();
    });
    const place = el("button", { type: "button", class: drawingEdit.placing === index ? "primary small" : "ghost small", text: drawingEdit.placing === index ? "Touche le dessin…" : `Placer (${number.format(label.x)} %, ${number.format(label.y)} %)` });
    place.addEventListener("click", () => {
      drawingEdit.placing = drawingEdit.placing === index ? -1 : index;
      drawingEdit.tracing = -1;
      refresh();
      showPreview();
    });
    const remove = el("button", { type: "button", class: "ghost small danger", text: "Retirer" });
    remove.addEventListener("click", () => {
      draft.labels.splice(index, 1);
      drawingEdit.placing = -1;
      refresh();
    });
    return el("li", { class: "drawing-item" },
      el("div", { class: "drawing-item-head" }, el("b", { text: `Valeur ${index + 1}` }), remove),
      el("div", { class: "drawing-grid" }, field("Appareil", entity), field("Nom affiché", name)),
      place);
  });
  const addLabel = el("button", { type: "button", class: "ghost small", text: "+ Ajouter une valeur" });
  addLabel.addEventListener("click", () => {
    draft.labels.push({ entity_id: "", x: 50, y: 50 });
    drawingEdit.placing = -1;
    refresh();
    // On choisit d'abord l'appareil ; « Placer » viendra ensuite.
    $("#drawing-form fieldset:last-of-type .drawing-item:last-child input")?.focus();
  });
  const labels = drawingFieldset(
    "Valeurs sur le dessin",
    "Une température, une puissance, un état… posé où tu veux : « Placer », puis touche le dessin.",
    el("ol", { class: "drawing-items" }, ...labelRows),
    addLabel,
  );

  // Les boutons rapides sous le dessin.
  const actionRows = draft.actions.map((id, index) => {
    const entity = el("input", { type: "text", list: "entity-list", value: id, placeholder: "light.porche", autocapitalize: "none", spellcheck: "false", "aria-label": `Bouton ${index + 1}` });
    entity.addEventListener("change", () => { draft.actions[index] = entity.value.trim(); });
    const remove = el("button", { type: "button", class: "ghost small danger", text: "Retirer" });
    remove.addEventListener("click", () => { draft.actions.splice(index, 1); refresh(); });
    return el("li", { class: "drawing-row" }, entity, remove);
  });
  const addAction = el("button", { type: "button", class: "ghost small", text: "+ Ajouter un bouton" });
  addAction.disabled = draft.actions.length >= 12;
  addAction.addEventListener("click", () => {
    draft.actions.push("");
    refresh();
    $("#drawing-form fieldset:last-of-type li:last-child input")?.focus();
  });
  const actions = drawingFieldset(
    "Boutons sous le dessin",
    "Le porche, le parking, les portes de garage… : un toucher allume, éteint, ouvre ou ferme (12 au plus).",
    el("ol", { class: "drawing-items" }, ...actionRows),
    addAction,
  );

  // Les zones à toucher sur le dessin.
  const zoneRows = draft.zones.map((zone, index) => {
    const entity = el("input", { type: "text", list: "entity-list", value: zone.entity_id, placeholder: "light.porche", autocapitalize: "none", spellcheck: "false" });
    entity.addEventListener("change", () => { zone.entity_id = entity.value.trim(); });
    const name = el("input", { type: "text", value: zone.name ?? "", placeholder: "(le nom de l'appareil)", maxlength: "60" });
    name.addEventListener("change", () => { if (name.value.trim()) zone.name = name.value.trim(); else delete zone.name; });
    const tracing = drawingEdit.tracing === index;
    const trace = el("button", { type: "button", class: tracing ? "primary small" : "ghost small", text: tracing ? "Trace sur le dessin…" : `Tracer (${number.format(zone.w)} × ${number.format(zone.h)} %)` });
    trace.addEventListener("click", () => {
      drawingEdit.tracing = tracing ? -1 : index;
      drawingEdit.placing = -1;
      refresh();
      showPreview();
    });
    const remove = el("button", { type: "button", class: "ghost small danger", text: "Retirer" });
    remove.addEventListener("click", () => {
      draft.zones.splice(index, 1);
      drawingEdit.tracing = -1;
      refresh();
    });
    return el("li", { class: "drawing-item" },
      el("div", { class: "drawing-item-head" }, el("b", { text: `Zone ${index + 1}` }), remove),
      el("div", { class: "drawing-grid" }, field("Appareil", entity), field("Nom affiché", name)),
      trace);
  });
  const addZone = el("button", { type: "button", class: "ghost small", text: "+ Ajouter une zone" });
  addZone.disabled = draft.zones.length >= 24;
  addZone.addEventListener("click", () => {
    draft.zones.push({ entity_id: "", x: 40, y: 40, w: 20, h: 20 });
    drawingEdit.tracing = -1;
    drawingEdit.placing = -1;
    refresh();
    $("#drawing-form .drawing-zones .drawing-item:last-child input")?.focus();
  });
  const zones = drawingFieldset(
    "Zones à toucher",
    "Le porche, la porte du garage, la piscine… : trace un rectangle sur le dessin. Un toucher allume, éteint, ouvre ou ferme ; un appui long ouvre l'historique (24 au plus).",
    el("ol", { class: "drawing-items" }, ...zoneRows),
    addZone,
  );
  zones.classList.add("drawing-zones");

  // Les courbes de la colonne de gauche.
  const chartRows = draft.charts.map((chart, index) => {
    const entity = el("input", { type: "text", list: "entity-list", value: chart.entity_id, placeholder: "sensor.puissance_nas", autocapitalize: "none", spellcheck: "false" });
    entity.addEventListener("change", () => { chart.entity_id = entity.value.trim(); });
    const name = el("input", { type: "text", value: chart.name ?? "", placeholder: "(le nom de l'appareil)", maxlength: "60" });
    name.addEventListener("change", () => { if (name.value.trim()) chart.name = name.value.trim(); else delete chart.name; });
    const color = el("select", {}, ...CHART_COLORS.map(([value, text]) => el("option", { value, text })));
    color.value = chart.color;
    color.addEventListener("change", () => { chart.color = color.value; });
    const hours = el("select", {}, ...CHART_HOURS.map(([value, text]) => el("option", { value: String(value), text })));
    hours.value = String(chart.hours);
    hours.addEventListener("change", () => { chart.hours = Number(hours.value); });
    const remove = el("button", { type: "button", class: "ghost small danger", text: "Retirer" });
    remove.addEventListener("click", () => { draft.charts.splice(index, 1); refresh(); });
    return el("li", { class: "drawing-item" },
      el("div", { class: "drawing-item-head" }, el("b", { text: `Courbe ${index + 1}` }), remove),
      el("div", { class: "drawing-grid" }, field("Capteur", entity), field("Nom affiché", name), field("Couleur", color), field("Durée", hours)));
  });
  const addChart = el("button", { type: "button", class: "ghost small", text: "+ Ajouter une courbe" });
  addChart.disabled = draft.charts.length >= 6;
  addChart.addEventListener("click", () => {
    draft.charts.push({ entity_id: "", color: CHART_COLORS[draft.charts.length % CHART_COLORS.length][0], hours: 24 });
    refresh();
    $("#drawing-form fieldset:last-of-type .drawing-item:last-child input")?.focus();
  });
  const charts = drawingFieldset(
    "Courbes",
    "Dans la colonne de gauche de l'onglet : la puissance d'un appareil, une température… sur 6 heures, 24 heures ou 7 jours (6 au plus). Toucher une courbe ouvre son historique.",
    el("ol", { class: "drawing-items" }, ...chartRows),
    addChart,
  );

  $("#drawing-form").replaceChildren(pagesPart(), images, backgrounds, layers, labels, zones, actions, charts);
}

/** Un identifiant d'onglet à partir de son nom (« Salle de jeux » → salle-de-jeux). */
function pageId(name) {
  const base = name.toLowerCase().normalize("NFD").replace(/[\u0300-\u036f]/g, "").replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "").slice(0, 30) || "onglet";
  let id = base;
  for (let n = 2; drawingEdit.pages.some((page) => page.id === id); n += 1) id = `${base.slice(0, 27)}-${n}`;
  return id;
}

/** Les onglets : lequel on règle, son nom, ses pièces ; en ajouter, en retirer. */
function pagesPart() {
  const refresh = () => renderDrawingEditor();
  const draft = drawingEdit.draft;
  const choose = el("select", { "aria-label": "Onglet à régler" }, ...drawingEdit.pages.map((page) => el("option", { value: page.id, text: page.name })));
  choose.value = draft.id;
  choose.addEventListener("change", () => {
    drawingEdit.draft = drawingEdit.pages.find((page) => page.id === choose.value) ?? drawingEdit.pages[0];
    drawingEdit.placing = -1;
    drawingEdit.tracing = -1;
    refresh();
  });
  const add = el("button", { type: "button", class: "ghost small", text: "+ Nouvel onglet" });
  add.disabled = drawingEdit.pages.length >= 8;
  add.addEventListener("click", () => {
    const page = { id: pageId("Nouvel onglet"), name: "Nouvel onglet", areas: [], ...emptyDrawing() };
    drawingEdit.pages.push(page);
    drawingEdit.draft = page;
    refresh();
    $("#drawing-form .drawing-part input")?.select();
  });
  const remove = el("button", { type: "button", class: "ghost small danger", text: "Retirer cet onglet" });
  remove.disabled = drawingEdit.pages.length === 1;
  remove.addEventListener("click", () => {
    drawingEdit.pages = drawingEdit.pages.filter((page) => page !== draft);
    drawingEdit.draft = drawingEdit.pages[0];
    refresh();
  });
  const first = el("button", { type: "button", class: "ghost small", text: "Mettre en premier" });
  first.disabled = drawingEdit.pages[0] === draft;
  first.addEventListener("click", () => {
    drawingEdit.pages = [draft, ...drawingEdit.pages.filter((page) => page !== draft)];
    refresh();
  });
  const name = el("input", { type: "text", value: draft.name, maxlength: "30", "aria-label": "Nom de l'onglet" });
  // Sans tout redessiner : « change » arrive quand le champ perd le focus.
  name.addEventListener("change", () => {
    draft.name = name.value.trim() || draft.name;
    name.value = draft.name;
    if (choose.selectedOptions[0]) choose.selectedOptions[0].textContent = draft.name;
  });
  // Les pièces de cet onglet (aucune cochée : toutes).
  const rooms = el("div", { class: "drawing-rooms" }, ...areas.map((area) => {
    const box = el("input", { type: "checkbox" });
    box.checked = draft.areas.includes(area.name);
    box.addEventListener("change", () => {
      draft.areas = box.checked ? [...draft.areas, area.name] : draft.areas.filter((a) => a !== area.name);
    });
    return el("label", { class: "check" }, box, ` ${area.name}`);
  }));
  return drawingFieldset(
    "Onglets",
    "Chaque onglet a son dessin, ses boutons, et les pièces qu'il montre (aucune cochée : toute la maison). Le premier s'ouvre par défaut.",
    el("div", { class: "drawing-row" }, choose, add),
    el("div", { class: "drawing-grid" }, field("Nom de l'onglet", name)),
    areas.length > 0 ? field("Pièces montrées", rooms) : "",
    el("div", { class: "drawing-row" }, first, remove),
  );
}

// Poser une valeur : un toucher sur l'aperçu donne sa place, en % de l'image.
$("#drawing-preview").addEventListener("click", (event) => {
  const index = drawingEdit.placing;
  if (index < 0 || !drawingEdit.draft) return;
  const box = $("#drawing-preview .house-base")?.getBoundingClientRect();
  if (!box || box.width === 0) return;
  const percent = (value) => Math.round(Math.min(100, Math.max(0, value)) * 10) / 10;
  const label = drawingEdit.draft.labels[index];
  label.x = percent(((event.clientX - box.left) / box.width) * 100);
  label.y = percent(((event.clientY - box.top) / box.height) * 100);
  drawingEdit.placing = -1;
  renderDrawingEditor();
});

// Tracer une zone : on fait glisser sur l'aperçu, du premier coin à l'autre.
const zoneDraw = { start: null, rect: null };

function previewPercent(event) {
  const box = $("#drawing-preview .house-base")?.getBoundingClientRect();
  if (!box || box.width === 0) return null;
  const clamp = (value) => Math.min(100, Math.max(0, value));
  return { x: clamp(((event.clientX - box.left) / box.width) * 100), y: clamp(((event.clientY - box.top) / box.height) * 100) };
}

function traceBox(a, b) {
  const round = (value) => Math.round(value * 10) / 10;
  const x = round(Math.min(a.x, b.x));
  const y = round(Math.min(a.y, b.y));
  return { x, y, w: round(Math.min(100 - x, Math.abs(a.x - b.x))), h: round(Math.min(100 - y, Math.abs(a.y - b.y))) };
}

$("#drawing-preview").addEventListener("pointerdown", (event) => {
  if (drawingEdit.tracing < 0 || !drawingEdit.draft) return;
  const start = previewPercent(event);
  if (!start) return;
  event.preventDefault();
  zoneDraw.start = start;
  zoneDraw.rect = el("span", { class: "house-zone edit tracing" });
  placeZone(zoneDraw.rect, { ...start, w: 0, h: 0 });
  $("#drawing-preview").append(zoneDraw.rect);
  $("#drawing-preview").setPointerCapture(event.pointerId);
});
$("#drawing-preview").addEventListener("pointermove", (event) => {
  if (!zoneDraw.start) return;
  const point = previewPercent(event);
  if (point) placeZone(zoneDraw.rect, traceBox(zoneDraw.start, point));
});
for (const type of ["pointerup", "pointercancel"]) {
  $("#drawing-preview").addEventListener(type, (event) => {
    if (!zoneDraw.start) return;
    const point = type === "pointerup" ? previewPercent(event) : null;
    const box = point ? traceBox(zoneDraw.start, point) : null;
    zoneDraw.start = null;
    zoneDraw.rect?.remove();
    zoneDraw.rect = null;
    if (!box || box.w < 1 || box.h < 1) return; // un simple toucher : on recommence
    Object.assign(drawingEdit.draft.zones[drawingEdit.tracing], box);
    drawingEdit.tracing = -1;
    renderDrawingEditor();
  });
}

$("#drawing-save").addEventListener("click", async () => {
  if (!drawingEdit.pages) return;
  for (const page of drawingEdit.pages) {
    page.actions = page.actions.filter((id) => id !== "");
    page.charts = page.charts.filter((chart) => chart.entity_id !== "");
    page.zones = page.zones.filter((zone) => zone.entity_id !== "");
  }
  const button = $("#drawing-save");
  button.disabled = true;
  const { status, data } = await api("PUT", "/api/home/drawing", { pages: drawingEdit.pages }).catch(() => ({ status: 0, data: {} }));
  button.disabled = false;
  if (status !== 200) return drawingStatus(data.error ?? "Enregistrement impossible.", "error");
  house.pages = cleanPages(data.config);
  selectPage(drawingEdit.draft.id);
  $("#drawing").close();
  flash("Dessin de la maison enregistré");
});

for (const id of ["#drawing-close", "#drawing-cancel"]) $(id).addEventListener("click", () => $("#drawing").close());
$("#open-drawing").addEventListener("click", openDrawingEditor);
addEventListener("resize", () => { if (house.view) fitLabels(house.view); }, { passive: true });
// La hauteur des volets, pour que le dessin et les pièces prennent juste le
// reste de l'écran (mise en page « tableau » : tout tient sur un écran).
new ResizeObserver(([entry]) => {
  document.documentElement.style.setProperty("--covers-h", `${Math.ceil(entry.target.offsetHeight)}px`);
  if (house.view) fitLabels(house.view);
}).observe($("#board-covers"));
