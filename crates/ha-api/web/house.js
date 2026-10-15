// Le style « plan d'architecte » du tableau de bord : le dessin de la maison,
// ses calques et ses valeurs, les repères du haut et le cartouche.
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
  config: { layers: [], labels: [] },
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

// ---------------------------------------------------------------- Chargement

function emptyDrawing() {
  return { layers: [], labels: [] };
}

/** Recopie l'assemblage reçu en ne gardant que ce qu'on attend. */
function cleanDrawing(config) {
  const text = (value) => (typeof value === "string" && value !== "" ? value : undefined);
  const out = emptyDrawing();
  if (text(config?.background)) out.background = config.background;
  if (text(config?.night)) out.night = config.night;
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
  return out;
}

async function loadHouse(force = false) {
  if (house.loading && !force) return house.loading;
  house.loading = (async () => {
    const { status, data } = await api("GET", "/api/home/drawing");
    if (status !== 200) return;
    house.config = cleanDrawing(data.config);
    house.images = Array.isArray(data.images) ? data.images : [];
    house.editable = data.editable === true;
    const used = [house.config.background, house.config.night, ...house.config.layers.map((l) => l.image)];
    await Promise.all(used.filter(Boolean).map(houseImage));
    buildHouse();
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
  let night = null;
  const nightUrl = config.night ? houseUrl(config.night) : null;
  if (nightUrl) {
    night = el("img", { class: "house-layer house-night", alt: "", draggable: "false" });
    night.src = nightUrl;
    parts.push(night);
  }
  const layers = config.layers.map((layer) => {
    const url = houseUrl(layer.image);
    const img = el("img", { class: "house-layer", alt: "", draggable: "false" });
    if (url) img.src = url;
    parts.push(img);
    return { layer, img };
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
  const view = { stage, night, layers, labels };
  // La taille du dessin n'est connue qu'une fois l'image chargée.
  base.addEventListener("load", () => fitLabels(view));
  return view;
}

function updateHouse(view) {
  if (!view) return;
  const isNight = houseNight();
  view.stage.classList.toggle("is-night", isNight);
  view.night?.classList.toggle("off", !isNight);
  for (const { layer, img } of view.layers) img.classList.toggle("off", !layerShown(layer, isNight));
  for (const { label, name, value, node } of view.labels) {
    const state = entities.get(label.entity_id);
    name.textContent = label.name ?? (state ? nameOf(state) : label.entity_id);
    value.textContent = labelValue(state);
    node.setAttribute("aria-label", `${name.textContent} : ${value.textContent}`);
  }
  fitLabels(view);
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

function buildHouse() {
  house.view = drawHouse($("#house-stage"), house.config, { interactive: true });
  updateHouse(house.view);
  houseCaption();
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

function gauge(icon, label, value) {
  return el("span", { class: "gauge" }, svgIcon(icon), el("span", { text: label }), el("strong", { text: value }));
}

function renderGauges() {
  const box = $("#gauges");
  const items = [];
  const weather = [...entities.values()].find((s) => domainOf(s) === "weather");
  const outside = entities.get("sensor.meteo_temperature");
  const outsideTemp = finite(weather?.attributes.temperature) ? weather.attributes.temperature
    : outside && Number.isFinite(Number(outside.state)) ? Number(outside.state) : NaN;
  if (Number.isFinite(outsideTemp)) items.push(gauge(ICONS.temperature, "Dehors", `${number.format(outsideTemp)} °C`));
  const temps = areas.map(roomInfo).map((info) => info.temperature).filter((t) => Number.isFinite(t));
  if (temps.length > 0) {
    items.push(gauge(ICONS.home, "Dedans", `${number.format(temps.reduce((a, b) => a + b, 0) / temps.length)} °C`));
  }
  const power = entities.get("sensor.linky_puissance");
  if (power && Number.isFinite(Number(power.state))) {
    items.push(gauge(ICONS.flame, "Puissance", `${number.format(Number(power.state))} ${power.attributes.unit_of_measurement ?? "VA"}`));
  }
  const lights = [...entities.values()].filter((s) => domainOf(s) === "light" && s.state === "on").length;
  items.push(gauge(ICONS.bulb, "Lumières", lights === 0 ? "éteintes" : `${lights} allumée${lights > 1 ? "s" : ""}`));
  const zone = entities.get("zone.home");
  if (Array.isArray(zone?.attributes.persons)) {
    const count = zone.attributes.persons.length;
    items.push(gauge(ICONS.person, "Présents", String(count)));
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

/** Appelé par app.js à chaque changement (au plus une fois par image). */
function renderHouse() {
  const plan = isPlan();
  $("#house").hidden = !plan;
  $("#title-block").hidden = !plan;
  if (!plan) {
    $("#gauges").hidden = true;
    return;
  }
  if (house.loading === null && token.get()) loadHouse();
  updateHouse(house.view);
  renderGauges();
  renderTitleBlock();
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

const drawingEdit = { draft: null, placing: -1, view: null };

async function openDrawingEditor() {
  $("#account-dialog").close();
  await loadHouse(true);
  await Promise.all(house.images.map((image) => houseImage(image.name)));
  drawingEdit.draft = structuredClone(house.config);
  drawingEdit.placing = -1;
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
  $("#drawing-place-hint").hidden = drawingEdit.placing < 0;
  $("#drawing-preview").classList.toggle("placing", drawingEdit.placing >= 0);
}

/** En mode « placer », l'aperçu remonte en vue pour qu'on puisse le toucher. */
function showPreview() {
  if (drawingEdit.placing < 0) return;
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

  // Les fonds.
  const day = imageOptions(draft.background, "Dessin d'exemple");
  day.addEventListener("change", () => {
    if (day.value) draft.background = day.value;
    else { delete draft.background; delete draft.night; }
    refresh();
  });
  const night = imageOptions(draft.night, "Aucun (le même jour et nuit)");
  night.disabled = !draft.background;
  night.addEventListener("change", () => {
    if (night.value) draft.night = night.value;
    else delete draft.night;
    refresh();
  });
  const backgrounds = drawingFieldset("Fonds", "", el("div", { class: "drawing-grid" },
    field("Fond de jour", day), field("Fond de nuit (soleil couché)", night)));

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

  $("#drawing-form").replaceChildren(images, backgrounds, layers, labels);
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

$("#drawing-save").addEventListener("click", async () => {
  const draft = drawingEdit.draft;
  if (!draft) return;
  const button = $("#drawing-save");
  button.disabled = true;
  const { status, data } = await api("PUT", "/api/home/drawing", draft).catch(() => ({ status: 0, data: {} }));
  button.disabled = false;
  if (status !== 200) return drawingStatus(data.error ?? "Enregistrement impossible.", "error");
  house.config = cleanDrawing(data.config);
  buildHouse();
  $("#drawing").close();
  flash("Dessin de la maison enregistré");
});

for (const id of ["#drawing-close", "#drawing-cancel"]) $(id).addEventListener("click", () => $("#drawing").close());
$("#open-drawing").addEventListener("click", openDrawingEditor);
addEventListener("resize", () => { if (house.view) fitLabels(house.view); }, { passive: true });
