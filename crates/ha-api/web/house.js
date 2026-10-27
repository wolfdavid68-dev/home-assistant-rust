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
//
// Le reste est rangé à côté, chargé juste après par index.html :
// house-effets.js (pluie, neige, ciel des chiffres), house-tableau.js
// (onglets, repères, boutons, volets, chauffage, courbes, pièce) et
// house-reglage.js (la fenêtre de réglage du dessin).
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
// « crepuscule » (le soleil entre −6° et +6°, comme sur domo-blog) n'est
// choisi que si l'onglet a au moins un fond de crépuscule.
const MOMENTS = [["jour", "Jour"], ["crepuscule", "Crépuscule"], ["nuit", "Nuit"]];
const DUSK_ELEVATION = 6;
// Pluie annoncée et 1 °C ou moins dehors (la sonde) : il neige, comme sur domo-blog.
const SNOW_BELOW = 1;
// Tous les temps de la météo, avec le temps « parent » qu'on prend s'il
// manque un fond (orage → pluie → nuageux → soleil).
const WEATHERS = [
  ["soleil", "Soleil", null], ["eclaircies", "Éclaircies", "soleil"], ["nuageux", "Nuageux", "soleil"],
  ["brouillard", "Brouillard", "nuageux"], ["vent", "Vent", "nuageux"], ["pluie", "Pluie", "nuageux"],
  ["averses", "Averses", "pluie"], ["orage", "Orage", "pluie"], ["grele", "Grêle", "orage"],
  ["neige", "Neige", "nuageux"], ["pluie-neige", "Pluie et neige", "neige"],
];
const WEATHER_PARENT = Object.fromEntries(WEATHERS.map(([key, , parent]) => [key, parent]));
const AMBIANCE_KEYS = ["crepuscule", ...MOMENTS.flatMap(([moment]) => WEATHERS.filter(([key]) => key !== "soleil").map(([key]) => `${moment}-${key}`))];
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
  if (config?.effects === "off" || EFFECTS.some(([key]) => key === config?.effects)) out.effects = config.effects;
  if (["discret", "fort"].includes(config?.effects_level)) out.effects_level = config.effects_level;
  for (const layer of Array.isArray(config?.layers) ? config.layers : []) {
    const when = ["day", "night"].includes(layer.when) ? layer.when : "always";
    // Une pièce : le calque s'allume avec ses lampes (étape 104).
    if (text(layer.area)) {
      out.layers.push({ image: text(layer.image) ?? "", area: layer.area, states: [], not: false, position: false, when });
      continue;
    }
    out.layers.push({
      image: text(layer.image) ?? "",
      entity_id: text(layer.entity_id) ?? "",
      states: Array.isArray(layer.states) ? layer.states.filter((s) => typeof s === "string") : [],
      not: layer.not === true,
      position: layer.position === true,
      when,
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
    const target = text(zone.area) ? { area: zone.area } : text(zone.entity_id) ? { entity_id: zone.entity_id } : null;
    if (target === null) continue;
    const entry = { ...target, x: Number(zone.x) || 0, y: Number(zone.y) || 0, w: Number(zone.w) || 1, h: Number(zone.h) || 1 };
    if (text(zone.name)) entry.name = zone.name;
    out.zones.push(entry);
  }
  return out;
}

async function loadHouse(force = false) {
  if (house.loading && !force) return house.loading;
  const loading = (async () => {
    const { status, data } = await api("GET", "/api/home/drawing");
    if (status !== 200) throw new Error(`dessin : erreur ${status}`);
    house.pages = cleanPages(data.config);
    house.images = Array.isArray(data.images) ? data.images : [];
    house.editable = data.editable === true;
    const used = house.pages.flatMap((page) => [page.background, page.night, ...Object.values(page.ambiances), ...page.layers.map((l) => l.image)]);
    await Promise.all(used.filter(Boolean).map(houseImage));
    selectPage(house.config?.id ?? pageWanted(), false);
  })().catch(() => {
    // Raté (serveur redémarré, réseau coupé) : un prochain affichage
    // réessaiera, pas avant 10 s.
    setTimeout(() => { if (house.loading === loading) house.loading = null; }, 10_000);
  });
  house.loading = loading;
  return loading;
}

/** L'adresse locale (blob:) d'une image : une balise <img> ne sait pas
 *  envoyer le jeton, on la lit donc avec fetch. */
async function houseImage(name) {
  const info = house.images.find((image) => image.name === name);
  if (!info) return null;
  const key = `${name}@${info.updated_at}`;
  if (house.urls.has(key)) return house.urls.get(key);
  const response = await authFetch(`/api/home/images/${encodeURIComponent(name)}`);
  if (!response?.ok) return null;
  const url = URL.createObjectURL(await response.blob());
  // Une ancienne version de la même image n'a plus d'usage.
  for (const [old, oldUrl] of house.urls) {
    if (old.startsWith(`${name}@`)) {
      URL.revokeObjectURL(oldUrl);
      house.urls.delete(old);
      forgetCutout(oldUrl);
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
  const kind = lookup(CONDITION_WEATHER, weather?.state) ?? "soleil";
  // La pluie par 1 °C ou moins dehors devient de la neige.
  if ((kind === "pluie" || kind === "averses") && outsideTemperature() <= SNOW_BELOW) return "neige";
  return kind;
}

/** La température dehors : la sonde de la pompe à chaleur, sinon la météo. */
function outsideTemperature() {
  const reading = (id) => {
    const state = entities.get(id);
    return state && state.state !== "" ? Number(state.state) : NaN;
  };
  const probe = reading("sensor.temperature_exterieure");
  if (Number.isFinite(probe)) return probe;
  const weather = [...entities.values()].find((s) => domainOf(s) === "weather");
  if (finite(weather?.attributes.temperature)) return weather.attributes.temperature;
  return reading("sensor.meteo_temperature");
}

/** Le crépuscule : le soleil entre −6° et +6°. */
function houseDusk() {
  const elevation = entities.get("sun.sun")?.attributes.elevation;
  return finite(elevation) && Math.abs(elevation) < DUSK_ELEVATION;
}

/** L'onglet a-t-il un fond de crépuscule ? Sinon, jour ou nuit comme avant. */
const hasDusk = (page) => Object.keys(page.ambiances ?? {}).some((key) => key.startsWith("crepuscule"));

/** Le moment d'une ambiance (« crepuscule-pluie » → crepuscule). */
const momentOf = (ambiance) => ambiance.split("-")[0];

/** L'ambiance du moment (« nuit-pluie »…), ou celle imposée par le réglage. */
function ambianceOf(page) {
  if (page.forced) return page.forced;
  const moment = hasDusk(page) && houseDusk() ? "crepuscule" : houseNight() ? "nuit" : "jour";
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
      const clear = when === "jour" ? page.background : when === "nuit" ? page.night : page.ambiances.crepuscule;
      names.push(w === "soleil" ? clear : page.ambiances[`${when}-${w}`]);
    }
    return names;
  };
  // Au crépuscule sans son fond : la nuit si le soleil est couché, sinon le jour.
  const dusk = moment === "crepuscule" ? chain("crepuscule") : [];
  const night = moment === "nuit" || (moment === "crepuscule" && houseNight());
  const candidates = [...dusk, ...(night ? [...chain("nuit"), ...chain("jour")] : chain("jour"))];
  return candidates.find((name) => name && houseUrl(name)) ?? page.background;
}

function layerShown(layer, night) {
  if (layer.area !== undefined) {
    if (!roomLit(layer.area)) return false;
    return layer.when === "always" || (layer.when === "night") === night;
  }
  const state = entities.get(layer.entity_id);
  if (!state) return false;
  if (layer.position) {
    // Un volet (étape 71) : visible dès qu'il n'est plus grand ouvert.
    const open = coverOpenPercent(state);
    if (open === null || open >= 98) return false;
  } else if (layer.states.includes(state.state) === layer.not) return false;
  return layer.when === "always" || (layer.when === "night") === night;
}

/** Une lampe de la pièce est-elle allumée ? (une fenêtre éclairée, étape 104) */
function roomLit(name) {
  const area = areas.find((a) => a.name === name);
  return area !== undefined && area.entities.some((e) => {
    const state = entities.get(e.entity_id);
    return state !== undefined && isLight(state) && state.state === "on";
  });
}

/** L'ouverture d'un volet, de 0 (fermé) à 100 (ouvert) ; `null` si on ne
 *  sait pas (indisponible). */
function coverOpenPercent(state) {
  const position = Number(state.attributes.current_position);
  if (state.attributes.current_position !== undefined && Number.isFinite(position)) return Math.min(100, Math.max(0, position));
  if (state.state === "closed") return 0;
  if (state.state === "open") return 100;
  return null;
}

/** La forme du volet fermé dans son image : où commence et où finit la
 *  partie dessinée, à gauche et à droite (en fraction de l'image). Lue une
 *  fois, sur une copie réduite. */
function shutterShape(img) {
  if (img._shape !== undefined) return img._shape;
  if (!img.complete || img.naturalWidth === 0) return null;
  const width = Math.min(480, img.naturalWidth);
  const height = Math.max(1, Math.round((img.naturalHeight * width) / img.naturalWidth));
  const canvas = document.createElement("canvas");
  canvas.width = width;
  canvas.height = height;
  const context = canvas.getContext("2d", { willReadFrequently: true });
  let data;
  try {
    context.drawImage(img, 0, 0, width, height);
    data = context.getImageData(0, 0, width, height).data;
  } catch {
    img._shape = null;
    return null;
  }
  const opaque = (x, y) => data[(y * width + x) * 4 + 3] > 128;
  const column = (x) => {
    let top = -1;
    let bottom = -1;
    for (let y = 0; y < height; y += 1) {
      if (opaque(x, y)) {
        if (top < 0) top = y;
        bottom = y;
      }
    }
    return top < 0 ? null : { top, bottom };
  };
  let left = -1;
  let right = -1;
  for (let x = 0; x < width; x += 1) {
    if (column(x)) {
      if (left < 0) left = x;
      right = x;
    }
  }
  if (left < 0) {
    img._shape = null;
    return null;
  }
  // Un peu à l'intérieur des bords, là où le volet a toute sa hauteur.
  const inset = Math.max(1, Math.round((right - left) * 0.06));
  const xl = Math.min(right, left + inset);
  const xr = Math.max(left, right - inset);
  const cl = column(xl);
  const cr = column(xr);
  img._shape = {
    xl: xl / width, tl: cl.top / height, bl: (cl.bottom + 1) / height,
    xr: xr / width, tr: cr.top / height, br: (cr.bottom + 1) / height,
  };
  return img._shape;
}

const shutterWaiting = new WeakSet();

/** Dessine le volet descendu de ce qu'il faut : le haut de l'image, coupé
 *  par une ligne qui suit la pente de la fenêtre. */
function placeShutter(img, state) {
  const open = state ? coverOpenPercent(state) : null;
  const shape = shutterShape(img);
  if (open === null || !shape || open <= 2) {
    img.style.clipPath = "";
    // Image pas encore chargée : on réessaie à son chargement (une seule
    // attente par image, même si le dessin est mis à jour entre-temps).
    if (!shape && img.naturalWidth === 0 && !shutterWaiting.has(img)) {
      shutterWaiting.add(img);
      img.addEventListener("load", () => {
        shutterWaiting.delete(img);
        placeShutter(img, entities.get(img.dataset.entity));
      }, { once: true });
    }
    return;
  }
  const closed = 1 - open / 100;
  const yl = shape.tl + (shape.bl - shape.tl) * closed;
  const yr = shape.tr + (shape.br - shape.tr) * closed;
  const slope = shape.xr > shape.xl ? (yr - yl) / (shape.xr - shape.xl) : 0;
  const y0 = (yl - slope * shape.xl) * 100;
  const y1 = (yl + slope * (1 - shape.xl)) * 100;
  img.style.clipPath = `polygon(0% 0%, 100% 0%, 100% ${y1.toFixed(2)}%, 0% ${y0.toFixed(2)}%)`;
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
    if (layer.position) img.dataset.entity = layer.entity_id;
    if (url) img.src = url;
    parts.push(img);
    return { layer, img };
  });
  // Les effets animés (pluie, neige…), par-dessus le dessin et ses calques,
  // sous les zones et les valeurs.
  const fx = makeFx();
  parts.push(fx.canvas);
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
  const view = { stage, config, ambiance, layers, fx, zones, labels };
  // La taille du dessin n'est connue qu'une fois l'image chargée : ses
  // proportions servent à le faire tenir dans la hauteur de l'écran.
  stage.classList.remove("cutout");
  base.addEventListener("load", () => {
    if (base.naturalWidth > 0 && base.naturalHeight > 0) stage.style.setProperty("--ratio", String(base.naturalWidth / base.naturalHeight));
    fitLabels(view);
    fxSchedule();
    if (custom) applyCutout(stage, base);
  });
  return view;
}

