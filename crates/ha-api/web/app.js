// Tableau de bord — JavaScript moderne, sans bibliothèque ni étape de construction.
//
// Ce fichier est le cœur : l'apparence, la connexion, le temps réel, les
// commandes, les pièces, et l'appel à l'API. Les autres parties sont dans
// leurs propres fichiers, chargés dans l'ordre par index.html : utils.js
// (avant celui-ci), puis tiles.js, dialogs.js, editor.js, account.js et
// start.js (le démarrage, en dernier), puis canvas.js, house.js et agenda.js.
//
// Sécurité : les noms d'appareils viennent du réseau (MQTT, pont Hue). On ne
// les insère JAMAIS avec `innerHTML` : uniquement avec `textContent`, qui
// affiche le texte tel quel. Un nom comme `<img onerror=…>` reste du texte.
"use strict";

const TOKEN_KEY = "ha_token";
const $ = (selector) => document.querySelector(selector);

// ---------------------------------------------------------------- Apparence
// Clair, sombre (ou comme l'appareil), le style « plan d'architecte », de
// jour ou de nuit, et « maquette » (fond sombre dégradé, house.js). Les trois
// derniers prennent la mise en page « tableau » : les pièces à gauche, le
// dessin de la maison au centre, les volets en dessous. Un réglage de cet appareil seulement
// (localStorage) : la télé du salon peut être en plan, le téléphone en sombre.
// Appliqué tout de suite, avant le premier affichage de la page.
const THEME_KEY = "ha-theme";
const THEMES = ["auto", "light", "dark", "plan", "plan-nuit", "maquette"];
const darkQuery = matchMedia("(prefers-color-scheme: dark)");

function themeWanted() {
  try {
    const value = localStorage.getItem(THEME_KEY);
    return THEMES.includes(value) ? value : "auto";
  } catch {
    return "auto"; // stockage bloqué (navigation privée…)
  }
}

function applyTheme(choice = themeWanted()) {
  const root = document.documentElement;
  root.dataset.theme = choice === "auto" ? (darkQuery.matches ? "dark" : "light") : choice;
  root.dataset.layout = ["plan", "plan-nuit", "maquette"].includes(choice) ? "board" : "classic";
  // La barre du navigateur (téléphone) prend la couleur du fond.
  const color = getComputedStyle(root).getPropertyValue("--bar").trim();
  if (color) for (const meta of document.querySelectorAll('meta[name="theme-color"]')) meta.content = color;
}

const isPlan = () => document.documentElement.dataset.theme?.startsWith("plan") === true;
const isBoard = () => document.documentElement.dataset.layout === "board";

applyTheme();
darkQuery.addEventListener("change", () => applyTheme());

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
  $("#account-dialog").close();
  const current = token.get();
  // Se déconnecter retire aussi cet appareil des notifications : sans ça,
  // la tablette partagée recevrait encore les « Ordre à valider » (et leurs
  // boutons) du compte qui vient de partir.
  if (current) await forgetPushHere(current).catch(() => {});
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
          refreshLogbookSoon();
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
  scheduleHome();
  // La tuile est dans les sections, et peut-être aussi dans la fenêtre d'une
  // pièce (house.js) : on remplace chacune.
  const existing = document.querySelectorAll(`:is(.sections, .room-body) #${CSS.escape(tileId(entity_id))}`);
  if (existing.length > 0 && new_state) for (const tile of existing) tile.replaceWith(renderTile(new_state));
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
  } else if (response.status === 403) {
    const body = await response.json().catch(() => ({}));
    // Déverrouiller une porte : le serveur redemande le mot de passe.
    if (body.confirm === true) {
      unmarkPending(data.entity_id);
      askConfirmation(() => callService(domain, service, data));
      return;
    }
    flash(typeof body.error === "string" ? `${body.error}.` : "Ton compte ne permet pas cette commande.");
    unmarkPending(data.entity_id);
  } else if (response.status === 202) {
    // Pièce protégée, heures calmes : l'ordre attend qu'un membre de la
    // famille le valide.
    const body = await response.json().catch(() => ({}));
    flash(typeof body.message === "string" ? `${body.message}.` : "Ordre en attente de validation.");
    unmarkPending(data.entity_id);
  } else if (response.status === 400) {
    // L'ordre ne convient pas à l'appareil (« consigne entre 7 et 30 ») :
    // le serveur l'a refusé avant de l'envoyer.
    const body = await response.json().catch(() => ({}));
    flash(typeof body.error === "string" ? `${body.error}.` : "Commande refusée.");
    unmarkPending(data.entity_id);
  } else if ([502, 503, 504].includes(response.status)) {
    // L'appareil a refusé, son intégration est en panne, ou il n'a pas
    // répondu : on le dit, plutôt que de laisser la tuile clignoter.
    const body = await response.json().catch(() => ({}));
    flash(typeof body.error === "string" ? `${body.error}.` : "L'appareil n'a pas répondu.");
    unmarkPending(data.entity_id);
  }
}

// La tuile « clignote » jusqu'à ce que l'appareil confirme (ou 5 s au plus).
// (La même tuile peut être deux fois dans la page : dans les sections et dans
// la fenêtre d'une pièce.)
const tilesOf = (entityId) => document.querySelectorAll(`#${CSS.escape(tileId(entityId))}`);

function markPending(entityId) {
  for (const tile of tilesOf(entityId)) tile.classList.add("pending");
  setTimeout(() => unmarkPending(entityId), 5000);
}

