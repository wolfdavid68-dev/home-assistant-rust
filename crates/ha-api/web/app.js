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
      }),
    });
    form.password.value = "";
    if (response.status === 429) return showLogin("Trop de tentatives. Réessaie dans 15 minutes.");
    if (!response.ok) return showLogin("Nom d'utilisateur ou mot de passe incorrect.");
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
  { title: "Lumières", domains: ["light"] },
  { title: "Prises et interrupteurs", domains: ["switch"] },
  { title: "Capteurs", domains: ["sensor"] },
  { title: "Détecteurs", domains: ["binary_sensor"] },
];

function renderAll() {
  const container = $("#sections");
  const sorted = [...entities.values()].sort((a, b) => nameOf(a).localeCompare(nameOf(b), "fr"));
  const used = new Set();
  const groups = SECTIONS.map(({ title, domains }) => {
    const items = sorted.filter((s) => domains.includes(domainOf(s)));
    items.forEach((s) => used.add(s.entity_id));
    return { title, items };
  });
  groups.push({ title: "Autres", items: sorted.filter((s) => !used.has(s.entity_id)) });

  container.replaceChildren(
    ...groups
      .filter((group) => group.items.length > 0)
      .map(({ title, items }) =>
        el("section", {}, el("h2", { text: title }), el("div", { class: "grid" }, ...items.map(renderTile))),
      ),
  );
  $("#empty").hidden = entities.size > 0;
}

function renderTile(state) {
  switch (domainOf(state)) {
    case "light": return lightTile(state);
    case "switch": return switchTile(state);
    case "binary_sensor": return binaryTile(state);
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

function switchTile(state) {
  const on = state.state === "on";
  const tile = el(
    "button",
    { id: tileId(state.entity_id), class: tileClass(state, "switch"), type: "button", "aria-pressed": String(on) },
    tileHead(state, ICONS.plug, statusText(state)),
  );
  tile.addEventListener("click", () => {
    markPending(state.entity_id);
    callService("switch", "toggle", { entity_id: state.entity_id });
  });
  return tile;
}

function binaryTile(state) {
  const labels = lookup(BINARY_LABELS, state.attributes.device_class) ?? ["Actif", "Inactif"];
  const text = state.state === "on" ? labels[0] : state.state === "off" ? labels[1] : statusText(state);
  return el(
    "div",
    { id: tileId(state.entity_id), class: tileClass(state, "binary") },
    tileHead(state, state.attributes.device_class === "motion" ? ICONS.motion : ICONS.door, text),
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
    el("div", {}, el("div", { class: "name", text: nameOf(state) }), el("div", { class: "sub", text: subtitle })),
  );
}

function tileClass(state, kind) {
  const classes = ["tile", kind];
  if (state.state === "on") classes.push("on");
  if (state.state === "unavailable" || state.state === "unknown") classes.push("unavailable");
  return classes.join(" ");
}

function statusText(state) {
  return lookup(STATUS_LABELS, state.state) ?? state.state;
}

const STATUS_LABELS = { on: "Allumé", off: "Éteint", unknown: "Inconnu", unavailable: "Indisponible" };
const BINARY_LABELS = {
  door: ["Ouverte", "Fermée"], window: ["Ouverte", "Fermée"], opening: ["Ouvert", "Fermé"],
  motion: ["Mouvement", "Calme"], occupancy: ["Présence", "Personne"],
};

// ---------------------------------------------------------------- Utilitaires

/** Cherche une clé venue du réseau dans un de nos tableaux de libellés.
 *  Sécurité : `Object.hasOwn` ignore les propriétés héritées. Sans lui, un
 *  appareil qui annonce `device_class: "__proto__"` ou un état "constructor"
 *  obtiendrait des objets internes de JavaScript au lieu d'un libellé. */
const lookup = (table, key) => (typeof key === "string" && Object.hasOwn(table, key) ? table[key] : undefined);
const domainOf = (state) => state.entity_id.split(".")[0];
const nameOf = (state) => state.attributes.friendly_name || state.entity_id.split(".")[1].replaceAll("_", " ");
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
};

// ---------------------------------------------------------------- Démarrage
function showDashboard() {
  $("#login-view").hidden = true;
  $("#dashboard-view").hidden = false;
  live.open();
}

// Rafraîchit les « il y a … » chaque minute.
// (Sauf si un curseur de luminosité est en train d'être utilisé.)
setInterval(() => {
  if (entities.size && document.activeElement?.type !== "range") renderAll();
}, 60_000);

if (token.get()) showDashboard();
else showLogin();
