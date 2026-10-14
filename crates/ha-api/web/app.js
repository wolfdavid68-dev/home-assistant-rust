// Tableau de bord — JavaScript moderne, sans bibliothèque ni étape de construction.
//
// Sécurité : les noms d'appareils viennent du réseau (MQTT, pont Hue). On ne
// les insère JAMAIS avec `innerHTML` : uniquement avec `textContent`, qui
// affiche le texte tel quel. Un nom comme `<img onerror=…>` reste du texte.
"use strict";

const TOKEN_KEY = "ha_token";
const $ = (selector) => document.querySelector(selector);

// ---------------------------------------------------------------- Jeton
// sessionStorage : le jeton disparaît à la fermeture de l'onglet.
const token = {
  get: () => sessionStorage.getItem(TOKEN_KEY),
  set: (value) => sessionStorage.setItem(TOKEN_KEY, value),
  clear: () => sessionStorage.removeItem(TOKEN_KEY),
};

// ---------------------------------------------------------------- Connexion
function showLogin(message = "") {
  $("#dashboard-view").hidden = true;
  $("#login-view").hidden = false;
  const error = $("#login-error");
  error.textContent = message;
  error.hidden = !message;
  $("#login-form [name=username]").focus();
}

$("#login-form").addEventListener("submit", async (event) => {
  event.preventDefault();
  const form = event.currentTarget;
  const button = form.querySelector("button");
  button.disabled = true;
  try {
    const response = await fetch("/api/auth/login", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        username: form.username.value.trim(),
        password: form.password.value,
        // Le code n'est envoyé que s'il a été saisi.
        ...(form.code.value.trim() && { code: form.code.value.trim() }),
      }),
    });
    form.password.value = "";
    form.code.value = "";
    if (response.status === 429) return showLogin("Trop de tentatives. Réessaie dans 15 minutes.");
    if (!response.ok) return showLogin("Nom d'utilisateur, mot de passe ou code incorrect.");
    token.set((await response.json()).access_token);
    showDashboard();
  } catch {
    showLogin("Serveur injoignable.");
  } finally {
    button.disabled = false;
  }
});

$("#logout").addEventListener("click", async () => {
  const current = token.get();
  token.clear();
  live.close();
  if (current) {
    await fetch("/api/auth/logout", {
      method: "POST",
      headers: { Authorization: `Bearer ${current}` },
    }).catch(() => {});
  }
  showLogin();
});

// ---------------------------------------------------------------- Temps réel
// Connexion WebSocket, avec reconnexion automatique.
const entities = new Map();

const live = {
  socket: null,
  retry: 1000,
  stopped: false,

  open() {
    this.stopped = false;
    const scheme = location.protocol === "https:" ? "wss" : "ws";
    const socket = new WebSocket(`${scheme}://${location.host}/api/websocket`);
    this.socket = socket;
    setStatus("Connexion…", false);

    socket.addEventListener("message", (event) => {
      const message = JSON.parse(event.data);
      switch (message.type) {
        case "auth_required":
          // Le jeton passe dans le premier message, jamais dans l'adresse.
          socket.send(JSON.stringify({ type: "auth", access_token: token.get() }));
          break;
        case "auth_invalid":
          token.clear();
          this.close();
          showLogin("Session expirée : reconnecte-toi.");
          break;
        case "auth_ok":
          this.retry = 1000;
          setStatus("En direct", true);
          socket.send(JSON.stringify({ id: 1, type: "get_states" }));
          socket.send(JSON.stringify({ id: 2, type: "subscribe_events" }));
          break;
        case "result":
          if (message.id === 1 && message.success) {
            entities.clear();
            for (const state of message.result) entities.set(state.entity_id, state);
            renderAll();
          }
          break;
        case "event":
          if (message.event.event_type === "state_changed") onStateChanged(message.event.data);
          break;
      }
    });

    socket.addEventListener("close", () => {
      if (this.stopped) return;
      setStatus("Reconnexion…", false);
      setTimeout(() => this.open(), this.retry);
      this.retry = Math.min(this.retry * 2, 30000);
    });
  },

  close() {
    this.stopped = true;
    this.socket?.close();
  },
};

function setStatus(text, isLive) {
  const status = $("#connection");
  status.textContent = text;
  status.classList.toggle("live", isLive);
}

