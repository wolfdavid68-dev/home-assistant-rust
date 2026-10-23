// L'agenda, façon Google Agenda (étape 75) : tous les agendas réunis
// (TimeTree, la maison, les vacances, la collecte), en semaine, en mois, en
// jour ou en liste ; un rendez-vous s'ouvre avec son lieu, son étiquette et
// ses participants. Sur la page Maison : aujourd'hui, la semaine, et le
// prochain rendez-vous.
//
// Les rendez-vous viennent des entités `calendar.*` (attribut `events` :
// la semaine passée et les deux mois à venir). Tout est construit avec
// `el()` et `textContent`, jamais en HTML.

"use strict";

const AGENDA_HIDDEN_KEY = "ha-agenda-caches";
const AGENDA_VIEW_KEY = "ha-agenda-vue";
const AGENDA_PALETTE = ["#4caf50", "#b05dd6", "#ff9800", "#e91e63", "#00bcd4", "#9e9e9e", "#ffc107", "#795548"];
const AGENDA_HOME_COLOR = "#03a9f4";
const DAY_MS = 86_400_000;
const agendaMonthTitle = new Intl.DateTimeFormat("fr", { month: "long", year: "numeric" });
const agendaDayTitle = new Intl.DateTimeFormat("fr", { weekday: "long", day: "numeric", month: "long" });
const agendaWeekday = new Intl.DateTimeFormat("fr", { weekday: "short" });
const agendaLongDay = new Intl.DateTimeFormat("fr", { weekday: "long", day: "numeric", month: "long" });
const AGENDA_VIEWS = [["day", "Jour"], ["week", "Semaine"], ["month", "Mois"], ["list", "Liste"]];

const agenda = {
  open: false,
  view: readStored(AGENDA_VIEW_KEY) ?? (matchMedia("(max-width: 899px)").matches ? "list" : "week"),
  anchor: dayStart(new Date()),
  hidden: new Set(readHidden()),
  // Remettre la vue semaine à l'heure du moment (sinon : garder le défilement).
};
if (!AGENDA_VIEWS.some(([id]) => id === agenda.view)) agenda.view = "week";

function readHidden() {
  try {
    const list = JSON.parse(readStored(AGENDA_HIDDEN_KEY) ?? "[]");
    return Array.isArray(list) ? list.filter((id) => typeof id === "string") : [];
  } catch {
    return [];
  }
}

function readStored(key) {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}

function store(key, value) {
  try {
    localStorage.setItem(key, value);
  } catch {
    // Stockage bloqué : le choix ne sera pas retenu.
  }
}

// ---------------------------------------------------------------- Dates

function dayStart(date) {
  return new Date(date.getFullYear(), date.getMonth(), date.getDate());
}

function addDays(date, days) {
  return new Date(date.getFullYear(), date.getMonth(), date.getDate() + days);
}

/** Le lundi de la semaine. */
function weekStart(date) {
  const day = (date.getDay() + 6) % 7;
  return addDays(dayStart(date), -day);
}

const sameDay = (a, b) => dayStart(a).getTime() === dayStart(b).getTime();
const hoursOf = (date) => date.getHours() + date.getMinutes() / 60;

function inMinutes(date) {
  const minutes = Math.round((date.getTime() - Date.now()) / 60_000);
  if (minutes < 60) return `dans ${Math.max(1, minutes)} min`;
  const hours = Math.floor(minutes / 60);
  const rest = minutes % 60;
  return `dans ${hours} h${rest > 0 ? ` ${String(rest).padStart(2, "0")}` : ""}`;
}

// ---------------------------------------------------------------- Données

const isHexColor = (value) => typeof value === "string" && /^#[0-9a-f]{6}$/i.test(value);

/** Les agendas visibles, chacun avec sa couleur. */
function agendaCalendars() {
  const list = [...entities.values()]
    .filter((s) => domainOf(s) === "calendar")
    .sort((a, b) => (a.entity_id === "calendar.maison" ? -1 : b.entity_id === "calendar.maison" ? 1 : nameOf(a).localeCompare(nameOf(b), "fr")));
  let next = 0;
  return list.map((state) => {
    let color = state.attributes.couleur;
    if (!isHexColor(color)) color = state.entity_id === "calendar.maison" ? AGENDA_HOME_COLOR : AGENDA_PALETTE[next++ % AGENDA_PALETTE.length];
    const source = typeof state.attributes.source === "string" ? state.attributes.source : "";
    return { state, id: state.entity_id, name: nameOf(state), color, source, readOnly: state.attributes.read_only === true };
  });
}

