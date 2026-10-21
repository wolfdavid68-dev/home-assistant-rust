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
}

/** Retour à l'accueil (account.js, 2 minutes sans contact) : la vue hub
 *  revient, agenda refermé. */
function hubBack() {
  hub.away = false;
  if (typeof agenda !== "undefined" && agenda.open) closeAgenda();
  updateHub();
}

/** Quitte la vue hub pour le tableau de bord complet. */
function leaveHub() {
  hub.away = true;
  updateHub();
}

function renderHub() {
  const now = new Date();
  $("#hub-clock").textContent = hourMinute.format(now);
  $("#hub-date").textContent = fullDate.format(now);
  const connection = $("#connection");
  $("#hub-live").replaceChildren(el("i", { class: connection.classList.contains("live") ? "on" : "" }), connection.textContent);
  renderHubPeople();
  renderHubAlert();
  renderHubGreeting(now);
  renderHubWeather();
  renderHubAgenda(now);
  renderHubViews();
  renderHubScenes();
  renderHubRooms();
  renderHubDock();
  if (house.view) {
    updateHouse(house.view);
    fitLabels(house.view);
  }
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
  box.replaceChildren(...people.map((state) => {
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
  pill.textContent = [
    here.length === 0 ? "Personne à la maison" : `${here.length} à la maison`,
    away.length > 0 && here.length > 0 ? `${joinNames(away)} ${away.length > 1 ? "ne sont pas là" : "n'est pas là"}` : "",
  ].filter(Boolean).join(" · ");
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
    pill.replaceChildren(svgIcon(HUB_ICONS.warn),
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
  pill.replaceChildren(...parts);
}

function renderHubGreeting(now) {
  const hour = now.getHours();
  const hello = hour >= 5 && hour < 18 ? "Bonjour." : "Bonsoir.";
  const found = alerts();
  const soon = typeof agendaSoon === "function" ? agendaSoon() : null;
  const line = $("#hub-greeting");
  if (found.length > 1) line.replaceChildren(`${hello} ${found.length} choses demandent ton attention.`);
  else if (soon) line.replaceChildren(`${hello} `, soon, ".");
  else line.replaceChildren(`${hello} ${found.length === 0 ? "Tout est calme." : ""}`.trim());
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
  card.replaceChildren(
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
  const events = agendaEvents(now, addDays(tomorrow, 1), calendars)
    .sort((a, b) => day(a) - day(b) || Number(a.allDay) - Number(b.allDay) || a.startDate - b.startDate)
    .slice(0, 5);
  const next = events.find((e) => !e.allDay && e.startDate > now);
  const rows = events.map((e) => {
    const later = e.startDate >= tomorrow;
    const sub = [later ? "demain" : "", e.location || "", e.calendar.name].filter(Boolean).join(" · ");
    const row = el("button", { type: "button", class: e === next ? "hub-event next" : "hub-event" },
      el("span", { class: "hub-event-bar" }),
      el("span", { class: "hub-event-time", text: e.allDay ? "jour" : hourMinute.format(e.startDate) }),
      el("span", { class: "hub-event-text" }, el("b", { text: e.summary }), el("small", { text: sub })),
      e === next && !later ? el("span", { class: "hub-event-when", text: inMinutes(e.startDate) }) : "");
    row.style.setProperty("--ev", e.color);
    row.addEventListener("click", () => openEventDetails(e));
    return row;
  });
  card.replaceChildren(
    el("h2", {}, "Agenda", el("small", { text: "aujourd'hui et demain" })),
    ...(rows.length > 0 ? rows : [el("p", { class: "hub-empty", text: "Rien de prévu d'ici demain soir." })]),
  );
}

/** « Devant », « Jardin »… : les onglets du dessin. */
function renderHubViews() {
  const box = $("#hub-views");
  box.hidden = house.pages.length < 2;
  box.replaceChildren(...house.pages.map((page) => {
    const current = page.id === house.config?.id;
    const button = el("button", { type: "button", class: current ? "on" : "", "aria-pressed": String(current), text: page.name });
    button.addEventListener("click", () => {
      if (page.id !== house.config?.id) selectPage(page.id);
    });
    return button;
  }));
}

const HUB_SCENE_TONES = ["go", "home", "night", "cine"];

function renderHubScenes() {
  const list = [...entities.values()]
    .filter((s) => ["scene", "script"].includes(domainOf(s)) && canControl(s))
    .sort((a, b) => nameOf(a).localeCompare(nameOf(b), "fr"))
    .slice(0, 6);
  const box = $("#hub-scenes");
  box.hidden = list.length === 0;
  box.replaceChildren(...list.map((state, index) => {
    const isScript = domainOf(state) === "script";
    const running = isScript && state.state === "on";
    const button = el("button", { type: "button", class: `hub-scene t-${HUB_SCENE_TONES[index % HUB_SCENE_TONES.length]}${running ? " on" : ""}` },
      el("span", { class: "hub-scene-icon" }, svgIcon(isScript ? (running ? ICONS.stop : ICONS.play) : ICONS.sparkle)),
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
  box.replaceChildren(...areas.map((area) => {
    const info = roomInfo(area);
    const detail = info.alert ? info.alert
      : info.lights > 0 ? `${info.lights} lumière${info.lights > 1 ? "s" : ""}`
      : info.open > 0 ? `${info.open} ouverture${info.open > 1 ? "s" : ""}`
      : info.heating ? "chauffe" : "";
    const classes = ["hub-room"];
    if (info.alert) classes.push("alert");
    else if (info.lights > 0) classes.push("lit");
    const room = el("button", { type: "button", class: classes.join(" ") },
      el("span", { class: "hub-room-icon" }, svgIcon(roomIcon(area.name))),
      el("span", { class: "hub-room-text" }, el("b", { text: area.name }), el("small", { text: detail })),
      el("span", { class: "hub-room-temp", text: Number.isFinite(info.temperature) ? `${number.format(info.temperature)}°` : "" }));
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
  const energy = $("#hub-energy");
  energy.hidden = $("#open-energy").hidden;
  const power = hubPower();
  energy.lastChild.textContent = power || "Énergie";
  $("#hub-announce").hidden = hubAnnouncer() === null;
}

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
$("#tablet-mode").addEventListener("change", () => {
  hub.away = false;
  updateHub();
});
hubWide.addEventListener("change", updateHub);
updateHub();