function onStateChanged({ entity_id, new_state }) {
  if (new_state) entities.set(entity_id, new_state);
  else entities.delete(entity_id);
  const existing = document.getElementById(tileId(entity_id));
  if (existing && new_state) existing.replaceWith(renderTile(new_state));
  else renderAll(); // entité ajoutée ou supprimée : on redessine tout
}

// ---------------------------------------------------------------- Commandes
async function callService(domain, service, data) {
  const response = await fetch(`/api/services/${domain}/${service}`, {
    method: "POST",
    headers: { "Content-Type": "application/json", Authorization: `Bearer ${token.get()}` },
    body: JSON.stringify(data),
  });
  if (response.status === 401) {
    token.clear();
    live.close();
    showLogin("Session expirée : reconnecte-toi.");
  }
}

// La tuile « clignote » jusqu'à ce que l'appareil confirme (ou 5 s au plus).
function markPending(entityId) {
  const tile = document.getElementById(tileId(entityId));
  tile?.classList.add("pending");
  setTimeout(() => document.getElementById(tileId(entityId))?.classList.remove("pending"), 5000);
}

// ---------------------------------------------------------------- Affichage
const SECTIONS = [
  { title: "Scènes et scripts", domains: ["scene", "script"] },
  { title: "Présence et soleil", domains: ["device_tracker", "sun"] },
  { title: "Lumières", domains: ["light"] },
  { title: "Volets", domains: ["cover"] },
  { title: "Prises et interrupteurs", domains: ["switch"] },
  { title: "Capteurs", domains: ["sensor"] },
  { title: "Détecteurs", domains: ["binary_sensor"] },
];

// ---------------------------------------------------------------- Pièces
// Les pièces viennent du fichier areas.yaml (GET /api/areas).
let areas = [];
const customNames = new Map();

async function loadAreas() {
  const response = await fetch("/api/areas", { headers: { Authorization: `Bearer ${token.get()}` } });
  if (!response.ok) return;
  areas = (await response.json()).areas ?? [];
  customNames.clear();
  for (const area of areas) {
    for (const entity of area.entities) {
      if (typeof entity.name === "string") customNames.set(entity.entity_id, entity.name);
    }
  }
  renderAll();
}

function renderAll() {
  const container = $("#sections");
  const sorted = [...entities.values()].sort((a, b) => nameOf(a).localeCompare(nameOf(b), "fr"));
  const used = new Set();
  // D'abord une section par pièce, dans l'ordre du fichier…
  const areaGroups = areas.map((area) => {
    const items = area.entities
      .map((entity) => entities.get(entity.entity_id))
      .filter((state) => state !== undefined);
    items.forEach((s) => used.add(s.entity_id));
    return { title: area.name, items };
  });
  // … puis, par type, les appareils qui ne sont dans aucune pièce.
  const groups = SECTIONS.map(({ title, domains }) => {
    const items = sorted.filter((s) => !used.has(s.entity_id) && domains.includes(domainOf(s)));
    items.forEach((s) => used.add(s.entity_id));
    return { title, items };
  });
  groups.push({ title: "Autres", items: sorted.filter((s) => !used.has(s.entity_id)) });

  container.replaceChildren(
    ...[...areaGroups, ...groups]
      .filter((group) => group.items.length > 0)
      .map(({ title, items }) =>
        el("section", {}, el("h2", { text: title }), el("div", { class: "grid" }, ...items.map(renderTile))),
      ),
  );
  $("#empty").hidden = entities.size > 0;
}

function renderTile(state) {
  const tile = renderTileContent(state);
  tile.append(historyButton(state));
  return tile;
}

function renderTileContent(state) {
  switch (domainOf(state)) {
    case "light": return lightTile(state);
    case "cover": return coverTile(state);
    case "device_tracker": return presenceTile(state);
    case "sun": return sunTile(state);
    case "scene":
    case "script": return actionTile(state);
    case "switch": return switchTile(state);
    case "binary_sensor": return binaryTile(state);
    case "sensor": return state.attributes.device_class === "alarm" ? alarmTile(state) : sensorTile(state);
    default: return sensorTile(state);
  }
}