/** Les rendez-vous entre `from` et `to` (exclu), des agendas affichés. */
function agendaEvents(from, to, calendars = agendaCalendars()) {
  const out = [];
  for (const calendar of calendars) {
    if (agenda.hidden.has(calendar.id)) continue;
    for (const event of calendarEvents(calendar.state)) {
      // Une journée entière finit à minuit le lendemain (exclu).
      const end = event.endDate.getTime() > event.startDate.getTime() ? event.endDate : new Date(event.startDate.getTime() + 1);
      if (event.startDate >= to || end <= from) continue;
      out.push({
        ...event,
        endDate: end,
        calendar,
        color: isHexColor(event.color) ? event.color : calendar.color,
        allDay: event.all_day === true || end.getTime() - event.startDate.getTime() >= DAY_MS,
      });
    }
  }
  return out.sort((a, b) => a.startDate - b.startDate || b.endDate - a.endDate);
}

/** Les rendez-vous d'un jour : d'abord les journées entières. */
function eventsOfDay(day, calendars) {
  return agendaEvents(day, addDays(day, 1), calendars).sort((a, b) => Number(b.allDay) - Number(a.allDay) || a.startDate - b.startDate);
}

function tint(node, color, alpha = 24) {
  node.style.setProperty("--ev", color);
  node.style.setProperty("--ev-alpha", `${alpha}%`);
}

function colorDot(color, cls = "") {
  const dot = el("span", { class: `agenda-dot ${cls}`.trim(), "aria-hidden": "true" });
  dot.style.setProperty("--ev", color);
  return dot;
}

const eventTime = (e) => (e.allDay ? "Journée" : hourMinute.format(e.startDate));
const eventSpan = (e) => (e.allDay ? "Toute la journée" : `${hourMinute.format(e.startDate)} – ${hourMinute.format(e.endDate)}`);

// ---------------------------------------------------------------- Ouvrir, fermer

function openAgenda(date, view) {
  if (date) agenda.anchor = dayStart(date);
  if (view) setAgendaView(view, false);
  // Une pièce ouverte dans son onglet (house.js) : on la referme.
  if (typeof closeRoomTab === "function") closeRoomTab(false);
  // La page Musique (musique.js) aussi.
  if (typeof music !== "undefined" && music.open) closeMusic(false);
  agenda.open = true;
  $(".layout").hidden = true;
  $("#empty").hidden = true;
  $("#agenda-view").hidden = false;
  document.documentElement.dataset.agenda = "open";
  agendaNav();
  renderAgendaView();
  window.scrollTo({ top: 0 });
}

function closeAgenda(render = true) {
  if (!agenda.open) return;
  agenda.open = false;
  $("#agenda-view").hidden = true;
  $(".layout").hidden = false;
  delete document.documentElement.dataset.agenda;
  agendaNav();
  if (render) renderAll();
}

/** Le bouton du rail (ou de la barre du bas) et l'onglet du tableau. */
function agendaNav() {
  const any = agendaCalendars().length > 0;
  const dock = $("#dock-agenda");
  if (dock.hidden === any) {
    dock.hidden = !any;
    const visible = $(".dock").querySelectorAll(":scope > :not([hidden])").length;
    $(".dock").style.gridTemplateColumns = `repeat(${visible}, minmax(0, 1fr))`;
  }
  $("#open-agenda").hidden = !any;
  dock.classList.toggle("active", agenda.open);
  $("#dock-home").classList.toggle("active", !agenda.open && !(typeof roomTab !== "undefined" && roomTab.open) && !(typeof music !== "undefined" && music.open));
  if (typeof renderTabs === "function" && typeof house !== "undefined" && house.pages?.length) renderTabs();
}

function setAgendaView(view, render = true) {
  agenda.view = view;
  store(AGENDA_VIEW_KEY, view);
  if (render) renderAgendaView();
}

$("#dock-agenda").addEventListener("click", () => openAgenda());
$("#open-agenda").addEventListener("click", () => openAgenda());
$("#dock-home").addEventListener("click", () => closeAgenda());
$("#agenda-today-button").addEventListener("click", () => {
  agenda.anchor = dayStart(new Date());
  renderAgendaView();
});
for (const [id, step] of [["#agenda-prev", -1], ["#agenda-next", 1]]) {
  $(id).addEventListener("click", () => {
    const a = agenda.anchor;
    agenda.anchor = agenda.view === "month" ? new Date(a.getFullYear(), a.getMonth() + step, 1)
      : addDays(a, step * (agenda.view === "day" ? 1 : 7));
    renderAgendaView();
  });
}

// ---------------------------------------------------------------- La vue

