// Tableau de bord — la vue « hub » de la tablette murale (étape 80).
//
// Sur la tablette accrochée au mur, à l'horizontale : le dessin de la maison
// au centre, et autour ce qu'on regarde en passant. En haut, qui est là et
// l'alerte du moment ; à gauche, l'heure, la météo et l'agenda ; à droite,
// les scènes et les pièces ; en bas, le tableau de bord complet, l'agenda,
// l'énergie et une annonce sur les enceintes.
//
// Le dessin n'est pas recopié : comme pour l'écran de veille, c'est le même
// élément (#house-stage), avec ses calques et ses zones, qui est déplacé
// ici, puis remis à sa place.
//
// Chargé après house.js et agenda.js : il utilise leurs fonctions (openRoom,
// selectPage, agendaEvents…). Pas d'innerHTML ici non plus.
"use strict";

const HUB_KEY = "ha-hub";
// Plus étroit que ça (téléphone, tablette à la verticale) : le tableau de
// bord habituel.
const hubWide = matchMedia("(min-width: 900px) and (orientation: landscape)");
const hub = {
  on: false,
  // « Toute la maison » : le tableau de bord complet, jusqu'au retour à
  // l'accueil (2 minutes sans contact).
  away: false,
};
const HUB_ICONS = {
  grid: ["M3 3h7v7H3z", "M14 3h7v7h-7z", "M3 14h7v7H3z", "M14 14h7v7h-7z"],
  bolt: ["M13 2L4 14h7l-1 8 9-12h-7z"],
  warn: ["M12 3l10 18H2z", "M12 10v4", "M12 17.5v.5"],
};
// Une couleur par personne, toujours la même pour un même nom.
const HUB_PEOPLE_COLORS = ["#03a9f4", "#f06292", "#66bb6a", "#ffb74d", "#9575cd", "#4dd0e1"];

function hubWanted() {
  try {
    return localStorage.getItem(HUB_KEY) !== "0";
  } catch {
    return true;
  }
}

function hubShouldShow() {
  return document.body.classList.contains("tablet")
    && $("#hub-mode").checked
    && !$("#dashboard-view").hidden
    && !hub.away
    && hubWide.matches;
}

/** Affiche ou cache la vue hub selon le moment, et la redessine. Appelé à
 *  chaque changement d'état (renderHome), et quand un réglage change. */
function updateHub() {
  const want = hubShouldShow();
  if (want && !hub.on) showHub();
  else if (!want && hub.on) hideHub();
  if (hub.on) renderHub();
}

function showHub() {
  hub.on = true;
  $("#hub").hidden = false;
  document.documentElement.dataset.hub = "on";
  // Le dessin vient au centre (sauf s'il est dans l'écran de veille : il
  // reviendra ici au réveil).
  const stage = $("#house-stage");
  if (!saver.on) $("#hub-house").append(stage);
  if (house.loading === null && token.get()) loadHouse();
}

function hideHub() {
  hub.on = false;
  $("#hub").hidden = true;
  delete document.documentElement.dataset.hub;
  const stage = $("#hub-house #house-stage");
  if (stage) {
    $("#house .house-frame").append(stage);
    if (house.view) fitLabels(house.view);
  }
  // Le tableau de bord n'était plus redessiné sous la vue hub : il se remet à jour.
  scheduleHome();
}

/** Retour à l'accueil (account.js, 2 minutes sans contact) : la vue hub
 *  revient, agenda et musique refermés. */
function hubBack() {
  hub.away = false;
  if (typeof agenda !== "undefined" && agenda.open) closeAgenda();
  if (typeof music !== "undefined" && music.open) closeMusic();
  updateHub();
}

/** Quitte la vue hub pour le tableau de bord complet. */
function leaveHub() {
  hub.away = true;
  updateHub();
}

function renderHub() {
  const now = new Date();
  setText($("#hub-clock"), hourMinute.format(now));
  setText($("#hub-date"), fullDate.format(now));
  const connection = $("#connection");
  swap($("#hub-live"), el("i", { class: connection.classList.contains("live") ? "on" : "" }), connection.textContent);
  renderHubPeople();
  renderHubAlert();
  renderHubGreeting(now);
  renderHubWeather();
  renderHubAgenda(now);
  renderHubViews();
  renderHubScenes();
  renderHubRooms();
  renderHubDock();
  if (house.view) updateHouse(house.view);
}

