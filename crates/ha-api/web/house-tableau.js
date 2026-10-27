// La mise en page « tableau » — autour du dessin : les onglets, les
// repères, le cartouche, les boutons rapides, les volets, le chauffage, les
// courbes, les personnes, les machines en marche et la pièce ouverte.
//
// Un des fichiers de house.js, chargé après house-effets.js par index.html.
// Les scripts partagent leurs déclarations. Pas d'innerHTML ici non plus.
"use strict";

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

/** La couleur d'une température, de 0 °C (bleu) à 28 °C (rouge), en passant
 *  par le turquoise d'un matin frais et l'ambre d'un après-midi doux. */
const TEMP_RAMP = [[0, "#4fb8f5"], [8, "#4fd6c4"], [15, "#ffc15a"], [22, "#ff8f4d"], [28, "#ff5a45"]];

function tempColor(value) {
  const v = Math.min(28, Math.max(0, value));
  const i = Math.max(1, TEMP_RAMP.findIndex(([t]) => t >= v));
  const [[t0, c0], [t1, c1]] = [TEMP_RAMP[i - 1], TEMP_RAMP[i]];
  return `color-mix(in oklab, ${c1} ${Math.round((v - t0) / (t1 - t0) * 100)}%, ${c0})`;
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

/** Les boutons rapides de l'onglet, ceux dont l'appareil existe. */
function actionStates() {
  return house.config.actions.map((id) => entities.get(id)).filter((state) => state !== undefined);
}

function renderActions(states = actionStates()) {
  const box = $("#house-actions");
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

// ---------------------------------------------------------------- Chauffage en cadrans
// Dans la colonne de gauche, sous la musique : un cadran par zone (Etage,
// RdC…) et un pour l'eau chaude. L'arc se remplit du froid au chaud jusqu'à
// la consigne (en grand au centre), un point marque la température mesurée,
// et les boutons −/+ sont au bout de l'arc.

const HEAT_SWEEP = 240; // l'arc fait 240°, ouvert en bas
const HEAT_BOX = { cx: 60, cy: 54, r: 44 };

/** Un point de l'arc, de 0 (en bas à gauche) à 1 (en bas à droite). */
function heatPoint(t) {
  const angle = ((150 + HEAT_SWEEP * Math.min(1, Math.max(0, t))) * Math.PI) / 180;
  return [HEAT_BOX.cx + HEAT_BOX.r * Math.cos(angle), HEAT_BOX.cy + HEAT_BOX.r * Math.sin(angle)];
}

const HEAT_ARC = (() => {
  const [x0, y0] = heatPoint(0);
  const [x1, y1] = heatPoint(1);
  const f = (v) => v.toFixed(2);
  return `M${f(x0)} ${f(y0)} A${HEAT_BOX.r} ${HEAT_BOX.r} 0 1 1 ${f(x1)} ${f(y1)}`;
})();

/** Les consignes en cours d'envoi : on voit tout de suite la nouvelle valeur,
 *  et plusieurs appuis rapides partent en un seul ordre à la box. */
const heatPending = new Map();

function heatTarget(state) {
  const pending = heatPending.get(state.entity_id);
  if (pending) return pending.value;
  return finite(state.attributes.temperature) ? state.attributes.temperature : null;
}

function heatLimits(state) {
  const a = state.attributes;
  return {
    step: finite(a.target_temp_step) && a.target_temp_step > 0 ? a.target_temp_step : 0.5,
    min: finite(a.min_temp) ? a.min_temp : 5,
    max: finite(a.max_temp) ? a.max_temp : 30,
  };
}

/** −/+ : relit l'état au moment du toucher (le cadran affiché peut être ancien). */
function nudgeHeating(entityId, direction) {
  const state = entities.get(entityId);
  if (!state || state.state !== "heat") return;
  const target = heatTarget(state);
  if (target === null) return;
  const { step, min, max } = heatLimits(state);
  const next = Math.round(Math.min(max, Math.max(min, target + direction * step)) * 10) / 10;
  if (next === target) return;
  const old = heatPending.get(entityId);
  clearTimeout(old?.send);
  clearTimeout(old?.expire);
  const pending = { value: next };
  pending.send = setTimeout(() => {
    callService("climate", "set_temperature", { entity_id: entityId, temperature: next });
    // Si la box ne confirme pas, on revient à ce qu'elle dit.
    pending.expire = setTimeout(() => {
      if (heatPending.get(entityId) === pending) heatPending.delete(entityId);
      renderHeating();
    }, 20000);
  }, 700);
  heatPending.set(entityId, pending);
  renderHeating();
}

/** « 19 » puis « ,5 » en petit, comme sur un thermostat. */
function heatNumber(value) {
  const [whole, decimals] = number.format(value).split(",");
  return el("strong", { class: "heat-set" }, whole, decimals ? el("small", { text: `,${decimals}` }) : "", el("i", { text: "°" }));
}

function heatMode(state) {
  const a = state.attributes;
  if (state.state === "unavailable") return "Indisponible";
  if (state.state === "off") return "Arrêt";
  if (a.derogation === true) return finite(a.derogation_minutes) ? `Manuel · ${a.derogation_minutes} min` : "Manuel";
  if (typeof a.preset_mode === "string") return lookup(PRESET_LABELS, a.preset_mode) ?? a.preset_mode;
  return statusText(state);
}

function heatingDial(state) {
  const a = state.attributes;
  const id = state.entity_id;
  const on = state.state === "heat";
  const water = id === "climate.eau_chaude";
  const target = heatTarget(state);
  const current = finite(a.current_temperature) ? a.current_temperature : null;
  const { step, min, max } = heatLimits(state);
  const ratio = (v) => (max > min ? (v - min) / (max - min) : 0);
  const warming = on && current !== null && target !== null && current < target - 0.05;
  const slug = id.replace(/[^a-z0-9]/gi, "-");

  const arc = svg("svg", { class: "heat-arc", viewBox: "0 0 120 112", "aria-hidden": "true" });
  const gradient = svg("linearGradient", { id: `heat-grad-${slug}`, gradientUnits: "userSpaceOnUse", x1: 16, y1: 0, x2: 104, y2: 0 });
  gradient.append(
    svg("stop", { offset: "0", class: "heat-stop-cold" }),
    svg("stop", { offset: "0.55", class: "heat-stop-mild" }),
    svg("stop", { offset: "1", class: "heat-stop-hot" }));
  const defs = svg("defs");
  defs.append(gradient);
  arc.append(defs, svg("path", { class: "heat-track", d: HEAT_ARC, pathLength: 100 }));
  if (on && target !== null) {
    const fill = Math.max(0.5, ratio(target) * 100);
    arc.append(svg("path", { class: "heat-fill", d: HEAT_ARC, pathLength: 100, stroke: `url(#heat-grad-${slug})`, "stroke-dasharray": `${fill.toFixed(1)} 100` }));
  }
  if (current !== null) {
    const [x, y] = heatPoint(ratio(current));
    arc.append(svg("circle", { class: "heat-now-dot", cx: x.toFixed(2), cy: y.toFixed(2), r: 4.2 }));
  }

  const button = (direction) => {
    const next = target === null ? null : target + direction * step;
    const b = el("button", {
      type: "button",
      class: `heat-btn ${direction > 0 ? "plus" : "minus"}`,
      "aria-label": `${direction > 0 ? "Monter" : "Baisser"} la consigne de ${nameOf(state)}`,
      text: direction > 0 ? "+" : "−",
    });
    b.disabled = !on || !canControl(state) || next === null || next < min - 1e-9 || next > max + 1e-9;
    b.addEventListener("click", () => nudgeHeating(id, direction));
    return b;
  };

  const center = el("div", { class: "heat-center" },
    on && target !== null ? heatNumber(target) : el("strong", { class: "heat-set off", text: state.state === "off" ? "Arrêt" : "—" }),
    current !== null
      ? el("span", { class: "heat-now", title: "Température mesurée", text: `${number.format(current)}°` })
      : el("span", { class: "heat-now glyph" }, svgIcon(water ? ICONS.humidity : ICONS.flame)));

  const label = [`${nameOf(state)} : ${heatMode(state)}`];
  if (on && target !== null) label.push(`consigne ${number.format(target)} °C`);
  if (current !== null) label.push(`${number.format(current)} °C mesurés`);

  return el("article", {
    class: `heat-dial${on ? " on" : ""}${warming ? " warming" : ""}${water ? " water" : ""}${heatPending.has(id) ? " pending" : ""}`,
    "aria-label": label.join(", "),
  },
  el("div", { class: "heat-face" }, arc, center, button(-1), button(1)),
  el("div", { class: "heat-label" },
    el("b", {}, warming ? svgIcon(ICONS.flame) : "", nameOf(state)),
    el("span", { text: heatMode(state) })));
}

/** L'en-tête : la pompe à chaleur et la température dehors, s'il y en a. */
function heatingHead() {
  const chips = [];
  const pump = entities.get("sensor.pompe_a_chaleur");
  if (pump && pump.state !== "unknown" && pump.state !== "unavailable") {
    const error = finite(pump.attributes.code_erreur);
    const text = error ? `PAC · erreur ${pump.attributes.code_erreur}` : `PAC · ${pump.state}`;
    chips.push(el("span", { class: `heat-chip${error ? " alert" : pump.state === "chauffage" ? " running" : ""}`, text }));
  }
  const outside = entities.get("sensor.temperature_exterieure");
  if (outside && outside.state !== "" && Number.isFinite(Number(outside.state))) {
    chips.push(el("span", { class: "heat-chip", text: `Dehors ${number.format(Number(outside.state))}°` }));
  }
  return el("div", { class: "heat-head" }, el("h2", { text: "Chauffage" }), el("span", { class: "heat-chips" }, ...chips));
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

/** Les appareils des pièces de l'onglet, ou `null` : toute la maison. */
function pageEntityIds() {
  return pageAreas === null ? null
    : new Set(areas.filter((a) => pageAreas.has(a.name)).flatMap((a) => a.entities.map((e) => e.entity_id)));
}

/** Les cadrans à montrer. Une consigne confirmée par la box n'a plus besoin
 *  d'être retenue ici. */
function heatingShown() {
  const heating = heatingStates(pageEntityIds());
  for (const state of heating) {
    const pending = heatPending.get(state.entity_id);
    if (pending?.expire && state.attributes.temperature === pending.value) {
      clearTimeout(pending.expire);
      heatPending.delete(state.entity_id);
    }
  }
  return heating;
}

/** Le chauffage, sous le lecteur de musique. */
function renderHeating(heating = heatingShown()) {
  const box = $("#board-heating");
  box.hidden = heating.length === 0;
  if (heating.length === 0) {
    box.replaceChildren();
    return;
  }
  swap(box, heatingHead(), el("div", { class: "heat-dials" }, ...heating.map(heatingDial)));
}

/** Les volets (pas les portes de garage ni les portails, ni ceux déjà en
 *  boutons rapides), en lignes sous le dessin. */
function coverStates() {
  const inPage = pageEntityIds();
  return [...entities.values()]
    .filter((s) => domainOf(s) === "cover" && !ALERT_COVERS.includes(s.attributes.device_class)
      && !house.config.actions.includes(s.entity_id) && (inPage === null || inPage.has(s.entity_id)))
    .sort((a, b) => nameOf(a).localeCompare(nameOf(b), "fr"));
}

function renderCovers(covers = coverStates()) {
  const box = $("#board-covers");
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
        entry.points = seriesPoints(data);
        if (isBoard()) {
          renderCharts();
          renderGauges();
        }
        if (roomShown()) renderRoomHead();
      })
      .catch(() => { entry.loading = false; });
  }
  return series.get(key)?.points ?? null;
}