// ---------------------------------------------------------------- Détourage
// Une maquette 3D est rendue sur un fond uni (noir, le plus souvent). Sur un
// thème clair, ce fond fait un grand rectangle noir. Le navigateur détoure
// la maison une fois : tout ce qui touche le bord de l'image et a la couleur
// du fond devient transparent (un « pot de peinture » parti des bords). Le
// masque et une ombre douce sont gardés pour chaque image ; le CSS décide où
// s'en servir (thèmes clairs, vue hub).

const houseCutouts = new Map();

function applyCutout(stage, base) {
  const key = base.currentSrc || base.src;
  if (!houseCutouts.has(key)) houseCutouts.set(key, makeCutout(base).catch(() => null));
  houseCutouts.get(key).then((cut) => {
    // Le dessin a pu changer entre-temps (autre vue, autre image).
    if (!cut || !stage.contains(base)) return;
    stage.style.setProperty("--house-cut", `url("${cut.mask}")`);
    stage.style.setProperty("--house-shadow", `url("${cut.shadow}")`);
    stage.classList.add("cutout");
  });
}

async function makeCutout(img) {
  const scale = Math.min(1, 1024 / img.naturalWidth);
  const w = Math.max(1, Math.round(img.naturalWidth * scale));
  const h = Math.max(1, Math.round(img.naturalHeight * scale));
  const source = document.createElement("canvas");
  source.width = w;
  source.height = h;
  const ctx = source.getContext("2d", { willReadFrequently: true });
  ctx.drawImage(img, 0, 0, w, h);
  const pixels = ctx.getImageData(0, 0, w, h);
  const keep = cutoutMask(pixels.data, w, h);
  if (!keep) return null;
  // Le masque : opaque sur la maison, transparent sur le fond.
  const out = new ImageData(w, h);
  for (let i = 0; i < w * h; i += 1) out.data[i * 4 + 3] = keep[i] ? 255 : 0;
  const mask = document.createElement("canvas");
  mask.width = w;
  mask.height = h;
  mask.getContext("2d").putImageData(out, 0, 0);
  // L'ombre : la même silhouette, floue, un peu plus bas, bleu-gris léger.
  // Elle déborde sous l'image : 8 % de hauteur en plus (le CSS en tient
  // compte). La silhouette est dessinée hors du canevas, seule son ombre
  // (shadowBlur) y tombe : le flou n'est jamais rogné.
  const shadow = document.createElement("canvas");
  shadow.width = w;
  shadow.height = Math.round(h * 1.08);
  const sctx = shadow.getContext("2d");
  sctx.shadowColor = "rgb(30 40 70 / 0.32)";
  sctx.shadowBlur = Math.round(w / 35);
  sctx.shadowOffsetX = w;
  sctx.shadowOffsetY = Math.round(h * 0.025);
  sctx.drawImage(mask, -w, 0);
  const [maskBlob, shadowBlob] = await Promise.all([toBlob(mask), toBlob(shadow)]);
  if (!maskBlob || !shadowBlob) return null;
  return { mask: URL.createObjectURL(maskBlob), shadow: URL.createObjectURL(shadowBlob) };
}