function personColor(name) {
  let sum = 0;
  for (const char of name) sum = (sum * 31 + char.codePointAt(0)) % 997;
  return HUB_PEOPLE_COLORS[sum % HUB_PEOPLE_COLORS.length];
}

function renderHubPeople() {
  const people = [...entities.values()]
    .filter((s) => domainOf(s) === "person")
    .sort((a, b) => nameOf(a).localeCompare(nameOf(b), "fr"));
  const box = $("#hub-people");
  box.hidden = people.length === 0;
  swap(box, ...people.map((state) => {
    const name = nameOf(state);
    const home = state.state === "home";
    const avatar = el("span", {
      class: home ? "hub-avatar" : "hub-avatar away",
      title: `${name} : ${home ? "à la maison" : "absent"}`,
      text: name.charAt(0).toUpperCase(),
    });
    avatar.style.setProperty("--person", personColor(name));
    return avatar;
  }));
  const pill = $("#hub-home");
  pill.hidden = people.length === 0;
  const here = people.filter((s) => s.state === "home");
  const away = people.filter((s) => s.state !== "home").map(nameOf);
  setText(pill, [
    here.length === 0 ? "Personne à la maison" : `${here.length} à la maison`,
    away.length > 0 && here.length > 0 ? `${joinNames(away)} ${away.length > 1 ? "ne sont pas là" : "n'est pas là"}` : "",
  ].filter(Boolean).join(" · "));
}

/** L'alerte du moment, et de quoi y répondre (fermer le garage). Les ordres
 *  à valider (étape 48) passent en premier. */
function renderHubAlert() {
  const found = alerts();
  const pill = $("#hub-alert");
  const waiting = noticeStates().filter((s) => s.state === "notifying" && s.attributes.kind === "approval").length;
  pill.hidden = found.length === 0 && waiting === 0;
  if (waiting > 0) {
    const open = el("button", { type: "button", class: "hub-alert-action", text: "Voir" });
    open.addEventListener("click", () => $("#open-notices").click());
    swap(pill, svgIcon(HUB_ICONS.warn),
      el("b", { text: waiting === 1 ? "1 ordre attend ta validation" : `${waiting} ordres attendent ta validation` }), open);
    return;
  }
  if (found.length === 0) return;
  const { state, text } = found[0];
  const parts = [svgIcon(HUB_ICONS.warn), el("b", { text: `${nameOf(state)} : ${text}` })];
  if (found.length > 1) parts.push(el("span", { text: ` et ${found.length - 1} de plus` }));
  if (domainOf(state) === "cover" && canControl(state)) {
    const close = el("button", { type: "button", class: "hub-alert-action", text: "Fermer" });
    close.addEventListener("click", () => callService("cover", "close_cover", { entity_id: state.entity_id }));
    parts.push(close);
  }
  swap(pill, ...parts);
}

function renderHubGreeting(now) {
  const hour = now.getHours();
  const hello = hour >= 5 && hour < 18 ? "Bonjour." : "Bonsoir.";
  const found = alerts();
  const soon = typeof agendaSoon === "function" ? agendaSoon() : null;
  const parts = found.length > 1 ? [`${hello} ${found.length} choses demandent ton attention.`]
    : soon ? [`${hello} `, soon, "."]
    : [`${hello} ${found.length === 0 ? "Tout est calme." : ""}`.trim()];
  // Une machine en marche (lave-linge SmartThings, étape 86) : quand elle finit.
  const machines = [...entities.values()]
    .filter((s) => typeof s.attributes.appareil === "string")
    // « a fini » pendant une heure, comme la carte sur le dessin (house.js).
    .filter((s) => s.state === "En marche" || (s.state === "Terminé" && Date.now() - Date.parse(s.last_changed) < MACHINE_DONE_SHOWN))
    .map((s) => (s.state === "Terminé" ? `${nameOf(s)} a fini` : typeof s.attributes.fin_heure === "string" ? `${nameOf(s)} : fin vers ${s.attributes.fin_heure}` : `${nameOf(s)} en marche`));
  // Les colis Amazon (colis.js, étape 87).
  const parcels = typeof parcelGreeting === "function" ? parcelGreeting() : null;
  if (parcels) machines.push(parcels);
  if (machines.length) parts.push(` ${machines.join(" · ")}.`);
  swap($("#hub-greeting"), ...parts);
}