function unmarkPending(entityId) {
  for (const tile of tilesOf(entityId)) tile.classList.remove("pending");
}

// ---------------------------------------------------------------- Affichage
const SECTIONS = [
  { title: "Rappels", domains: ["alert"] },
  { title: "Agenda et listes", domains: ["calendar", "todo"] },
  { title: "Entrées", domains: ["input_boolean", "input_number", "input_select", "input_button", "timer", "counter", "input_text", "input_datetime", "schedule"] },
  // Les capteurs de la météo (sensor.meteo_…) vont avec la météo.
  { title: "Météo, présence et soleil", domains: ["weather", "zone", "person", "device_tracker", "sun", "notify"], extra: (s) => s.entity_id.startsWith("sensor.meteo_") || s.entity_id === VIGILANCE_TODAY },
  { title: "Chauffage", domains: ["climate"] },
  // Fonctions appelées plus tard : isLight est défini plus bas.
  { title: "Lumières", domains: [], extra: (s) => isLight(s) },
  { title: "Volets", domains: ["cover"] },
  { title: "Prises et interrupteurs", domains: ["switch", "fan"], extra: (s) => isOutlet(s) },
  { title: "Musique et annonces", domains: ["media_player", "tts"] },
  { title: "Alarme, serrures et sirènes", domains: ["alarm_control_panel", "lock", "siren"] },
  { title: "Boutons et réglages", domains: ["button", "number", "select", "text", "event"] },
  { title: "Eau et air", domains: ["valve", "humidifier", "water_heater"] },
  { title: "Aspirateurs", domains: ["vacuum"] },
  { title: "Capteurs", domains: ["sensor"] },
  { title: "Détecteurs", domains: ["binary_sensor"] },
];

// ---------------------------------------------------------------- Pièces
// Les pièces viennent du fichier areas.yaml (GET /api/areas).
let areas = [];
const customNames = new Map();
// Les pièces de l'onglet affiché (mise en page « tableau », house.js), ou
// `null` : toute la maison.
let pageAreas = null;

/** Les pièces à montrer : celles de l'onglet, ou toutes. */
function shownAreas() {
  return pageAreas !== null && isBoard() ? areas.filter((area) => pageAreas.has(area.name)) : areas;
}

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

// La pièce affichée seule (après un toucher sur le plan), ou `null`.
const view = { room: null };

function renderAll() {
  renderHome();
  const container = $("#sections");
  // Par nom, sauf la météo, la maison et les personnes, en tête de leur section.
  const first = (state) => (state.entity_id === VIGILANCE_TODAY ? 0.5 : lookup({ weather: 0, zone: 1, person: 2 }, domainOf(state)) ?? 3);
  const sorted = [...entities.values()].sort(
    (a, b) => first(a) - first(b) || nameOf(a).localeCompare(nameOf(b), "fr"),
  );
  const section = (title, items) =>
    el("section", {}, el("h2", { text: title }), el("div", { class: "grid" }, ...items.map(renderTile)));

  // Une seule pièce : son nom, un bouton retour, ses appareils.
  const room = view.room === null ? undefined : areas.find((area) => area.name === view.room);
  if (room) {
    const items = room.entities.map((entity) => entities.get(entity.entity_id)).filter((state) => state !== undefined);
    const back = el("button", { type: "button", class: "back", "aria-label": "Revenir à toute la maison" }, svgIcon(ICONS.back));
    back.addEventListener("click", () => showRoom(null));
    container.replaceChildren(
      el("section", {},
        el("div", { class: "room-view-head" }, back, el("h2", { text: room.name })),
        el("div", { class: "grid" }, ...items.map(renderTile)),
      ),
    );
    $("#empty").hidden = true;
    return;
  }

  const used = new Set();
  // D'abord une section par pièce, dans l'ordre du fichier…
  const areaGroups = shownAreas().map((area) => {
    const items = area.entities
      .map((entity) => entities.get(entity.entity_id))
      .filter((state) => state !== undefined);
    items.forEach((s) => used.add(s.entity_id));
    return { title: area.name, items };
  });
  // … puis, par type, les appareils qui ne sont dans aucune pièce. Les scènes
  // et les scripts sont déjà en haut, en boutons.
  // Les notifications ont leur cloche, en haut.
  // Demain est dans la tuile de la vigilance (ou de Tempo) du jour.
  const loose = sorted.filter((s) => !used.has(s.entity_id) && s.entity_id !== VIGILANCE_TOMORROW && s.entity_id !== TEMPO_TOMORROW
    && !["scene", "script", "persistent_notification"].includes(domainOf(s)));
  const groups = SECTIONS.map(({ title, domains, extra }) => {
    const items = loose.filter((s) => !used.has(s.entity_id) && (domains.includes(domainOf(s)) || extra?.(s)));
    items.forEach((s) => used.add(s.entity_id));
    return { title, items };
  });
  groups.push({ title: "Autres", items: loose.filter((s) => !used.has(s.entity_id)) });
  // Un onglet limité à quelques pièces (Garage…) ne montre que leurs appareils.
  if (pageAreas !== null && isBoard()) groups.length = 0;

  container.replaceChildren(
    ...[...areaGroups, ...groups]
      .filter((group) => group.items.length > 0)
      .map(({ title, items }) => section(title, items)),
  );
  $("#empty").hidden = entities.size > 0;
}

function showRoom(name) {
  view.room = name;
  renderAll();
  $("#sections").scrollIntoView({ behavior: matchMedia("(prefers-reduced-motion: reduce)").matches ? "auto" : "smooth", block: "start" });
}

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
