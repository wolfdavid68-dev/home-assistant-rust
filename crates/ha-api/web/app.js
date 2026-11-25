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
// Clair ou sombre, ou comme l'appareil (« Automatique »). Tout le tableau de
// bord suit ce choix, l'accueil v3/v4 aussi (hub-v3.css). Un réglage de cet
// appareil seulement (localStorage), appliqué avant le premier affichage.
// Les anciens styles (« plan », « plan-nuit », « maquette ») ne sont plus
// proposés : un appareil qui les avait gardés repasse en « Automatique ».
const THEME_KEY = "ha-theme";
const THEMES = ["auto", "light", "dark"];
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

// « Tablette murale » (étape 78) : le nom de la tablette n'apparaît que coché.
$("#login-form [name=wall]").addEventListener("change", (event) => {
  $("#wall-label-field").hidden = !event.currentTarget.checked;
});

$("#login-form").addEventListener("submit", async (event) => {
  event.preventDefault();
  const form = event.currentTarget;
  const button = form.querySelector("button");
  button.disabled = true;
  const wall = form.wall.checked;
  try {
    const response = await fetch("/api/auth/login", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        username: form.username.value.trim(),
        password: form.password.value,
        // Le code n'est envoyé que s'il a été saisi.
        ...(form.code.value.trim() && { code: form.code.value.trim() }),
        ...(wall && { wall_label: form.wall_label.value.trim() || "Tablette murale" }),
      }),
    });
    form.password.value = "";
    form.code.value = "";
    if (response.status === 429) return showLogin("Trop de tentatives. Réessaie dans 15 minutes.");
    // Refus propre à la tablette murale (compte administrateur, nom invalide).
    if (response.status === 400 || response.status === 403) {
      const { error } = await response.json().catch(() => ({}));
      return showLogin(typeof error === "string" ? error.charAt(0).toUpperCase() + error.slice(1) : "Connexion refusée.");
    }
    if (!response.ok) return showLogin("Nom d'utilisateur, mot de passe ou code incorrect.");
    token.set((await response.json()).access_token);
    showDashboard();
    // Une tablette murale passe tout de suite en mode tablette (plein écran,
    // écran de veille) : c'est pour ça qu'on l'associe.
    if (wall) setTablet(true, true);
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
  timer: 0,

  open() {
    this.stopped = false;
    clearTimeout(this.timer);
    this.timer = 0;
    // Une seule connexion à la fois : l'ancienne est fermée, et ce qu'elle
    // dirait encore est ignoré (chaque écouteur vérifie qu'il est le bon).
    const old = this.socket;
    this.socket = null;
    old?.close();
    const scheme = location.protocol === "https:" ? "wss" : "ws";
    const socket = new WebSocket(`${scheme}://${location.host}/api/websocket`);
    this.socket = socket;
    setStatus("Connexion…", false);

    socket.addEventListener("message", (event) => {
      if (socket !== this.socket) return;
      const message = JSON.parse(event.data);
      if (message.type === "result" || message.type === "event") link.lastData = Date.now();
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
            // Un colis annoncé pendant que la tablette était déconnectée (colis.js).
            if (typeof parcelChanged === "function") parcelChanged("sensor.colis_amazon", null, entities.get("sensor.colis_amazon"));
            // Les enceintes : le bouton « Musique », et la page si elle est ouverte.
            if (typeof musicLoaded === "function") musicLoaded();
          }
          break;
        case "event":
          if (message.event.event_type === "state_changed") onStateChanged(message.event.data);
          refreshLogbookSoon();
          break;
      }
    });

    socket.addEventListener("close", () => {
      if (socket !== this.socket || this.stopped) return;
      setStatus("Reconnexion…", false);
      clearTimeout(this.timer);
      this.timer = setTimeout(() => {
        this.timer = 0;
        this.open();
      }, this.retry);
      this.retry = Math.min(this.retry * 2, 30000);
    });
  },

  close() {
    this.stopped = true;
    clearTimeout(this.timer);
    this.timer = 0;
    const socket = this.socket;
    this.socket = null;
    socket?.close();
    linkLine(true);
  },
};

