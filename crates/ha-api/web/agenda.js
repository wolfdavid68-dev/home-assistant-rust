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
  const dot = el("span", { class: `agenda-dot w-[9px] h-[9px] rounded-full [background:var(--ev)] flex-none inline-block [.agenda-strip-dots_&]:w-[5px] [.agenda-strip-dots_&]:h-[5px] ${cls}`.trim(), "aria-hidden": "true" });
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
    const button = el("button", { type: "button", class: "min-h-11", "aria-pressed": String(agenda.view === id), text: label });
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

  const grid = el("div", { class: "agenda-week flex flex-col min-h-0 flex-1" });
  grid.style.setProperty("--days", String(days.length));

  // L'en-tête : les jours.
  const head = el("div", { class: "agenda-days grid [grid-template-columns:52px_repeat(var(--days),_minmax(0,_1fr))]" }, el("span"));
  for (const day of days) {
    const cell = el("button", { type: "button", class: sameDay(day, now) ? "agenda-day is-today" : "agenda-day" },
      el("span", { text: agendaWeekday.format(day) }), el("b", { text: String(day.getDate()) }));
    cell.addEventListener("click", () => openAgenda(day, "day"));
    head.append(cell);
  }

  // Les journées entières, en bandeaux, sur une ou plusieurs lignes.
  const lanes = [];
  const bandBox = el("div", { class: "agenda-bands grid [:not(:where(.o122501))&]:[grid-template-columns:52px_repeat(var(--days),_minmax(0,_1fr))] [:not(:where(.o122900))&]:[grid-template-rows:repeat(var(--lanes),_22px)] gap-y-1 pb-1.5 [border-bottom:1px_solid_var(--line)]" }, el("span", { class: "agenda-bands-label [:not(:where(.o123000))&]:[grid-column:1] [:not(:where(.o123001))&]:[grid-row:1] text-micro text-muted text-right pt-1 pr-1.5 pb-0 pl-0", text: "journée" }));
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
  const hours = el("div", { class: "agenda-hours relative [:not(:where(.o123500))&_span]:absolute [:not(:where(.o123501))&_span]:[right:6px] [&_span]:[transform:translateY(-50%)] [:not(:where(.o123503))&_span]:[font:0.66rem_var(--font-mono)] [:not(:where(.o123504))&_span]:text-muted" });
  for (let h = first + 1; h < last; h++) {
    const mark = el("span", { text: `${String(h).padStart(2, "0")}:00` });
    mark.style.setProperty("top", at(h));
    hours.append(mark);
  }
  const columns = el("div", { class: "agenda-columns grid [grid-template-columns:52px_repeat(var(--days),_minmax(0,_1fr))] relative flex-none [height:calc(var(--hours,_14)_*_30px)] overflow-hidden" }, hours);
  columns.style.setProperty("--hours", String(span));
  for (const day of days) {
    const column = el("div", { class: sameDay(day, now) ? "agenda-column is-today" : "agenda-column" });
    for (let h = first + 1; h < last; h++) {
      const line = el("span", { class: "agenda-hour-line absolute [:not(:where(.o123801))&]:[left:0] [:not(:where(.o123802))&]:[right:0] [border-top:1px_solid_color-mix(in_srgb,_var(--line)_55%,_transparent)]" });
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
      const line = el("span", { class: "agenda-now absolute [:not(:where(.o124401))&]:[left:0] [:not(:where(.o124402))&]:[right:0] [border-top:2px_solid_var(--alert)] [z-index:2] pointer-events-none [&::before]:[content:''] [&::before]:absolute [:not(:where(.o124502))&::before]:[left:-5px] [:not(:where(.o124503))&::before]:[top:-5px] [&::before]:w-[8px] [&::before]:h-[8px] [&::before]:rounded-full [&::before]:bg-alert" });
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
  const box = el("div", { class: "agenda-month flex-1 grid [grid-template-rows:auto_repeat(var(--weeks),_minmax(96px,_1fr))]" });
  box.style.setProperty("--weeks", String(weeks));
  box.append(el("div", { class: "agenda-month-head grid [grid-template-columns:repeat(7,_minmax(0,_1fr))] [border-bottom:1px_solid_var(--line)] [&_span]:text-center [:not(:where(.o124801))&_span]:text-micro [:not(:where(.o124802))&_span]:text-muted [&_span]:uppercase [&_span]:pt-1 [&_span]:px-0 [&_span]:pb-1.5" },
    ...["lun.", "mar.", "mer.", "jeu.", "ven.", "sam.", "dim."].map((d) => el("span", { text: d }))));
  for (let w = 0; w < weeks; w++) {
    const monday = addDays(start, w * 7);
    const row = el("div", { class: "agenda-month-row relative grid [grid-template-columns:repeat(7,_minmax(0,_1fr))] [border-bottom:1px_solid_var(--line)] [&:last-child]:[border-bottom:0]" });
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
      const cell = el("div", { class: `[border-left:1px_solid_var(--line)] py-1 px-1.5 min-w-0 flex flex-col gap-px overflow-hidden [&:first-of-type]:[border-left:0] [&.out]:[opacity:0.45] agenda-month-cell${out ? " out" : ""}${sameDay(day, now) ? " is-today" : ""}` });
      const number = el("button", { type: "button", class: "agenda-month-number [:where(&)]:[all:unset] cursor-pointer [:not(:where(.o125402))&]:text-petit [:not(:where(.o125403))&]:font-semibold w-[24px] h-[24px] [:not(:where(.o125406))&]:[line-height:24px] text-center rounded-full [.agenda-month-cell.is-today_&]:bg-accent [.agenda-month-cell.is-today_&]:text-accent-ink", text: String(day.getDate()), "aria-label": agendaLongDay.format(day) });
      number.addEventListener("click", () => openAgenda(day, "day"));
      cell.append(number, el("span", { class: "agenda-month-gap [height:calc(var(--lanes,_0)_*_20px)] flex-none" }));
      const short = events.filter((e) => !long.includes(e) && e.startDate < addDays(day, 1) && e.endDate > day);
      for (const e of short.slice(0, 3)) {
        const line = el("button", { type: "button", class: "agenda-month-event [:where(&)]:[all:unset] cursor-pointer flex items-center gap-1.5 text-legende whitespace-nowrap overflow-hidden min-w-0 rounded-trait py-px px-0.5 [&:hover]:bg-card-2 [:not(:where(.o126200))&_small]:[font:0.66rem_var(--font-mono)] [:not(:where(.o126201))&_small]:text-muted [:not(:where(.o126300))&_span]:overflow-hidden [:not(:where(.o126301))&_span]:text-ellipsis" }, colorDot(e.color));
        if (!e.allDay) line.append(el("small", { text: hourMinute.format(e.startDate) }));
        line.append(el("span", { text: e.summary }));
        line.addEventListener("click", () => openEventDetails(e));
        cell.append(line);
      }
      if (short.length > 3) {
        const more = el("button", { type: "button", class: "agenda-month-more [:where(&)]:[all:unset] cursor-pointer text-micro text-muted pl-0.5", text: `+ ${short.length - 3} autres` });
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
  const strip = el("div", { class: "agenda-strip grid [grid-template-columns:repeat(7,_minmax(0,_1fr))] pt-1 px-0 pb-2" });
  for (let i = 0; i < 7; i++) {
    const day = addDays(monday, i);
    const dots = el("span", { class: "agenda-strip-dots flex justify-center gap-1 h-[6px] mt-1" },
      ...eventsOfDay(day, calendars).slice(0, 3).map((e) => colorDot(e.color)));
    const button = el("button", { type: "button", class: `[:where(&)]:[all:unset] cursor-pointer text-center text-micro text-muted [:not(:where(.o126800))&_b]:block [&_b]:mt-0.5 [&_b]:mx-auto [&_b]:mb-0 [:not(:where(.o126802))&_b]:w-[32px] [:not(:where(.o126803))&_b]:h-[32px] [:not(:where(.o126805))&_b]:rounded-full [:not(:where(.o126806))&_b]:[font:600_1rem_var(--font-display)] [:not(:where(.o126807))&_b]:text-text [&.chosen_b]:[box-shadow:inset_0_0_0_2px_var(--accent)] [:not(:where(.o127000))&.is-today_b]:bg-accent [:not(:where(.o127001))&.is-today_b]:text-accent-ink agenda-strip-day${sameDay(day, now) ? " is-today" : ""}${sameDay(day, from) ? " chosen" : ""}` },
      el("span", { text: agendaWeekday.format(day) }), el("b", { text: String(day.getDate()) }), dots);
    button.addEventListener("click", () => {
      agenda.anchor = day;
      renderAgendaView();
    });
    strip.append(button);
  }
  const list = el("div", { class: "agenda-list-days grid gap-2" });
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
        list.append(el("div", { class: "agenda-list-now flex items-center gap-1.5 text-alert [font:600_0.7rem_var(--font-mono)] [&::after]:[content:''] [&::after]:flex-1 [&::after]:[border-top:2px_solid_var(--alert)]", text: hourMinute.format(now) }));
        lineDrawn = true;
      }
      list.append(listEvent(e, now));
      shown++;
    }
  }
  if (shown === 0) {
    const add = $("#agenda-add");
    list.append(etatVide(ICONS.calendar, "Rien de prévu dans les trois semaines qui viennent.", add.hidden ? null : { texte: "Ajouter un rendez-vous", onClick: () => add.click() }));
  }
  return el("div", { class: "agenda-list list-none m-0 grid gap-0.5 pt-0 px-3 pb-3 [overflow-y:auto]" }, strip, list);
}