function renderHubWeather() {
  const weather = [...entities.values()].find((s) => domainOf(s) === "weather");
  const card = $("#hub-weather");
  card.hidden = !weather;
  if (!weather) return;
  const a = weather.attributes;
  const label = lookup(WEATHER_LABELS, weather.state)?.[0] ?? "";
  const days = (Array.isArray(a.forecast) ? a.forecast : [])
    .filter((day) => typeof day?.datetime === "string" && !Number.isNaN(Date.parse(day.datetime)));
  const today = days[0];
  const range = today && finite(today.templow) && finite(today.temperature)
    ? `min ${Math.round(today.templow)}° · max ${Math.round(today.temperature)}°` : "";
  swap(card,
    el("div", { class: "hub-weather-now" },
      svgIcon(weatherIcon(weather.state)),
      el("strong", { text: finite(a.temperature) ? `${number.format(a.temperature)}°` : "—" }),
      el("span", {}, el("b", { text: label }), el("small", { text: range }))),
    el("ol", { class: "hub-forecast", "aria-label": "Prévisions" }, ...days.slice(1, 5).map((day) =>
      el("li", {},
        el("span", { text: dayName.format(new Date(`${day.datetime}T12:00`)) }),
        svgIcon(weatherIcon(day.condition)),
        el("b", { text: finite(day.temperature) ? `${Math.round(day.temperature)}°` : "—" })))),
  );
}

/** Ce soir et demain : les prochains rendez-vous, le prochain mis en avant. */
function renderHubAgenda(now) {
  const card = $("#hub-agenda");
  const calendars = typeof agendaCalendars === "function" ? agendaCalendars() : [];
  card.hidden = calendars.length === 0;
  if (calendars.length === 0) return;
  const tomorrow = addDays(dayStart(now), 1);
  // Aujourd'hui d'abord, puis demain ; dans chaque jour, les heures avant les
  // journées entières (on les voit déjà dans l'agenda).
  const day = (e) => Number(e.startDate >= tomorrow);
  const events = hubMergeSame(agendaEvents(now, addDays(tomorrow, 1), calendars))
    .sort((a, b) => day(a) - day(b) || Number(a.allDay) - Number(b.allDay) || a.startDate - b.startDate)
    .slice(0, 5);
  const next = events.find((e) => !e.allDay && e.startDate > now);
  const rows = events.map((e) => {
    const later = e.startDate >= tomorrow;
    // Pas le nom de l'agenda : sa couleur, sur la barre, suffit.
    const sub = [later ? "demain" : "", e.location || ""].filter(Boolean).join(" · ");
    const row = el("button", { type: "button", class: e === next ? "hub-event next" : "hub-event" },
      el("span", { class: "hub-event-bar" }),
      el("span", { class: "hub-event-time", text: e.allDay ? "jour" : hourMinute.format(e.startDate) }),
      el("span", { class: "hub-event-text" }, el("b", { text: e.summary }), el("small", { text: sub })),
      hubEventWhen(e, next, later, now));
    row.style.setProperty("--ev", e.color);
    row.addEventListener("click", () => openEventDetails(e));
    return row;
  });
  swap(card,
    el("h2", {}, "Agenda", el("small", { text: "aujourd'hui et demain" })),
    ...(rows.length > 0 ? rows : [el("p", { class: "hub-empty", text: "Rien de prévu d'ici demain soir." })]),
  );
}

/** « dans 23 min » pour le prochain, « en cours » pour celui qui a commencé. */
function hubEventWhen(e, next, later, now) {
  if (!e.allDay && e.startDate <= now && e.endDate > now) return el("span", { class: "hub-event-when now", text: "en cours" });
  if (e === next && !later) return el("span", { class: "hub-event-when", text: inMinutes(e.startDate) });
  return "";
}

/** Le même rendez-vous dans deux agendas (les vacances dans l'agenda de la
 *  famille et dans celui des vacances scolaires) : une seule ligne. */
function hubMergeSame(events) {
  const seen = new Map();
  const out = [];
  for (const e of events) {
    // Sur deux jours, deux journées entières du même nom sont la même chose,
    // même si l'une commence un jour plus tôt.
    const key = `${e.allDay ? "jour" : e.startDate.getTime()}|${String(e.summary ?? "").trim().toLowerCase()}`;
    if (seen.has(key)) continue;
    seen.set(key, e);
    out.push(e);
  }
  return out;
}