/** Appelé par app.js à chaque rafraîchissement. */
function renderAgendaView() {
  agendaNav();
  if (!agenda.open) return;
  const calendars = agendaCalendars();

  $("#agenda-views").replaceChildren(...AGENDA_VIEWS.map(([id, label]) => {
    const button = el("button", { type: "button", "aria-pressed": String(agenda.view === id), text: label });
    button.addEventListener("click", () => setAgendaView(id));
    return button;
  }));
  const home = entities.get("calendar.maison");
  $("#agenda-add").hidden = !home || (typeof canControl === "function" && !canControl(home));

  const a = agenda.anchor;
  const title = agenda.view === "day" ? agendaDayTitle.format(a)
    : agenda.view === "week" ? weekTitle(weekStart(a))
      : agendaMonthTitle.format(agenda.view === "month" ? new Date(a.getFullYear(), a.getMonth(), 1) : a);
  $("#agenda-title").textContent = title;

  $("#agenda-today").replaceChildren(...todayCard(calendars, false));
  $("#agenda-mini").replaceChildren(...miniMonth());
  $("#agenda-calendars").replaceChildren(...calendarToggles(calendars));

  const body = agenda.view === "month" ? monthView(calendars)
    : agenda.view === "list" ? listView(calendars)
      : weekView(calendars, agenda.view === "day" ? [dayStart(a)] : [...Array(7)].map((_, i) => addDays(weekStart(a), i)));
  $("#agenda-body").replaceChildren(body);
}

function weekTitle(monday) {
  const sunday = addDays(monday, 6);
  if (monday.getMonth() === sunday.getMonth()) return agendaMonthTitle.format(monday);
  const short = new Intl.DateTimeFormat("fr", { month: "short" });
  return `${short.format(monday)} – ${short.format(sunday)} ${sunday.getFullYear()}`;
}

// ----- Semaine et jour

function weekView(calendars, days) {
  const from = days[0];
  const to = addDays(days.at(-1), 1);
  const events = agendaEvents(from, to, calendars);
  const bands = events.filter((e) => e.allDay);
  const timed = events.filter((e) => !e.allDay);
  const now = new Date();

  // Les heures montrées : 8 h à 21 h, élargies si un rendez-vous déborde.
  let first = 8;
  let last = 21;
  for (const e of timed) {
    first = Math.min(first, Math.floor(hoursOf(e.startDate)));
    last = Math.max(last, sameDay(e.startDate, e.endDate) ? Math.ceil(hoursOf(e.endDate)) : 24);
  }
  first = Math.max(0, Math.min(first, 23));
  last = Math.min(24, Math.max(last, first + 1));

  const grid = el("div", { class: "agenda-week" });
  grid.style.setProperty("--days", String(days.length));

  // L'en-tête : les jours.
  const head = el("div", { class: "agenda-days" }, el("span"));
  for (const day of days) {
    const cell = el("button", { type: "button", class: sameDay(day, now) ? "agenda-day is-today" : "agenda-day" },
      el("span", { text: agendaWeekday.format(day) }), el("b", { text: String(day.getDate()) }));
    cell.addEventListener("click", () => openAgenda(day, "day"));
    head.append(cell);
  }

  // Les journées entières, en bandeaux, sur une ou plusieurs lignes.
  const lanes = [];
  const bandBox = el("div", { class: "agenda-bands" }, el("span", { class: "agenda-bands-label", text: "journée" }));
  for (const e of bands) {
    const start = Math.max(0, Math.round((dayStart(e.startDate) - from) / DAY_MS));
    const end = Math.min(days.length - 1, Math.round((dayStart(new Date(e.endDate.getTime() - 1)) - from) / DAY_MS));
    let lane = lanes.findIndex((busy) => busy < start);
    if (lane === -1) {
      lane = lanes.length;
      lanes.push(-1);
    }
    lanes[lane] = end;
    const band = eventButton(e, "agenda-band", e.summary);
    band.style.setProperty("--col", String(start + 2));
    band.style.setProperty("--span", String(end - start + 1));
    band.style.setProperty("--lane", String(lane + 1));
    bandBox.append(band);
  }
  bandBox.style.setProperty("--lanes", String(Math.max(1, lanes.length)));

  // Les heures, et les rendez-vous de chaque jour, côte à côte s'ils se
  // chevauchent. Toutes les heures tiennent dans la hauteur de l'écran, sans
  // défiler (étape 90) : les positions sont en pourcentage de la grille.
  const span = last - first;
  const at = (h) => `${(100 * (h - first)) / span}%`;
  // Une heure, 30 pixels : la semaine tient sans défiler, sans prendre
  // tout l'écran.
  const hourPx = 30;
  const hours = el("div", { class: "agenda-hours" });
  for (let h = first + 1; h < last; h++) {
    const mark = el("span", { text: `${String(h).padStart(2, "0")}:00` });
    mark.style.setProperty("top", at(h));
    hours.append(mark);
  }
  const columns = el("div", { class: "agenda-columns" }, hours);
  columns.style.setProperty("--hours", String(span));
  for (const day of days) {
    const column = el("div", { class: sameDay(day, now) ? "agenda-column is-today" : "agenda-column" });
    for (let h = first + 1; h < last; h++) {
      const line = el("span", { class: "agenda-hour-line" });
      line.style.setProperty("top", at(h));
      column.append(line);
    }
    const mine = timed.filter((e) => e.startDate < addDays(day, 1) && e.endDate > day);
    for (const { event, col, cols } of packColumns(mine, day)) {
      const startH = Math.max(first, sameDay(event.startDate, day) ? hoursOf(event.startDate) : first);
      const endH = Math.min(last, sameDay(event.endDate, day) ? hoursOf(event.endDate) : last);
      const height = (endH - startH) * hourPx;
      const block = eventButton(event, "agenda-block", event.summary);
      block.style.setProperty("top", at(startH));
      block.style.setProperty("height", `max(calc(${(100 * (endH - startH)) / span}% - 2px), 18px)`);
      block.style.setProperty("left", `calc(${(100 * col) / cols}% + 2px)`);
      block.style.setProperty("width", `calc(${100 / cols}% - 4px)`);
      if (height > 34) block.append(el("small", { text: `${hourMinute.format(event.startDate)} – ${hourMinute.format(event.endDate)}` }));
      if (height > 58 && event.location) block.append(el("small", { text: event.location }));
      column.append(block);
    }
    if (sameDay(day, now) && hoursOf(now) >= first && hoursOf(now) <= last) {
      const line = el("span", { class: "agenda-now" });
      line.style.setProperty("top", at(hoursOf(now)));
      column.append(line);
    }
    columns.append(column);
  }
  grid.append(head, bandBox, columns);
  return grid;
}