function lightTile(state) {
  const on = state.state === "on";
  const brightness = state.attributes.brightness ?? 255;
  const slider = el("input", {
    type: "range", min: "1", max: "255", value: String(brightness),
    "aria-label": `Luminosité de ${nameOf(state)}`,
  });
  slider.addEventListener("change", () => {
    markPending(state.entity_id);
    callService("light", "turn_on", { entity_id: state.entity_id, brightness: Number(slider.value) });
  });
  const toggle = el(
    "button",
    { class: "tile-toggle", type: "button", "aria-pressed": String(on) },
    tileHead(state, ICONS.bulb, on ? `Allumée · ${Math.round((brightness / 255) * 100)} %` : state.state === "off" ? "Éteinte" : statusText(state)),
  );
  toggle.addEventListener("click", () => {
    markPending(state.entity_id);
    callService("light", "toggle", { entity_id: state.entity_id });
  });
  return el(
    "div",
    { id: tileId(state.entity_id), class: tileClass(state, "light") },
    toggle,
    on ? slider : el("span"),
  );
}

function coverTile(state) {
  const position = state.attributes.current_position;
  const subtitle =
    typeof position === "number"
      ? position >= 100 ? "Ouvert" : position <= 0 ? "Fermé" : `Ouvert à ${Math.round(position)} %`
      : lookup(COVER_LABELS, state.state) ?? statusText(state);
  const command = (service, data = {}) => {
    markPending(state.entity_id);
    callService("cover", service, { entity_id: state.entity_id, ...data });
  };
  const button = (service, icon, label) => {
    const b = el("button", { type: "button", class: "round", "aria-label": `${label} ${nameOf(state)}` }, svgIcon(icon));
    b.addEventListener("click", () => command(service));
    return b;
  };
  const children = [
    tileHead(state, ICONS.blinds, subtitle),
    el(
      "div",
      { class: "cover-buttons" },
      button("open_cover", ICONS.up, "Ouvrir"),
      button("stop_cover", ICONS.stop, "Arrêter"),
      button("close_cover", ICONS.down, "Fermer"),
    ),
  ];
  if (typeof position === "number") {
    const slider = el("input", {
      type: "range", min: "0", max: "100", step: "5", value: String(Math.round(position)),
      "aria-label": `Position de ${nameOf(state)}`,
    });
    slider.addEventListener("change", () => command("set_cover_position", { position: Number(slider.value) }));
    children.push(slider);
  }
  return el("div", { id: tileId(state.entity_id), class: tileClass(state, "cover") }, ...children);
}

function switchTile(state) {
  const on = state.state === "on";
  // Le bouton est à l'intérieur de la tuile (et non la tuile entière), pour
  // que le bouton « historique » puisse être à côté : un bouton ne peut pas
  // en contenir un autre.
  const toggle = el(
    "button",
    { class: "tile-toggle", type: "button", "aria-pressed": String(on) },
    tileHead(state, ICONS.plug, statusText(state)),
  );
  toggle.addEventListener("click", () => {
    markPending(state.entity_id);
    callService("switch", "toggle", { entity_id: state.entity_id });
  });
  return el("div", { id: tileId(state.entity_id), class: tileClass(state, "switch") }, toggle);
}

function binaryTile(state) {
  const labels = lookup(BINARY_LABELS, state.attributes.device_class) ?? ["Actif", "Inactif"];
  const text = state.state === "on" ? labels[0] : state.state === "off" ? labels[1] : statusText(state);
  const icon = lookup(BINARY_ICONS, state.attributes.device_class) ?? ICONS.door;
  return el(
    "div",
    { id: tileId(state.entity_id), class: tileClass(state, "binary") },
    tileHead(state, icon, text),
  );
}

// Présence (téléphone connecté au Wi-Fi de la Freebox).
function presenceTile(state) {
  const home = state.state === "home";
  const text = home ? "À la maison" : state.state === "not_home" ? "Absent" : statusText(state);
  return el(
    "div",
    { id: tileId(state.entity_id), class: tileClass(state, "binary") + (home ? " on" : "") },
    tileHead(state, ICONS.person, text),
  );
}