function setStatus(text, isLive) {
  const status = $("#connection");
  status.textContent = text;
  status.classList.toggle("live", isLive);
  linkLine(isLive);
}

// La ligne d'état (étape 101) : quand la tablette perd la maison plus de
// quelques secondes, une seule ligne ambre, en haut, dit depuis quand les
// valeurs n'ont plus bougé ; elle part d'elle-même au retour. Pas de fenêtre
// qui s'ouvre : on continue de voir la maison (avec ses dernières valeurs).
const LINK_GRACE_MS = 3_000;
const link = { lastData: 0, timer: 0 };

function linkLine(isLive) {
  const line = document.getElementById("link-line");
  if (!line) return;
  if (isLive) {
    clearTimeout(link.timer);
    link.timer = 0;
    line.hidden = true;
    return;
  }
  // Déjà prévue ou affichée : on garde l'heure de la première coupure.
  if (link.timer || !line.hidden) return;
  link.timer = setTimeout(() => {
    link.timer = 0;
    if (live.stopped) return;
    const since = link.lastData ? ` · dernière donnée à ${hourMinute.format(new Date(link.lastData))}` : "";
    line.textContent = `Connexion perdue avec la maison, nouvel essai…${since}`;
    line.hidden = false;
  }, LINK_GRACE_MS);
}

function onStateChanged({ entity_id, old_state, new_state }) {
  if (new_state) entities.set(entity_id, new_state);
  else entities.delete(entity_id);
  // La sonnette (sonnette.js, étape 82) : la photo de la porte en grand.
  if (typeof doorbellChanged === "function") doorbellChanged(entity_id, old_state, new_state);
  // Les colis Amazon (colis.js, étape 87) : « Plaid va arriver aujourd'hui ».
  if (typeof parcelChanged === "function") parcelChanged(entity_id, old_state, new_state);
  // Le lecteur de musique (musique.js, étape 92) : la page suit l'enceinte.
  if (typeof musicChanged === "function") musicChanged(entity_id, old_state, new_state);
  scheduleHome();
  // La tuile est dans les sections, et peut-être aussi dans la fenêtre d'une
  // pièce (house.js) : on remplace chacune.
  const existing = shownTiles(entity_id);
  if (existing.length > 0 && new_state) updateTiles(entity_id);
  else if (!old_state || !new_state) {
    // Entité ajoutée ou supprimée : on redessine tout. Une entité sans tuile
    // (rangée dans une pièce du tableau, une scène…) n'a rien à remplacer :
    // ses cartes (pièces, volets) suivent avec scheduleHome().
    renderAll();
    refreshEntityList();
  }
}

// ---------------------------------------------------------------- Tuile en main
// Un doigt posé sur une tuile (un curseur qu'on glisse) : elle n'est pas
// remplacée sous lui, mais quand il se lève. Le champ où l'on écrit, lui,
// passe dans la nouvelle tuile avec son texte : une liste de courses se
// met à jour pendant qu'on tape l'article suivant.
const tileHold = { pointer: null, waiting: new Set(), moved: new WeakMap() };
const TILE_FIELDS = "button, input, select, textarea";
const TYPED_INPUT = /^(text|search|number|email|url|tel|password|date|time|datetime-local)$/;

function shownTiles(entityId) {
  return document.querySelectorAll(`:is(.sections, .room-body) #${CSS.escape(tileId(entityId))}`);
}

function updateTiles(entityId) {
  const state = entities.get(entityId);
  if (!state) return;
  for (const tile of shownTiles(entityId)) {
    if (tileHold.pointer && tile.contains(tileHold.pointer)) tileHold.waiting.add(entityId);
    else replaceTile(tile, state);
  }
  // Le résumé de la pièce suit (une lumière allumée, une température).
  for (const summary of document.querySelectorAll("#sections .section-summary[data-room]")) {
    const text = sectionSummary(summary.dataset.room);
    if (summary.textContent !== text) summary.textContent = text;
  }
}