/** Les rendez-vous qui se chevauchent passent côte à côte. */
function packColumns(events, day) {
  const sorted = [...events].sort((a, b) => a.startDate - b.startDate || b.endDate - a.endDate);
  const placed = [];
  let group = [];
  let groupEnd = 0;
  const flush = () => {
    const cols = Math.max(1, ...group.map((p) => p.col + 1));
    for (const p of group) p.cols = cols;
    placed.push(...group);
    group = [];
  };
  for (const event of sorted) {
    const start = Math.max(event.startDate.getTime(), day.getTime());
    if (group.length > 0 && start >= groupEnd) flush();
    const used = new Set(group.filter((p) => p.event.endDate.getTime() > start).map((p) => p.col));
    let col = 0;
    while (used.has(col)) col++;
    group.push({ event, col, cols: 1 });
    groupEnd = Math.max(groupEnd, event.endDate.getTime());
  }
  if (group.length > 0) flush();
  return placed;
}

/** Un rendez-vous qu'on touche pour l'ouvrir. */
function eventButton(event, cls, label) {
  const button = el("button", { type: "button", class: cls, title: `${event.summary} · ${eventSpan(event)}` }, el("b", { text: label }));
  tint(button, event.color);
  button.addEventListener("click", (click) => {
    click.stopPropagation();
    openEventDetails(event);
  });
  return button;
}

// ----- Mois