/** « Devant », « Jardin »… : les onglets du dessin. */
function renderHubViews() {
  const box = $("#hub-views");
  box.hidden = house.pages.length < 2;
  swap(box, ...house.pages.map((page) => {
    const current = page.id === house.config?.id;
    const button = el("button", { type: "button", class: current ? "on" : "", "aria-pressed": String(current), text: page.name });
    button.addEventListener("click", () => {
      if (page.id !== house.config?.id) selectPage(page.id);
    });
    return button;
  }));
}

const HUB_SCENE_TONES = ["go", "home", "night", "cine"];
const HUB_SCENES_KEY = "ha-hub-scenes";
const HUB_SCENES_MAX = 6;
// L'icône et la couleur d'une scène, devinées d'après son nom.
const HUB_SCENE_LOOKS = [
  [/nuit|coucher|dodo/i, "moon", "night"],
  [/pars|d[ée]part|absen|quitt|vacances/i, "door", "go"],
  [/cin[ée]|film|t[ée]l[ée]|\btv\b/i, "sofa", "cine"],
  [/matin|r[ée]veil|lever/i, "illuminance", "morning"],
  [/retour|arriv|rentr|bienvenue/i, "home", "home"],
  [/repas|d[îi]ner|d[ée]jeuner|cuisine/i, "pot", "morning"],
  [/lecture|calme|d[ée]tente/i, "bulb", "cine"],
];

function hubSceneLook(state, index) {
  const found = HUB_SCENE_LOOKS.find(([pattern]) => pattern.test(nameOf(state)));
  if (found) return { icon: ICONS[found[1]], tone: found[2] };
  return {
    icon: domainOf(state) === "script" ? ICONS.play : ICONS.sparkle,
    tone: HUB_SCENE_TONES[index % HUB_SCENE_TONES.length],
  };
}

/** Les scènes et scripts qu'on peut lancer depuis ce compte. */
function hubSceneChoices() {
  return [...entities.values()]
    .filter((s) => ["scene", "script"].includes(domainOf(s)) && canControl(s))
    .sort((a, b) => nameOf(a).localeCompare(nameOf(b), "fr"));
}

/** Le choix de cet appareil (Mon compte), ou null s'il n'y en a pas. */
function hubChosenScenes() {
  try {
    const saved = JSON.parse(localStorage.getItem(HUB_SCENES_KEY) ?? "null");
    return Array.isArray(saved) ? saved.filter((id) => typeof id === "string").slice(0, HUB_SCENES_MAX) : null;
  } catch {
    return null;
  }
}

function hubScenes() {
  const choices = hubSceneChoices();
  const chosen = hubChosenScenes();
  const picked = chosen ? choices.filter((s) => chosen.includes(s.entity_id)) : [];
  // Un choix dont plus rien n'existe (scène supprimée) : le choix par défaut.
  return picked.length > 0 ? picked : choices.slice(0, HUB_SCENES_MAX);
}

function renderHubScenes() {
  const list = hubScenes();
  const box = $("#hub-scenes");
  box.hidden = list.length === 0;
  // Pas de case vide : 3 colonnes pour 3, 5 ou 6 scènes, 2 pour 2 ou 4.
  const columns = list.length === 1 ? 1 : list.length % 3 === 0 || list.length === 5 ? 3 : 2;
  box.style.setProperty("--cols", String(columns));
  box.classList.toggle("narrow", columns === 3);
  swap(box, ...list.map((state, index) => {
    const isScript = domainOf(state) === "script";
    const running = isScript && state.state === "on";
    const look = hubSceneLook(state, index);
    const button = el("button", { type: "button", class: `hub-scene t-${look.tone}${running ? " on" : ""}` },
      el("span", { class: "hub-scene-icon" }, svgIcon(running ? ICONS.stop : look.icon)),
      el("b", { text: nameOf(state) }));
    button.addEventListener("click", () => {
      button.classList.add("pending");
      setTimeout(() => button.classList.remove("pending"), 5000);
      callService(domainOf(state), running ? "turn_off" : "turn_on", { entity_id: state.entity_id });
    });
    return button;
  }));
}