/** Remplace une tuile ; le focus (et le texte en cours) passe au même champ
 *  de la nouvelle. */
function replaceTile(tile, state) {
  const next = renderTile(state);
  if (tile.dataset.room) shortenName(next, tile.dataset.room);
  const focused = tile.contains(document.activeElement) ? document.activeElement : null;
  const index = focused ? [...tile.querySelectorAll(TILE_FIELDS)].indexOf(focused) : -1;
  const twin = index < 0 ? null : next.querySelectorAll(TILE_FIELDS)[index];
  const same = twin && twin.tagName === focused.tagName && twin.type === focused.type;
  const typed = same && (focused.tagName === "TEXTAREA" || TYPED_INPUT.test(focused.type));
  if (typed) {
    twin.value = focused.value;
    tileHold.moved.set(focused, twin);
  }
  tile.replaceWith(next);
  if (!same) return;
  twin.focus({ preventScroll: true });
  if (typed) {
    try {
      twin.setSelectionRange(focused.selectionStart, focused.selectionEnd);
    } catch {
      // Champ sans curseur de texte (nombre, date) : le focus suffit.
    }
  }
}

/** Le champ qui a pris la place de `field` dans une tuile redessinée
 *  (ou `field` lui-même) : c'est lui qu'on vide après un envoi. */
function liveField(field) {
  let node = field;
  while (!node.isConnected && tileHold.moved.has(node)) node = tileHold.moved.get(node);
  return node;
}

addEventListener("pointerdown", (event) => {
  tileHold.pointer = event.target instanceof Element ? event.target : null;
}, { capture: true, passive: true });
for (const type of ["pointerup", "pointercancel"]) {
  addEventListener(type, () => {
    tileHold.pointer = null;
    // Un peu plus tard : le clic et le `change` du même geste visent encore
    // l'ancienne tuile.
    setTimeout(() => {
      if (tileHold.pointer) return;
      const waiting = [...tileHold.waiting];
      tileHold.waiting.clear();
      for (const id of waiting) updateTiles(id);
    }, 50);
  }, { capture: true, passive: true });
}

// ---------------------------------------------------------------- Commandes
/** Envoie un ordre. Renvoie true s'il est parti (ou attend sa validation),
 *  false sinon : la raison est déjà dite à l'écran. */
async function callService(domain, service, data) {
  const response = await authFetch(`/api/services/${domain}/${service}`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(data),
  });
  if (!response) {
    flash("Serveur injoignable : l'ordre n'est pas parti.");
    unmarkPending(data.entity_id);
    return false;
  }
  if (response.ok && response.status !== 202) return true;
  if (response.status === 401) return false;
  const body = await response.json().catch(() => ({}));
  unmarkPending(data.entity_id);
  if (response.status === 403) {
    // Déverrouiller une porte : le serveur redemande le mot de passe.
    if (body.confirm === true) {
      askConfirmation(() => callService(domain, service, data));
      return false;
    }
    flash(typeof body.error === "string" ? `${body.error}.` : "Ton compte ne permet pas cette commande.");
  } else if (response.status === 202) {
    // Pièce protégée, heures calmes : l'ordre attend qu'un membre de la
    // famille le valide.
    flash(typeof body.message === "string" ? `${body.message}.` : "Ordre en attente de validation.");
    return true;
  } else if (response.status === 400) {
    // L'ordre ne convient pas à l'appareil (« consigne entre 7 et 30 ») :
    // le serveur l'a refusé avant de l'envoyer.
    flash(typeof body.error === "string" ? `${body.error}.` : "Commande refusée.");
  } else if ([502, 503, 504].includes(response.status)) {
    // L'appareil a refusé, son intégration est en panne, ou il n'a pas
    // répondu : on le dit, plutôt que de laisser la tuile clignoter.
    flash(typeof body.error === "string" ? `${body.error}.` : "L'appareil n'a pas répondu.");
  } else {
    flash(`La commande a échoué (erreur ${response.status}).`);
  }
  return false;
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
  const response = await authFetch("/api/areas");
  if (!response?.ok) return;
  areas = (await response.json()).areas ?? [];
  customNames.clear();
  for (const area of areas) {
    for (const entity of area.entities) {
      if (typeof entity.name === "string") customNames.set(entity.entity_id, entity.name);
    }
  }
  renderAll();
}