function monthView(calendars) {
  const first = new Date(agenda.anchor.getFullYear(), agenda.anchor.getMonth(), 1);
  const start = weekStart(first);
  const lastDay = new Date(first.getFullYear(), first.getMonth() + 1, 0);
  const weeks = Math.ceil((Math.round((lastDay - start) / DAY_MS) + 1) / 7);
  const now = new Date();
  const box = el("div", { class: "agenda-month" });
  box.style.setProperty("--weeks", String(weeks));
  box.append(el("div", { class: "agenda-month-head" },
    ...["lun.", "mar.", "mer.", "jeu.", "ven.", "sam.", "dim."].map((d) => el("span", { text: d }))));
  for (let w = 0; w < weeks; w++) {
    const monday = addDays(start, w * 7);
    const row = el("div", { class: "agenda-month-row" });
    const events = agendaEvents(monday, addDays(monday, 7), calendars);
    // Les rendez-vous de plusieurs jours : un bandeau sur la semaine.
    const long = events.filter((e) => e.allDay && Math.round((dayStart(new Date(e.endDate.getTime() - 1)) - dayStart(e.startDate)) / DAY_MS) >= 1);
    const lanes = [];
    for (const e of long.slice(0, 6)) {
      const from = Math.max(0, Math.round((dayStart(e.startDate) - monday) / DAY_MS));
      const to = Math.min(6, Math.round((dayStart(new Date(e.endDate.getTime() - 1)) - monday) / DAY_MS));
      let lane = lanes.findIndex((busy) => busy < from);
      if (lane === -1) {
        if (lanes.length >= 2) continue;
        lane = lanes.length;
        lanes.push(-1);
      }
      lanes[lane] = to;
      const band = eventButton(e, "agenda-month-band", e.summary);
      band.style.setProperty("--col", String(from + 1));
      band.style.setProperty("--span", String(to - from + 1));
      band.style.setProperty("--lane", String(lane));
      row.append(band);
    }
    row.style.setProperty("--lanes", String(lanes.length));
    for (let d = 0; d < 7; d++) {
      const day = addDays(monday, d);
      const out = day.getMonth() !== first.getMonth();
      const cell = el("div", { class: `agenda-month-cell${out ? " out" : ""}${sameDay(day, now) ? " is-today" : ""}` });
      const number = el("button", { type: "button", class: "agenda-month-number", text: String(day.getDate()), "aria-label": agendaLongDay.format(day) });
      number.addEventListener("click", () => openAgenda(day, "day"));
      cell.append(number, el("span", { class: "agenda-month-gap" }));
      const short = events.filter((e) => !long.includes(e) && e.startDate < addDays(day, 1) && e.endDate > day);
      for (const e of short.slice(0, 3)) {
        const line = el("button", { type: "button", class: "agenda-month-event" }, colorDot(e.color));
        if (!e.allDay) line.append(el("small", { text: hourMinute.format(e.startDate) }));
        line.append(el("span", { text: e.summary }));
        line.addEventListener("click", () => openEventDetails(e));
        cell.append(line);
      }
      if (short.length > 3) {
        const more = el("button", { type: "button", class: "agenda-month-more", text: `+ ${short.length - 3} autres` });
        more.addEventListener("click", () => openAgenda(day, "day"));
        cell.append(more);
      }
      row.append(cell);
    }
    box.append(row);
  }
  return box;
}

// ----- Liste (le téléphone)

function listView(calendars) {
  const now = new Date();
  const from = agenda.anchor;
  const monday = weekStart(from);
  const strip = el("div", { class: "agenda-strip" });
  for (let i = 0; i < 7; i++) {
    const day = addDays(monday, i);
    const dots = el("span", { class: "agenda-strip-dots" },
      ...eventsOfDay(day, calendars).slice(0, 3).map((e) => colorDot(e.color)));
    const button = el("button", { type: "button", class: `agenda-strip-day${sameDay(day, now) ? " is-today" : ""}${sameDay(day, from) ? " chosen" : ""}` },
      el("span", { text: agendaWeekday.format(day) }), el("b", { text: String(day.getDate()) }), dots);
    button.addEventListener("click", () => {
      agenda.anchor = day;
      renderAgendaView();
    });
    strip.append(button);
  }
  const list = el("div", { class: "agenda-list-days" });
  const seen = new Set();
  let shown = 0;
  for (let i = 0; i < 21 && shown < 60; i++) {
    const day = addDays(from, i);
    const events = eventsOfDay(day, calendars).filter((e) => {
      // Un rendez-vous de plusieurs jours : une seule fois.
      const key = `${e.calendar.id}|${e.uid}|${e.start}`;
      if (seen.has(key)) return false;
      seen.add(key);
      return true;
    });
    if (events.length === 0) continue;
    const label = agendaDay(day) === "Aujourd'hui" ? `Aujourd'hui · ${agendaLongDay.format(day)}`
      : agendaDay(day) === "Demain" ? `Demain · ${agendaLongDay.format(day)}` : agendaLongDay.format(day);
    list.append(el("h3", { class: sameDay(day, now) ? "agenda-list-day is-today" : "agenda-list-day", text: label }));
    let lineDrawn = !sameDay(day, now);
    for (const e of events) {
      if (!lineDrawn && !e.allDay && e.startDate > now) {
        list.append(el("div", { class: "agenda-list-now", text: hourMinute.format(now) }));
        lineDrawn = true;
      }
      list.append(listEvent(e, now));
      shown++;
    }
  }
  if (shown === 0) list.append(el("p", { class: "muted", text: "Rien de prévu dans les trois semaines qui viennent." }));
  return el("div", { class: "agenda-list" }, strip, list);
}

function listEvent(e, now) {
  if (e.allDay) {
    const pill = eventButton(e, "agenda-list-allday", e.summary);
    const last = new Date(e.endDate.getTime() - 1);
    if (!sameDay(e.startDate, last)) pill.append(el("small", { text: ` · jusqu'au ${shortDate.format(last)}` }));
    return pill;
  }
  const row = el("button", { type: "button", class: `agenda-list-event${e.endDate < now ? " past" : ""}` },
    colorDot(e.color, "agenda-list-icon"),
    el("span", { class: "agenda-list-text" }, el("b", { text: e.summary }),
      el("small", { text: [e.label, e.location].filter(Boolean).join(" · ") })),
    el("span", { class: "agenda-list-time", text: `${hourMinute.format(e.startDate)}\n${hourMinute.format(e.endDate)}` }));
  tint(row, e.color, 14);
  row.addEventListener("click", () => openEventDetails(e));
  return row;
}

