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
            refreshEntityList();
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
  else {
    // Entité ajoutée ou supprimée : on redessine tout.
    renderAll();
    refreshEntityList();
  }
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
  { title: "Entrées", domains: ["input_boolean", "input_number", "input_select", "input_button", "timer"] },
  // Les capteurs de la météo (sensor.meteo_…) vont avec la météo.
  { title: "Météo, présence et soleil", domains: ["weather", "zone", "person", "device_tracker", "sun", "notify"], extra: (s) => s.entity_id.startsWith("sensor.meteo_") },
  { title: "Chauffage", domains: ["climate"] },
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
  // Par nom, sauf la météo, la maison et les personnes, en tête de leur section.
  const first = (state) => lookup({ weather: 0, zone: 1, person: 2 }, domainOf(state)) ?? 3;
  const sorted = [...entities.values()].sort(
    (a, b) => first(a) - first(b) || nameOf(a).localeCompare(nameOf(b), "fr"),
  );
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
  const groups = SECTIONS.map(({ title, domains, extra }) => {
    const items = sorted.filter((s) => !used.has(s.entity_id) && (domains.includes(domainOf(s)) || extra?.(s)));
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
    case "device_tracker":
    case "person": return presenceTile(state);
    case "zone": return zoneTile(state);
    case "sun": return sunTile(state);
    case "weather": return weatherTile(state);
    case "climate": return climateTile(state);
    case "scene":
    case "script": return actionTile(state);
    case "notify": return notifyTile(state);
    case "input_boolean": return inputBooleanTile(state);
    case "input_number": return inputNumberTile(state);
    case "input_select": return inputSelectTile(state);
    case "input_button": return inputButtonTile(state);
    case "timer": return timerTile(state);
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

// La maison : combien de personnes sont là, et qui.
function zoneTile(state) {
  const count = Number(state.state);
  const names = Array.isArray(state.attributes.persons)
    ? state.attributes.persons.filter((name) => typeof name === "string") : [];
  const text = !Number.isFinite(count) ? statusText(state)
    : count === 0 ? "Personne à la maison"
    : `${names.join(", ") || count} ${count > 1 ? "sont" : "est"} à la maison`;
  return el(
    "div",
    { id: tileId(state.entity_id), class: `tile binary${count > 0 ? " on" : ""}` },
    tileHead(state, ICONS.home, text),
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

// Le téléphone : date de la dernière notification, et un bouton pour tester.
function notifyTile(state) {
  const subtitle = Number.isNaN(Date.parse(state.state))
    ? "Aucune notification envoyée"
    : `Dernière notification il y a ${ago(state.state)}`;
  const button = el("button", { type: "button", class: "action-button", text: "Envoyer un test" });
  button.addEventListener("click", () => {
    markPending(state.entity_id);
    callService("notify", "send_message", {
      entity_id: state.entity_id,
      title: "Maison",
      message: "Test depuis le tableau de bord : les notifications fonctionnent.",
    });
  });
  return el(
    "div",
    { id: tileId(state.entity_id), class: "tile action" },
    tileHead(state, ICONS.bell, subtitle),
    button,
  );
}

// La météo : condition, température, et les prévisions des prochains jours.
const WEATHER_LABELS = {
  "sunny": ["Ensoleillé", "sun"], "clear-night": ["Nuit claire", "moon"],
  "partlycloudy": ["Éclaircies", "partly"], "cloudy": ["Nuageux", "cloud"],
  "fog": ["Brouillard", "fog"], "rainy": ["Pluie", "rain"], "pouring": ["Fortes pluies", "rain"],
  "snowy": ["Neige", "snow"], "snowy-rainy": ["Pluie et neige", "snow"],
  "lightning-rainy": ["Orages", "storm"], "exceptional": ["Exceptionnel", "cloud"],
};
const dayName = new Intl.DateTimeFormat("fr", { weekday: "short" });
const finite = (value) => typeof value === "number" && Number.isFinite(value);
const degrees = (value) => (finite(value) ? `${number.format(value)} °C` : "—");

function weatherIcon(condition) {
  const [, icon] = lookup(WEATHER_LABELS, condition) ?? ["", "cloud"];
  return icon === "sun" ? ICONS.illuminance : lookup(ICONS, icon) ?? ICONS.cloud;
}

function weatherTile(state) {
  const a = state.attributes;
  const [label] = lookup(WEATHER_LABELS, state.state) ?? [statusText(state)];
  const feels = finite(a.apparent_temperature) ? ` · ressenti ${degrees(a.apparent_temperature)}` : "";
  const details = [
    finite(a.humidity) && `Humidité ${number.format(a.humidity)} %`,
    finite(a.wind_speed) && `Vent ${number.format(a.wind_speed)} km/h`,
  ].filter(Boolean).join(" · ");
  const days = (Array.isArray(a.forecast) ? a.forecast : [])
    .filter((day) => typeof day?.datetime === "string" && !Number.isNaN(Date.parse(day.datetime)))
    .slice(0, 5)
    .map((day) => {
      const [dayLabel] = lookup(WEATHER_LABELS, day.condition) ?? [""];
      const rain = finite(day.precipitation_probability) && day.precipitation_probability >= 30
        ? `${number.format(day.precipitation_probability)} %` : "";
      return el(
        "li",
        { title: dayLabel },
        el("span", { class: "day", text: dayName.format(new Date(`${day.datetime}T12:00`)) }),
        svgIcon(weatherIcon(day.condition)),
        el("span", { class: "max", text: finite(day.temperature) ? `${Math.round(day.temperature)}°` : "—" }),
        el("span", { class: "min", text: finite(day.templow) ? `${Math.round(day.templow)}°` : "" }),
        el("small", { class: "rain", text: rain }),
      );
    });
  const value = el("div", { class: "value", text: finite(a.temperature) ? number.format(a.temperature) : "—" });
  if (finite(a.temperature)) value.append(el("small", { text: "°C" }));
  return el(
    "div",
    { id: tileId(state.entity_id), class: `${tileClass(state, "weather")} wide` },
    tileHead(state, weatherIcon(state.state), `${label}${feels}`),
    value,
    el("div", { class: "sub", text: details }),
    el("ol", { class: "forecast", "aria-label": "Prévisions" }, ...days),
    // Licence CC BY 4.0 d'Open-Meteo : la source doit être citée.
    el("small", { class: "attribution", text: typeof a.attribution === "string" ? a.attribution : "" }),
  );
}

// Le thermostat : consigne (− / +), marche / arrêt, préréglages.
// ---------------------------------------------------------------- Entrées
// Interrupteurs, nombres, listes, boutons et minuteurs virtuels (inputs.yaml).
function inputCommand(state, service, data = {}) {
  markPending(state.entity_id);
  callService(domainOf(state), service, { entity_id: state.entity_id, ...data });
}

function inputBooleanTile(state) {
  const on = state.state === "on";
  const toggle = el(
    "button",
    { class: "tile-toggle", type: "button", "aria-pressed": String(on) },
    tileHead(state, ICONS.toggle, on ? "Activé" : state.state === "off" ? "Désactivé" : statusText(state)),
  );
  toggle.addEventListener("click", () => inputCommand(state, "toggle"));
  return el("div", { id: tileId(state.entity_id), class: tileClass(state, "switch") }, toggle);
}

function inputNumberTile(state) {
  const a = state.attributes;
  const value = Number(state.state);
  const min = finite(a.min) ? a.min : 0;
  const max = finite(a.max) ? a.max : 100;
  const step = finite(a.step) && a.step > 0 ? a.step : 1;
  const unit = typeof a.unit_of_measurement === "string" ? a.unit_of_measurement : "";
  const shown = el("div", { class: "value", text: Number.isFinite(value) ? number.format(value) : "—" });
  if (unit) shown.append(el("small", { text: unit }));
  const children = [tileHead(state, ICONS.sliders, `entre ${number.format(min)} et ${number.format(max)}`)];
  if (a.mode === "box") {
    const adjust = (service, text, label) => {
      const button = el("button", { type: "button", class: "round", "aria-label": `${label} ${nameOf(state)}`, text });
      button.addEventListener("click", () => inputCommand(state, service));
      return button;
    };
    children.push(el("div", { class: "setpoint" }, adjust("decrement", "−", "Diminuer"), shown, adjust("increment", "+", "Augmenter")));
  } else {
    const slider = el("input", {
      type: "range", min: String(min), max: String(max), step: String(step),
      value: String(Number.isFinite(value) ? value : min), "aria-label": `Valeur de ${nameOf(state)}`,
    });
    // Pendant le glissement, la valeur s'affiche ; elle est envoyée au lâcher.
    slider.addEventListener("input", () => {
      shown.firstChild.textContent = number.format(Number(slider.value));
    });
    slider.addEventListener("change", () => inputCommand(state, "set_value", { value: Number(slider.value) }));
    children.push(shown, slider);
  }
  return el("div", { id: tileId(state.entity_id), class: tileClass(state, "sensor") }, ...children);
}

function inputSelectTile(state) {
  const options = (Array.isArray(state.attributes.options) ? state.attributes.options : [])
    .filter((option) => typeof option === "string");
  const select = el("select", { class: "tile-select", "aria-label": `Choix pour ${nameOf(state)}` });
  for (const option of options) select.append(el("option", { value: option, text: option }));
  select.value = state.state;
  select.addEventListener("change", () => inputCommand(state, "select_option", { option: select.value }));
  return el(
    "div",
    { id: tileId(state.entity_id), class: "tile sensor" },
    tileHead(state, ICONS.list, options.includes(state.state) ? state.state : statusText(state)),
    select,
  );
}

function inputButtonTile(state) {
  const subtitle = Number.isNaN(Date.parse(state.state)) ? "Jamais appuyé" : `Appuyé il y a ${ago(state.state)}`;
  const button = el("button", { type: "button", class: "action-button", text: "Appuyer" });
  button.addEventListener("click", () => inputCommand(state, "press"));
  return el("div", { id: tileId(state.entity_id), class: "tile action" }, tileHead(state, ICONS.button, subtitle), button);
}

const TIMER_LABELS = { idle: "Arrêté", active: "En marche", paused: "En pause" };

/** `1:05:09`, comme les attributs du serveur. */
function hms(seconds) {
  const s = Math.max(0, Math.ceil(seconds));
  return `${Math.floor(s / 3600)}:${String(Math.floor(s / 60) % 60).padStart(2, "0")}:${String(s % 60).padStart(2, "0")}`;
}

function timerRemaining(finishesAt) {
  return hms((Date.parse(finishesAt) - Date.now()) / 1000);
}

function timerTile(state) {
  const a = state.attributes;
  const active = state.state === "active" && typeof a.finishes_at === "string" && !Number.isNaN(Date.parse(a.finishes_at));
  const remaining = el("div", {
    class: "value countdown",
    text: active ? timerRemaining(a.finishes_at) : typeof a.remaining === "string" ? a.remaining : "—",
  });
  // Le compte à rebours avance tout seul (voir `tickCountdowns`).
  if (active) remaining.dataset.finishesAt = a.finishes_at;
  const button = (service, text) => {
    const b = el("button", { type: "button", class: "period", text });
    b.addEventListener("click", () => inputCommand(state, service));
    return b;
  };
  const buttons = state.state === "active" ? [button("pause", "Pause"), button("cancel", "Annuler")]
    : state.state === "paused" ? [button("start", "Reprendre"), button("cancel", "Annuler")]
    : [button("start", "Démarrer")];
  const subtitle = [lookup(TIMER_LABELS, state.state) ?? statusText(state), typeof a.duration === "string" ? `durée ${a.duration}` : ""]
    .filter(Boolean).join(" · ");
  return el(
    "div",
    { id: tileId(state.entity_id), class: `tile sensor${state.state === "active" ? " on" : ""}` },
    tileHead(state, ICONS.timer, subtitle),
    remaining,
    el("div", { class: "presets", role: "group", "aria-label": `Commandes de ${nameOf(state)}` }, ...buttons),
  );
}

function tickCountdowns() {
  for (const node of document.querySelectorAll("[data-finishes-at]")) {
    node.textContent = timerRemaining(node.dataset.finishesAt);
  }
}
setInterval(tickCountdowns, 1000);

const PRESET_LABELS = { confort: "Confort", eco: "Éco", absent: "Absent", hors_gel: "Hors-gel", nuit: "Nuit" };

function climateTile(state) {
  const a = state.attributes;
  const heating = a.hvac_action === "heating";
  const on = state.state === "heat";
  const target = finite(a.temperature) ? a.temperature : null;
  const step = finite(a.target_temp_step) && a.target_temp_step > 0 ? a.target_temp_step : 0.5;
  const measured = finite(a.current_temperature) ? `${degrees(a.current_temperature)} mesurés` : "température inconnue";
  const subtitle = [on ? (heating ? "Chauffe" : "En veille") : "Arrêté", measured].join(" · ");
  const command = (service, data = {}) => {
    markPending(state.entity_id);
    callService("climate", service, { entity_id: state.entity_id, ...data });
  };
  const adjust = (delta, label) => {
    const button = el("button", { type: "button", class: "round", "aria-label": `${label} la consigne de ${nameOf(state)}`, text: delta > 0 ? "+" : "−" });
    button.disabled = target === null;
    button.addEventListener("click", () => {
      const min = finite(a.min_temp) ? a.min_temp : 5;
      const max = finite(a.max_temp) ? a.max_temp : 30;
      command("set_temperature", { temperature: Math.min(max, Math.max(min, target + delta)) });
    });
    return button;
  };
  const power = el("button", { type: "button", class: "action-button", text: on ? "Éteindre" : "Allumer" });
  power.addEventListener("click", () => command("set_hvac_mode", { hvac_mode: on ? "off" : "heat" }));
  const presets = (Array.isArray(a.preset_modes) ? a.preset_modes : [])
    .filter((preset) => typeof preset === "string")
    .map((preset) => {
      const label = lookup(PRESET_LABELS, preset) ?? preset.replaceAll("_", " ");
      const button = el("button", { type: "button", class: "period", text: label, "aria-pressed": String(a.preset_mode === preset) });
      button.addEventListener("click", () => command("set_preset_mode", { preset_mode: preset }));
      return button;
    });
  const children = [
    tileHead(state, ICONS.flame, subtitle),
    el(
      "div",
      { class: "setpoint" },
      adjust(-step, "Baisser"),
      el("div", { class: "value", text: target === null ? "—" : number.format(target) }, el("small", { text: "°C" })),
      adjust(step, "Monter"),
    ),
  ];
  if (typeof a.info === "string") children.push(el("p", { class: "error climate-info", text: a.info }));
  if (presets.length > 0) children.push(el("div", { class: "presets", role: "group", "aria-label": "Préréglages" }, ...presets));
  children.push(power);
  return el(
    "div",
    // Pas de `tileClass` : l'état « heat » n'est pas « on », et « chauffe » se voit à part.
    { id: tileId(state.entity_id), class: `tile climate${heating ? " heating" : ""}${on ? "" : " stopped"}` },
    ...children,
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
  if (state.attributes.device_class === "timestamp") return timestampTile(state);
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

// Une date (dernière sauvegarde…) : « il y a 3 heures », et l'erreur éventuelle.
function timestampTile(state) {
  const valid = !Number.isNaN(Date.parse(state.state));
  const failure = typeof state.attributes.erreur === "string" ? state.attributes.erreur : "";
  const tile = el(
    "div",
    { id: tileId(state.entity_id), class: tileClass(state, "sensor") },
    tileHead(state, ICONS.archive, valid ? dateTime.format(new Date(state.state)) : "Jamais"),
    el("div", { class: "value small-value", text: valid ? `il y a ${ago(state.state)}` : "—" }),
  );
  if (failure) tile.append(el("p", { class: "error climate-info", text: `Dernier essai échoué : ${failure}` }));
  return tile;
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
  bell: ["M6 8a6 6 0 0 1 12 0c0 7 3 9 3 9H3s3-2 3-9", "M10 21a2 2 0 0 0 4 0"],
  play: ["M8 5v14l11-7z"],
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
  if (domain === "device_tracker" || domain === "person") return value === "home" ? "À la maison" : value === "not_home" ? "Absent" : value;
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
  confirm: "identité confirmée",
  confirm_failed: "échec de confirmation",
  backup: "sauvegarde",
  backup_failed: "échec de sauvegarde",
  backup_restore: "sauvegarde restaurée",
  automation_create: "automatisation créée",
  automation_update: "automatisation modifiée",
  automation_delete: "automatisation supprimée",
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
        { class: ["login_failed", "confirm_failed", "backup_failed", "freebox_new_device"].includes(entry.action) ? "failed" : "" },
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

// ---------------------------------------------------------------- Automatisations
// L'éditeur travaille directement sur la configuration JSON renvoyée par le
// serveur : le formulaire n'est qu'une vue de cet objet. Un champ que le
// formulaire ne sait pas afficher (paramètre rare…) est donc conservé tel quel.
// Le serveur vérifie tout, à chaque enregistrement : rien n'est fait confiance
// côté navigateur.
const auto = {
  list: [],
  editable: false,
  // Automatisation en cours d'édition.
  config: null,
  existingId: null, // identifiant si on modifie, null si on crée
  source: "editor",
  mode: "form",
  checkTimer: null,
  checkSeq: 0,
};

const TRIGGER_KINDS = {
  state: ["Un appareil change d'état", () => ({ trigger: "state", entity_id: "" })],
  numeric_state: ["Une valeur passe un seuil", () => ({ trigger: "numeric_state", entity_id: "", below: 18 })],
  time: ["À une heure précise", () => ({ trigger: "time", at: "07:30" })],
  sun: ["Lever ou coucher du soleil", () => ({ trigger: "sun", event: "sunset" })],
  event: ["Un événement (minuteur fini…)", () => ({ trigger: "event", event_type: "timer.finished", event_data: {} })],
};
const CONDITION_KINDS = {
  state: ["Un appareil est dans un état", () => ({ condition: "state", entity_id: "", state: "on" })],
  numeric_state: ["Une valeur est au-dessus / au-dessous", () => ({ condition: "numeric_state", entity_id: "", above: 20 })],
  time: ["Plage horaire, jours", () => ({ condition: "time", after: "18:00", before: "23:00" })],
  sun: ["Il fait jour / nuit", () => ({ condition: "sun", after: "sunset", before: "sunrise" })],
  or: ["Au moins une de ces conditions (ou)", () => ({ condition: "or", conditions: [] })],
  and: ["Toutes ces conditions (et)", () => ({ condition: "and", conditions: [] })],
  not: ["Aucune de ces conditions (non)", () => ({ condition: "not", conditions: [] })],
  trigger: ["Selon le déclencheur", () => ({ condition: "trigger", id: "" })],
};
const ACTION_KINDS = {
  service: ["Commander un appareil", () => ({ action: "light.turn_on", target: { entity_id: "" } })],
  delay: ["Attendre", () => ({ delay: "5min" })],
  if: ["Si… alors… sinon", () => ({ if: [], then: [] })],
  condition: ["Continuer seulement si…", () => ({ condition: "state", entity_id: "", state: "on" })],
  choose: ["Selon le cas (choose)", () => ({ choose: [] })],
};
const SERVICES = {
  light: [["turn_on", "Allumer"], ["turn_off", "Éteindre"], ["toggle", "Inverser"]],
  switch: [["turn_on", "Allumer"], ["turn_off", "Éteindre"], ["toggle", "Inverser"]],
  cover: [["open_cover", "Ouvrir"], ["close_cover", "Fermer"], ["stop_cover", "Arrêter"], ["set_cover_position", "Mettre à une position"]],
  script: [["turn_on", "Lancer"], ["turn_off", "Arrêter"]],
  scene: [["turn_on", "Activer"]],
  notify: [["send_message", "Envoyer une notification"]],
  climate: [["set_preset_mode", "Choisir un préréglage"], ["set_temperature", "Régler la consigne"], ["turn_on", "Allumer"], ["turn_off", "Éteindre"]],
  input_boolean: [["turn_on", "Activer"], ["turn_off", "Désactiver"], ["toggle", "Inverser"]],
  input_number: [["set_value", "Régler la valeur"], ["increment", "Augmenter d'un pas"], ["decrement", "Diminuer d'un pas"]],
  input_select: [["select_option", "Choisir"], ["select_next", "Choix suivant"], ["select_previous", "Choix précédent"], ["select_first", "Premier choix"]],
  input_button: [["press", "Appuyer"]],
  timer: [["start", "Démarrer"], ["pause", "Mettre en pause"], ["cancel", "Annuler"], ["finish", "Terminer"]],
};
const SUN_EVENTS = [["sunrise", "lever du soleil"], ["sunset", "coucher du soleil"], ["dawn", "aube"], ["dusk", "crépuscule"]];
const WEEKDAYS = [["mon", "lun"], ["tue", "mar"], ["wed", "mer"], ["thu", "jeu"], ["fri", "ven"], ["sat", "sam"], ["sun", "dim"]];

/** Appel à l'API : renvoie { status, data }. */
async function api(method, url, body) {
  const response = await fetch(url, {
    method,
    headers: {
      Authorization: `Bearer ${token.get()}`,
      ...(body !== undefined && { "Content-Type": "application/json" }),
    },
    ...(body !== undefined && { body: JSON.stringify(body) }),
  });
  if (response.status === 401) {
    for (const dialog of document.querySelectorAll("dialog[open]")) dialog.close();
    token.clear();
    live.close();
    showLogin("Session expirée : reconnecte-toi.");
    throw new Error("session expirée");
  }
  const data = await response.json().catch(() => ({}));
  return { status: response.status, data };
}

$("#open-automations").addEventListener("click", () => {
  $("#automations").showModal();
  showAutoList();
});
$("#automations-close").addEventListener("click", () => $("#automations").close());

// ----- Liste

async function showAutoList(notice = "") {
  $("#auto-edit-view").hidden = true;
  $("#auto-list-view").hidden = false;
  const message = $("#auto-notice");
  message.textContent = notice;
  message.hidden = !notice;
  const list = $("#auto-list");
  list.replaceChildren(el("li", { class: "auto-empty", text: "Chargement…" }));
  const { status, data } = await api("GET", "/api/automations");
  if (status !== 200) return list.replaceChildren(el("li", { class: "auto-empty", text: "Automatisations indisponibles." }));
  auto.list = data.automations ?? [];
  auto.editable = data.editable === true;
  $("#auto-readonly").hidden = auto.editable;
  $("#auto-new").hidden = !auto.editable;
  list.replaceChildren(...auto.list.map(autoCard));
  if (auto.list.length === 0) {
    list.replaceChildren(el("li", { class: "auto-empty", text: "Aucune automatisation pour l'instant." }));
  }
}

function autoCard(entry) {
  const badges = el("div", { class: "badges" }, el("span", { class: "badge", text: entry.source === "file" ? "fichier" : "éditeur" }));
  if (entry.error) badges.append(el("span", { class: "badge bad", text: "invalide" }));
  else if (!entry.enabled) badges.append(el("span", { class: "badge off", text: "désactivée" }));
  const card = el(
    "li",
    { class: "auto-card" },
    el("div", { class: "auto-card-head" }, el("strong", { text: entry.alias }), badges),
    el("p", { class: "auto-summary", text: entry.config ? describe(entry.config) : "" }),
  );
  if (entry.error) card.append(el("p", { class: "error", text: entry.error }));
  if (entry.updated_by && entry.updated_at) {
    card.append(el("small", { class: "auto-meta", text: `Modifiée par ${entry.updated_by} le ${dateTime.format(new Date(entry.updated_at))}` }));
  }
  const buttons = el("div", { class: "auto-card-buttons" });
  const open = el("button", { type: "button", class: "action-button", text: entry.source === "editor" && auto.editable ? "Modifier" : "Voir" });
  open.addEventListener("click", () => openAutoEditor(entry));
  buttons.append(open);
  if (entry.source === "editor" && auto.editable && entry.config) {
    const toggle = el("button", { type: "button", class: "action-button", text: entry.enabled ? "Désactiver" : "Activer" });
    toggle.addEventListener("click", () => {
      const config = structuredClone(entry.config);
      if (entry.enabled) config.enabled = false;
      else delete config.enabled;
      saveAutomation(entry.id, { config }, entry.enabled ? "Automatisation désactivée." : "Automatisation activée.");
    });
    buttons.append(toggle);
  }
  card.append(buttons);
  return card;
}

/** Résumé en français : « Quand … · Si … · Alors … ». */
function describe(config) {
  const parts = [`Quand ${(config.triggers ?? []).map(describeTrigger).join(", ou ")}`];
  if (config.conditions?.length) parts.push(`si ${config.conditions.map(describeCondition).join(" et ")}`);
  parts.push(`alors ${(config.actions ?? []).map(describeAction).join(", puis ")}`);
  return parts.join(" · ");
}

const asList = (value) => (Array.isArray(value) ? value : value === undefined || value === "" ? [] : [value]);
const entityNames = (value) => asList(value).map((id) => (entities.has(id) ? nameOf(entities.get(id)) : id)).join(" ou ") || "?";
const sunLabel = (event) => SUN_EVENTS.find(([key]) => key === event)?.[1] ?? event;

function describeTrigger(t) {
  switch (t.trigger) {
    case "state":
      return `${entityNames(t.entity_id)} ${t.to !== undefined ? `passe à « ${t.to} »` : "change"}${t.for ? ` depuis ${t.for}` : ""}`;
    case "numeric_state":
      return `${entityNames(t.entity_id)} ${[t.above !== undefined && `dépasse ${t.above}`, t.below !== undefined && `descend sous ${t.below}`].filter(Boolean).join(" et ")}`;
    case "time":
      return `à ${asList(t.at).join(", ")}`;
    case "sun":
      return `${offsetText(t.offset)}${sunLabel(t.event)}`;
    case "event":
      return `événement ${t.event_type}${t.event_data?.entity_id ? ` de ${entityNames(t.event_data.entity_id)}` : ""}`;
    default:
      return String(t.trigger);
  }
}

function describeCondition(c) {
  const list = (items) => (Array.isArray(items) ? items : []).map(describeCondition);
  switch (c.condition) {
    case "or":
      return `(${list(c.conditions).join(" ou ")})`;
    case "and":
      return `(${list(c.conditions).join(" et ")})`;
    case "not":
      return `pas (${list(c.conditions).join(" ou ")})`;
    case "trigger":
      return `déclenchée par « ${asList(c.id).join(" » ou « ")} »`;
    case "state":
      return `${entityNames(c.entity_id)} est « ${asList(c.state).join(" » ou « ")} »`;
    case "numeric_state":
      return `${entityNames(c.entity_id)} ${[c.above !== undefined && `> ${c.above}`, c.below !== undefined && `< ${c.below}`].filter(Boolean).join(" et ")}`;
    case "time": {
      const days = asList(c.weekday);
      return [c.after && `après ${c.after}`, c.before && `avant ${c.before}`, days.length && `le ${days.join(", ")}`].filter(Boolean).join(" ");
    }
    case "sun":
      return [c.after && `après le ${sunLabel(c.after)}`, c.before && `avant le ${sunLabel(c.before)}`].filter(Boolean).join(" et ");
    default:
      return String(c.condition);
  }
}

function describeAction(a) {
  const actions = (items) => (Array.isArray(items) ? items : []).map(describeAction).join(", puis ");
  switch (actionKind(a)) {
    case "delay":
      return `attendre ${a.delay}`;
    case "condition":
      return `seulement si ${describeCondition(a)}`;
    case "if": {
      const conditions = (Array.isArray(a.if) ? a.if : []).map(describeCondition).join(" et ");
      const otherwise = Array.isArray(a.else) && a.else.length ? ` sinon ${actions(a.else)}` : "";
      return `si ${conditions} alors ${actions(a.then)}${otherwise}`;
    }
    case "choose":
      return `selon le cas (${(a.choose ?? []).length} option(s))`;
  }
  const [domain, service] = String(a.action).split(".");
  const label = (lookup(SERVICES, domain) ?? []).find(([key]) => key === service)?.[1];
  return `${(label ?? a.action).toLowerCase()} ${entityNames(a.target?.entity_id)}`;
}

function offsetText(offset) {
  const minutes = offsetMinutes(offset);
  if (!minutes) return "au ";
  return `${Math.abs(minutes)} min ${minutes < 0 ? "avant le " : "après le "}`;
}

// ----- Édition

$("#auto-new").addEventListener("click", () =>
  openAutoEditor({
    id: null,
    source: "editor",
    config: { id: "", alias: "", triggers: [TRIGGER_KINDS.state[1]()], actions: [ACTION_KINDS.service[1]()] },
  }),
);

function openAutoEditor(entry) {
  auto.existingId = entry.id;
  auto.source = entry.source;
  auto.config = structuredClone(entry.config ?? { id: entry.id, triggers: [], actions: [] });
  auto.mode = entry.source === "file" || !auto.editable ? "yaml" : "form";
  $("#auto-list-view").hidden = true;
  $("#auto-edit-view").hidden = false;
  $("#auto-version-list").hidden = true;
  $("#auto-file-note").hidden = entry.source !== "file";
  $("#auto-yaml").value = entry.yaml ?? "";
  setAutoStatus("");
  renderAutoEditor();
  if (entry.id === null) $("#auto-form input")?.focus();
}

function readOnly() {
  return auto.source === "file" || !auto.editable;
}

function renderAutoEditor() {
  const ro = readOnly();
  $("#auto-tab-form").setAttribute("aria-pressed", String(auto.mode === "form"));
  $("#auto-tab-yaml").setAttribute("aria-pressed", String(auto.mode === "yaml"));
  $("#auto-tab-form").hidden = ro;
  $("#auto-tab-yaml").hidden = ro;
  $("#auto-form").hidden = auto.mode !== "form";
  $("#auto-yaml-label").hidden = auto.mode !== "yaml";
  $("#auto-yaml").readOnly = ro;
  $("#auto-save").hidden = ro;
  $("#auto-delete").hidden = ro || auto.existingId === null;
  $("#auto-duplicate").hidden = !auto.editable || auto.existingId === null;
  $("#auto-versions").hidden = auto.source === "file";
  if (auto.mode === "form") renderAutoForm();
}

$("#auto-tab-form").addEventListener("click", async () => {
  if (auto.mode === "form") return;
  // YAML → formulaire : le serveur lit le YAML et renvoie le JSON.
  const result = await checkAutomation({ yaml: $("#auto-yaml").value });
  if (!result) return;
  auto.config = result.config;
  auto.mode = "form";
  renderAutoEditor();
});

$("#auto-tab-yaml").addEventListener("click", async () => {
  if (auto.mode === "yaml") return;
  // Formulaire → YAML : il faut une automatisation valide.
  const result = await checkAutomation({ config: auto.config });
  if (!result) return setAutoStatus("Corrige d'abord l'erreur ci-dessus pour passer en YAML.", "error", true);
  $("#auto-yaml").value = result.yaml;
  auto.mode = "yaml";
  renderAutoEditor();
});

$("#auto-yaml").addEventListener("input", () => scheduleCheck());

function currentRequest() {
  return auto.mode === "yaml" ? { yaml: $("#auto-yaml").value } : { config: auto.config };
}

/** Vérifie auprès du serveur ; affiche le résultat. Renvoie la réponse si valide. */
async function checkAutomation(request = currentRequest()) {
  const seq = ++auto.checkSeq;
  const { status, data } = await api("POST", "/api/automations/check", request);
  if (seq !== auto.checkSeq) return null; // une vérification plus récente est partie
  if (status !== 200) {
    setAutoStatus(data.error ?? "Automatisation invalide.", "error");
    return null;
  }
  setAutoStatus("✓ Automatisation valide.", "ok", false, data.warnings);
  return data;
}

function scheduleCheck() {
  clearTimeout(auto.checkTimer);
  auto.checkTimer = setTimeout(() => checkAutomation().catch(() => {}), 600);
}

function setAutoStatus(text, kind = "", append = false, warnings = []) {
  const status = $("#auto-status");
  if (append && status.textContent) status.append(el("br"), text);
  else status.textContent = text;
  status.className = `auto-status ${kind}`;
  $("#auto-warnings").replaceChildren(...(warnings ?? []).map((w) => el("li", { text: `⚠ ${w}` })));
}

// ----- Le formulaire

function renderAutoForm() {
  const c = auto.config;
  const form = $("#auto-form");
  const scroll = $("#automations").scrollTop;
  const alias = textField("Nom", c, "alias", { placeholder: "Volets au coucher du soleil" });
  alias.querySelector("input").addEventListener("input", () => {
    // À la création, l'identifiant suit le nom.
    if (auto.existingId === null) {
      c.id = slug(c.alias ?? "");
      $("#auto-id").value = c.id;
    }
  });
  const id = textField("Identifiant", c, "id", { placeholder: "volets_coucher_soleil" });
  const idInput = id.querySelector("input");
  idInput.id = "auto-id";
  idInput.disabled = auto.existingId !== null;
  idInput.setAttribute("autocapitalize", "none");
  const enabled = el("input", { type: "checkbox" });
  enabled.checked = c.enabled !== false;
  enabled.addEventListener("change", () => {
    if (enabled.checked) delete c.enabled;
    else c.enabled = false;
    scheduleCheck();
  });

  const mode = el("select");
  for (const [value, text] of [
    ["single", "ignorer le nouveau déclenchement (par défaut)"],
    ["restart", "recommencer depuis le début"],
    ["queued", "attendre la fin, puis recommencer"],
    ["parallel", "en lancer une autre en même temps"],
  ]) mode.append(el("option", { value, text }));
  mode.value = c.mode ?? "single";
  mode.addEventListener("change", () => {
    if (mode.value === "single") delete c.mode;
    else c.mode = mode.value;
    if (mode.value !== "queued" && mode.value !== "parallel") delete c.max;
    scheduleCheck();
  });

  form.replaceChildren(
    el("div", { class: "auto-row" }, alias, id),
    el("label", { class: "check" }, enabled, "Activée"),
    el("label", {}, el("span", { class: "label", text: "Si elle se redéclenche pendant qu'elle tourne encore" }), mode),
    itemsBlock("Quand…", "Déclencheurs : l'un d'eux suffit.", c, "triggers", TRIGGER_KINDS, "trigger", triggerFields),
    itemsBlock("Si… (facultatif)", "Conditions : toutes doivent être vraies.", c, "conditions", CONDITION_KINDS, "condition", conditionFields),
    itemsBlock("Alors…", "Actions, dans l'ordre.", c, "actions", ACTION_KINDS, null, actionFields),
  );
  $("#automations").scrollTop = scroll;
}

/** Un bloc « Quand / Si / Alors » : la liste, et un bouton pour ajouter. */
function itemsBlock(title, hint, config, key, kinds, tag, fields) {
  const items = config[key] ?? [];
  const block = el("fieldset", { class: "auto-block" }, el("legend", { text: title }), el("p", { class: "journal-hint", text: hint }));
  items.forEach((item, index) => {
    const kind = tag ? item[tag] : actionKind(item);
    const label = lookup(kinds, kind)?.[0] ?? kind;
    const remove = el("button", { type: "button", class: "ghost small", text: "Retirer", "aria-label": `Retirer : ${label}` });
    remove.addEventListener("click", () => {
      items.splice(index, 1);
      // Liste facultative vidée : on retire la clé (comme dans le YAML).
      if (items.length === 0 && (key === "conditions" || key === "else")) delete config[key];
      renderAutoForm();
      scheduleCheck();
    });
    const head = el("div", { class: "auto-item-head" }, el("strong", { text: label }));
    if (["actions", "then", "else"].includes(key) && index > 0) {
      const up = el("button", { type: "button", class: "ghost small", text: "↑", "aria-label": "Monter" });
      up.addEventListener("click", () => {
        [items[index - 1], items[index]] = [items[index], items[index - 1]];
        renderAutoForm();
        scheduleCheck();
      });
      head.append(up);
    }
    head.append(remove);
    // Seule cette ligne est redessinée quand un champ change sa forme (autre
    // appareil, autre commande) : les autres gardent le focus et le curseur.
    const box = el("div", { class: "auto-fields" });
    let busy = false;
    const refresh = () => {
      // Retirer le champ qui a le focus déclenche son événement « change »,
      // qui redemanderait un dessin : on l'ignore pendant celui-ci.
      if (busy) return;
      busy = true;
      const focusables = () => [...box.querySelectorAll("input, select")];
      const focused = focusables().indexOf(document.activeElement);
      box.replaceChildren(...fields(item, refresh));
      if (focused >= 0) focusables()[focused]?.focus();
      busy = false;
    };
    refresh();
    block.append(el("div", { class: "auto-item" }, head, box));
  });
  const add = el("select", { "aria-label": `Ajouter (${title})` }, el("option", { value: "", text: "+ Ajouter…" }));
  for (const [value, [text]] of Object.entries(kinds)) {
    // « Selon le cas » se crée dans l'onglet YAML (le formulaire l'affiche seulement).
    if (value !== "choose") add.append(el("option", { value, text }));
  }
  add.addEventListener("change", () => {
    const make = lookup(kinds, add.value)?.[1];
    if (!make) return;
    config[key] = [...items, make()];
    renderAutoForm();
    scheduleCheck();
  });
  block.append(add);
  return block;
}

/** La sorte d'une action, d'après sa clé. */
function actionKind(action) {
  for (const key of ["delay", "if", "choose", "condition"]) {
    if (action[key] !== undefined) return key;
  }
  return "service";
}

function triggerFields(t) {
  return [...triggerKindFields(t), textField("Identifiant (facultatif)", t, "id", { placeholder: "arrivee" })];
}

function triggerKindFields(t) {
  switch (t.trigger) {
    case "state":
      return [
        entityField(t, "entity_id"),
        textField("De (facultatif)", t, "from", { placeholder: "ex. : off" }),
        textField("À (facultatif)", t, "to", { placeholder: "ex. : on" }),
        durationField("Depuis au moins (facultatif)", t, "for", true),
      ];
    case "numeric_state":
      return [entityField(t, "entity_id"), numberField("Au-dessus de", t, "above"), numberField("Au-dessous de", t, "below")];
    case "time":
      return [timeField("Heure", t, "at")];
    case "sun":
      return [selectField("Événement", t, "event", SUN_EVENTS), offsetField(t, "offset")];
    case "event": {
      t.event_data ??= {};
      const entity = entityField(t.event_data, "entity_id");
      // Vide : n'importe quel appareil (et non « l'appareil sans nom »).
      entity.querySelector("input").addEventListener("input", () => {
        if (t.event_data.entity_id === "") delete t.event_data.entity_id;
      });
      return [textField("Événement", t, "event_type", { placeholder: "timer.finished" }), entity];
    }
    default:
      return [];
  }
}

/** Un bloc imbriqué (dans « ou », « si »…) occupe toute la largeur. */
function nested(block) {
  block.classList.add("nested");
  return block;
}

function conditionFields(c) {
  switch (c.condition) {
    case "or":
    case "and":
    case "not":
      c.conditions ??= [];
      return [nested(itemsBlock("Conditions", lookup(CONDITION_KINDS, c.condition)?.[0] ?? "", c, "conditions", CONDITION_KINDS, "condition", conditionFields))];
    case "trigger":
      return [textField("Identifiant du déclencheur", c, "id", { placeholder: "arrivee", list: true })];
    case "state":
      return [entityField(c, "entity_id"), textField("État", c, "state", { placeholder: "ex. : on", list: true })];
    case "numeric_state":
      return [entityField(c, "entity_id"), numberField("Au-dessus de", c, "above"), numberField("Au-dessous de", c, "below")];
    case "time":
      return [timeField("Après (facultatif)", c, "after", true), timeField("Avant (facultatif)", c, "before", true), weekdayField(c)];
    case "sun":
      return [
        selectField("Après", c, "after", SUN_EVENTS, true),
        offsetField(c, "after_offset"),
        selectField("Avant", c, "before", SUN_EVENTS, true),
        offsetField(c, "before_offset"),
      ];
    default:
      return [];
  }
}

function actionFields(a, refresh) {
  switch (actionKind(a)) {
    case "delay":
      return [durationField("Durée", a, "delay", false)];
    case "condition":
      return conditionFields(a);
    case "if":
      a.if ??= [];
      a.then ??= [];
      return [
        nested(itemsBlock("Si…", "Toutes doivent être vraies.", a, "if", CONDITION_KINDS, "condition", conditionFields)),
        nested(itemsBlock("Alors…", "Si c'est vrai.", a, "then", ACTION_KINDS, null, actionFields)),
        nested(itemsBlock("Sinon… (facultatif)", "Si c'est faux.", a, "else", ACTION_KINDS, null, actionFields)),
      ];
    case "choose":
      return [el("p", { class: "journal-hint", text: `Bloc « selon le cas » (${(a.choose ?? []).length} option(s)) : il se modifie dans l'onglet YAML.` })];
  }
  a.target ??= { entity_id: "" };
  const entity = entityField(a.target, "entity_id", () => {
    // L'appareil change de type : on propose sa première commande.
    const domain = asList(a.target.entity_id)[0]?.split(".")[0];
    const services = lookup(SERVICES, domain);
    if (services && !String(a.action).startsWith(`${domain}.`)) {
      a.action = `${domain}.${services[0][0]}`;
      refresh();
      scheduleCheck();
    }
  });
  const domain = asList(a.target.entity_id)[0]?.split(".")[0] ?? String(a.action).split(".")[0];
  const options = (lookup(SERVICES, domain) ?? []).map(([service, label]) => [`${domain}.${service}`, label]);
  if (!options.some(([value]) => value === a.action)) options.push([a.action, a.action]);
  const fields = [entity, selectField("Commande", a, "action", options, false, refresh)];

  a.data ??= {};
  const extra = { ...a.data };
  if (a.action === "light.turn_on") {
    fields.push(percentField("Luminosité % (facultatif)", a.data, "brightness"));
    delete extra.brightness;
  } else if (a.action === "cover.set_cover_position") {
    fields.push(numberField("Position (0 = fermé, 100 = ouvert)", a.data, "position"));
    delete extra.position;
  } else if (a.action === "climate.set_temperature") {
    fields.push(numberField("Consigne (°C)", a.data, "temperature"));
    delete extra.temperature;
  } else if (a.action === "climate.set_preset_mode") {
    fields.push(textField("Préréglage", a.data, "preset_mode", { placeholder: "eco" }));
    delete extra.preset_mode;
  } else if (a.action === "input_number.set_value") {
    fields.push(numberField("Valeur", a.data, "value"));
    delete extra.value;
  } else if (a.action === "input_select.select_option") {
    fields.push(textField("Choix", a.data, "option"));
    delete extra.option;
  } else if (a.action === "timer.start") {
    fields.push(durationField("Durée (facultatif, sinon celle du fichier)", a.data, "duration", true));
    delete extra.duration;
  } else if (a.action === "notify.send_message") {
    fields.push(textField("Titre (facultatif)", a.data, "title"), textField("Message", a.data, "message"));
    fields.push(selectField("Priorité", a.data, "priority", [[1, "1 (discrète)"], [2, "2"], [3, "3 (normale)"], [4, "4"], [5, "5 (urgente)"]], true));
    delete extra.title;
    delete extra.message;
    delete extra.priority;
  }
  if (Object.keys(extra).length > 0) {
    fields.push(el("p", { class: "journal-hint", text: `Autres paramètres, conservés : ${JSON.stringify(extra)}` }));
  }
  return fields;
}

// ----- Champs du formulaire (chacun lit et modifie une clé de l'objet)

function field(label, input) {
  return el("label", {}, el("span", { class: "label", text: label }), input);
}

function changed() {
  scheduleCheck();
}

/** Texte. `list` : plusieurs valeurs séparées par des virgules. Vide = clé retirée. */
function textField(label, object, key, { placeholder = "", list = false } = {}) {
  const value = object[key];
  const input = el("input", { placeholder, value: Array.isArray(value) ? value.join(", ") : value ?? "" });
  input.addEventListener("input", () => {
    const parts = list ? input.value.split(",").map((s) => s.trim()).filter(Boolean) : [input.value];
    if (list ? parts.length === 0 : input.value === "") delete object[key];
    else object[key] = list && parts.length > 1 ? parts : parts[0];
    changed();
  });
  return field(label, input);
}

/** Appareil(s) : suggestions tirées des appareils connus. */
function entityField(object, key, onChange) {
  const value = object[key];
  const input = el("input", {
    list: "entity-list", placeholder: "light.salon", autocapitalize: "none", spellcheck: "false",
    value: Array.isArray(value) ? value.join(", ") : value ?? "",
  });
  input.addEventListener("input", () => {
    const parts = input.value.split(",").map((s) => s.trim()).filter(Boolean);
    object[key] = parts.length > 1 ? parts : parts[0] ?? "";
    changed();
  });
  if (onChange) input.addEventListener("change", onChange);
  const names = asList(value).filter((id) => entities.has(id)).map((id) => nameOf(entities.get(id)));
  input.title = names.join(", ");
  const label = names.length ? `Appareil(s) · ${names.join(", ")}` : "Appareil(s)";
  return field(label, input);
}

function numberField(label, object, key) {
  const input = el("input", { type: "number", step: "any", inputmode: "decimal", value: object[key] ?? "" });
  input.addEventListener("input", () => {
    if (input.value === "" || !Number.isFinite(Number(input.value))) delete object[key];
    else object[key] = Number(input.value);
    changed();
  });
  return field(label, input);
}

/** Luminosité en % à l'écran, 1 à 255 dans la configuration. */
function percentField(label, object, key) {
  const value = typeof object[key] === "number" ? Math.round((object[key] / 255) * 100) : "";
  const input = el("input", { type: "number", min: "1", max: "100", inputmode: "numeric", value });
  input.addEventListener("input", () => {
    const percent = Number(input.value);
    if (input.value === "" || !Number.isFinite(percent)) delete object[key];
    else object[key] = Math.min(255, Math.max(1, Math.round(percent * 2.55)));
    changed();
  });
  return field(label, input);
}

function selectField(label, object, key, options, optional = false, onChange) {
  const select = el("select");
  if (optional) select.append(el("option", { value: "", text: "—" }));
  for (const [value, text] of options) select.append(el("option", { value: String(value), text }));
  select.value = object[key] === undefined ? "" : String(object[key]);
  select.addEventListener("change", () => {
    const option = options.find(([value]) => String(value) === select.value);
    if (!option) delete object[key];
    else object[key] = option[0];
    changed();
    onChange?.();
  });
  return field(label, select);
}

/** Heure : un sélecteur d'heure, ou du texte pour plusieurs heures. */
function timeField(label, object, key, optional = false) {
  const value = object[key];
  const simple = value === undefined || (typeof value === "string" && /^\d{2}:\d{2}$/.test(value));
  if (!simple) return textField(label, object, key, { placeholder: "07:30, 19:00", list: true });
  const input = el("input", { type: "time", value: value ?? "" });
  input.addEventListener("input", () => {
    if (input.value === "" && optional) delete object[key];
    else object[key] = input.value;
    changed();
  });
  return field(label, input);
}

/** Durée : un nombre et une unité (`5min`, `30s`, `2h`). */
function durationField(label, object, key, optional) {
  const seconds = durationSeconds(object[key]);
  const [amount, unit] =
    seconds === null ? ["", "min"] : seconds % 3600 === 0 && seconds > 0 ? [seconds / 3600, "h"] : seconds % 60 === 0 ? [seconds / 60, "min"] : [seconds, "s"];
  const number = el("input", { type: "number", min: "0", inputmode: "numeric", value: String(amount), "aria-label": label });
  const select = el("select", { "aria-label": `Unité : ${label}` });
  for (const [value, text] of [["s", "secondes"], ["min", "minutes"], ["h", "heures"]]) select.append(el("option", { value, text }));
  select.value = unit;
  const update = () => {
    if (number.value === "" && optional) delete object[key];
    else object[key] = `${Math.max(0, Math.round(Number(number.value) || 0))}${select.value}`;
    changed();
  };
  number.addEventListener("input", update);
  select.addEventListener("change", update);
  return el("label", {}, el("span", { class: "label", text: label }), el("span", { class: "inline" }, number, select));
}

/** `5min` → 300 ; `00:05:00` → 300 ; illisible → null. */
function durationSeconds(value) {
  if (typeof value !== "string") return null;
  let match = /^(\d+)(s|min|h)$/.exec(value);
  if (match) return Number(match[1]) * { s: 1, min: 60, h: 3600 }[match[2]];
  match = /^(\d{1,2}):(\d{2})(?::(\d{2}))?$/.exec(value);
  if (match) return Number(match[1]) * 3600 + Number(match[2]) * 60 + Number(match[3] ?? 0);
  return null;
}

/** Décalage du soleil, en minutes : `-00:30:00` ↔ 30 minutes avant. */
function offsetMinutes(value) {
  if (typeof value !== "string") return 0;
  const negative = value.startsWith("-");
  const seconds = durationSeconds(value.replace(/^[-+]/, ""));
  return seconds === null ? 0 : Math.round(seconds / 60) * (negative ? -1 : 1);
}

function offsetField(object, key) {
  const minutes = offsetMinutes(object[key]);
  const number = el("input", { type: "number", min: "0", max: "1440", inputmode: "numeric", value: minutes ? String(Math.abs(minutes)) : "", "aria-label": "Décalage en minutes" });
  const select = el("select", { "aria-label": "Avant ou après" });
  for (const [value, text] of [["after", "après"], ["before", "avant"]]) select.append(el("option", { value, text }));
  select.value = minutes < 0 ? "before" : "after";
  const update = () => {
    const total = Math.min(1440, Math.round(Math.abs(Number(number.value) || 0)));
    if (total === 0) delete object[key];
    else {
      const hh = String(Math.floor(total / 60)).padStart(2, "0");
      const mm = String(total % 60).padStart(2, "0");
      object[key] = `${select.value === "before" ? "-" : ""}${hh}:${mm}:00`;
    }
    changed();
  };
  number.addEventListener("input", update);
  select.addEventListener("change", update);
  return el("label", {}, el("span", { class: "label", text: "Décalage en min (facultatif)" }), el("span", { class: "inline" }, number, select));
}

function weekdayField(c) {
  const days = new Set(asList(c.weekday));
  const box = el("div", { class: "weekdays", role: "group", "aria-label": "Jours" });
  for (const [en, fr] of WEEKDAYS) {
    const input = el("input", { type: "checkbox" });
    input.checked = days.has(en) || days.has(fr);
    input.addEventListener("change", () => {
      const boxes = box.querySelectorAll("input");
      const chosen = WEEKDAYS.filter((_, i) => boxes[i].checked).map(([en]) => en);
      if (chosen.length === 0) delete c.weekday;
      else c.weekday = chosen;
      changed();
    });
    box.append(el("label", { class: "check" }, input, fr));
  }
  return el("div", { class: "weekdays-field" }, el("span", { class: "label", text: "Jours (aucun = tous)" }), box);
}

/** « Volets au coucher ! » → « volets_au_coucher » (identifiant valide). */
function slug(text) {
  return text
    .normalize("NFD")
    .replace(/[̀-ͯ]/g, "")
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "_")
    .replace(/^_+|_+$/g, "")
    .slice(0, 64);
}

// ----- Enregistrer, supprimer, versions

$("#auto-save").addEventListener("click", () => {
  const request = currentRequest();
  const id = auto.mode === "yaml" ? auto.existingId ?? yamlId($("#auto-yaml").value) : auto.config.id;
  if (!id) return setAutoStatus("Donne un nom (ou un identifiant) à l'automatisation.", "error");
  saveAutomation(id, request, auto.existingId === null ? "Automatisation créée." : "Automatisation enregistrée.");
});

/** L'identifiant écrit dans un YAML (`id: …`), pour l'adresse de la requête. */
function yamlId(yaml) {
  return /^\s*-?\s*id:\s*["']?([a-z0-9_]+)["']?\s*$/m.exec(yaml)?.[1] ?? null;
}

async function saveAutomation(id, request, done) {
  const { status, data } = await api("PUT", `/api/automations/${encodeURIComponent(id)}`, request);
  if (status === 403 && data.confirm) return askConfirmation(() => saveAutomation(id, request, done));
  if (status !== 200 && status !== 201) {
    if (!$("#auto-edit-view").hidden) return setAutoStatus(data.error ?? "Enregistrement impossible.", "error");
    return showAutoList(data.error ?? "Enregistrement impossible.");
  }
  const warnings = data.warnings?.length ? ` Attention : ${data.warnings.join(" ; ")}.` : "";
  showAutoList(done + warnings);
}

$("#auto-delete").addEventListener("click", () => {
  const id = auto.existingId;
  if (id === null || !window.confirm(`Supprimer l'automatisation « ${auto.config?.alias || id} » ? (Ses versions sont gardées.)`)) return;
  deleteAutomation(id);
});

async function deleteAutomation(id) {
  const { status, data } = await api("DELETE", `/api/automations/${encodeURIComponent(id)}`);
  if (status === 403 && data.confirm) return askConfirmation(() => deleteAutomation(id));
  if (status !== 204) return setAutoStatus(data.error ?? "Suppression impossible.", "error");
  showAutoList("Automatisation supprimée.");
}

$("#auto-duplicate").addEventListener("click", async () => {
  // Depuis le YAML, on repasse par le serveur pour obtenir le JSON.
  let config = auto.config;
  if (auto.mode === "yaml") {
    const result = await checkAutomation({ yaml: $("#auto-yaml").value }).catch(() => null);
    if (!result) return;
    config = result.config;
  }
  const copy = structuredClone(config);
  const ids = new Set(auto.list.map((entry) => entry.id));
  let id = `${copy.id}_copie`.slice(0, 64);
  for (let n = 2; ids.has(id); n++) id = `${copy.id}_copie_${n}`.slice(0, 64);
  copy.id = id;
  copy.alias = `${copy.alias ?? config.id} (copie)`;
  openAutoEditor({ id: null, source: "editor", config: copy });
  setAutoStatus("Copie prête : modifie-la puis enregistre.", "ok");
});

$("#auto-versions").addEventListener("click", async () => {
  const id = auto.existingId ?? auto.config?.id;
  const list = $("#auto-version-list");
  if (!id) return;
  const { status, data } = await api("GET", `/api/automations/${encodeURIComponent(id)}/versions`);
  if (status !== 200) return setAutoStatus("Versions indisponibles.", "error");
  list.hidden = false;
  list.replaceChildren(
    ...data.versions.map((version) => {
      const item = el(
        "li",
        {},
        el("time", { datetime: version.changed_at, text: dateTime.format(new Date(version.changed_at)) }),
        el("span", { text: `${version.changed_by}${version.deleted ? " · supprimée" : ""}` }),
      );
      if (version.config && !readOnly()) {
        const load = el("button", { type: "button", class: "ghost small", text: "Charger" });
        load.addEventListener("click", () => {
          auto.config = version.config;
          $("#auto-yaml").value = version.yaml ?? "";
          renderAutoEditor();
          setAutoStatus("Version chargée : enregistre pour la rétablir.", "ok");
        });
        item.append(load);
      }
      return item;
    }),
  );
  if (data.versions.length === 0) list.replaceChildren(el("li", { text: "Aucune version enregistrée." }));
});

$("#auto-cancel").addEventListener("click", () => showAutoList());

// ----- Confirmation d'identité

let afterConfirmation = null;

function askConfirmation(then) {
  afterConfirmation = then;
  const form = $("#confirm-form");
  form.reset();
  $("#confirm-error").hidden = true;
  $("#confirm").showModal();
  form.password.focus();
}

$("#confirm-cancel").addEventListener("click", () => {
  afterConfirmation = null;
  $("#confirm").close();
});

$("#confirm-form").addEventListener("submit", async (event) => {
  event.preventDefault();
  const form = event.currentTarget;
  const button = form.querySelector("button[type=submit]");
  button.disabled = true;
  try {
    const code = form.code.value.trim();
    const { status } = await api("POST", "/api/auth/confirm", {
      password: form.password.value,
      ...(code && { code }),
    });
    form.password.value = "";
    form.code.value = "";
    if (status !== 200) {
      const error = $("#confirm-error");
      error.textContent = status === 429 ? "Trop de tentatives. Réessaie dans 15 minutes." : "Mot de passe ou code incorrect.";
      error.hidden = false;
      return;
    }
    $("#confirm").close();
    const then = afterConfirmation;
    afterConfirmation = null;
    then?.();
  } finally {
    button.disabled = false;
  }
});

/** Suggestions d'appareils pour les champs « Appareil(s) ». */
function refreshEntityList() {
  const list = $("#entity-list");
  const sorted = [...entities.values()].sort((a, b) => a.entity_id.localeCompare(b.entity_id));
  list.replaceChildren(...sorted.map((state) => el("option", { value: state.entity_id, label: nameOf(state) })));
}

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