// Scène (« Soirée cinéma ») ou script (« Je pars ») : un bouton pour lancer.
function actionTile(state) {
  const isScript = domainOf(state) === "script";
  const running = isScript && state.state === "on";
  const subtitle = isScript
    ? running ? "En cours…" : "Prêt"
    : Number.isNaN(Date.parse(state.state)) ? "Jamais activée" : `Activée il y a ${ago(state.state)}`;
  const button = el("button", { type: "button", class: "action-button" });
  button.textContent = running ? "Arrêter" : isScript ? "Lancer" : "Activer";
  button.addEventListener("click", () => {
    markPending(state.entity_id);
    callService(domainOf(state), running ? "turn_off" : "turn_on", { entity_id: state.entity_id });
  });
  return el(
    "div",
    // Pas de `tileClass` ici : une scène jamais activée (« unknown ») n'est pas
    // un appareil indisponible, elle ne doit pas être grisée.
    { id: tileId(state.entity_id), class: running ? "tile action on" : "tile action" },
    tileHead(state, isScript ? ICONS.play : ICONS.sparkle, subtitle),
    button,
  );
}

// Le soleil : levé ou couché, et l'heure du prochain changement.
const hourMinute = new Intl.DateTimeFormat("fr", { hour: "2-digit", minute: "2-digit" });
function sunTile(state) {
  const up = state.state === "above_horizon";
  const next = up ? state.attributes.next_setting : state.attributes.next_rising;
  const when = typeof next === "string" && !Number.isNaN(Date.parse(next))
    ? `${up ? "coucher" : "lever"} à ${hourMinute.format(new Date(next))}`
    : "";
  const text = [up ? "Levé" : "Couché", when].filter(Boolean).join(" · ");
  return el(
    "div",
    { id: tileId(state.entity_id), class: tileClass(state, "binary") + (up ? " on" : "") },
    tileHead(state, up ? ICONS.illuminance : ICONS.moon, text),
  );
}

function alarmTile(state) {
  const [text, armed] = lookup(ALARM_LABELS, state.state) ?? [statusText(state), false];
  return el(
    "div",
    { id: tileId(state.entity_id), class: tileClass(state, "binary") + (armed ? " on" : "") },
    tileHead(state, ICONS.shield, text),
  );
}

function sensorTile(state) {
  const unit = state.attributes.unit_of_measurement;
  const value = el("div", { class: "value", text: state.state });
  if (unit) value.append(el("small", { text: unit }));
  return el(
    "div",
    { id: tileId(state.entity_id), class: tileClass(state, "sensor") },
    tileHead(state, lookup(ICONS, state.attributes.device_class) ?? ICONS.gauge, `il y a ${ago(state.last_changed)}`),
    value,
  );
}

function tileHead(state, icon, subtitle) {
  return el(
    "div",
    { class: "tile-head" },
    el("span", { class: "icon" }, svgIcon(icon)),
    el("div", {}, el("div", { class: "name", text: nameOf(state), title: state.entity_id }), el("div", { class: "sub", text: subtitle })),
  );
}

function tileClass(state, kind) {
  const classes = ["tile", kind];
  if (state.state === "on" || (kind === "cover" && state.state === "open")) classes.push("on");
  if (state.state === "unavailable" || state.state === "unknown") classes.push("unavailable");
  return classes.join(" ");
}

function statusText(state) {
  return lookup(STATUS_LABELS, state.state) ?? state.state;
}

// Modes d'alarme Somfy : [texte affiché, activée ?]
const ALARM_LABELS = {
  off: ["Désactivée", false], disarmed: ["Désactivée", false],
  total: ["Activée (totale)", true], armed: ["Activée", true],
  partial: ["Activée (partielle)", true], partial1: ["Activée (partielle)", true], partial2: ["Activée (partielle)", true],
  detection: ["Intrusion détectée !", true], sos: ["Alerte SOS !", true],
};
const COVER_LABELS = { open: "Ouvert", closed: "Fermé", opening: "Ouverture…", closing: "Fermeture…" };
const STATUS_LABELS = { on: "Allumé", off: "Éteint", unknown: "Inconnu", unavailable: "Indisponible" };
const BINARY_LABELS = {
  door: ["Ouverte", "Fermée"], window: ["Ouverte", "Fermée"], opening: ["Ouvert", "Fermé"],
  motion: ["Mouvement", "Calme"], occupancy: ["Présence", "Personne"],
  smoke: ["Fumée !", "Rien à signaler"],
  connectivity: ["Connecté", "Déconnecté"],
};


// ---------------------------------------------------------------- Utilitaires