// ----- À gauche : aujourd'hui, le mini-mois, les agendas

/** Les rendez-vous d'aujourd'hui (et le premier de demain). */
function todayCard(calendars, home) {
  const now = new Date();
  const today = dayStart(now);
  const events = eventsOfDay(today, calendars);
  // Demain : ce qui commence demain (pas un séjour déjà en cours).
  const tomorrow = eventsOfDay(addDays(today, 1), calendars)
    .filter((e) => e.startDate >= addDays(today, 1))
    .slice(0, home ? 2 : 1);
  const next = events.find((e) => !e.allDay && e.startDate > now);
  const head = el("div", { class: "agenda-card-head" }, el("h3", { text: home ? "Agenda · aujourd'hui" : "Aujourd'hui" }));
  if (home) {
    const all = el("button", { type: "button", class: "agenda-link", text: "Tout l'agenda ›" });
    all.addEventListener("click", () => openAgenda(today));
    head.append(all);
  }
  const rows = [];
  // Sur la page Maison, la liste reste courte : ce qui vient d'abord.
  const list = [...events, ...tomorrow];
  const shown = home && list.length > 6 ? list.filter((e) => !(e.endDate < now && !e.allDay)).slice(0, 6) : list;
  for (const e of shown) {
    const isTomorrow = !sameDay(e.startDate, today) && e.startDate > today;
    const past = !isTomorrow && e.endDate < now && !e.allDay;
    const when = isTomorrow ? "demain" : e.allDay ? "journée" : hourMinute.format(e.startDate);
    const note = e === next ? inMinutes(e.startDate) : past ? "fait" : e.label || e.location || "";
    const row = el("button", { type: "button", class: `agenda-today-line${e === next ? " next" : ""}${past ? " past" : ""}` },
      el("span", { class: "agenda-today-time", text: when }), colorDot(e.color, "agenda-today-icon"),
      el("span", { class: "agenda-today-text" }, el("b", { text: e.summary }), el("small", { text: isTomorrow && !e.allDay ? hourMinute.format(e.startDate) : e.location || "" })),
      el("span", { class: "agenda-today-note", text: note }));
    row.addEventListener("click", () => openEventDetails(e));
    rows.push(row);
  }
  if (rows.length === 0) rows.push(el("p", { class: "muted", text: "Rien de prévu aujourd'hui." }));
  return [head, ...rows];
}

function miniMonth() {
  const a = agenda.anchor;
  const first = new Date(a.getFullYear(), a.getMonth(), 1);
  const start = weekStart(first);
  const now = new Date();
  const calendars = agendaCalendars();
  const busy = new Set(agendaEvents(start, addDays(start, 42), calendars).flatMap((e) => {
    const days = [];
    for (let d = dayStart(e.startDate); d < e.endDate && days.length < 42; d = addDays(d, 1)) days.push(d.getTime());
    return days;
  }));
  const prev = el("button", { type: "button", class: "agenda-mini-arrow", "aria-label": "Mois précédent", text: "‹" });
  const next = el("button", { type: "button", class: "agenda-mini-arrow", "aria-label": "Mois suivant", text: "›" });
  prev.addEventListener("click", () => { agenda.anchor = new Date(a.getFullYear(), a.getMonth() - 1, 1); renderAgendaView(); });
  next.addEventListener("click", () => { agenda.anchor = new Date(a.getFullYear(), a.getMonth() + 1, 1); renderAgendaView(); });
  const grid = el("div", { class: "agenda-mini-grid" }, ...["L", "M", "M", "J", "V", "S", "D"].map((d) => el("span", { class: "agenda-mini-name", text: d })));
  const chosenWeek = weekStart(a).getTime();
  for (let i = 0; i < 42; i++) {
    const day = addDays(start, i);
    if (i >= 35 && day.getMonth() !== first.getMonth()) break;
    const cls = ["agenda-mini-day"];
    if (day.getMonth() !== first.getMonth()) cls.push("out");
    if (sameDay(day, now)) cls.push("is-today");
    if (agenda.view === "week" && weekStart(day).getTime() === chosenWeek) cls.push("week");
    if (agenda.view === "day" && sameDay(day, a)) cls.push("chosen");
    if (busy.has(day.getTime())) cls.push("busy");
    const button = el("button", { type: "button", class: cls.join(" "), text: String(day.getDate()), "aria-label": agendaLongDay.format(day) });
    button.addEventListener("click", () => {
      agenda.anchor = day;
      if (agenda.view === "month") agenda.view = "day";
      renderAgendaView();
    });
    grid.append(button);
  }
  return [el("div", { class: "agenda-card-head" }, el("h3", { text: agendaMonthTitle.format(first) }), el("span", {}, prev, next)), grid];
}