/** Les appareils rangés dans aucune pièce : par type sur « Toute la maison »,
 *  ou dans l'onglet « Autres » du tableau (house.js). Les scènes et les
 *  scripts sont déjà en boutons, les notifications ont leur cloche, et demain
 *  est dans la tuile de la vigilance (ou de Tempo) du jour. */
function looseStates(states = [...entities.values()]) {
  const inArea = new Set(areas.flatMap((area) => area.entities.map((entity) => entity.entity_id)));
  return states.filter((s) => !inArea.has(s.entity_id) && s.entity_id !== VIGILANCE_TOMORROW && s.entity_id !== TEMPO_TOMORROW
    && !["scene", "script", "persistent_notification"].includes(domainOf(s)));
}

// La pièce affichée seule (après un toucher sur le plan), ou `null`.
const view = { room: null };

// Les tuiles d'une section, en grille.
const TILE_GRID = "tile-grid grid [grid-template-columns:repeat(auto-fill,_minmax(min(100%,_200px),_1fr))] [align-items:start] gap-[12px] [@media(min-width:1000px)]:[:not(:where(.o079300))[data-layout=board]_&]:[grid-template-columns:repeat(auto-fill,_minmax(260px,_1fr))] [:not(:where(.o102200)).room-body_&]:[grid-template-columns:1fr] [.room-body_&>*]:[grid-column:1_/_-1] [@media(min-width:1000px)]:[.room-view_.room-body_&>*]:[break-inside:avoid]";
// Sur téléphone, les tuiles d'une section forment une seule carte : une
// ligne par appareil, séparées d'un filet (au lieu d'une carte chacune).
const PHONE_LIST = "phone-list max-sm:gap-0 max-sm:bg-card max-sm:rounded-carte max-sm:shadow-carte max-sm:overflow-hidden max-sm:[&>.tile]:rounded-none max-sm:[&>.tile]:shadow-none max-sm:[&>.tile]:bg-transparent max-sm:[&>.tile]:px-[14px] max-sm:[&>.tile]:py-[11px] max-sm:[&>.tile]:gap-[8px] max-sm:[&>.tile+.tile]:border-t max-sm:[&>.tile+.tile]:border-line max-sm:[&>.tile.light.on]:text-text max-sm:[&>.tile>input[type=range]]:h-[18px] max-sm:[&>.tile>input[type=range]]:ml-[50px] max-sm:[&>.tile>input[type=range]]:w-[calc(100%-50px)] max-sm:[&>.tile.sensor]:grid-cols-[minmax(0,1fr)_auto] max-sm:[&>.tile.sensor]:items-center max-sm:[&>.tile.sensor>.tile-head]:pr-0 max-sm:[&>.tile.sensor>.value]:text-titre max-sm:[&>.tile.sensor>.value]:mr-[2rem] max-sm:[&>.tile.sensor>:not(.tile-head,.value,.history-button)]:col-span-full max-sm:[&_.history-button]:top-[7px] max-sm:[&_.history-button]:right-[6px] max-sm:[&_.history-button]:opacity-70";

/** Dans la section d'une pièce, « Salon lampe 1 » devient « Lampe 1 » et
 *  « Température Salon », « Température » : le titre dit déjà la pièce. */
function shortenName(tile, room) {
  tile.dataset.room = room;
  const name = tile.querySelector(".tile-head .name");
  if (!name) return tile;
  const full = name.textContent;
  const lower = full.toLocaleLowerCase("fr");
  const key = room.toLocaleLowerCase("fr");
  let short = full;
  if (lower.startsWith(`${key} `)) short = full.slice(room.length + 1);
  else if (lower.endsWith(` ${key}`)) short = full.slice(0, -room.length - 1);
  short = short.trim();
  if (short && short !== full) name.textContent = short.charAt(0).toLocaleUpperCase("fr") + short.slice(1);
  return tile;
}

