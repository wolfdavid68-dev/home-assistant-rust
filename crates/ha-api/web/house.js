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
  if (config?.effects === "off" || EFFECTS.some(([key]) => key === config?.effects)) out.effects = config.effects;
  if (["discret", "fort"].includes(config?.effects_level)) out.effects_level = config.effects_level;
  for (const layer of Array.isArray(config?.layers) ? config.layers : []) {
    out.layers.push({
      image: text(layer.image) ?? "",
      entity_id: text(layer.entity_id) ?? "",
      states: Array.isArray(layer.states) ? layer.states.filter((s) => typeof s === "string") : [],
      not: layer.not === true,
      position: layer.position === true,
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
  if (layer.position) {
    // Un volet (étape 71) : visible dès qu'il n'est plus grand ouvert.
    const open = coverOpenPercent(state);
    if (open === null || open >= 98) return false;
  } else if (layer.states.includes(state.state) === layer.not) return false;
  return layer.when === "always" || (layer.when === "night") === night;
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

/** Dessine le volet descendu de ce qu'il faut : le haut de l'image, coupé
 *  par une ligne qui suit la pente de la fenêtre. */
function placeShutter(img, state) {
  const open = state ? coverOpenPercent(state) : null;
  const shape = shutterShape(img);
  if (open === null || !shape || open <= 2) {
    img.style.clipPath = "";
    if (!shape && img.naturalWidth === 0) img.addEventListener("load", () => placeShutter(img, entities.get(img.dataset.entity)), { once: true });
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

// ---------------------------------------------------------------- Effets animés
// Sur le dessin, le temps qu'il fait : la pluie, la neige, un éclair… Un seul
// <canvas> par dessin, et une seule boucle (requestAnimationFrame) pour tous,
// arrêtée quand la page est cachée. Sans animation si l'appareil demande de
// réduire les mouvements : une image fixe, sans éclair.

const EFFECTS = [
  ["pluie", "Pluie"], ["averses", "Averses"], ["orage", "Orage"], ["grele", "Grêle"], ["neige", "Neige"],
  ["pluie-neige", "Pluie et neige"], ["brouillard", "Brouillard"], ["vent", "Vent"], ["etoiles", "Étoiles (nuit claire)"],
];
const EFFECT_LEVELS = [["discret", "Discrète"], ["", "Normale"], ["fort", "Forte"]];
const WEATHER_EFFECT = { "pluie": "pluie", "averses": "averses", "orage": "orage", "grele": "grele", "neige": "neige", "pluie-neige": "pluie-neige", "brouillard": "brouillard", "vent": "vent" };
// Particules pour 10 000 px² du dessin, à l'intensité normale.
const FX_DENSITY = { "pluie": 2.2, "averses": 4, "orage": 3.2, "grele": 1.6, "neige": 1.7, "pluie-neige": 2.2, "brouillard": 0, "vent": 0.3, "etoiles": 0.4 };
const FX_LEVEL = { "discret": 0.5, "": 1, "fort": 1.8 };
const fxViews = new Set();
const fxMotion = matchMedia("(prefers-reduced-motion: reduce)");
let fxFrame = 0;
let fxLast = 0;

/** L'effet d'un onglet : celui imposé, sinon celui du temps affiché (le même
 *  que l'ambiance : la nuit claire donne des étoiles). */
function effectOf(page) {
  if (page.effects === "off") return null;
  if (page.effects) return page.effects;
  const ambiance = ambianceOf(page);
  const cut = ambiance.indexOf("-");
  if (cut < 0) return ambiance === "nuit" ? "etoiles" : null;
  return lookup(WEATHER_EFFECT, ambiance.slice(cut + 1)) ?? null;
}

function makeFx() {
  const canvas = el("canvas", { class: "house-fx", "aria-hidden": "true" });
  return { canvas, ctx: canvas.getContext("2d"), kind: null, level: "", w: 0, h: 0, dpr: 1, ink: "", parts: [], fog: [], flash: 0, nextFlash: 0 };
}

function setFx(fx, kind, level) {
  if (fx.kind === kind && fx.level === level) return;
  fx.kind = kind;
  fx.level = level;
  fx.w = 0; // à remesurer, et à repeupler
  if (kind === null) {
    fxViews.delete(fx);
    fx.ctx?.clearRect(0, 0, fx.canvas.width, fx.canvas.height);
  } else {
    fxViews.add(fx);
  }
  fxSchedule();
}

function fxSchedule() {
  if (fxFrame === 0 && fxViews.size > 0 && !document.hidden) fxFrame = requestAnimationFrame(fxTick);
}

function fxTick(now) {
  fxFrame = 0;
  const still = fxMotion.matches;
  const dt = still || fxLast === 0 ? 0 : Math.min(0.05, (now - fxLast) / 1000);
  fxLast = now;
  for (const fx of fxViews) {
    if (!fx.canvas.isConnected) { fxViews.delete(fx); continue; }
    fxStep(fx, dt, now / 1000, still);
  }
  if (!still) fxSchedule();
}

document.addEventListener("visibilitychange", () => { fxLast = 0; fxSchedule(); });
fxMotion.addEventListener("change", () => { fxLast = 0; fxSchedule(); });

const rand = (a, b) => a + Math.random() * (b - a);

/** Une particule, n'importe où (ou en haut, `top`, quand elle revient). */
function fxPart(fx, type, top = false) {
  const { w, h } = fx;
  const p = { type, x: rand(-0.1 * w, 1.05 * w), y: top ? rand(-0.15 * h, -5) : rand(-0.1 * h, h), phase: rand(0, Math.PI * 2) };
  if (type === "rain") Object.assign(p, { vy: rand(650, 950), vx: rand(110, 160), len: rand(10, 18), a: rand(0.35, 0.6) });
  else if (type === "pour") Object.assign(p, { vy: rand(900, 1300), vx: rand(160, 220), len: rand(16, 28), a: rand(0.3, 0.6) });
  else if (type === "hail") Object.assign(p, { vy: rand(520, 760), vx: rand(40, 80), r: rand(1.2, 2.1), a: rand(0.6, 0.9) });
  else if (type === "snow") Object.assign(p, { vy: rand(25, 65), vx: rand(-8, 12), r: rand(1, 3), a: rand(0.55, 0.95), sway: rand(10, 26), freq: rand(0.4, 1.1) });
  else if (type === "streak") Object.assign(p, { x: top ? rand(-0.3 * w, -20) : rand(-0.2 * w, w), y: rand(0, h), vx: rand(480, 760), vy: rand(-10, 20), len: rand(30, 70), a: rand(0.12, 0.28) });
  else if (type === "leaf") Object.assign(p, { x: top ? rand(-0.3 * w, -10) : rand(0, w), y: rand(0, h), vx: rand(140, 260), vy: rand(-15, 35), r: rand(2.5, 4.5), a: rand(0.55, 0.85), spin: rand(-6, 6), sway: rand(8, 20), freq: rand(0.8, 1.8) });
  else if (type === "star") Object.assign(p, { y: rand(0.02 * h, 0.38 * h), r: rand(0.5, 1.5), a: rand(0.35, 0.9), freq: rand(0.6, 1.8) });
  return p;
}

/** Peuple l'effet pour la taille du dessin. */
function fxSeed(fx) {
  const area = (fx.w * fx.h) / 10_000;
  const phone = fx.w < 600 ? 0.7 : 1;
  const count = Math.min(900, Math.round(area * (FX_DENSITY[fx.kind] ?? 0) * (FX_LEVEL[fx.level] ?? 1) * phone));
  const types = {
    "pluie": ["rain"], "averses": ["pour"], "orage": ["pour", "rain"], "grele": ["hail", "rain"], "neige": ["snow"],
    "pluie-neige": ["rain", "snow"], "vent": ["streak", "streak", "leaf"], "etoiles": ["star"], "brouillard": [],
  }[fx.kind] ?? [];
  fx.parts = types.length === 0 ? [] : Array.from({ length: count }, (_, i) => fxPart(fx, types[i % types.length]));
  fx.fog = fx.kind === "brouillard" || fx.kind === "pluie-neige"
    ? Array.from({ length: Math.round(4 * (FX_LEVEL[fx.level] ?? 1)) + 1 }, () => ({ x: rand(0, fx.w), y: rand(0.35 * fx.h, fx.h), rx: rand(0.25, 0.5) * fx.w, ry: rand(0.08, 0.16) * fx.h, vx: rand(6, 18) * (Math.random() < 0.5 ? -1 : 1), a: rand(0.1, 0.2) * (fx.kind === "brouillard" ? 1 : 0.5) }))
    : [];
  fx.nextFlash = performance.now() / 1000 + rand(2, 6);
}

function fxStep(fx, dt, t, still) {
  const { canvas, ctx } = fx;
  const cw = canvas.clientWidth;
  const ch = canvas.clientHeight;
  if (cw === 0 || ch === 0 || !ctx) return;
  if (cw !== fx.w || ch !== fx.h) {
    fx.dpr = Math.min(2, devicePixelRatio || 1);
    canvas.width = Math.round(cw * fx.dpr);
    canvas.height = Math.round(ch * fx.dpr);
    fx.w = cw;
    fx.h = ch;
    fx.ink = getComputedStyle(canvas).getPropertyValue("--fx-ink").trim() || "225 235 255";
    fxSeed(fx);
  }
  const { w, h, ink } = fx;
  ctx.setTransform(fx.dpr, 0, 0, fx.dpr, 0, 0);
  ctx.clearRect(0, 0, w, h);

  // Le brouillard : de grands voiles flous qui dérivent.
  for (const f of fx.fog) {
    f.x += f.vx * dt;
    if (f.x - f.rx > w) f.x = -f.rx; else if (f.x + f.rx < 0) f.x = w + f.rx;
    ctx.save();
    ctx.translate(f.x, f.y);
    ctx.scale(1, f.ry / f.rx);
    const g = ctx.createRadialGradient(0, 0, 0, 0, 0, f.rx);
    g.addColorStop(0, `rgb(${ink} / ${f.a})`);
    g.addColorStop(1, `rgb(${ink} / 0)`);
    ctx.fillStyle = g;
    ctx.fillRect(-f.rx, -f.rx, f.rx * 2, f.rx * 2);
    ctx.restore();
  }

  // Les traits (pluie, vent) : un seul tracé par opacité, c'est bien plus rapide.
  ctx.lineCap = "round";
  for (const p of fx.parts) {
    if (p.type === "star" || p.type === "leaf" || p.type === "snow" || p.type === "hail") continue;
    p.x += p.vx * dt;
    p.y += p.vy * dt;
    if (p.y - p.len > h || p.x - p.len > w) Object.assign(p, fxPart(fx, p.type, true));
  }
  for (const [type, width] of [["rain", 1], ["pour", 1.2], ["streak", 1]]) {
    ctx.beginPath();
    let alpha = 0;
    let n = 0;
    for (const p of fx.parts) {
      if (p.type !== type) continue;
      const speed = Math.hypot(p.vx, p.vy);
      ctx.moveTo(p.x, p.y);
      ctx.lineTo(p.x - (p.vx / speed) * p.len, p.y - (p.vy / speed) * p.len);
      alpha += p.a;
      n += 1;
    }
    if (n === 0) continue;
    ctx.strokeStyle = `rgb(${ink} / ${(alpha / n).toFixed(2)})`;
    ctx.lineWidth = width;
    ctx.stroke();
  }

  // Les grains (neige, grêle).
  ctx.fillStyle = `rgb(${ink} / 0.85)`;
  ctx.beginPath();
  for (const p of fx.parts) {
    if (p.type !== "snow" && p.type !== "hail") continue;
    p.y += p.vy * dt;
    p.x += p.vx * dt;
    const x = p.type === "snow" ? p.x + Math.sin(t * p.freq + p.phase) * p.sway : p.x;
    if (p.y - p.r > h) Object.assign(p, fxPart(fx, p.type, true));
    ctx.moveTo(x + p.r, p.y);
    ctx.arc(x, p.y, p.r, 0, Math.PI * 2);
  }
  ctx.fill();

  // Les feuilles (vent) et les étoiles (nuit claire), une à une.
  for (const p of fx.parts) {
    if (p.type === "leaf") {
      p.x += p.vx * dt;
      p.y += (p.vy + Math.sin(t * p.freq + p.phase) * p.sway) * dt;
      if (p.x - p.r > w || p.y > h + 10) Object.assign(p, fxPart(fx, "leaf", true));
      ctx.save();
      ctx.translate(p.x, p.y);
      ctx.rotate(t * p.spin + p.phase);
      ctx.fillStyle = `rgb(${ink} / ${p.a})`;
      ctx.beginPath();
      ctx.ellipse(0, 0, p.r, p.r * 0.45, 0, 0, Math.PI * 2);
      ctx.fill();
      ctx.restore();
    } else if (p.type === "star") {
      const twinkle = still ? 1 : 0.55 + 0.45 * Math.sin(t * p.freq + p.phase);
      ctx.fillStyle = `rgb(${ink} / ${(p.a * twinkle).toFixed(2)})`;
      ctx.beginPath();
      ctx.arc(p.x, p.y, p.r, 0, Math.PI * 2);
      ctx.fill();
    }
  }

  // L'orage : un éclair de temps en temps, en deux temps (jamais si l'appareil
  // demande de réduire les mouvements).
  if (fx.kind === "orage" && !still) {
    if (t >= fx.nextFlash) {
      fx.flash = 1;
      fx.second = t + rand(0.12, 0.22);
      fx.nextFlash = t + rand(5, 13) / (FX_LEVEL[fx.level] ?? 1);
    }
    if (fx.second && t >= fx.second) { fx.flash = Math.max(fx.flash, 0.7); fx.second = 0; }
    if (fx.flash > 0.01) {
      ctx.fillStyle = `rgb(255 255 255 / ${(fx.flash * 0.32).toFixed(3)})`;
      ctx.fillRect(0, 0, w, h);
      fx.flash *= Math.exp(-dt * 9);
    }
  }
}

// ---------------------------------------------------------------- Demandes pour l'IA
// Pour faire les images (le fond, les ambiances, les calques) avec une IA qui
// retouche les photos (Gemini, ChatGPT…). La méthode est celle de domo-blog :
// une image de départ, au soleil et « au repos », puis chaque variante faite
// à partir d'elle, sans rien bouger. Les demandes sont en anglais : c'est ce
// qui marche le mieux.

const AI_REFERENCE = "Use the attached image as the strict reference. Keep EXACTLY the same composition, camera angle, framing, scale, image size and the position of every object. Change ONLY what is described below. Do not add or remove anything else.";
const AI_BACKGROUND = { maquette: "Keep the plain uniform dark background (#1C1C1C), no sky, no horizon.", plan: "Keep the plain paper background, no sky, no horizon." };
/** Le temps, en mots pour l'IA. */
const AI_WEATHER = {
  "eclaircies": "PARTLY CLOUDY: bright light with a few soft cloud shadows drifting over the plot, slightly softer shadows.",
  "nuageux": "OVERCAST: soft diffuse grey light, no hard shadows, slightly muted colours.",
  "brouillard": "FOG: a light mist around the property, the far edges of the plot fading softly, diffuse light, low contrast.",
  "vent": "WINDY: trees and hedges bent by the wind, a few leaves in the air, cool grey-blue light.",
  "pluie": "RAIN: grey overcast light, visible falling rain, wet reflective ground, small puddles on the driveway and terrace.",
  "averses": "HEAVY DOWNPOUR: dark grey light, dense falling rain, streaming wet ground and roofs, larger puddles.",
  "orage": "THUNDERSTORM: very dark stormy light, heavy rain, a cold flash of lightning catching the edges of the roofs, wet reflective ground.",
  "grele": "HAIL: small white hailstones scattered on the roofs, lawn and driveway, cold grey light.",
  "neige": "SNOW: fresh snow covering the roofs, ground, hedges and trees, a few falling snowflakes, cold soft light, driveway lightly covered.",
  "pluie-neige": "SLEET: wet slushy snow patches on the roofs and lawn, rain and snow falling together, cold grey light.",
};
const AI_NIGHT = "NIGHT: deep blue ambient light, soft cool moonlight. All interior and exterior lights stay OFF (they will be added later as separate layers).";
const AI_CORRECTIONS = [
  ["camera", "Correction : le cadrage a bougé", "You moved the camera. Go back to the exact framing, camera angle and scale of the reference image, then apply only the requested change."],
  ["size", "Correction : la taille a changé", "Keep exactly the same image size and aspect ratio as the reference image."],
  ["style", "Retenir le style", "Remember this style, framing and camera angle: I will ask you for more variations of this same model, always from this reference image."],
];

/** Le nom d'image conseillé pour une ambiance (« nuit-pluie » → nuit_pluie). */
const aiFileName = (key) => key.replaceAll("-", "_");

/** Les appareils pour lesquels proposer un calque : ceux de l'onglet (boutons,
 *  zones, calques), plus les lumières et les volets de ses pièces. */
function aiLayerEntities(draft) {
  const ids = new Set([...draft.actions, ...draft.zones.map((z) => z.entity_id), ...draft.layers.map((l) => l.entity_id)]);
  const rooms = draft.areas.length > 0 ? areas.filter((a) => draft.areas.includes(a.name)) : areas;
  for (const area of rooms) for (const e of area.entities) if (/^(light|cover)\./.test(e.entity_id)) ids.add(e.entity_id);
  return [...ids].filter(Boolean).map((id) => entities.get(id)).filter((state) => state !== undefined && ["light", "cover", "switch", "lock", "valve", "fan"].includes(domainOf(state)));
}

/** « Volet · Salon » : le nom, et la pièce (il y a souvent plusieurs volets). */
function aiLayerName(state) {
  const area = areas.find((a) => a.entities.some((e) => e.entity_id === state.entity_id));
  return area && area.name !== nameOf(state) ? `${nameOf(state)} · ${area.name}` : nameOf(state);
}

/** La demande pour une image : { text, attach, save }. */
function aiPrompt(choice, draft, style) {
  const base = draft.background ?? "jour";
  const bg = AI_BACKGROUND[style];
  if (choice === "analyse") {
    return {
      attach: "1 à 3 photos de ta maison (de trois quarts, en pleine lumière)",
      save: "rien : corrige sa fiche si besoin, puis demande le fond de jour",
      text: "Look carefully at the attached photos of my house. Do NOT generate any image yet.\nWrite a numbered HOUSE SHEET: 1. overall shape and number of floors, 2. roof (shape, material, colour, chimneys, solar panels), 3. walls and colours, 4. windows on each side, 5. doors, 6. garage doors and gates, 7. plot and garden (pool, terrace, hedges, trees, driveway), 8. exterior lights.\nThen list, as questions, everything you are unsure about.",
    };
  }
  if (choice === "jour") {
    const look = style === "plan"
      ? "STYLE: clean isometric architectural blueprint drawing, fine dark-blue ink lines on white paper, light blue tints for glass and water, technical hatching on the roofs."
      : "STYLE: clean, slightly stylised 3D render, soft global illumination, premium architectural scale-model look.";
    return {
      attach: "la meilleure photo de ta maison",
      save: `${base}.webp (le fond de jour)`,
      text: [
        style === "plan" ? "Transform the attached photo of my house into an isometric architectural blueprint." : "Transform the attached photo of my house into a highly detailed 3D isometric miniature model (diorama).",
        "FIDELITY: keep the exact architecture, proportions, roof shape, number and position of windows and doors, garage, garden, pool and terrace. Do not add or remove any part of the building.",
        "CAMERA: true isometric view from a front three-quarter angle, the whole property centred with a small margin, nothing cropped.",
        look,
        "LIGHTING: bright sunny day, soft shadows. All interior and exterior lights OFF. All garage doors, gates and shutters CLOSED.",
        style === "plan" ? "BACKGROUND: plain white paper, no sky, no horizon." : "BACKGROUND: plain uniform dark background (#1C1C1C), no sky, no horizon.",
        "EXCLUDE: no text, no labels, no people, no cars, no watermark.",
        "FORMAT: landscape 16:9, highest resolution.",
      ].join("\n"),
    };
  }
  if (choice === "nuit" || AMBIANCE_KEYS.includes(choice)) {
    const night = choice.startsWith("nuit");
    const weather = choice.includes("-") ? choice.slice(choice.indexOf("-") + 1) : null;
    // La nuit sous la pluie : à partir de la pluie de jour, si on l'a déjà.
    const dayWeather = weather ? draft.ambiances[`jour-${weather}`] : null;
    const reference = night && weather ? `${dayWeather ?? aiFileName(`jour-${weather}`)}.webp (la même météo de jour ; sinon ${base}.webp)` : `${base}.webp (le fond de jour)`;
    const lines = [AI_REFERENCE];
    if (night && style === "plan") lines.push("NIGHT BLUEPRINT: white ink lines on deep blue paper, like a cyanotype print. All lights stay OFF.");
    else if (night) lines.push(AI_NIGHT);
    if (weather) lines.push(AI_WEATHER[weather]);
    if (choice === "nuit" && style !== "plan") lines.push("Clear sky feeling, calm night.");
    lines.push(bg);
    return { attach: reference, save: `${aiFileName(choice)}.webp`, text: lines.join("\n") };
  }
  if (choice.startsWith("calque:")) {
    const state = entities.get(choice.slice(7));
    if (!state) return { attach: "", save: "", text: "" };
    const name = `"${aiLayerName(state)}"`;
    const file = slug(aiLayerName(state)) || "calque";
    const domain = domainOf(state);
    const dc = state.attributes.device_class;
    let change;
    let from = `${draft.night ?? "nuit"}.webp (le fond de nuit), et refais-le avec ${base}.webp si tu veux aussi le calque de jour`;
    let save = `${file}_allume.webp`;
    if (domain === "light") {
      change = `Only turn ON the light named ${name} (location: WHERE IT IS, e.g. above the front door): warm 2700 K light with a soft halo on the nearby wall and ground.`;
    } else if (domain === "cover" && ["garage", "gate", "door"].includes(dc)) {
      change = `Only the door named ${name} (location: WHERE IT IS, e.g. the left garage door) is fully OPEN, with the dark interior visible behind it.`;
      from = `${base}.webp (le fond de jour), puis la même chose avec ${draft.night ?? "nuit"}.webp`;
      save = `${file}_ouvert.webp`;
    } else if (domain === "cover") {
      change = `Only the shutters named ${name} (location: WHERE THEY ARE, e.g. the large living-room window) are fully OPEN.`;
      from = `${base}.webp (le fond de jour)`;
      save = `${file}_ouvert.webp`;
    } else {
      change = `Only change: the device named ${name} (location: WHERE IT IS) is ON — DESCRIBE what it looks like when it is on.`;
      from = `${base}.webp (le fond de jour)`;
    }
    return {
      attach: from,
      save: `${save} — remplace d'abord « WHERE IT IS » dans la demande. Puis, dans GIMP : superpose-le au fond en mode « Différence », garde seulement ce qui change, et exporte en WebP transparent, à la taille du fond`,
      text: [AI_REFERENCE, change, "Nothing else changes: same light, same weather, same framing, same image size."].join("\n"),
    };
  }
  const correction = AI_CORRECTIONS.find(([key]) => `correction:${key}` === choice);
  return { attach: "la même conversation", save: "—", text: correction?.[2] ?? "" };
}

/** Copie un texte : le presse-papiers si la page le permet (HTTPS), sinon
 *  la sélection de la zone de texte (le réseau local en HTTP). */
async function copyText(area) {
  try {
    await navigator.clipboard.writeText(area.value);
    return true;
  } catch {
    area.focus();
    area.select();
    try {
      return document.execCommand("copy");
    } catch {
      return false;
    }
  }
}

// ---------------------------------------------------------------- Onglets
// Maison, Garage, Piscine… : chacun son dessin, ses boutons, et les pièces
// qu'il montre. L'onglet ouvert est retenu sur cet appareil (localStorage) :
// la tablette du garage reste sur « Garage ».
const PAGE_KEY = "ha-page";
// La pièce ouverte dans son onglet (étape 90, plus bas), et l'onglet des
// appareils rangés dans aucune pièce.
const roomTab = { open: false };
const OTHERS = "";
const OTHERS_NAME = "Autres";

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
  // L'agenda ou une pièce ouverts : aucun onglet de maison n'est l'onglet actif.
  const agendaOpen = typeof agenda !== "undefined" && agenda.open;
  const musicOpen = typeof music !== "undefined" && music.open;
  const roomOpen = roomTab.open ? roomView.name : null;
  const specs = house.pages.map((page) => ({
    key: `page:${page.id}`,
    label: page.name,
    current: !agendaOpen && !musicOpen && roomOpen === null && page.id === house.config?.id,
    open: () => {
      const wasRoom = roomTab.open;
      closeRoomTab(false);
      if (agendaOpen) closeAgenda();
      else if (musicOpen) closeMusic();
      else if (wasRoom) renderAll();
      if (page.id !== house.config?.id) selectPage(page.id);
    },
  }));
  // Mise en page « tableau » (étape 90) : un onglet par pièce, pour que
  // l'onglet de la maison ne garde que l'essentiel. Un point jaune : des
  // lumières y sont allumées.
  if (isBoard()) {
    for (const area of shownAreas()) {
      specs.push({ key: `room:${area.name}`, label: area.name, lit: roomInfo(area).lights > 0, current: roomOpen === area.name, open: () => openRoomTab(area.name) });
    }
    if (pageAreas === null && looseStates().length > 0) {
      specs.push({ key: "room:", label: OTHERS_NAME, current: roomOpen === OTHERS, open: () => openRoomTab(OTHERS) });
    }
  }
  // L'onglet Agenda, s'il y a un agenda (étape 75).
  const withAgenda = typeof agendaCalendars === "function" && agendaCalendars().length > 0;
  // Sur un grand écran, le rail de gauche a déjà ses boutons : ces deux
  // onglets y sont cachés (`.in-rail`).
  if (withAgenda) specs.push({ key: "agenda", label: "Agenda", icon: ICONS.calendar, inRail: true, current: agendaOpen, open: () => openAgenda() });
  // L'onglet Musique, s'il y a une enceinte (étape 92, musique.js).
  const withMusic = typeof musicPlayers === "function" && musicPlayers().length > 0;
  if (withMusic) specs.push({ key: "music", label: "Musique", icon: ICONS.music, inRail: true, current: musicOpen, open: () => openMusic() });

  // Rien n'a changé : on garde les boutons (et le focus du clavier).
  const signature = specs.map((s) => `${s.key}|${s.label}|${s.current ? 1 : 0}|${s.lit ? 1 : 0}`).join("\n");
  if (nav.dataset.signature === signature) return;
  nav.dataset.signature = signature;
  const tabs = specs.map((spec) => {
    const classes = ["board-tab-item"];
    if (spec.current) classes.push("active");
    if (spec.lit) classes.push("lit");
    if (spec.inRail) classes.push("in-rail");
    const tab = el("button", { type: "button", class: classes.join(" "), "aria-current": spec.current ? "page" : "false" },
      ...(spec.icon ? [svgIcon(spec.icon)] : []), spec.label);
    if (spec.lit) tab.setAttribute("aria-label", `${spec.label}, lumières allumées`);
    tab.addEventListener("click", spec.open);
    return tab;
  });
  nav.replaceChildren(...tabs);
  // Un seul onglet : rien à choisir sur téléphone. Beaucoup d'onglets : ils
  // prennent leur propre ligne en haut de l'écran.
  nav.classList.toggle("single", tabs.length === 1);
  nav.classList.toggle("many", tabs.length > 4);
  // Une ligne qui déborde : l'onglet ouvert reste en vue.
  const active = nav.querySelector(".active");
  if (active && active.offsetParent !== null) {
    const left = active.offsetLeft - nav.offsetLeft;
    if (left < nav.scrollLeft || left + active.offsetWidth > nav.scrollLeft + nav.clientWidth) {
      nav.scrollLeft = left - (nav.clientWidth - active.offsetWidth) / 2;
    }
  }
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
  const reading = (id) => {
    const state = entities.get(id);
    return state && state.state !== "" && Number.isFinite(Number(state.state)) ? Number(state.state) : NaN;
  };
  // D'abord la sonde de la pompe à chaleur, posée dehors ; sinon la météo.
  const probe = reading("sensor.temperature_exterieure");
  const outsideTemp = Number.isFinite(probe) ? probe
    : finite(weather?.attributes.temperature) ? weather.attributes.temperature
      : reading("sensor.meteo_temperature");
  if (Number.isFinite(outsideTemp)) items.push(gauge(ICONS.temperature, "Dehors", `${number.format(outsideTemp)} °C`, "outside"));
  const temps = areas.map(roomInfo).map((info) => info.temperature).filter((t) => Number.isFinite(t));
  if (temps.length > 0) {
    items.push(gauge(ICONS.home, "Dedans", `${number.format(temps.reduce((a, b) => a + b, 0) / temps.length)} °C`, "inside"));
  }
  const power = entities.get("sensor.linky_puissance");
  if (power && Number.isFinite(Number(power.state))) {
    items.push(gauge(ICONS.flame, "Puissance", `${number.format(Number(power.state))} ${power.attributes.unit_of_measurement ?? "VA"}`, "power"));
  }
  const lights = [...entities.values()].filter((s) => isLight(s) && s.state === "on").length;
  items.push(gauge(ICONS.bulb, "Lumières", lights === 0 ? "éteintes" : `${lights} allumée${lights > 1 ? "s" : ""}`, "lights"));
  // Le prochain rendez-vous d'aujourd'hui (agenda.js, étape 75).
  const next = typeof agendaGauge === "function" ? agendaGauge() : null;
  if (next) items.push(next);
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

// ---------------------------------------------------------------- Chauffage en lignes
// Au-dessus des volets : chaque zone (Etage, RdC…) et l'eau chaude, avec la
// température mesurée, la consigne et ses boutons −/+.

const degreesText = (value) => `${number.format(value)} °C`;

/** « 19,5 °C mesurés · Confort », ou « Arrêté ». */
function heatingText(state) {
  const a = state.attributes;
  if (state.state === "unavailable") return "Indisponible";
  const parts = [];
  if (finite(a.current_temperature)) parts.push(`${degreesText(a.current_temperature)} mesurés`);
  if (state.state === "off") parts.push("Arrêté");
  else if (typeof a.preset_mode === "string") parts.push(lookup(PRESET_LABELS, a.preset_mode) ?? a.preset_mode);
  if (a.derogation === true) parts.push("manuel");
  return parts.join(" · ") || statusText(state);
}

function heatingRow(state) {
  const a = state.attributes;
  const on = state.state === "heat";
  const target = finite(a.temperature) ? a.temperature : null;
  const step = finite(a.target_temp_step) && a.target_temp_step > 0 ? a.target_temp_step : 0.5;
  const min = finite(a.min_temp) ? a.min_temp : 5;
  const max = finite(a.max_temp) ? a.max_temp : 30;
  const water = state.entity_id === "climate.eau_chaude";
  const adjust = (delta, label) => {
    const next = target === null ? null : Math.min(max, Math.max(min, target + delta));
    const button = el("button", { type: "button", class: "round", "aria-label": `${label} la consigne de ${nameOf(state)}`, text: delta > 0 ? "+" : "−" });
    button.disabled = !on || next === null || next === target;
    button.addEventListener("click", () => {
      row.classList.add("pending");
      setTimeout(() => row.classList.remove("pending"), 5000);
      callService("climate", "set_temperature", { entity_id: state.entity_id, temperature: next });
    });
    return button;
  };
  const targetText = on && target !== null ? degreesText(target) : "—";
  const row = el("div", { class: `cover-row heat-row${on ? " on" : ""}${a.hvac_action === "heating" ? " heating" : ""}` },
    el("span", { class: "house-action-icon" }, svgIcon(water ? ICONS.humidity : ICONS.flame)),
    el("span", { class: "house-action-text" }, el("b", { text: nameOf(state) }), el("span", { text: heatingText(state) })),
    canControl(state)
      ? el("span", { class: "cover-row-buttons" },
        adjust(-step, "Baisser"),
        el("strong", { class: "heat-target", title: "Consigne", text: targetText }),
        adjust(step, "Monter"))
      : el("strong", { class: "heat-target", title: "Consigne", text: targetText }));
  return row;
}

/** Le chauffage piloté par la box Somfy (les entités `climate`), les zones
 *  d'abord, l'eau chaude ensuite. */
function heatingStates(inPage) {
  const placed = new Set(areas.flatMap((a) => a.entities.map((e) => e.entity_id)));
  return [...entities.values()]
    .filter((s) => domainOf(s) === "climate"
      // Une entité rangée dans une pièce ne s'affiche que sur la page de cette pièce.
      && (inPage === null || !placed.has(s.entity_id) || inPage.has(s.entity_id)))
    .sort((a, b) => (a.entity_id === "climate.eau_chaude") - (b.entity_id === "climate.eau_chaude")
      || nameOf(a).localeCompare(nameOf(b), "fr"));
}

/** Le chauffage, puis les volets (pas les portes de garage ni les portails,
 *  ni ceux déjà en boutons rapides), en lignes sous le dessin. */
function renderCovers() {
  const box = $("#board-covers");
  const inPage = pageAreas === null ? null
    : new Set(areas.filter((a) => pageAreas.has(a.name)).flatMap((a) => a.entities.map((e) => e.entity_id)));
  const covers = [...entities.values()]
    .filter((s) => domainOf(s) === "cover" && !ALERT_COVERS.includes(s.attributes.device_class)
      && !house.config.actions.includes(s.entity_id) && (inPage === null || inPage.has(s.entity_id)))
    .sort((a, b) => nameOf(a).localeCompare(nameOf(b), "fr"));
  const heating = heatingStates(inPage);
  box.hidden = covers.length === 0 && heating.length === 0;
  const title = heating.length === 0 ? "Volets" : covers.length === 0 ? "Chauffage" : "Chauffage et volets";
  box.setAttribute("aria-label", title);
  box.replaceChildren(el("h2", { text: title }), el("div", { class: "cover-rows" }, ...heating.map(heatingRow), ...covers.map(coverRow)));
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
        if (roomShown()) renderRoomHead();
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
  if (roomShown()) renderRoomHead();
  const board = isBoard();
  $("#house").hidden = !board;
  $("#title-block").hidden = !isPlan();
  if (!board) {
    $("#board-covers").hidden = true;
    $("#board-people").hidden = true;
    $("#board-charts").hidden = true;
    $("#board-agenda").hidden = true;
    $("#board-music").hidden = true;
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
  if (typeof renderBoardAgenda === "function") renderBoardAgenda();
  if (typeof renderBoardMusic === "function") renderBoardMusic();
  if (isPlan()) renderTitleBlock();
}

// ---------------------------------------------------------------- Une pièce, en fenêtre
// Toucher une pièce (sa tuile, ou une zone de la maquette) l'ouvre par-dessus
// le tableau : ses mesures, sa température sur 24 heures, puis tous ses
// appareils, rangés comme les sections, avec les tuiles habituelles.

const roomView = { name: null };
// Les groupes de la fenêtre : ceux des sections, plus les scènes.
const ROOM_GROUPS = [...SECTIONS, { title: "Scènes", domains: ["scene", "script"] }];

function roomArea() {
  if (roomView.name === OTHERS) {
    const states = looseStates().sort((a, b) => nameOf(a).localeCompare(nameOf(b), "fr"));
    return { name: OTHERS_NAME, entities: states.map((state) => ({ entity_id: state.entity_id })) };
  }
  return areas.find((area) => area.name === roomView.name) ?? null;
}

// ---------------------------------------------------------------- Une pièce, en onglet (étape 90)
// Mise en page « tableau » : chaque pièce a son onglet en haut de l'écran. La
// pièce prend alors toute la page, à la place de la maison : on y déplace le
// contenu de la fenêtre (mêmes éléments, mêmes mises à jour).
/** La pièce se voit : en fenêtre ou en onglet. */
function roomShown() {
  return $("#room").open || roomTab.open;
}

function openRoomTab(name) {
  if (typeof agenda !== "undefined" && agenda.open) closeAgenda(false);
  if (typeof music !== "undefined" && music.open) closeMusic(false);
  if ($("#room").open) $("#room").close();
  roomView.name = name;
  if (!roomArea()) {
    roomView.name = null;
    return;
  }
  roomTab.open = true;
  $(".layout").hidden = true;
  $("#empty").hidden = true;
  $("#room-view").append($(".room-sheet"));
  $("#room-view").hidden = false;
  document.documentElement.dataset.roomTab = "open";
  renderRoom();
  renderTabs();
  $("#dock-home").classList.remove("active");
  window.scrollTo({ top: 0 });
}

/** Retour à la maison. `render` : redessiner le tableau (sauf si l'appelant
 *  s'en charge). */
function closeRoomTab(render = true) {
  if (!roomTab.open) return;
  roomTab.open = false;
  roomView.name = null;
  $("#room").append($(".room-sheet"));
  $("#room-body").replaceChildren();
  $("#room-view").hidden = true;
  $(".layout").hidden = false;
  delete document.documentElement.dataset.roomTab;
  $("#dock-home").classList.add("active");
  if (render) renderAll();
  renderTabs();
}

$("#dock-home").addEventListener("click", () => closeRoomTab());

function roomStates(area) {
  return area.entities.map((entity) => entities.get(entity.entity_id)).filter((state) => state !== undefined);
}

/** Le capteur de température de la pièce (pour la courbe), s'il y en a un. */
function roomThermometer(states) {
  return states.find((s) => domainOf(s) === "sensor" && s.attributes.device_class === "temperature" && Number.isFinite(Number(s.state))) ?? null;
}

function openRoom(name) {
  // Le tableau : la pièce s'ouvre dans son onglet. La vue hub garde la fenêtre.
  if (isBoard() && document.documentElement.dataset.hub !== "on") return openRoomTab(name);
  closeRoomTab();
  roomView.name = name;
  if (!roomArea()) return;
  renderRoom();
  const dialog = $("#room");
  if (!dialog.open) dialog.showModal();
  $("#room-close").focus();
}

/** Toute la fenêtre : à l'ouverture seulement. Ensuite, les tuiles sont
 *  remplacées une à une (app.js) et l'en-tête est redessiné. */
function renderRoom() {
  const area = roomArea();
  if (!area) return;
  $("#room-title").textContent = area.name;
  $("#room-icon").replaceChildren(svgIcon(roomIcon(area.name)));
  const states = roomStates(area);
  const used = new Set();
  const groups = ROOM_GROUPS.map(({ title, domains, extra }) => {
    const items = states.filter((s) => !used.has(s.entity_id) && (domains.includes(domainOf(s)) || extra?.(s)));
    items.forEach((s) => used.add(s.entity_id));
    return { title, items };
  });
  groups.push({ title: "Autres", items: states.filter((s) => !used.has(s.entity_id)) });
  const shown = groups.filter((group) => group.items.length > 0);
  $("#room-body").replaceChildren(...(shown.length > 0
    ? shown.map(({ title, items }) => el("section", { class: "room-group" }, el("h3", { text: title }), el("div", { class: "grid" }, ...items.map(renderTile))))
    : [el("p", { class: "auto-meta", text: "Aucun appareil dans cette pièce." })]));
  renderRoomHead();
}

/** Les mesures, la courbe et « Éteindre les lumières ». */
function renderRoomHead() {
  const area = roomArea();
  if (!area) return;
  const states = roomStates(area);
  const info = roomInfo(area);
  const parts = [];
  if (Number.isFinite(info.temperature)) parts.push(`${number.format(info.temperature)} °C`);
  if (finite(info.humidity)) parts.push(`${number.format(info.humidity)} %`);
  if (info.alert) parts.push(info.alert);
  if (info.lights > 0) parts.push(`${info.lights} lumière${info.lights > 1 ? "s" : ""} allumée${info.lights > 1 ? "s" : ""}`);
  if (info.open > 0) parts.push(`${info.open} ouverture${info.open > 1 ? "s" : ""}`);
  if (info.heating) parts.push("chauffe");
  $("#room-measures").textContent = parts.join(" · ") || `${states.length} appareil${states.length > 1 ? "s" : ""}`;
  $("#room-icon").className = `room-icon${info.alert ? " alert" : info.lights > 0 ? " lit" : info.heating ? " heat" : ""}`;

  // La température sur 24 heures.
  const box = $("#room-chart");
  const thermometer = roomThermometer(states);
  box.hidden = thermometer === null || account.permissions.history === false;
  if (!box.hidden) {
    const chart = { entity_id: thermometer.entity_id, hours: 24, color: "orange" };
    const card = el("button", { type: "button", class: "room-chart-card chart-orange", "aria-label": `${nameOf(thermometer)} : ${labelValue(thermometer)}. Ouvrir l'historique.` },
      el("span", { class: "room-chart-label" }, el("b", { text: "Température" }), el("span", { text: "24 heures" })),
      sparkline(chart, seriesFor(chart) ?? []));
    card.addEventListener("click", () => openHistory(thermometer.entity_id));
    box.replaceChildren(card);
  }

  // Éteindre d'un coup les lumières allumées (celles que ce compte commande).
  // Une prise Hue n'est pas une lumière : un radiateur peut y être branché.
  const lit = states.filter((s) => isLight(s) && s.state === "on" && canControl(s));
  const actions = $("#room-actions");
  actions.hidden = lit.length === 0;
  if (lit.length > 0) {
    const off = el("button", { type: "button", class: "ghost room-off" }, svgIcon(ICONS.bulb), `Éteindre les lumières (${lit.length})`);
    off.addEventListener("click", () => {
      for (const state of lit) {
        markPending(state.entity_id);
        callService("light", "turn_off", { entity_id: state.entity_id });
      }
    });
    actions.replaceChildren(off);
  }
}

$("#room-close").addEventListener("click", () => $("#room").close());
// Un toucher à côté de la fenêtre (sur le fond) la ferme.
$("#room").addEventListener("click", (event) => { if (event.target === $("#room")) $("#room").close(); });
$("#room").addEventListener("close", () => { roomView.name = null; $("#room-body").replaceChildren(); $(".room-sheet").style.transform = ""; });

// Sur téléphone, la fenêtre monte du bas : on la ferme en la tirant vers le bas
// par son en-tête.
{
  const sheet = $(".room-sheet");
  let start = null;
  $(".room-top").addEventListener("pointerdown", (event) => {
    if (event.pointerType === "mouse" || event.target.closest("button") || roomTab.open) return;
    start = event.clientY;
    $(".room-top").setPointerCapture(event.pointerId);
  });
  $(".room-top").addEventListener("pointermove", (event) => {
    if (start === null) return;
    sheet.style.transform = `translateY(${Math.max(0, event.clientY - start)}px)`;
  });
  for (const type of ["pointerup", "pointercancel"]) {
    $(".room-top").addEventListener(type, (event) => {
      if (start === null) return;
      const moved = event.clientY - start;
      start = null;
      if (type === "pointerup" && moved > 90) $("#room").close();
      else sheet.style.transform = "";
    });
  }
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
const drawingEdit = { pages: null, draft: null, placing: -1, tracing: -1, view: null, cut: null };

async function openDrawingEditor() {
  $("#account-dialog").close();
  await loadHouse(true);
  await Promise.all(house.images.map((image) => houseImage(image.name)));
  drawingEdit.pages = structuredClone(house.pages);
  drawingEdit.draft = drawingEdit.pages.find((page) => page.id === house.config?.id) ?? drawingEdit.pages[0];
  drawingEdit.placing = -1;
  drawingEdit.tracing = -1;
  drawingEdit.cut = null;
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

// ---------------------------------------------------------------- Découpe
// Étape 72 : découper les calques tout seul. Deux images de la même taille,
// « tout éteint » et « allumé » (ou « volets fermés ») : ce qui change devient
// un calque par endroit, à nommer et à relier à un appareil. Tout se fait dans
// le navigateur ; seules les images finies sont envoyées.

/** Les pixels d'une image du dessin (déjà chargée). */
async function imagePixels(name) {
  const url = houseUrl(name) ?? (await houseImage(name));
  if (!url) throw new Error(`image « ${name} » introuvable`);
  const img = new Image();
  img.src = url;
  await img.decode();
  const canvas = document.createElement("canvas");
  canvas.width = img.naturalWidth;
  canvas.height = img.naturalHeight;
  const context = canvas.getContext("2d", { willReadFrequently: true });
  context.drawImage(img, 0, 0);
  return { width: canvas.width, height: canvas.height, data: context.getImageData(0, 0, canvas.width, canvas.height).data };
}

/** Ce qui change entre deux images : des régions sur une grille de
 *  `cell` pixels, regroupées à `gap` cases près. */
function findChanges(before, after, { threshold, gap, cell = 4 }) {
  const { width, height } = before;
  const diff = new Uint8Array(width * height);
  for (let i = 0, p = 0; p < diff.length; p += 1, i += 4) {
    diff[p] = Math.max(
      Math.abs(before.data[i] - after.data[i]),
      Math.abs(before.data[i + 1] - after.data[i + 1]),
      Math.abs(before.data[i + 2] - after.data[i + 2]),
    );
  }
  const gw = Math.ceil(width / cell);
  const gh = Math.ceil(height / cell);
  const marked = new Uint8Array(gw * gh);
  for (let gy = 0; gy < gh; gy += 1) {
    for (let gx = 0; gx < gw; gx += 1) {
      let hits = 0;
      let count = 0;
      for (let y = gy * cell; y < Math.min(height, (gy + 1) * cell); y += 1) {
        for (let x = gx * cell; x < Math.min(width, (gx + 1) * cell); x += 1) {
          count += 1;
          if (diff[y * width + x] > threshold) hits += 1;
        }
      }
      marked[gy * gw + gx] = hits * 4 > count ? 1 : 0;
    }
  }
  // Regrouper : on élargit, puis on cherche les morceaux d'un seul tenant.
  const grown = new Uint8Array(marked);
  for (let gy = 0; gy < gh; gy += 1) {
    for (let gx = 0; gx < gw; gx += 1) {
      if (!marked[gy * gw + gx]) continue;
      for (let dy = -gap; dy <= gap; dy += 1) {
        for (let dx = -gap; dx <= gap; dx += 1) {
          const x = gx + dx;
          const y = gy + dy;
          if (x >= 0 && y >= 0 && x < gw && y < gh) grown[y * gw + x] = 1;
        }
      }
    }
  }
  const label = new Int32Array(gw * gh).fill(-1);
  const regions = [];
  for (let start = 0; start < grown.length; start += 1) {
    if (!grown[start] || label[start] >= 0) continue;
    const id = regions.length;
    const cells = [];
    const stack = [start];
    label[start] = id;
    let minX = gw; let minY = gh; let maxX = 0; let maxY = 0; let core = 0;
    while (stack.length > 0) {
      const at = stack.pop();
      cells.push(at);
      if (marked[at]) core += 1;
      const x = at % gw;
      const y = (at - x) / gw;
      minX = Math.min(minX, x); maxX = Math.max(maxX, x);
      minY = Math.min(minY, y); maxY = Math.max(maxY, y);
      for (const next of [x > 0 ? at - 1 : -1, x < gw - 1 ? at + 1 : -1, y > 0 ? at - gw : -1, y < gh - 1 ? at + gw : -1]) {
        if (next >= 0 && grown[next] && label[next] < 0) {
          label[next] = id;
          stack.push(next);
        }
      }
    }
    regions.push({ id, cells, core, box: { x: minX * cell, y: minY * cell, w: (maxX - minX + 1) * cell, h: (maxY - minY + 1) * cell } });
  }
  // Les miettes (moins de 40 pixels changés) ne font pas un calque.
  const kept = regions.filter((region) => region.core * cell * cell >= 40 * 4).sort((a, b) => b.core - a.core).slice(0, 30);
  return { width, height, cell, gw, label, diff, regions: kept };
}

/** Le calque d'une région : les pixels de l'image « après », opaques là où
 *  ça change franchement, en fondu là où ça change peu (un halo). */
async function cutLayer(found, after, region, threshold, solid) {
  const { width, height, cell, gw, label, diff } = found;
  // Un volet est plein : on bouche les petits trous (les joints sombres des
  // lames, aussi sombres que la vitre derrière) par une fermeture de 2 px.
  const filled = solid ? closeMask(found, region, Math.max(2, threshold / 3)) : null;
  const canvas = document.createElement("canvas");
  canvas.width = width;
  canvas.height = height;
  const context = canvas.getContext("2d");
  const out = context.createImageData(width, height);
  const soft = Math.max(2, threshold / 3);
  const { x: bx, y: by, w: bw, h: bh } = region.box;
  for (let y = by; y < Math.min(height, by + bh); y += 1) {
    for (let x = bx; x < Math.min(width, bx + bw); x += 1) {
      if (label[Math.floor(y / cell) * gw + Math.floor(x / cell)] !== region.id) continue;
      const p = y * width + x;
      const alpha = filled
        ? filled[(y - by) * bw + (x - bx)]
        : Math.min(1, Math.max(0, (diff[p] - soft) / (threshold * 1.5 - soft)));
      if (alpha <= 0) continue;
      const i = p * 4;
      out.data[i] = after.data[i];
      out.data[i + 1] = after.data[i + 1];
      out.data[i + 2] = after.data[i + 2];
      out.data[i + 3] = Math.round(alpha * 255);
    }
  }
  context.putImageData(out, 0, 0);
  const webp = await new Promise((resolve) => canvas.toBlob(resolve, "image/webp", 0.85));
  if (webp && webp.type === "image/webp") return webp;
  return new Promise((resolve) => canvas.toBlob(resolve, "image/png"));
}

/** Le masque plein d'une région (dilatation puis érosion de 2 px), sur
 *  son cadre. */
function closeMask(found, region, soft) {
  const { width, cell, gw, label, diff } = found;
  const { x: bx, y: by, w: bw, h: bh } = region.box;
  let mask = new Uint8Array(bw * bh);
  for (let y = 0; y < bh; y += 1) {
    for (let x = 0; x < bw; x += 1) {
      const gx = Math.floor((bx + x) / cell);
      const gy = Math.floor((by + y) / cell);
      const p = (by + y) * width + bx + x;
      if (label[gy * gw + gx] === region.id && diff[p] > soft) mask[y * bw + x] = 1;
    }
  }
  const pass = (source, grow) => {
    const out = new Uint8Array(source.length);
    for (let y = 0; y < bh; y += 1) {
      for (let x = 0; x < bw; x += 1) {
        let any = false;
        let all = true;
        for (let dy = -2; dy <= 2; dy += 1) {
          for (let dx = -2; dx <= 2; dx += 1) {
            const xx = x + dx;
            const yy = y + dy;
            const on = xx >= 0 && yy >= 0 && xx < bw && yy < bh && source[yy * bw + xx] === 1;
            any ||= on;
            all &&= on;
          }
        }
        out[y * bw + x] = (grow ? any : all) ? 1 : 0;
      }
    }
    return out;
  };
  mask = pass(pass(mask, true), false);
  return mask;
}

/** Un aperçu réduit de l'image « après », avec les régions numérotées. */
function cutPreview(found, after) {
  const scale = Math.min(1, 720 / found.width);
  const full = document.createElement("canvas");
  full.width = found.width;
  full.height = found.height;
  full.getContext("2d").putImageData(new ImageData(new Uint8ClampedArray(after.data), found.width, found.height), 0, 0);
  const canvas = el("canvas", { class: "cut-preview" });
  canvas.width = Math.round(found.width * scale);
  canvas.height = Math.round(found.height * scale);
  const context = canvas.getContext("2d");
  context.drawImage(full, 0, 0, canvas.width, canvas.height);
  context.lineWidth = 2;
  context.font = "bold 14px sans-serif";
  found.regions.forEach((region, index) => {
    const { x, y, w, h } = region.box;
    context.strokeStyle = "#00e5ff";
    context.strokeRect(x * scale, y * scale, w * scale, h * scale);
    context.fillStyle = "#00e5ff";
    context.fillRect(x * scale, y * scale - 16, 22, 16);
    context.fillStyle = "#000";
    context.fillText(String(index + 1), x * scale + 4, y * scale - 3);
  });
  return canvas;
}

function cutPart(draft, refresh) {
  const cut = drawingEdit.cut ?? (drawingEdit.cut = { before: draft.background ?? "", after: "", threshold: 24, gap: 3, found: null, entries: [] });
  const before = imageOptions(cut.before, "— choisir —");
  before.addEventListener("change", () => { cut.before = before.value; });
  const after = imageOptions(cut.after, "— choisir —");
  after.addEventListener("change", () => { cut.after = after.value; });
  const threshold = el("input", { type: "range", min: "8", max: "80", value: String(cut.threshold), "aria-label": "Sensibilité" });
  threshold.addEventListener("change", () => { cut.threshold = Number(threshold.value); });
  const gap = el("input", { type: "range", min: "0", max: "10", value: String(cut.gap), "aria-label": "Regrouper" });
  gap.addEventListener("change", () => { cut.gap = Number(gap.value); });
  const search = el("button", { type: "button", class: "ghost small", text: "Chercher les différences" });
  search.addEventListener("click", async () => {
    if (!cut.before || !cut.after || cut.before === cut.after) return drawingStatus("Choisis deux images différentes.", "error");
    search.disabled = true;
    drawingStatus("Comparaison…");
    try {
      const [a, b] = await Promise.all([imagePixels(cut.before), imagePixels(cut.after)]);
      if (a.width !== b.width || a.height !== b.height) {
        throw new Error(`les deux images n'ont pas la même taille (${a.width}×${a.height} et ${b.width}×${b.height})`);
      }
      // Une sensibilité haute = un seuil bas.
      cut.found = findChanges(a, b, { threshold: 88 - cut.threshold, gap: cut.gap });
      cut.pixels = b;
      cut.entries = cut.found.regions.map((_, index) => ({ keep: true, name: `${cut.after}_${index + 1}`.slice(0, 40), kind: "light", entity_id: "" }));
      drawingStatus(cut.found.regions.length === 0 ? "Aucune différence trouvée : augmente la sensibilité." : `${cut.found.regions.length} endroit(s) qui changent.`, "ok");
    } catch (error) {
      drawingStatus(`Comparaison impossible : ${error.message}.`, "error");
    }
    search.disabled = false;
    refresh();
  });
  const children = [
    el("div", { class: "drawing-grid" },
      field("Avant (tout éteint, volets ouverts)", before),
      field("Après (allumé, ou volets fermés)", after),
      field("Sensibilité", threshold),
      field("Regrouper les endroits proches", gap)),
    search,
  ];
  if (cut.found && cut.found.regions.length > 0) {
    children.push(cutPreview(cut.found, cut.pixels));
    const rows = cut.entries.map((entry, index) => {
      const keep = el("input", { type: "checkbox" });
      keep.checked = entry.keep;
      keep.addEventListener("change", () => { entry.keep = keep.checked; });
      const name = el("input", { type: "text", value: entry.name, maxlength: "40", autocapitalize: "none", spellcheck: "false", "aria-label": `Nom du calque ${index + 1}` });
      name.addEventListener("change", () => { entry.name = name.value.trim(); });
      const kind = el("select", { "aria-label": `Sorte du calque ${index + 1}` },
        el("option", { value: "light", text: "Lumière allumée" }),
        el("option", { value: "cover", text: "Volet (suit sa position)" }),
        el("option", { value: "other", text: "Autre (état « on »)" }));
      kind.value = entry.kind;
      kind.addEventListener("change", () => { entry.kind = kind.value; });
      const entity = el("input", { type: "text", list: "entity-list", value: entry.entity_id, placeholder: "light.porche", autocapitalize: "none", spellcheck: "false", "aria-label": `Appareil du calque ${index + 1}` });
      entity.addEventListener("change", () => { entry.entity_id = entity.value.trim(); });
      return el("li", { class: "drawing-item" },
        el("label", { class: "check" }, keep, el("b", { text: ` Endroit ${index + 1}` })),
        el("div", { class: "drawing-grid" }, field("Nom de l'image", name), field("Sorte", kind), field("Appareil", entity)));
    });
    const create = el("button", { type: "button", class: "ghost small", text: "Créer les calques cochés" });
    create.addEventListener("click", async () => {
      const chosen = cut.entries.map((entry, index) => ({ entry, region: cut.found.regions[index] })).filter(({ entry }) => entry.keep);
      for (const { entry } of chosen) {
        if (!/^[a-z0-9_-]{1,40}$/.test(entry.name)) return drawingStatus(`Nom « ${entry.name} » : lettres minuscules, chiffres, _ et - (40 au plus).`, "error");
        if (entry.entity_id === "") return drawingStatus(`Choisis l'appareil de « ${entry.name} ».`, "error");
        if (entry.kind === "cover" && !entry.entity_id.startsWith("cover.")) return drawingStatus(`« ${entry.name} » suit un volet : un cover.… est attendu.`, "error");
      }
      create.disabled = true;
      let done = 0;
      for (const { entry, region } of chosen) {
        drawingStatus(`Découpe ${done + 1} sur ${chosen.length}…`);
        const blob = await cutLayer(cut.found, cut.pixels, region, 88 - cut.threshold, entry.kind === "cover");
        if (!blob || blob.size > MAX_HOUSE_IMAGE) {
          create.disabled = false;
          return drawingStatus(`« ${entry.name} » : image trop grosse.`, "error");
        }
        const response = await fetch(`/api/home/images/${encodeURIComponent(entry.name)}`, {
          method: "PUT",
          headers: { Authorization: `Bearer ${token.get()}`, "Content-Type": blob.type },
          body: blob,
        }).catch(() => null);
        if (!response?.ok) {
          const data = await response?.json().catch(() => ({}));
          create.disabled = false;
          return drawingStatus(data?.error ?? `Envoi de « ${entry.name} » impossible.`, "error");
        }
        const layer = entry.kind === "cover"
          ? { image: entry.name, entity_id: entry.entity_id, states: [], not: false, position: true, when: "always" }
          : { image: entry.name, entity_id: entry.entity_id, states: ["on"], not: false, when: "always" };
        draft.layers = draft.layers.filter((other) => other.image !== entry.name);
        draft.layers.push(layer);
        done += 1;
      }
      await loadHouse(true);
      await Promise.all(house.images.map((image) => houseImage(image.name)));
      cut.found = null;
      cut.entries = [];
      drawingStatus(`${done} calque(s) créé(s) : vérifie-les dans l'aperçu, puis « Enregistrer ».`, "ok");
      refresh();
    });
    children.push(el("ol", { class: "drawing-items" }, ...rows), create);
  }
  return drawingFieldset(
    "Découper des calques",
    "Deux images de la même taille : « avant » (tout éteint, volets ouverts) et « après » (lumières allumées, ou volets fermés). Ce qui change devient un calque par endroit : donne-lui un nom et son appareil.",
    ...children,
  );
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

  // Les demandes pour l'IA : choisir l'image à faire, copier la demande.
  const aiStyle = el("select", { "aria-label": "Style des images" },
    el("option", { value: "maquette", text: "Maquette 3D (diorama)" }), el("option", { value: "plan", text: "Plan d'architecte" }));
  aiStyle.value = drawingEdit.aiStyle ?? (isPlan() ? "plan" : "maquette");
  const aiChoice = el("select", { "aria-label": "Image à faire" },
    el("optgroup", { label: "Pour commencer" },
      el("option", { value: "analyse", text: "1. Faire décrire la maison (sans image)" }),
      el("option", { value: "jour", text: "2. Le fond de jour, au soleil" })),
    ...MOMENTS.map(([moment, momentLabel]) => el("optgroup", { label: `Ambiances : ${momentLabel.toLowerCase()}` },
      ...WEATHERS.filter(([key]) => !(moment === "jour" && key === "soleil")).map(([key, label]) => {
        const value = key === "soleil" ? moment : `${moment}-${key}`;
        const weatherText = moment === "nuit" && key === "soleil" ? "ciel clair" : label.toLowerCase();
        return el("option", { value, text: `${momentLabel}, ${weatherText}` });
      }))),
    el("optgroup", { label: "Calques" }, ...aiLayerEntities(draft).map((state) => el("option", { value: `calque:${state.entity_id}`, text: aiLayerName(state) }))),
    el("optgroup", { label: "Si l'IA se trompe" }, ...AI_CORRECTIONS.map(([key, label]) => el("option", { value: `correction:${key}`, text: label }))));
  aiChoice.value = drawingEdit.aiChoice ?? (draft.background ? "nuit" : "analyse");
  // Modifiable : on peut préciser où est le porche avant de copier.
  const aiText = el("textarea", { class: "ai-prompt", rows: "8", spellcheck: "false", "aria-label": "Demande à copier (modifiable)" });
  const aiAttach = el("span");
  const aiSave = el("span");
  const aiCopy = el("button", { type: "button", class: "primary small", text: "Copier la demande" });
  const aiShow = () => {
    drawingEdit.aiStyle = aiStyle.value;
    drawingEdit.aiChoice = aiChoice.value;
    const prompt = aiPrompt(aiChoice.value, draft, aiStyle.value);
    aiText.value = prompt.text;
    aiAttach.textContent = prompt.attach;
    aiSave.textContent = prompt.save;
  };
  aiStyle.addEventListener("change", aiShow);
  aiChoice.addEventListener("change", aiShow);
  aiCopy.addEventListener("click", async () => {
    const done = await copyText(aiText);
    drawingStatus(done ? "Demande copiée : colle-la dans Gemini ou ChatGPT, avec l'image à joindre." : "Copie impossible ici : sélectionne le texte et copie-le.", done ? "ok" : "error");
  });
  aiShow();
  const ai = drawingFieldset(
    "Faire les images avec l'IA",
    "Choisis l'image à faire, copie la demande et colle-la dans une IA qui retouche les photos (Gemini, ChatGPT…), avec l'image indiquée. Fais toujours les variantes à partir du fond de jour, jamais l'une de l'autre : le dessin ne bouge pas.",
    el("div", { class: "drawing-grid" }, field("Style", aiStyle), field("Image à faire", aiChoice)),
    aiText,
    el("dl", { class: "ai-steps" }, el("dt", { text: "Joindre" }), el("dd", {}, aiAttach), el("dt", { text: "Enregistrer sous" }), el("dd", {}, aiSave)),
    aiCopy,
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

  // Les effets animés.
  const effectMode = el("select", { "aria-label": "Effets animés" },
    el("option", { value: "", text: "Automatiques (selon la météo)" }),
    el("option", { value: "off", text: "Désactivés" }),
    ...EFFECTS.map(([key, label]) => el("option", { value: key, text: `Toujours : ${label.toLowerCase()}` })));
  effectMode.value = draft.effects ?? "";
  effectMode.addEventListener("change", () => {
    if (effectMode.value) draft.effects = effectMode.value; else delete draft.effects;
    renderDrawingPreview();
  });
  const effectLevel = el("select", { "aria-label": "Intensité des effets" }, ...EFFECT_LEVELS.map(([value, text]) => el("option", { value, text })));
  effectLevel.value = draft.effects_level ?? "";
  effectLevel.addEventListener("change", () => {
    if (effectLevel.value) draft.effects_level = effectLevel.value; else delete draft.effects_level;
    renderDrawingPreview();
  });
  const nowEffect = effectOf({ ...draft, effects: undefined });
  const effects = drawingFieldset(
    "Effets animés",
    "Sur le dessin, le temps qu'il fait : pluie, averses, orage, grêle, neige, brouillard, vent, et des étoiles par nuit claire. Ils suivent la météo comme les ambiances ; « Toujours » sert à essayer. Pas d'animation si l'appareil demande de réduire les mouvements.",
    el("div", { class: "drawing-grid" }, field("Effets", effectMode), field("Intensité", effectLevel)),
    el("p", { class: "auto-meta", text: `Maintenant : ${nowEffect ? (EFFECTS.find(([key]) => key === nowEffect)?.[1] ?? nowEffect).toLowerCase() : "aucun effet"}` }),
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
    // Un volet qui suit sa position : l'image est le volet fermé.
    const position = el("input", { type: "checkbox" });
    position.checked = layer.position === true;
    states.disabled = position.checked;
    not.disabled = position.checked;
    position.addEventListener("change", () => {
      layer.position = position.checked;
      states.disabled = position.checked;
      not.disabled = position.checked;
      renderDrawingPreview();
    });
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
      el("label", { class: "check" }, not, " Sauf ces états (visible pour tous les autres)"),
      el("label", { class: "check" }, position, " Suit la position du volet (l'image est le volet fermé ; à 25 %, il est dessiné aux trois quarts)"));
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
    // Une zone commande un appareil, ou ouvre une pièce.
    const kind = el("select", {}, el("option", { value: "entity", text: "Commander un appareil" }), el("option", { value: "area", text: "Ouvrir une pièce" }));
    kind.value = zone.area !== undefined ? "area" : "entity";
    kind.disabled = areas.length === 0 && zone.area === undefined;
    kind.addEventListener("change", () => {
      if (kind.value === "area") { delete zone.entity_id; zone.area = areas[0]?.name ?? ""; } else { delete zone.area; zone.entity_id = ""; }
      refresh();
    });
    let target;
    if (zone.area !== undefined) {
      target = el("select", {}, ...areas.map((area) => el("option", { value: area.name, text: area.name })));
      target.value = zone.area;
      target.addEventListener("change", () => { zone.area = target.value; });
    } else {
      target = el("input", { type: "text", list: "entity-list", value: zone.entity_id ?? "", placeholder: "light.porche", autocapitalize: "none", spellcheck: "false" });
      target.addEventListener("change", () => { zone.entity_id = target.value.trim(); });
    }
    const name = el("input", { type: "text", value: zone.name ?? "", placeholder: zone.area !== undefined ? "(le nom de la pièce)" : "(le nom de l'appareil)", maxlength: "60" });
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
      el("div", { class: "drawing-grid" }, field("Un toucher…", kind), field(zone.area !== undefined ? "Pièce" : "Appareil", target), field("Nom affiché", name)),
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
    "Le porche, la porte du garage, le salon… : trace un rectangle sur le dessin. Un toucher allume, éteint, ouvre ou ferme l'appareil, ou ouvre la pièce ; un appui long ouvre l'historique (24 au plus).",
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

  $("#drawing-form").replaceChildren(pagesPart(), ai, images, cutPart(draft, refresh), backgrounds, effects, layers, labels, zones, actions, charts);
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
    page.zones = page.zones.filter((zone) => (zone.area ?? zone.entity_id ?? "") !== "");
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
// Le dessin change de taille (déplacé dans la vue hub ou l'écran de veille,
// image chargée) : les étiquettes sont replacées, une fois la page calculée.
new ResizeObserver(() => { if (house.view) fitLabels(house.view); }).observe($("#house-stage"));
// La hauteur de l'en-tête (une ou deux lignes, selon le nombre d'onglets) :
// le reste de l'écran est pour la maison, l'agenda ou la pièce (étape 90).
new ResizeObserver(([entry]) => {
  document.documentElement.style.setProperty("--hero-h", `${Math.ceil(entry.target.offsetHeight)}px`);
}).observe($(".hero"));

/** Un fondu au bord d'une liste qui défile (les onglets, les volets sur une
 *  tablette), tant qu'il reste quelque chose à voir de ce côté. */
function edgeFade(box, horizontal) {
  const update = () => {
    const pos = horizontal ? box.scrollLeft : box.scrollTop;
    const max = horizontal ? box.scrollWidth - box.clientWidth : box.scrollHeight - box.clientHeight;
    box.classList.toggle("fade-start", pos > 1);
    box.classList.toggle("fade-end", pos < max - 1);
  };
  box.addEventListener("scroll", update, { passive: true });
  new ResizeObserver(update).observe(box);
  new MutationObserver(update).observe(box, { childList: true, subtree: true });
  update();
}
edgeFade($("#board-tabs"), true);
edgeFade($("#board-covers"), false);