function calendarToggles(calendars) {
  const rows = calendars.map((calendar) => {
    const shown = !agenda.hidden.has(calendar.id);
    const failure = typeof calendar.state.attributes.erreur === "string" ? calendar.state.attributes.erreur : "";
    const where = calendar.state.state === "unavailable" ? (failure || "pas encore chargé")
      : calendar.source === "TimeTree" ? "TimeTree" : calendar.readOnly ? `en ligne (${calendar.source})` : "sur le serveur";
    const toggle = el("button", { type: "button", class: "agenda-calendar", role: "switch", "aria-checked": String(shown) },
      colorDot(calendar.color, "agenda-calendar-icon"),
      el("span", { class: "agenda-today-text" }, el("b", { text: calendar.name }), el("small", { class: calendar.state.state === "unavailable" ? "error" : "", text: where })),
      el("span", { class: "agenda-switch", "aria-hidden": "true" }));
    toggle.style.setProperty("--ev", calendar.color);
    toggle.addEventListener("click", () => {
      if (shown) agenda.hidden.add(calendar.id);
      else agenda.hidden.delete(calendar.id);
      store(AGENDA_HIDDEN_KEY, JSON.stringify([...agenda.hidden]));
      renderAgendaView();
    });
    return toggle;
  });
  const timetree = calendars.filter((c) => c.source === "TimeTree");
  const out = [el("div", { class: "agenda-card-head" }, el("h3", { text: "Agendas" })), ...rows];
  if (timetree.length > 0) {
    const ok = timetree.every((c) => c.state.state !== "unavailable");
    const changed = Math.max(...timetree.map((c) => Date.parse(c.state.last_updated) || 0));
    out.push(el("p", { class: `agenda-sync${ok ? "" : " error"}`, text: ok ? `TimeTree relu ${changed ? `à ${hourMinute.format(new Date(changed))}` : ""}, toutes les 15 min` : "TimeTree injoignable" }));
  }
  if (calendars.length === 0) out.push(el("p", { class: "muted", text: "Aucun agenda : voir docs/agenda.md et docs/timetree.md." }));
  return out;
}

// ----- Un rendez-vous

function openEventDetails(e) {
  const dialog = $("#agenda-event");
  const close = el("button", { type: "button", class: "ghost", text: "Fermer" });
  close.addEventListener("click", () => dialog.close());
  const title = el("h2", { id: "agenda-event-title" }, colorDot(e.color, "agenda-event-dot"), e.summary);
  const lastDay = new Date(e.endDate.getTime() - 1);
  const when = e.allDay
    ? (sameDay(e.startDate, lastDay) ? agendaLongDay.format(e.startDate) : `du ${agendaLongDay.format(e.startDate)} au ${agendaLongDay.format(lastDay)}`)
    : `${agendaLongDay.format(e.startDate)} · ${eventSpan(e)}`;
  const rows = [el("p", { class: "agenda-event-when", text: when })];
  const line = (icon, ...content) => el("div", { class: "agenda-event-row" }, el("span", { class: "agenda-event-key", "aria-hidden": "true" }, svgIcon(icon)), el("div", {}, ...content));
  if (e.location) rows.push(line(ICONS.globe, e.location));
  const where = [];
  if (e.label) {
    const chip = el("span", { class: "agenda-label", text: e.label });
    tint(chip, e.color, 30);
    where.push(chip, " ");
  }
  where.push(`agenda ${e.calendar.name}${e.calendar.source === "TimeTree" ? " · TimeTree" : ""}`);
  rows.push(line(ICONS.calendar, ...where));
  if (Array.isArray(e.attendees) && e.attendees.length > 0) {
    rows.push(line(ICONS.person, ...e.attendees.filter((n) => typeof n === "string").map((name) =>
      el("span", { class: "agenda-person" }, el("span", { class: "agenda-person-initial", text: name.charAt(0).toUpperCase(), "aria-hidden": "true" }), name))));
  }
  if (typeof e.description === "string" && e.description.trim() !== "") rows.push(line(ICONS.list, el("span", { class: "agenda-event-note", text: e.description })));
  const buttons = el("div", { class: "agenda-event-buttons" });
  if (e.calendar.source === "TimeTree") {
    buttons.append(el("a", { class: "action-button", href: "https://timetreeapp.com/", target: "_blank", rel: "noopener noreferrer", text: "Ouvrir TimeTree" }));
  }
  if (!e.calendar.readOnly && (typeof canControl !== "function" || canControl(e.calendar.state)) && typeof e.uid === "string") {
    const remove = el("button", { type: "button", class: "ghost danger", text: "Supprimer" });
    remove.addEventListener("click", () => {
      if (!window.confirm(`Supprimer « ${e.summary} » ?`)) return;
      callService("calendar", "delete_event", { entity_id: e.calendar.id, uid: e.uid });
      dialog.close();
    });
    buttons.append(remove);
  }
  dialog.replaceChildren(el("div", { class: "history-top" }, title, close), ...rows);
  if (buttons.childElementCount > 0) dialog.append(buttons);
  if (!dialog.open) dialog.showModal();
}