// Les sections repliées, retenues par cet appareil. Sur téléphone, elles
// sont repliées d'office (la carte « La maison » montre déjà les pièces) :
// on retient alors celles qu'on a ouvertes.
const FOLDED_KEY = "ha-sections-pliees";
const OPENED_KEY = "ha-sections-ouvertes";
const PHONE = window.matchMedia("(max-width: 639px)");
PHONE.addEventListener("change", () => renderAll());
function storedSet(key) {
  try {
    const list = JSON.parse(localStorage.getItem(key) ?? "[]");
    return new Set(Array.isArray(list) ? list.filter((name) => typeof name === "string") : []);
  } catch {
    return new Set();
  }
}
function isFolded(title) {
  return PHONE.matches ? !storedSet(OPENED_KEY).has(title) : storedSet(FOLDED_KEY).has(title);
}
function setFolded(title, fold) {
  const key = PHONE.matches ? OPENED_KEY : FOLDED_KEY;
  const set = storedSet(key);
  if (fold === !PHONE.matches) set.add(title);
  else set.delete(title);
  try { localStorage.setItem(key, JSON.stringify([...set])); } catch { /* stockage plein ou interdit */ }
}

/** Le résumé d'une pièce à droite de son titre : « 20,6° · 3 allumées ». */
function sectionSummary(title) {
  const area = areas.find((a) => a.name === title);
  if (!area) return "";
  const info = roomInfo(area);
  return [
    Number.isFinite(info.temperature) ? `${number.format(info.temperature)}°` : "",
    info.lights > 0 ? `${info.lights} allumée${info.lights > 1 ? "s" : ""}` : "",
    info.open > 0 ? `${info.open} ouverte${info.open > 1 ? "s" : ""}` : "",
  ].filter(Boolean).join(" · ");
}

/** Le titre d'une section : un bouton qui la replie ou la déplie. Une pièce
 *  repliée garde son résumé (température, lumières allumées). */
function sectionHead(title, grid, room) {
  const folded = isFolded(title);
  grid.hidden = folded;
  const summary = el("span", { class: "section-summary ml-auto shrink-0 normal-case tracking-normal font-medium text-petit tabular-nums", text: room ? sectionSummary(title) : "" });
  if (room) summary.dataset.room = title;
  const chevron = svgIcon(ICONS.down);
  chevron.setAttribute("class", `size-4 shrink-0 transition-transform ${folded ? "-rotate-90" : ""}`);
  const button = el("button", {
    type: "button", "aria-expanded": String(!folded),
    class: "section-toggle [:where(&)]:[all:unset] flex w-full min-h-6 max-sm:min-h-10 items-center gap-2 cursor-pointer rounded-md hover:text-text focus-visible:outline-3 focus-visible:outline-accent focus-visible:outline-offset-2",
  }, el("span", { class: "min-w-0 truncate", text: title }), summary, chevron);
  button.addEventListener("click", () => {
    const fold = !grid.hidden;
    setFolded(title, fold);
    grid.hidden = fold;
    button.setAttribute("aria-expanded", String(!fold));
    chevron.classList.toggle("-rotate-90", fold);
  });
  return el("h2", {}, button);
}