/** L'image `url` est remplacée : son masque et son ombre ne servent plus. */
function forgetCutout(url) {
  const cut = houseCutouts.get(url);
  if (!cut) return;
  houseCutouts.delete(url);
  cut.then((done) => {
    if (!done) return;
    URL.revokeObjectURL(done.mask);
    URL.revokeObjectURL(done.shadow);
  });
}

const toBlob = (canvas) => new Promise((resolve) => canvas.toBlob(resolve, "image/png"));

/**
 * Les pixels à garder (1) et le fond (0), ou null si l'image n'a pas de fond
 * uni (une photo, un dessin qui touche les bords) : on n'y touche pas.
 */
function cutoutMask(data, w, h) {
  const border = [];
  for (let x = 0; x < w; x += 1) border.push(x, (h - 1) * w + x);
  for (let y = 1; y < h - 1; y += 1) border.push(y * w, y * w + w - 1);
  const median = (channel) => {
    const values = border.map((i) => data[i * 4 + channel]).sort((a, b) => a - b);
    return values[values.length >> 1];
  };
  const ref = [median(0), median(1), median(2)];
  const distance = (i) => Math.max(Math.abs(data[i * 4] - ref[0]), Math.abs(data[i * 4 + 1] - ref[1]), Math.abs(data[i * 4 + 2] - ref[2]));
  const spread = border.map(distance).sort((a, b) => a - b);
  // Au moins 90 % du bord de la même couleur, sinon ce n'est pas un fond uni.
  if (spread[Math.floor(spread.length * 0.9)] > 12) return null;
  // Assez serré pour ne pas manger une clôture sombre, assez large pour le
  // léger bruit d'une image compressée.
  const tolerance = Math.min(28, Math.max(8, spread[Math.floor(spread.length * 0.98)] + 8));
  const keep = new Uint8Array(w * h).fill(1);
  const stack = new Int32Array(w * h);
  let top = 0;
  for (const i of border) {
    if (keep[i] && distance(i) <= tolerance) {
      keep[i] = 0;
      stack[top++] = i;
    }
  }
  while (top > 0) {
    const i = stack[--top];
    const x = i % w;
    for (const n of [x > 0 ? i - 1 : -1, x < w - 1 ? i + 1 : -1, i - w, i + w]) {
      if (n >= 0 && n < w * h && keep[n] && distance(n) <= tolerance) {
        keep[n] = 0;
        stack[top++] = n;
      }
    }
  }
  // Presque rien, ou presque tout : le détourage s'est trompé.
  const kept = keep.reduce((sum, v) => sum + v, 0) / (w * h);
  return kept > 0.08 && kept < 0.97 ? keep : null;
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
  for (const { layer, img } of view.layers) {
    const shown = layerShown(layer, isNight);
    img.classList.toggle("off", !shown);
    if (layer.position && shown) placeShutter(img, entities.get(layer.entity_id));
  }
  setFx(view.fx, effectOf(view.config), view.config.effects_level ?? "");
  for (const { zone, node } of view.zones) {
    if (node.tagName !== "BUTTON") continue;
    if (zone.area) {
      const area = areas.find((a) => a.name === zone.area);
      const info = area ? roomInfo(area) : null;
      const temp = info && Number.isFinite(info.temperature) ? ` : ${number.format(info.temperature)} °C` : "";
      setAttr(node, "aria-label", `${zone.name ?? zone.area}${temp}. Toucher pour ouvrir la pièce.`);
      setAttr(node, "title", zone.name ?? zone.area);
      node.classList.toggle("on", (info?.lights ?? 0) > 0);
      continue;
    }
    const state = entities.get(zone.entity_id);
    // La sonnette : la porte s'allume quand on sonne (sonnette.js).
    if (state && isDoorbell(state)) {
      doorbellZone(node, zone, state);
      continue;
    }
    const name = zone.name ?? (state ? nameOf(state) : zone.entity_id);
    const command = state ? actionCommand(state) : null;
    const readonly = command === null || !canControl(state);
    node.classList.toggle("readonly", readonly);
    node.classList.toggle("on", Boolean(state) && ["on", "open", "opening", "unlocked"].includes(state.state));
    const what = readonly ? "" : command[1] === "turn_off" ? " Toucher pour éteindre." : command[1] === "turn_on" ? (["scene", "script"].includes(command[0]) ? " Toucher pour lancer." : " Toucher pour allumer.")
      : command[1] === "close_cover" ? " Toucher pour fermer." : command[1] === "open_cover" ? " Toucher pour ouvrir." : " Toucher pour appuyer.";
    setAttr(node, "aria-label", `${name} : ${labelValue(state)}.${what}`);
    setAttr(node, "title", name);
    if (node.dataset.state !== state?.state) node.classList.remove("pending");
    node.dataset.state = state?.state ?? "";
  }
  let moved = false;
  for (const { label, name, value, node } of view.labels) {
    const state = entities.get(label.entity_id);
    const before = node.textContent;
    setText(name, label.name ?? (state ? nameOf(state) : label.entity_id));
    setText(value, labelValue(state));
    setAttr(node, "aria-label", `${name.textContent} : ${value.textContent}`);
    if (node.textContent !== before) moved = true;
  }
  // Replacer les étiquettes oblige le navigateur à recalculer la page : seulement
  // si un texte a changé (sa largeur aussi). Un changement de taille du dessin
  // passe par l'observateur plus bas.
  if (moved) fitLabels(view);
  // Le lave-linge en marche, en incrustation (plus bas).
  if (view === house.view) renderMachineLive();
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
  if (zone.area) return openRoom(zone.area);
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
    if (zone.area) return openRoom(zone.area);
    const state = entities.get(zone.entity_id);
    if (!state) return;
    if (isDoorbell(state)) return openDoorbell(state);
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