/** Cherche une clé venue du réseau dans un de nos tableaux de libellés.
 *  Sécurité : `Object.hasOwn` ignore les propriétés héritées. Sans lui, un
 *  appareil qui annonce `device_class: "__proto__"` ou un état "constructor"
 *  obtiendrait des objets internes de JavaScript au lieu d'un libellé. */
const lookup = (table, key) => (typeof key === "string" && Object.hasOwn(table, key) ? table[key] : undefined);
const domainOf = (state) => state.entity_id.split(".")[0];
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
  play: ["M8 5v14l11-7z"],
  sparkle: ["M12 3v4", "M12 17v4", "M3 12h4", "M17 12h4", "M6 6l2.5 2.5", "M15.5 15.5L18 18", "M6 18l2.5-2.5", "M15.5 8.5L18 6"],
  chart: ["M4 19h16", "M4 15l4-5 4 3 6-7"],
  moon: ["M21 12.8A9 9 0 1 1 11.2 3a7 7 0 0 0 9.8 9.8z"],
  person: ["M12 3a4 4 0 1 0 0 8 4 4 0 1 0 0-8z", "M4 21a8 8 0 0 1 16 0"],
  globe: ["M12 3a9 9 0 1 0 0 18 9 9 0 1 0 0-18z", "M3 12h18", "M12 3a14 14 0 0 1 0 18", "M12 3a14 14 0 0 0 0 18"],
  data_rate: ["M4 17l5-5 4 4 7-8", "M15 8h5v5"],
};
const BINARY_ICONS = { motion: ICONS.motion, connectivity: ICONS.globe };

// ---------------------------------------------------------------- Historique
// Fenêtre d'historique : une courbe pour les valeurs numériques, une frise
// pour les autres (allumé / éteint, présent / absent…). Le dessin est en SVG,
// construit élément par élément : pas de bibliothèque, pas de HTML.
const SVG_NS = "http://www.w3.org/2000/svg";
const CHART = { width: 640, height: 220, left: 52, right: 12, top: 12, bottom: 28 };
const historyView = { entityId: null, hours: 24 };
const number = new Intl.NumberFormat("fr", { maximumFractionDigits: 1 });
const shortTime = new Intl.DateTimeFormat("fr", { hour: "2-digit", minute: "2-digit" });
const dayTime = new Intl.DateTimeFormat("fr", { weekday: "short", hour: "2-digit", minute: "2-digit" });
// Valeurs qui veulent dire « actif » : colorées dans la frise.
const ACTIVE_STATES = new Set(["on", "home", "open", "above_horizon", "playing", "unlocked"]);

function historyButton(state) {
  const button = el(
    "button",
    { class: "history-button", type: "button", title: "Historique", "aria-label": `Historique de ${nameOf(state)}` },
    svgIcon(ICONS.chart),
  );
  button.addEventListener("click", (event) => {
    event.stopPropagation();
    openHistory(state.entity_id);
  });
  return button;
}

function openHistory(entityId) {
  historyView.entityId = entityId;
  const state = entities.get(entityId);
  $("#history-title").textContent = state ? nameOf(state) : entityId;
  $("#history").showModal();
  loadHistory().catch(() => {
    $("#history-summary").textContent = "Historique indisponible.";
  });
}

$("#history-close").addEventListener("click", () => $("#history").close());
for (const button of document.querySelectorAll(".period")) {
  button.addEventListener("click", () => {
    historyView.hours = Number(button.dataset.hours);
    loadHistory().catch(() => {});
  });
}

async function loadHistory() {
  for (const button of document.querySelectorAll(".period")) {
    button.setAttribute("aria-pressed", String(Number(button.dataset.hours) === historyView.hours));
  }
  $("#history-summary").textContent = "Chargement…";
  const url = `/api/history/${encodeURIComponent(historyView.entityId)}/series?hours=${historyView.hours}`;
  const response = await fetch(url, { headers: { Authorization: `Bearer ${token.get()}` } });
  if (response.status === 401) {
    $("#history").close();
    token.clear();
    live.close();
    return showLogin("Session expirée : reconnecte-toi.");
  }
  if (!response.ok) throw new Error("historique");
  drawHistory(await response.json());
}