function renderHubRooms() {
  const areas = shownAreas();
  const box = $("#hub-rooms");
  box.hidden = areas.length === 0;
  // Plus de 6 pièces : deux colonnes plus serrées, pour tout voir sans faire
  // défiler. La température passe alors sous le nom.
  const compact = areas.length > 6;
  box.classList.toggle("compact", compact);
  swap(box, ...areas.map((area) => {
    const info = roomInfo(area);
    const detail = info.alert ? info.alert
      : info.lights > 0 ? `${info.lights} lumière${info.lights > 1 ? "s" : ""}`
      : info.open > 0 ? `${info.open} ouverture${info.open > 1 ? "s" : ""}`
      : info.heating ? "chauffe" : "";
    const temperature = Number.isFinite(info.temperature) ? `${number.format(info.temperature)}°` : "";
    const classes = ["hub-room"];
    if (info.alert) classes.push("alert");
    else if (info.lights > 0) classes.push("lit");
    const room = el("button", { type: "button", class: classes.join(" "), title: [area.name, detail].filter(Boolean).join(" : ") },
      el("span", { class: "hub-room-icon" }, svgIcon(roomIcon(area.name))),
      el("span", { class: "hub-room-text" }, el("b", { text: area.name }),
        el("small", { text: compact ? [temperature, detail].filter(Boolean).join(" · ") : detail })),
      compact ? "" : el("span", { class: "hub-room-temp", text: temperature }));
    room.addEventListener("click", () => openRoom(area.name));
    return room;
  }));
}

/** La puissance du moment, pour le bouton Énergie : le compteur d'abord. */
function hubPower() {
  const sensors = [...entities.values()].filter((s) => domainOf(s) === "sensor"
    && s.attributes.device_class === "power" && Number.isFinite(Number(s.state)));
  const meter = sensors.find((s) => /linky|compteur|papp|sinsts/.test(s.entity_id)) ?? sensors[0];
  if (!meter) return "";
  const watts = Number(meter.state) * (meter.attributes.unit_of_measurement === "kW" ? 1000 : 1);
  return watts >= 1000 ? `${number.format(Math.round(watts / 100) / 10)} kW` : `${Math.round(watts)} W`;
}

/** Le service d'annonce (étape 73), s'il y a des enceintes et qu'on peut s'en servir. */
function hubAnnouncer() {
  const tts = [...entities.values()].find((s) => domainOf(s) === "tts" && canControl(s));
  if (!tts) return null;
  const speakers = (Array.isArray(tts.attributes.enceintes) ? tts.attributes.enceintes : []).filter((id) => entities.has(id));
  const voice = typeof tts.attributes.voix === "string" ? tts.attributes.voix : "";
  return speakers.length > 0 && voice !== "" && !voice.startsWith("aucune") ? { tts, speakers } : null;
}

function renderHubDock() {
  $("#hub-open-agenda").hidden = $("#hub-agenda").hidden;
  renderHubCourses();
  const energy = $("#hub-energy");
  energy.hidden = $("#open-energy").hidden;
  const power = hubPower();
  setText(energy.lastChild, power || "Énergie");
  $("#hub-announce").hidden = hubAnnouncer() === null;
}

// ---------------------------------------------------------------- Courses
// La liste de courses (étape 81 : celle de l'appli Iglou quand elle est
// reliée), en grand : on coche en rangeant les courses, on ajoute en passant.

const HUB_LIST = "todo.courses";

function renderHubCourses() {
  const state = entities.get(HUB_LIST);
  const button = $("#hub-courses");
  button.hidden = !state;
  if (!state) {
    if ($("#hub-list").open) $("#hub-list").close();
    return;
  }
  const items = todoItems(state);
  const left = items.filter((item) => item.status !== "completed").length;
  setText(button.lastChild, state.state === "unavailable" ? "Courses" : left === 0 ? "Courses" : `Courses · ${left}`);
  if ($("#hub-list").open) renderHubList(state);
}

function renderHubList(state) {
  const items = todoItems(state);
  const iglou = state.attributes.source === "iglou";
  const left = items.filter((item) => item.status !== "completed").length;
  $("#hub-list-title").textContent = left === 0 ? "Courses" : `Courses · ${left} à acheter`;
  $("#hub-list-note").textContent = state.state === "unavailable"
    ? "Iglou ne répond pas : la liste sera relue dans une minute."
    : iglou ? "La liste de l'appli Iglou. Coche ce qui est dans le panier ; on range dans l'appli." : "";
  $("#hub-list-items").replaceChildren(...(items.length > 0
    ? todoRows(state, items)
    : [el("li", { class: "hub-list-empty", text: "Rien à acheter." })]));
}