/** Les points d'une courbe (réponse de /api/history/…/series) : la moyenne
 *  de chaque tranche, précédée de la valeur au début de la période. Sans
 *  elle, un capteur qui change rarement (une température dehors) n'avait
 *  que deux ou trois relevés récents, et la courbe se tassait au bord droit. */
function seriesPoints(data) {
  const points = data.numeric
    .map((p) => [Date.parse(p.at), Number(p.mean)])
    .filter(([t, v]) => Number.isFinite(t) && Number.isFinite(v));
  const since = Date.parse(data.since);
  const first = Array.isArray(data.changes) ? data.changes[0] : undefined;
  const start = typeof first?.state === "string" && first.state.trim() !== "" ? Number(first.state) : NaN;
  if (Number.isFinite(since) && Date.parse(first?.at) === since && Number.isFinite(start) && !(points[0]?.[0] <= since)) {
    points.unshift([since, start]);
  }
  return points;
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

function peopleStates() {
  return [...entities.values()]
    .filter((s) => domainOf(s) === "person")
    .sort((a, b) => nameOf(a).localeCompare(nameOf(b), "fr"));
}

function renderPeople(people = peopleStates()) {
  const box = $("#board-people");
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

// ---------------------------------------------------------------- Ce qui a changé
// renderHouse passe à chaque changement d'état (au plus une fois par image).
// Chaque panneau n'est refait que si sa clé change (utils.js, `panelChanged`).
// Ce qui suit l'heure (la courbe qui glisse, le prochain rendez-vous, la
// date du cartouche) avance avec la demi-minute, qui redessine tout
// (tiles.js) ; un panneau appelé directement (courbe chargée, consigne
// touchée, onglet changé) est refait sans clé.

const marksOf = (states) => states.map(stateMark).join(",");
const marksOfIds = (ids) => marksOf(ids.map((id) => entities.get(id)));

/** Les repères (house-effets.js) : dehors, dedans, la puissance, les
 *  lumières, le prochain rendez-vous, les présents. */
function gaugesKey() {
  let lights = 0;
  const read = [];
  for (const state of entities.values()) {
    if (isLight(state) && state.state === "on") lights += 1;
    const domain = domainOf(state);
    if (domain === "weather" || domain === "calendar") read.push(state);
  }
  const hidden = typeof agendaGauge === "function" ? [...agenda.hidden].join(" ") : "";
  return [
    marksOfIds(["sensor.temperature_exterieure", "sensor.meteo_temperature", "sensor.linky_puissance", "zone.home", "sun.sun"]),
    marksOf(read), lights, hidden, objectMark(areas),
    marksOfIds(areas.flatMap((a) => a.entities.map((e) => e.entity_id))),
    objectMark(account.permissions),
  ].join("|");
}

function heatingKey(heating) {
  return [
    marksOf(heating), marksOfIds(["sensor.pompe_a_chaleur", "sensor.temperature_exterieure"]),
    [...heatPending].map(([id, pending]) => `${id}=${pending.value}`).join(","),
  ].join("|");
}

/** Les courbes : la valeur du moment de chacune, et le réglage de l'onglet
 *  (les points chargés, eux, redessinent directement, voir `seriesFor`). */
function chartsKey() {
  const charts = house.config.charts ?? [];
  return [
    objectMark(house.config), objectMark(areas), marksOfIds(charts.map((chart) => chart.entity_id)),
    account.permissions.history,
  ].join("|");
}

function titleBlockKey() {
  let devices = 0;
  for (const state of entities.values()) if (!["persistent_notification", "zone", "sun"].includes(domainOf(state))) devices += 1;
  return [account.user, areas.length, devices, fullDate.format(new Date())].join("|");
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
    $("#board-heating").hidden = true;
    // Cachés : au retour, tout est refait (et remontré).
    forgetPanels();
    return;
  }
  if (house.loading === null && token.get()) loadHouse();
  if (house.config === null) return;
  updateHouse(house.view);
  // Ce que plusieurs panneaux lisent : l'onglet, les pièces (et leurs noms
  // personnalisés), les droits du compte.
  const page = `${objectMark(house.config)}|${objectMark(areas)}|${objectMark(pageAreas)}|${objectMark(account.permissions)}`;
  if (panelChanged("gauges", gaugesKey())) renderGauges();
  const actions = actionStates();
  if (panelChanged("actions", `${page}|${marksOf(actions)}`)) renderActions(actions);
  const covers = coverStates();
  if (panelChanged("covers", `${page}|${marksOf(covers)}`)) renderCovers(covers);
  const heating = heatingShown();
  if (panelChanged("heating", `${page}|${heatingKey(heating)}`)) renderHeating(heating);
  const people = peopleStates();
  if (panelChanged("people", `${page}|${marksOf(people)}`)) renderPeople(people);
  if (panelChanged("charts", chartsKey())) renderCharts();
  if (typeof renderBoardAgenda === "function") renderBoardAgenda();
  if (typeof renderBoardMusic === "function") renderBoardMusic();
  if (isPlan() && panelChanged("title", titleBlockKey())) renderTitleBlock();
}

// ---------------------------------------------------------------- Machines en marche
// Une machine suivie par SmartThings (le lave-linge, étape 86) s'incruste sur
// le dessin pendant qu'elle tourne : le temps restant, le programme, l'heure
// de fin et un anneau de progression autour du hublot. Finie, elle reste une
// heure (le temps de la vider), puis s'efface. La carte suit le dessin :
// tableau, vue hub et écran de veille.

const MACHINE_DONE_SHOWN = 3_600_000;
const machineLive = { box: null, cards: new Map(), timer: null };

/** À montrer : en marche, en pause, ou finie depuis moins d'une heure. */
function machineShown(state) {
  if (state.state === "En marche" || state.state === "En pause") return true;
  if (state.state !== "Terminé") return false;
  const since = Date.parse(state.last_changed);
  return Number.isFinite(since) && Date.now() - since < MACHINE_DONE_SHOWN;
}

/** « 42 min », « 1 h 05 ». */
function machineDuration(minutes) {
  const m = Math.max(0, Math.round(minutes));
  return m < 60 ? `${m} min` : `${Math.floor(m / 60)} h ${String(m % 60).padStart(2, "0")}`;
}

function machineCard(entityId) {
  // L'anneau de progression, et dedans le hublot : le linge tourne pendant
  // le lavage, une coche se dessine à la fin.
  const ring = svg("svg", { class: "ml-ring", viewBox: "0 0 48 48", "aria-hidden": "true" });
  const fill = svg("circle", { class: "ml-fill", cx: 24, cy: 24, r: 21, pathLength: 100 });
  const clothes = svg("g", { class: "ml-clothes" });
  clothes.append(
    svg("circle", { cx: 20.5, cy: 20.5, r: 2.2 }),
    svg("circle", { cx: 28, cy: 22.5, r: 1.5 }),
    svg("circle", { cx: 23, cy: 28.5, r: 1.2 }));
  const porthole = svg("g", { class: "ml-porthole" });
  porthole.append(
    svg("circle", { class: "ml-glass", cx: 24, cy: 24, r: 12 }),
    svg("path", { class: "ml-water", d: "M14 27c2.5-1.8 5-1.8 7.5 0s5 1.8 7.5 0 4-1.6 6.5 0" }),
    clothes);
  ring.append(
    svg("circle", { class: "ml-track", cx: 24, cy: 24, r: 21 }),
    fill,
    porthole,
    svg("path", { class: "ml-check", d: "M17.5 24.5l4.5 4.5 8.5-9", pathLength: 1 }));
  const name = el("small", { class: "ml-name" });
  const main = el("strong", { class: "ml-main" });
  const sub = el("span", { class: "ml-sub" });
  const node = el("button", { type: "button", class: "ml-card", tabindex: "-1", "aria-hidden": "true" },
    el("span", { class: "ml-dial" }, ring), el("span", { class: "ml-text" }, name, main, sub));
  node.addEventListener("click", () => {
    if (account.permissions.history !== false && entities.has(entityId)) openHistory(entityId);
  });
  return { node, fill, name, main, sub };
}

function updateMachineCard(card, state) {
  const a = state.attributes;
  const phase = state.state === "Terminé" ? "done" : state.state === "En pause" ? "paused" : "running";
  const cycle = typeof a.cycle === "string" && a.cycle ? a.cycle.charAt(0).toUpperCase() + a.cycle.slice(1) : "";
  const left = Number.isFinite(a.reste_minutes) ? a.reste_minutes : null;
  let main;
  let sub;
  if (phase === "done") {
    const since = Date.parse(state.last_changed);
    main = "Terminé";
    sub = [Number.isFinite(since) ? `fini à ${hourMinute.format(new Date(since))}` : "",
      Number.isFinite(a.dernier_cycle_minutes) ? `en ${machineDuration(a.dernier_cycle_minutes)}` : "", "à vider"];
  } else if (phase === "paused") {
    main = "En pause";
    sub = [cycle, left !== null ? `encore ${machineDuration(left)}` : ""];
  } else {
    main = left !== null ? machineDuration(left) : "En marche";
    sub = [cycle, typeof a.fin_heure === "string" ? `fin vers ${a.fin_heure}` : ""];
  }
  const said = sub.filter(Boolean).join(" · ");
  setText(card.name, nameOf(state));
  setText(card.main, main);
  setText(card.sub, said);
  setAttr(card.node, "aria-label", `${nameOf(state)} : ${main}${said ? `, ${said}` : ""}`);
  card.node.dataset.phase = phase;
  // Sans progression connue, l'anneau tourne en continu.
  const progress = phase === "done" ? 100 : Number.isFinite(a.progression) ? Math.min(100, Math.max(0, a.progression)) : null;
  card.node.classList.toggle("ml-unknown", progress === null);
  card.node.style.setProperty("--ml-progress", String(progress ?? 25));
}

function renderMachineLive() {
  const stage = $("#house-stage");
  const host = stage?.parentElement;
  if (!host) return;
  if (!machineLive.box) machineLive.box = el("div", { class: "machine-live", role: "group", "aria-label": "Machines en marche" });
  const box = machineLive.box;
  if (box.parentElement !== host) host.append(box);
  clearTimeout(machineLive.timer);
  const machines = [...entities.values()].filter((s) => domainOf(s) === "sensor" && typeof s.attributes.appareil === "string");
  for (const [id, card] of machineLive.cards) {
    if (!machines.some((s) => s.entity_id === id)) {
      card.node.remove();
      machineLive.cards.delete(id);
    }
  }
  let nextHide = Infinity;
  for (const state of machines) {
    const shown = machineShown(state);
    let card = machineLive.cards.get(state.entity_id);
    if (!card && !shown) continue;
    if (!card) {
      card = machineCard(state.entity_id);
      machineLive.cards.set(state.entity_id, card);
      box.append(card.node);
      // Dessinée une fois cachée, pour que l'arrivée soit animée.
      void card.node.offsetWidth;
    }
    if (shown) updateMachineCard(card, state);
    card.node.classList.toggle("shown", shown);
    setAttr(card.node, "aria-hidden", shown ? "false" : "true");
    setAttr(card.node, "tabindex", shown ? "0" : "-1");
    if (shown && state.state === "Terminé") nextHide = Math.min(nextHide, Date.parse(state.last_changed) + MACHINE_DONE_SHOWN);
  }
  // Une heure après la fin, sans autre changement : la carte s'efface seule.
  if (Number.isFinite(nextHide)) machineLive.timer = setTimeout(renderMachineLive, Math.max(0, nextHide - Date.now()) + 500);
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
  forgetPanels();
  if (!$("#dashboard-view").hidden) renderHome();
});

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