function svg(tag, attributes = {}) {
  const node = document.createElementNS(SVG_NS, tag);
  for (const [name, value] of Object.entries(attributes)) node.setAttribute(name, String(value));
  return node;
}

function drawHistory(data) {
  const since = Date.parse(data.since);
  const until = Date.parse(data.until);
  const state = entities.get(historyView.entityId);
  const unit = state?.attributes.unit_of_measurement ?? "";
  const points = (data.numeric ?? []).filter(
    (p) => Number.isFinite(p.mean) && Number.isFinite(p.min) && Number.isFinite(p.max) && !Number.isNaN(Date.parse(p.at)),
  );
  const changes = (data.changes ?? []).filter((c) => !Number.isNaN(Date.parse(c.at)));
  const x = (time) => CHART.left + ((time - since) / Math.max(1, until - since)) * (CHART.width - CHART.left - CHART.right);
  const chart = svg("svg", { viewBox: `0 0 ${CHART.width} ${CHART.height}`, role: "img", "aria-label": `Historique de ${$("#history-title").textContent}` });

  // Graduations du temps : début, milieu, fin.
  const format = historyView.hours > 24 ? dayTime : shortTime;
  for (const [time, anchor] of [[since, "start"], [(since + until) / 2, "middle"], [until, "end"]]) {
    const label = svg("text", { x: x(time), y: CHART.height - 8, class: "axis", "text-anchor": anchor });
    label.textContent = format.format(new Date(time));
    chart.append(label);
  }

  if (points.length > 0) {
    drawCurve(chart, points, x, unit);
    const low = Math.min(...points.map((p) => p.min));
    const high = Math.max(...points.map((p) => p.max));
    const mean = points.reduce((sum, p) => sum + p.mean, 0) / points.length;
    $("#history-summary").textContent =
      `min ${number.format(low)} · moyenne ${number.format(mean)} · max ${number.format(high)} ${unit}`.trim();
  } else if (changes.length > 0) {
    drawTimeline(chart, changes, x, until);
    $("#history-summary").textContent = `${Math.max(0, changes.length - 1)} changement(s) sur la période`;
  } else {
    $("#history-summary").textContent = "Aucune donnée sur cette période.";
  }
  $("#history-chart").replaceChildren(chart);

  // Pour une valeur qui n'est pas numérique : les derniers changements, en
  // texte (le plus récent d'abord).
  $("#history-changes").replaceChildren(
    ...(points.length > 0 ? [] : changes).slice(-8).reverse().map((change) =>
      el(
        "li",
        {},
        el("time", { datetime: change.at, text: format.format(new Date(change.at)) }),
        el("span", { text: historyLabel(change.state, unit) }),
      ),
    ),
  );
}

function drawCurve(chart, points, x, unit) {
  let low = Math.min(...points.map((p) => p.min));
  let high = Math.max(...points.map((p) => p.max));
  if (high - low < 1e-9) {
    low -= 1;
    high += 1;
  }
  const margin = (high - low) * 0.1;
  low -= margin;
  high += margin;
  const y = (value) => CHART.top + ((high - value) / (high - low)) * (CHART.height - CHART.top - CHART.bottom);

  for (const value of [high - margin, (high + low) / 2, low + margin]) {
    chart.append(svg("line", { x1: CHART.left, x2: CHART.width - CHART.right, y1: y(value), y2: y(value), class: "grid" }));
    const label = svg("text", { x: CHART.left - 6, y: y(value) + 4, class: "axis", "text-anchor": "end" });
    label.textContent = `${number.format(value)}${unit ? ` ${unit}` : ""}`;
    chart.append(label);
  }
  // Bande min–max, puis la courbe des moyennes.
  const top = points.map((p) => `${x(Date.parse(p.at))},${y(p.max)}`);
  const bottom = points.map((p) => `${x(Date.parse(p.at))},${y(p.min)}`).reverse();
  chart.append(svg("polygon", { points: [...top, ...bottom].join(" "), class: "band" }));
  chart.append(svg("polyline", { points: points.map((p) => `${x(Date.parse(p.at))},${y(p.mean)}`).join(" "), class: "curve" }));
}