$("#hub-courses").addEventListener("click", () => {
  const state = entities.get(HUB_LIST);
  if (!state) return;
  renderHubList(state);
  $("#hub-list").showModal();
});
$("#hub-list-close").addEventListener("click", () => $("#hub-list").close());
$("#hub-list-form").addEventListener("submit", (event) => {
  event.preventDefault();
  const input = $("#hub-list-text");
  const item = input.value.trim();
  if (item === "") return;
  callService("todo", "add_item", { entity_id: HUB_LIST, item });
  input.value = "";
});

// ---------------------------------------------------------------- Annonce
// « Annonce à la maison » : une phrase, lue sur toutes les enceintes.

$("#hub-say-form").addEventListener("submit", (event) => {
  event.preventDefault();
  const input = $("#hub-say-text");
  const announcer = hubAnnouncer();
  const message = input.value.trim();
  if (!announcer || message === "") return;
  callService("tts", "speak", {
    entity_id: announcer.tts.entity_id,
    media_player_entity_id: announcer.speakers,
    message,
  });
  input.value = "";
  $("#hub-say").close();
});
$("#hub-say-close").addEventListener("click", () => $("#hub-say").close());

// ---------------------------------------------------------------- Boutons et réglage

function hubButton(id, icon, text) {
  $(id).replaceChildren(svgIcon(icon), el("span", { text }));
}
hubButton("#hub-all", HUB_ICONS.grid, "Toute la maison");
hubButton("#hub-open-agenda", ICONS.calendar, "Agenda");
hubButton("#hub-courses", ICONS.list, "Courses");
hubButton("#hub-energy", HUB_ICONS.bolt, "Énergie");
hubButton("#hub-announce", ICONS.megaphone, "Annonce à la maison");

$("#hub-all").addEventListener("click", leaveHub);
$("#hub-open-agenda").addEventListener("click", () => {
  leaveHub();
  openAgenda();
});
$("#hub-energy").addEventListener("click", () => openEnergy());
$("#hub-announce").addEventListener("click", () => {
  $("#hub-say").showModal();
  $("#hub-say-text").focus();
});

$("#hub-mode").checked = hubWanted();
$("#hub-mode").addEventListener("change", (event) => {
  try {
    if (event.target.checked) localStorage.removeItem(HUB_KEY);
    else localStorage.setItem(HUB_KEY, "0");
  } catch {
    // Stockage bloqué : le choix ne durera que le temps de la page.
  }
  hub.away = false;
  updateHub();
});

// Le choix des scènes, dans Mon compte : une case par scène ou script.
function renderHubPick() {
  const chosen = new Set(hubScenes().map((s) => s.entity_id));
  const full = chosen.size >= HUB_SCENES_MAX;
  const choices = hubSceneChoices();
  $("#hub-pick-list").replaceChildren(...(choices.length === 0
    ? [el("p", { class: "account", text: "Aucune scène ni script pour l'instant." })]
    : choices.map((state) => {
      const box = el("input", { type: "checkbox", value: state.entity_id });
      box.checked = chosen.has(state.entity_id);
      box.disabled = full && !box.checked;
      box.addEventListener("change", saveHubPick);
      return el("label", { class: "check" }, box, ` ${nameOf(state)}`);
    })));
}

function saveHubPick() {
  const ids = [...document.querySelectorAll("#hub-pick-list input:checked")].map((box) => box.value);
  try {
    if (ids.length === 0) localStorage.removeItem(HUB_SCENES_KEY);
    else localStorage.setItem(HUB_SCENES_KEY, JSON.stringify(ids.slice(0, HUB_SCENES_MAX)));
  } catch {
    // Stockage bloqué : rien à garder.
  }
  renderHubPick();
  if (hub.on) renderHubScenes();
}

$("#hub-pick").addEventListener("toggle", () => {
  if ($("#hub-pick").open) renderHubPick();
});
$("#tablet-mode").addEventListener("change", () => {
  hub.away = false;
  updateHub();
});
hubWide.addEventListener("change", updateHub);
updateHub();