// ----- Nouveau rendez-vous (agenda de la maison)

$("#agenda-add").addEventListener("click", () => {
  const day = agenda.anchor;
  $("#agenda-new-day").value = `${day.getFullYear()}-${String(day.getMonth() + 1).padStart(2, "0")}-${String(day.getDate()).padStart(2, "0")}`;
  $("#agenda-new-summary").value = "";
  $("#agenda-new-start").value = "";
  $("#agenda-new-end").value = "";
  $("#agenda-new").showModal();
  $("#agenda-new-summary").focus();
});
$("#agenda-new-close").addEventListener("click", () => $("#agenda-new").close());
$("#agenda-new-form").addEventListener("submit", (event) => {
  event.preventDefault();
  const summary = $("#agenda-new-summary").value.trim();
  const day = $("#agenda-new-day").value;
  const start = $("#agenda-new-start").value;
  const end = $("#agenda-new-end").value;
  if (!summary || !day) return;
  const data = { entity_id: "calendar.maison", summary };
  if (start) {
    data.start_date_time = `${day} ${start}`;
    if (end && end > start) data.end_date_time = `${day} ${end}`;
  } else {
    data.start_date = day;
  }
  callService("calendar", "create_event", data);
  $("#agenda-new").close();
});

// ---------------------------------------------------------------- Page Maison

/** Mise en page « tableau » : aujourd'hui et la semaine, sous les pièces. */
function renderBoardAgenda() {
  const box = $("#board-agenda");
  const calendars = agendaCalendars();
  if (!isBoard() || calendars.length === 0) {
    box.hidden = true;
    return;
  }
  box.hidden = false;
  const today = el("div", { class: "agenda-card agenda-home-today" }, ...todayCard(calendars, true));
  const monday = weekStart(new Date());
  const now = new Date();
  const week = el("div", { class: "agenda-home-week" });
  for (let i = 0; i < 7; i++) {
    const day = addDays(monday, i);
    const bars = eventsOfDay(day, calendars).slice(0, 4).map((e) => {
      const bar = el("span", { class: "agenda-home-bar", title: `${eventTime(e)} ${e.summary}` });
      bar.style.setProperty("--ev", e.color);
      return bar;
    });
    const cell = el("button", { type: "button", class: `agenda-home-day${sameDay(day, now) ? " is-today" : ""}`, "aria-label": `${agendaLongDay.format(day)} : ${bars.length} rendez-vous` },
      el("span", { text: agendaWeekday.format(day) }), el("b", { text: String(day.getDate()) }), ...bars);
    cell.addEventListener("click", () => openAgenda(day, "day"));
    week.append(cell);
  }
  const weekCard = el("div", { class: "agenda-card agenda-home-weekcard" },
    el("div", { class: "agenda-card-head" }, el("h3", { text: "Cette semaine" })), week);
  box.replaceChildren(today, weekCard);
}

/** Le prochain rendez-vous d'aujourd'hui, pour les repères et l'accueil. */
function nextAppointment() {
  const now = new Date();
  const today = dayStart(now);
  return eventsOfDay(today, agendaCalendars()).find((e) => !e.allDay && e.startDate > now && sameDay(e.startDate, now)) ?? null;
}

/** Le repère « 15:00 Dentiste » (house.js). */
function agendaGauge() {
  const next = nextAppointment();
  if (!next) return null;
  const chip = gauge(ICONS.calendar, "Prochain rendez-vous", `${hourMinute.format(next.startDate)} ${next.summary}`, "agenda");
  chip.style.setProperty("--ev", next.color);
  chip.setAttribute("role", "button");
  chip.tabIndex = 0;
  chip.addEventListener("click", () => openEventDetails(next));
  return chip;
}

/** « Dentiste dans 40 min », dans la salutation, l'heure qui précède. */
function agendaSoon() {
  const next = nextAppointment();
  if (!next || next.startDate.getTime() - Date.now() > 60 * 60_000) return null;
  const link = el("a", { href: "#", text: `${next.summary} ${inMinutes(next.startDate)}` });
  link.addEventListener("click", (event) => {
    event.preventDefault();
    openEventDetails(next);
  });
  return link;
}