function drawTimeline(chart, changes, x, until) {
  const top = CHART.top + 30;
  const height = CHART.height - CHART.top - CHART.bottom - 60;
  changes.forEach((change, index) => {
    const start = Date.parse(change.at);
    const end = index + 1 < changes.length ? Date.parse(changes[index + 1].at) : until;
    const active = typeof change.state === "string" && ACTIVE_STATES.has(change.state);
    const missing = change.state === null || change.state === "unavailable" || change.state === "unknown";
    chart.append(svg("rect", {
      x: x(start), y: top, width: Math.max(1, x(end) - x(start)), height,
      class: active ? "segment active" : missing ? "segment missing" : "segment",
    }));
  });
}

/** Libellé lisible d'une valeur de l'historique. */
function historyLabel(value, unit) {
  if (value === null) return "supprimé";
  const state = entities.get(historyView.entityId);
  const domain = historyView.entityId.split(".")[0];
  if (domain === "device_tracker") return value === "home" ? "À la maison" : value === "not_home" ? "Absent" : value;
  if (domain === "sun") return value === "above_horizon" ? "Levé" : value === "below_horizon" ? "Couché" : value;
  if (domain === "cover") return lookup(COVER_LABELS, value) ?? value;
  if (domain === "binary_sensor") {
    const labels = lookup(BINARY_LABELS, state?.attributes.device_class);
    if (labels && (value === "on" || value === "off")) return value === "on" ? labels[0] : labels[1];
  }
  if (unit && Number.isFinite(Number(value))) return `${number.format(Number(value))} ${unit}`;
  return lookup(STATUS_LABELS, value) ?? value;
}

// ---------------------------------------------------------------- Journal
const AUDIT_LABELS = {
  login: "connexion",
  login_failed: "échec de connexion",
  logout: "déconnexion",
  service: "commande",
  state_set: "état modifié",
  state_delete: "état supprimé",
  user_add: "compte créé",
  user_passwd: "mot de passe changé",
  user_delete: "compte supprimé",
  user_totp: "double authentification activée",
  user_totp_off: "double authentification désactivée",
  hue_pair: "pont Hue associé",
  hue_forget: "pont Hue oublié",
  somfy_pair: "box Somfy associée",
  somfy_forget: "box Somfy oubliée",
  server_start: "démarrage du serveur",
  server_stop: "arrêt du serveur",
  freebox_pair: "Freebox associée",
  freebox_forget: "Freebox oubliée",
  freebox_track: "présence suivie",
  freebox_untrack: "présence plus suivie",
  freebox_new_device: "nouvel appareil sur le réseau",
};
const dateTime = new Intl.DateTimeFormat("fr", { dateStyle: "short", timeStyle: "medium" });

async function loadJournal() {
  const list = $("#journal-list");
  const response = await fetch("/api/audit?limit=50", {
    headers: { Authorization: `Bearer ${token.get()}` },
  });
  if (response.status === 401) {
    token.clear();
    live.close();
    return showLogin("Session expirée : reconnecte-toi.");
  }
  if (!response.ok) return list.replaceChildren(el("li", { text: "Journal indisponible." }));
  const { entries } = await response.json();
  list.replaceChildren(
    ...entries.map((entry) => {
      const what = [entry.target, entry.detail].filter(Boolean).join(" · ");
      return el(
        "li",
        { class: ["login_failed", "freebox_new_device"].includes(entry.action) ? "failed" : "" },
        el("time", { datetime: entry.at, text: dateTime.format(new Date(entry.at)) }),
        el("strong", { text: entry.actor }),
        el("span", { text: lookup(AUDIT_LABELS, entry.action) ?? entry.action }),
        el("span", { class: "what", text: what }),
        el("small", { text: entry.source }),
      );
    }),
  );
  if (entries.length === 0) list.replaceChildren(el("li", { text: "Aucune action pour l'instant." }));
}

$("#journal").addEventListener("toggle", (event) => {
  if (event.currentTarget.open) loadJournal().catch(() => {});
});

// ---------------------------------------------------------------- Démarrage
function showDashboard() {
  $("#login-view").hidden = true;
  $("#dashboard-view").hidden = false;
  live.open();
  loadAreas().catch(() => {});
}

// Rafraîchit les « il y a … » chaque minute.
// (Sauf si un curseur de luminosité est en train d'être utilisé.)
setInterval(() => {
  if (entities.size && document.activeElement?.type !== "range") renderAll();
}, 60_000);

if (token.get()) showDashboard();
else showLogin();