function renderAll() {
  // Tout redessiner : les panneaux du tableau aussi, même sans changement.
  forgetPanels();
  renderHome();
  const container = $("#sections");
  // Par nom, sauf la météo, la maison et les personnes, en tête de leur section.
  const first = (state) => (state.entity_id === VIGILANCE_TODAY ? 0.5 : lookup({ weather: 0, zone: 1, person: 2 }, domainOf(state)) ?? 3);
  const sorted = [...entities.values()].sort(
    (a, b) => first(a) - first(b) || nameOf(a).localeCompare(nameOf(b), "fr"),
  );
  const section = (title, items, room) => {
    const grid = el("div", { class: `${TILE_GRID} ${PHONE_LIST}` },
      ...items.map((state) => (room ? shortenName(renderTile(state), title) : renderTile(state))));
    return el("section", {}, sectionHead(title, grid, room), grid);
  };

  // Une seule pièce : son nom, un bouton retour, ses appareils.
  const room = view.room === null ? undefined : areas.find((area) => area.name === view.room);
  if (room) {
    const items = room.entities.map((entity) => entities.get(entity.entity_id)).filter((state) => state !== undefined);
    const back = el("button", { type: "button", class: "back", "aria-label": "Revenir à toute la maison" }, svgIcon(ICONS.back));
    back.addEventListener("click", () => showRoom(null));
    container.replaceChildren(
      el("section", {},
        el("div", { class: "room-view-head flex items-center gap-3 [:not(:where(.o008200))&_h2]:[font-family:var(--font-display)] [:not(:where(.o008201))&_h2]:text-titre-l [:not(:where(.o008202))&_h2]:font-semibold [:not(:where(.o008203))&_h2]:[letter-spacing:-0.02em] [:not(:where(.o008204))&_h2]:[text-transform:none] [:not(:where(.o008205))&_h2]:text-text" }, back, el("h2", { text: room.name })),
        el("div", { class: `${TILE_GRID} ${PHONE_LIST}` }, ...items.map((state) => shortenName(renderTile(state), room.name))),
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
    return { title: area.name, items, room: true };
  });
  // … puis, par type, les appareils qui ne sont dans aucune pièce.
  const loose = looseStates(sorted).filter((s) => !used.has(s.entity_id));
  const groups = SECTIONS.map(({ title, domains, extra }) => {
    const items = loose.filter((s) => !used.has(s.entity_id) && (domains.includes(domainOf(s)) || extra?.(s)));
    items.forEach((s) => used.add(s.entity_id));
    return { title, items };
  });
  groups.push({ title: "Autres", items: loose.filter((s) => !used.has(s.entity_id)) });
  // Un onglet limité à quelques pièces (Garage…) ne montre que leurs appareils.
  if (pageAreas !== null && isBoard()) groups.length = 0;

  // Mise en page « tableau » : chaque pièce a son onglet, et les appareils
  // sans pièce sont dans l'onglet « Autres » (étape 90). L'onglet de la
  // maison ne garde que l'essentiel, sur un seul écran.
  if (isBoard()) {
    areaGroups.length = 0;
    groups.length = 0;
  }

  container.replaceChildren(
    ...[...areaGroups, ...groups]
      .filter((group) => group.items.length > 0)
      .map(({ title, items, room }) => section(title, items, room)),
  );
  $("#empty").hidden = entities.size > 0;
}

function showRoom(name) {
  view.room = name;
  renderAll();
  $("#sections").scrollIntoView({ behavior: matchMedia("(prefers-reduced-motion: reduce)").matches ? "auto" : "smooth", block: "start" });
}

/** Le jeton n'est plus bon (expiré, révoqué) : retour à la connexion. */
function sessionExpired() {
  for (const dialog of document.querySelectorAll("dialog[open]")) dialog.close();
  token.clear();
  live.close();
  showLogin("Session expirée : reconnecte-toi.");
}

/** `fetch` avec le jeton. Renvoie la réponse, ou `null` si le serveur est
 *  injoignable. Un 401 ramène à la connexion (la réponse est quand même
 *  rendue : pas `ok`, l'appelant s'arrête là). */
async function authFetch(url, options = {}) {
  let response;
  try {
    response = await fetch(url, { ...options, headers: { ...options.headers, Authorization: `Bearer ${token.get()}` } });
  } catch {
    return null;
  }
  if (response.status === 401) sessionExpired();
  return response;
}

/** Appel à l'API : renvoie { status, data }. */
async function api(method, url, body) {
  const response = await authFetch(url, {
    method,
    ...(body !== undefined && { headers: { "Content-Type": "application/json" }, body: JSON.stringify(body) }),
  });
  if (!response) throw new Error("serveur injoignable");
  if (response.status === 401) throw new Error("session expirée");
  const data = await response.json().catch(() => ({}));
  return { status: response.status, data };
}