function listEvent(e, now) {
  if (e.allDay) {
    const pill = eventButton(e, "agenda-list-allday", e.summary);
    const last = new Date(e.endDate.getTime() - 1);
    if (!sameDay(e.startDate, last)) pill.append(el("small", { text: ` · jusqu'au ${shortDate.format(last)}` }));
    return pill;
  }
  const row = el("button", { type: "button", class: `[:where(&)]:[all:unset] cursor-pointer [box-sizing:border-box] flex items-center gap-3 py-2.5 px-3 rounded-champ bg-card-2 [box-shadow:inset_3px_0_0_var(--ev)] [&.past]:[opacity:0.45] agenda-list-event${e.endDate < now ? " past" : ""}` },
    colorDot(e.color, "agenda-list-icon"),
    el("span", { class: "agenda-list-text grid flex-1 min-w-0 [:not(:where(.o128200))&_b]:font-semibold [:not(:where(.o128201))&_b]:text-courant [:not(:where(.o128202))&_b]:overflow-hidden [:not(:where(.o128203))&_b]:text-ellipsis [:not(:where(.o128204))&_b]:whitespace-nowrap [:not(:where(.o128300))&_small]:text-legende [:not(:where(.o128301))&_small]:text-muted [:not(:where(.o128302))&_small]:overflow-hidden [:not(:where(.o128303))&_small]:text-ellipsis [:not(:where(.o128304))&_small]:whitespace-nowrap" }, el("b", { text: e.summary }),
      el("small", { text: [e.label, e.location].filter(Boolean).join(" · ") })),
    el("span", { class: "agenda-list-time [font:0.78rem_var(--font-mono)] text-muted [white-space:pre-line] text-right", text: `${hourMinute.format(e.startDate)}\n${hourMinute.format(e.endDate)}` }));
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
  const head = el("div", { class: "agenda-card-head flex items-center justify-between gap-2 mb-1.5 [:not(:where(.o117500))&_h3]:m-0 [:not(:where(.o117501))&_h3]:text-moyen [:not(:where(.o117502))&_h3]:font-semibold [:not(:where(.o117600))&_h3::first-letter]:uppercase" }, el("h3", { text: home ? "Agenda · aujourd'hui" : "Aujourd'hui" }));
  if (home) {
    const all = el("button", { type: "button", class: "agenda-link [:where(&)]:[all:unset] cursor-pointer text-accent [:not(:where(.o117703))&]:text-petit [:not(:where(.o117704))&]:font-medium", text: "Tout l'agenda ›" });
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
    const row = el("button", { type: "button", class: `[:where(&)]:[all:unset] cursor-pointer [box-sizing:border-box] w-full flex items-center gap-2.5 p-1.5 rounded-champ [:not(:where(.o118000))&:hover]:bg-card-2 [:not(:where(.o118100))&.next]:bg-accent-soft [&.past]:[opacity:0.5] [&:focus-visible]:[outline:3px_solid_var(--accent)] [&:focus-visible]:[outline-offset:-3px] agenda-today-line${e === next ? " next" : ""}${past ? " past" : ""}` },
      el("span", { class: "agenda-today-time [font:0.82rem_var(--font-mono)] text-muted w-[3.6rem] flex-none", text: when }), colorDot(e.color, "agenda-today-icon"),
      el("span", { class: "agenda-today-text grid min-w-0 flex-1 [:not(:where(.o118600))&_b]:font-semibold [:not(:where(.o118601))&_b]:text-corps [:not(:where(.o118602))&_b]:whitespace-nowrap [:not(:where(.o118603))&_b]:overflow-hidden [:not(:where(.o118604))&_b]:text-ellipsis [:not(:where(.o118700))&_small]:text-muted [:not(:where(.o118701))&_small]:text-legende [:not(:where(.o118702))&_small]:whitespace-nowrap [:not(:where(.o118703))&_small]:overflow-hidden [:not(:where(.o118704))&_small]:text-ellipsis [&_small.error]:[color:var(--alert-ink,_var(--alert))]" }, el("b", { text: e.summary }), el("small", { text: isTomorrow && !e.allDay ? hourMinute.format(e.startDate) : e.location || "" })),
      el("span", { class: "agenda-today-note text-legende text-muted text-right flex-none max-w-[40%] [.agenda-today-line.next_&]:text-accent [.agenda-today-line.next_&]:font-semibold", text: note }));
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
  const prev = el("button", { type: "button", class: "agenda-mini-arrow [:where(&)]:[all:unset] cursor-pointer inline-grid place-items-center min-w-11 min-h-11 rounded-puce text-muted text-moyen hover:bg-card-2", "aria-label": "Mois précédent", text: "‹" });
  const next = el("button", { type: "button", class: "agenda-mini-arrow [:where(&)]:[all:unset] cursor-pointer inline-grid place-items-center min-w-11 min-h-11 rounded-puce text-muted text-moyen hover:bg-card-2", "aria-label": "Mois suivant", text: "›" });
  prev.addEventListener("click", () => { agenda.anchor = new Date(a.getFullYear(), a.getMonth() - 1, 1); renderAgendaView(); });
  next.addEventListener("click", () => { agenda.anchor = new Date(a.getFullYear(), a.getMonth() + 1, 1); renderAgendaView(); });
  const grid = el("div", { class: "agenda-mini-grid grid [grid-template-columns:repeat(7,_minmax(0,_1fr))] text-center gap-y-0.5 [isolation:isolate]" }, ...["L", "M", "M", "J", "V", "S", "D"].map((d) => el("span", { class: "agenda-mini-name text-legende text-muted pb-0.5", text: d })));
  const chosenWeek = weekStart(a).getTime();
  for (let i = 0; i < 42; i++) {
    const day = addDays(start, i);
    if (i >= 35 && day.getMonth() !== first.getMonth()) break;
    // 44 px de haut au doigt ; le rond d'aujourd'hui reste un rond de 24 px.
    const cls = ["agenda-mini-day", "h-11", "leading-11", "before:[inset:calc(50%_-_12px)]"];
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
  return [el("div", { class: "agenda-card-head flex items-center justify-between gap-2 mb-1.5 [:not(:where(.o117500))&_h3]:m-0 [:not(:where(.o117501))&_h3]:text-moyen [:not(:where(.o117502))&_h3]:font-semibold [:not(:where(.o117600))&_h3::first-letter]:uppercase" }, el("h3", { text: agendaMonthTitle.format(first) }), el("span", {}, prev, next)), grid];
}

function calendarToggles(calendars) {
  const rows = calendars.map((calendar) => {
    const shown = !agenda.hidden.has(calendar.id);
    const failure = typeof calendar.state.attributes.erreur === "string" ? calendar.state.attributes.erreur : "";
    const where = calendar.state.state === "unavailable" ? (failure || "pas encore chargé")
      : calendar.source === "TimeTree" ? "TimeTree" : calendar.readOnly ? `en ligne (${calendar.source})` : "sur le serveur";
    const toggle = el("button", { type: "button", class: "agenda-calendar [:where(&)]:[all:unset] cursor-pointer [box-sizing:border-box] w-full flex items-center gap-2.5 py-1.5 px-0.5 [&[aria-checked=false]_.agenda-calendar-icon]:[background:transparent] [&[aria-checked=false]_.agenda-calendar-icon]:[box-shadow:inset_0_0_0_2px_var(--ev)] [&:focus-visible]:[outline:3px_solid_var(--accent)] [&:focus-visible]:[outline-offset:-3px]", role: "switch", "aria-checked": String(shown) },
      colorDot(calendar.color, "agenda-calendar-icon"),
      el("span", { class: "agenda-today-text grid min-w-0 flex-1 [:not(:where(.o118600))&_b]:font-semibold [:not(:where(.o118601))&_b]:text-corps [:not(:where(.o118602))&_b]:whitespace-nowrap [:not(:where(.o118603))&_b]:overflow-hidden [:not(:where(.o118604))&_b]:text-ellipsis [:not(:where(.o118700))&_small]:text-muted [:not(:where(.o118701))&_small]:text-legende [:not(:where(.o118702))&_small]:whitespace-nowrap [:not(:where(.o118703))&_small]:overflow-hidden [:not(:where(.o118704))&_small]:text-ellipsis [&_small.error]:[color:var(--alert-ink,_var(--alert))]" }, el("b", { text: calendar.name }), el("small", { class: calendar.state.state === "unavailable" ? "error" : "", text: where })),
      el("span", { class: "agenda-switch w-[30px] h-[18px] rounded-puce bg-card-2 [box-shadow:inset_0_0_0_1px_var(--line)] relative flex-none [&::after]:[content:''] [&::after]:absolute [:not(:where(.o120602))&::after]:[left:2px] [:not(:where(.o120603))&::after]:[top:2px] [&::after]:w-[14px] [&::after]:h-[14px] [&::after]:rounded-full [&::after]:bg-muted [&::after]:[transition:transform_0.15s] [.agenda-calendar[aria-checked=true]_&]:[background:color-mix(in_srgb,_var(--ev)_70%,_transparent)] [.agenda-calendar[aria-checked=true]_&]:[box-shadow:none] [.agenda-calendar[aria-checked=true]_&::after]:[transform:translateX(12px)] [.agenda-calendar[aria-checked=true]_&::after]:[background:#fff]", "aria-hidden": "true" }));
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
  const out = [el("div", { class: "agenda-card-head flex items-center justify-between gap-2 mb-1.5 [:not(:where(.o117500))&_h3]:m-0 [:not(:where(.o117501))&_h3]:text-moyen [:not(:where(.o117502))&_h3]:font-semibold [:not(:where(.o117600))&_h3::first-letter]:uppercase" }, el("h3", { text: "Agendas" })), ...rows];
  if (timetree.length > 0) {
    const ok = timetree.every((c) => c.state.state !== "unavailable");
    const changed = Math.max(...timetree.map((c) => Date.parse(c.state.last_updated) || 0));
    out.push(el("p", { class: `mt-1.5 mx-0 mb-0 text-legende text-muted [&::before]:[content:'●_'] [&::before]:text-ok [&.error::before]:text-alert agenda-sync${ok ? "" : " error"}`, text: ok ? `TimeTree relu ${changed ? `à ${hourMinute.format(new Date(changed))}` : ""}, toutes les 15 min` : "TimeTree injoignable" }));
  }
  if (calendars.length === 0) out.push(el("p", { class: "muted", text: "Aucun agenda : voir docs/agenda.md et docs/timetree.md." }));
  return out;
}

// ----- Un rendez-vous

function openEventDetails(e) {
  const dialog = $("#agenda-event");
  const title = el("span", { class: "inline-flex items-center gap-2.5" }, colorDot(e.color, "agenda-event-dot"), e.summary);
  const lastDay = new Date(e.endDate.getTime() - 1);
  const when = e.allDay
    ? (sameDay(e.startDate, lastDay) ? agendaLongDay.format(e.startDate) : `du ${agendaLongDay.format(e.startDate)} au ${agendaLongDay.format(lastDay)}`)
    : `${agendaLongDay.format(e.startDate)} · ${eventSpan(e)}`;
  const rows = [el("p", { class: "agenda-event-when mt-0.5 mr-0 mb-2.5 ml-6 text-text-2", text: when })];
  const line = (icon, ...content) => el("div", { class: "agenda-event-row flex gap-2.5 py-1.5 px-0 items-start text-text-2" }, el("span", { class: "agenda-event-key [:not(:where(.o129200))&_svg]:w-[16px] [:not(:where(.o129201))&_svg]:h-[16px] [:not(:where(.o129202))&_svg]:text-muted [&_svg]:mt-0.5", "aria-hidden": "true" }, svgIcon(icon)), el("div", {}, ...content));
  if (e.location) rows.push(line(ICONS.globe, e.location));
  const where = [];
  if (e.label) {
    const chip = el("span", { class: "agenda-label inline-block rounded-trait py-px px-2 [:not(:where(.o129403))&]:text-legende [:not(:where(.o129404))&]:font-semibold [background:color-mix(in_srgb,_var(--ev)_var(--ev-alpha,_30%),_transparent)] [color:color-mix(in_srgb,_var(--ev)_45%,_var(--text))]", text: e.label });
    tint(chip, e.color, 30);
    where.push(chip, " ");
  }
  where.push(`agenda ${e.calendar.name}${e.calendar.source === "TimeTree" ? " · TimeTree" : ""}`);
  rows.push(line(ICONS.calendar, ...where));
  if (Array.isArray(e.attendees) && e.attendees.length > 0) {
    rows.push(line(ICONS.person, ...e.attendees.filter((n) => typeof n === "string").map((name) =>
      el("span", { class: "agenda-person inline-flex items-center gap-1.5 pt-0.5 pr-2.5 pb-0.5 pl-1 mt-0 mr-1 mb-1 ml-0 rounded-full bg-card-2 text-petit" }, el("span", { class: "agenda-person-initial w-[20px] h-[20px] rounded-full bg-accent text-accent-ink grid [place-items:center] [:not(:where(.o129607))&]:text-micro [:not(:where(.o129608))&]:font-bold", text: name.charAt(0).toUpperCase(), "aria-hidden": "true" }), name))));
  }
  if (typeof e.description === "string" && e.description.trim() !== "") rows.push(line(ICONS.list, el("span", { class: "agenda-event-note [white-space:pre-line]", text: e.description })));
  const buttons = fenetrePied();
  buttons.classList.add("[&_a]:no-underline");
  if (e.calendar.source === "TimeTree") {
    buttons.append(el("a", { class: "ghost min-h-11 px-5 rounded-champ inline-flex items-center", href: "https://timetreeapp.com/", target: "_blank", rel: "noopener noreferrer", text: "Ouvrir TimeTree" }));
  }
  if (!e.calendar.readOnly && (typeof canControl !== "function" || canControl(e.calendar.state)) && typeof e.uid === "string") {
    const remove = fenetreBouton("Supprimer", "dangerDiscret");
    remove.addEventListener("click", async () => {
      if (!await confirmer(`« ${e.summary} » sera retiré de l'agenda.`, { titre: "Supprimer le rendez-vous ?", ok: "Supprimer", danger: true })) return;
      callService("calendar", "delete_event", { entity_id: e.calendar.id, uid: e.uid });
      dialog.close();
    });
    buttons.prepend(remove);
  }
  dialog.replaceChildren(fenetreEntete(title, "", () => dialog.close(), "agenda-event-title"), ...rows);
  if (buttons.childElementCount > 0) dialog.append(buttons);
  ouvrirFenetre(dialog);
}

// ----- Nouveau rendez-vous (agenda de la maison)

// Le formulaire est dans index.html ; ici, ce qui le rend commode : les
// heures proposées (le prochain quart d'heure, une heure de durée), la case
// « Toute la journée », les erreurs sous le champ concerné, le bouton qui
// dit « Ajout… » pendant l'envoi, et rien de perdu en fermant par erreur.
const agendaNew = { initial: "" };
const agendaNewFields = () => ["#agenda-new-summary", "#agenda-new-day", "#agenda-new-start", "#agenda-new-end", "#agenda-new-allday"].map((id) => $(id));
const agendaNewSnapshot = () => agendaNewFields().map((input) => (input.type === "checkbox" ? String(input.checked) : input.value)).join("|");

(function agendaNewSetup() {
  const form = $("#agenda-new-form");
  // Nos messages sous les champs, à la place des bulles du navigateur.
  form.noValidate = true;
  const summary = $("#agenda-new-summary");
  summary.setAttribute("enterkeyhint", "done");
  summary.setAttribute("autocapitalize", "sentences");
  for (const id of ["#agenda-new-day", "#agenda-new-start", "#agenda-new-end"]) $(id).setAttribute("enterkeyhint", "done");
  // Toute la journée : une case, plutôt que deux heures à vider.
  const times = form.querySelector(".agenda-new-times");
  // Une erreur sous « Fin » n'étire pas le champ « Début ».
  times.classList.add("items-start");
  const allDay = el("input", { id: "agenda-new-allday", type: "checkbox", class: "relative order-2 ml-auto shrink-0 w-[2.6rem] h-6 m-0 rounded-full bg-line appearance-none cursor-pointer transition-colors checked:bg-ok after:absolute after:top-[3px] after:left-[3px] after:size-[18px] after:rounded-full after:bg-white after:shadow after:transition-transform checked:after:translate-x-[1.1rem] motion-reduce:transition-none motion-reduce:after:transition-none" });
  // (Le formulaire met ses libellés en grille : la case passe à droite par les colonnes.)
  times.before(el("label", { class: "check grid grid-flow-col items-center justify-between gap-3 min-h-11" }, allDay, el("span", { class: "text-courant text-text", text: "Toute la journée" })));
  allDay.addEventListener("change", () => {
    times.hidden = allDay.checked;
    if (!allDay.checked && !$("#agenda-new-start").value) agendaNewTimes();
  });
  const hint = form.querySelector(".auto-meta");
  if (hint) hint.textContent = "Ajouté dans l'agenda de la maison.";
  // Le début bouge : la fin garde la même durée.
  const start = $("#agenda-new-start");
  let before = "";
  start.addEventListener("focus", () => { before = start.value; });
  start.addEventListener("change", () => {
    const end = $("#agenda-new-end");
    if (!start.value) return;
    const duration = before && end.value ? formMinutes(end.value) - formMinutes(before) : 60;
    end.value = formHeurePlus(start.value, duration > 0 ? duration : 60);
    before = start.value;
    champErreur(end);
  });
  // L'erreur venue du serveur : au-dessus des boutons.
  form.querySelector(".fenetre-pied").before(el("p", { id: "agenda-new-error", class: "m-0 text-petit font-semibold text-alert-ink", role: "alert", hidden: "" }));
  fenetreGarde($("#agenda-new"), () => agendaNewSnapshot() !== agendaNew.initial);
})();

/** Les heures proposées : le prochain quart d'heure, pour une heure. */
function agendaNewTimes() {
  const start = formProchainQuart(new Date());
  $("#agenda-new-start").value = start;
  $("#agenda-new-end").value = formHeurePlus(start, 60);
}

$("#agenda-add").addEventListener("click", () => {
  const day = agenda.anchor;
  $("#agenda-new-day").value = `${day.getFullYear()}-${String(day.getMonth() + 1).padStart(2, "0")}-${String(day.getDate()).padStart(2, "0")}`;
  $("#agenda-new-summary").value = "";
  $("#agenda-new-allday").checked = false;
  $("#agenda-new-form .agenda-new-times").hidden = false;
  agendaNewTimes();
  for (const input of agendaNewFields()) champErreur(input);
  $("#agenda-new-error").hidden = true;
  agendaNew.initial = agendaNewSnapshot();
  ouvrirFenetre($("#agenda-new"), $("#agenda-new-summary"));
});
for (const id of ["#agenda-new-close", "#agenda-new-cancel"]) $(id).addEventListener("click", () => fenetreFermer($("#agenda-new")));

/** Les erreurs du formulaire, sous leurs champs ; `true` s'il n'y en a pas. */
function agendaNewCheck() {
  const [summary, day, start, end, allDay] = agendaNewFields();
  const errors = [];
  if (!summary.value.trim()) errors.push([summary, "Donne un titre au rendez-vous."]);
  if (!day.value) errors.push([day, "Choisis le jour."]);
  if (!allDay.checked) {
    if (!start.value) errors.push([start, "Choisis l'heure de début, ou coche « Toute la journée »."]);
    else if (end.value && end.value <= start.value) errors.push([end, `La fin doit venir après le début (${start.value}).`]);
  }
  for (const input of [summary, day, start, end]) champErreur(input);
  for (const [input, message] of errors) champErreur(input, message);
  errors[0]?.[0].focus();
  return errors.length === 0;
}

$("#agenda-new-form").addEventListener("submit", async (event) => {
  event.preventDefault();
  const form = event.currentTarget;
  const submit = form.querySelector("button[type=submit]");
  if (submit.disabled || !agendaNewCheck()) return;
  const [summaryInput, dayInput, startInput, endInput, allDay] = agendaNewFields();
  const day = dayInput.value;
  const start = startInput.value;
  const end = endInput.value;
  const data = { entity_id: "calendar.maison", summary: summaryInput.value.trim() };
  if (!allDay.checked) {
    data.start_date_time = `${day} ${start}`;
    if (end && end > start) data.end_date_time = `${day} ${end}`;
  } else {
    data.start_date = day;
  }
  const error = $("#agenda-new-error");
  error.hidden = true;
  const restore = boutonEnCours(submit, "Ajout…");
  // Le rendez-vous n'est pas parti : la fenêtre reste ouverte, remplie, pour réessayer.
  let problem = "";
  try {
    const response = await authFetch("/api/services/calendar/create_event", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(data),
    });
    if (!response) problem = "Serveur injoignable : le rendez-vous n'est pas parti. Réessaie.";
    else if (response.status === 401) problem = "Session expirée : reconnecte-toi, puis réessaie.";
    else if (!response.ok) {
      const body = await response.json().catch(() => ({}));
      problem = typeof body.error === "string" ? `${body.error}.` : "Rendez-vous refusé par le serveur.";
    }
  } finally {
    restore();
  }
  if (problem) {
    error.textContent = problem;
    error.hidden = false;
    return;
  }
  agendaNew.initial = agendaNewSnapshot();
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
  const today = el("div", { class: "agenda-card agenda-home-today bg-card rounded-carte [box-shadow:var(--shadow)] py-3 px-3.5 min-w-0" }, ...todayCard(calendars, true));
  const monday = weekStart(new Date());
  const now = new Date();
  const week = el("div", { class: "agenda-home-week grid [grid-template-columns:repeat(7,_minmax(0,_1fr))] gap-1" });
  for (let i = 0; i < 7; i++) {
    const day = addDays(monday, i);
    const bars = eventsOfDay(day, calendars).slice(0, 4).map((e) => {
      const bar = el("span", { class: "agenda-home-bar [width:calc(100%_-_10px)] h-[5px] rounded-trait [background:var(--ev)]", title: `${eventTime(e)} ${e.summary}` });
      bar.style.setProperty("--ev", e.color);
      return bar;
    });
    const cell = el("button", { type: "button", class: `[:where(&)]:[all:unset] cursor-pointer grid [justify-items:center] [align-content:start] gap-1 text-micro text-muted pb-0.5 rounded-puce [&:hover]:bg-card-2 [:not(:where(.o130600))&_b]:[font:600_0.95rem_var(--font-display)] [:not(:where(.o130601))&_b]:text-text [:not(:where(.o130602))&_b]:w-[26px] [:not(:where(.o130603))&_b]:h-[26px] [:not(:where(.o130604))&_b]:[line-height:26px] [&_b]:text-center [:not(:where(.o130606))&_b]:rounded-full [:not(:where(.o130700))&.is-today_b]:bg-accent [:not(:where(.o130701))&.is-today_b]:text-accent-ink agenda-home-day${sameDay(day, now) ? " is-today" : ""}`, "aria-label": `${agendaLongDay.format(day)} : ${bars.length} rendez-vous` },
      el("span", { text: agendaWeekday.format(day) }), el("b", { text: String(day.getDate()) }), ...bars);
    cell.addEventListener("click", () => openAgenda(day, "day"));
    week.append(cell);
  }
  const weekCard = el("div", { class: "agenda-card agenda-home-weekcard bg-card rounded-carte [box-shadow:var(--shadow)] py-3 px-3.5 min-w-0" },
    el("div", { class: "agenda-card-head flex items-center justify-between gap-2 mb-1.5 [:not(:where(.o117500))&_h3]:m-0 [:not(:where(.o117501))&_h3]:text-moyen [:not(:where(.o117502))&_h3]:font-semibold [:not(:where(.o117600))&_h3::first-letter]:uppercase" }, el("h3", { text: "Cette semaine" })), week);
  const scrolled = box.querySelector(".agenda-home-today")?.scrollTop ?? 0;
  box.replaceChildren(today, weekCard);
  // Moins haute que ses rendez-vous (une tablette, avec le lecteur de musique
  // dessous) : la liste du jour défile, avec un fondu au bord, comme les volets.
  const fade = () => {
    const max = today.scrollHeight - today.clientHeight;
    today.classList.toggle("fade-start", today.scrollTop > 1);
    today.classList.toggle("fade-end", today.scrollTop < max - 1);
  };
  today.scrollTop = scrolled;
  today.addEventListener("scroll", fade, { passive: true });
  requestAnimationFrame(fade);
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
