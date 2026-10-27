// Tableau de bord — les fenêtres : historique, journal, journal de bord, notifications, énergie.
//
// Un des fichiers du tableau de bord, chargés dans l'ordre par index.html
// (utils.js, app.js, tiles.js, dialogs.js, editor.js, account.js, start.js,
// puis canvas.js, house.js, agenda.js). Ils partagent leurs déclarations :
// une fonction d'un fichier peut appeler celle d'un autre, une fois la page
// chargée.
"use strict";

// ---------------------------------------------------------------- Historique
// Fenêtre d'historique : une courbe pour les valeurs numériques, une frise
// pour les autres (allumé / éteint, présent / absent…). Le dessin est en SVG,
// construit élément par élément : pas de bibliothèque, pas de HTML.
const SVG_NS = "http://www.w3.org/2000/svg";
const CHART = { width: 640, height: 220, left: 52, right: 12, top: 12, bottom: 28 };
// `request` numérote les demandes : seule la dernière a le droit de dessiner.
const historyView = { entityId: null, hours: 24, request: 0 };
const number = new Intl.NumberFormat("fr", { maximumFractionDigits: 1 });
const shortTime = new Intl.DateTimeFormat("fr", { hour: "2-digit", minute: "2-digit" });
const dayTime = new Intl.DateTimeFormat("fr", { weekday: "short", hour: "2-digit", minute: "2-digit" });
const shortDay = new Intl.DateTimeFormat("fr", { day: "numeric", month: "short" });
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
$("#history-logbook").addEventListener("click", () => {
  $("#history").close();
  openLogbook(historyView.entityId);
});
for (const button of document.querySelectorAll("#history .period")) {
  button.addEventListener("click", () => {
    historyView.hours = Number(button.dataset.hours);
    loadHistory().catch(() => {});
  });
}

// Au-delà d'une semaine, l'historique détaillé n'existe plus : on lit les
// statistiques long terme (un point par jour).
const LONG_TERM_HOURS = 168;

async function loadHistory() {
  for (const button of document.querySelectorAll("#history .period")) {
    button.setAttribute("aria-pressed", String(Number(button.dataset.hours) === historyView.hours));
  }
  $("#history-summary").textContent = "Chargement…";
  // Un clic sur « 7 j » puis « 24 h » : la réponse lente de 7 j, arrivée
  // après, n'écrase pas la courbe de 24 h.
  const request = ++historyView.request;
  if (historyView.hours > LONG_TERM_HOURS) return loadStatistics(request);
  const url = `/api/history/${encodeURIComponent(historyView.entityId)}/series?hours=${historyView.hours}`;
  const response = await fetch(url, { headers: { Authorization: `Bearer ${token.get()}` } });
  if (request !== historyView.request) return;
  if (response.status === 401) {
    $("#history").close();
    token.clear();
    live.close();
    return showLogin("Session expirée : reconnecte-toi.");
  }
  if (!response.ok) throw new Error("historique");
  const data = await response.json();
  if (request === historyView.request) drawHistory(data);
}

async function loadStatistics(request) {
  const days = Math.round(historyView.hours / 24);
  const { status, data } = await api("GET", `/api/statistics/${encodeURIComponent(historyView.entityId)}?period=day&days=${days}`);
  if (request !== historyView.request) return;
  if (status !== 200) throw new Error("statistiques");
  const until = Date.now();
  const since = until - days * 86400000;
  const points = (Array.isArray(data.points) ? data.points : [])
    .filter((p) => Number.isFinite(p.mean) && !Number.isNaN(Date.parse(p.start)))
    .map((p) => ({ at: p.start, mean: p.mean, min: p.min, max: p.max, change: p.change }));
  const counter = points.some((p) => Number.isFinite(p.change));
  if (!counter) {
    drawHistory({ since: new Date(since).toISOString(), until: new Date(until).toISOString(), numeric: points, changes: [] });
    if (points.length === 0) {
      $("#history-summary").textContent = "Pas encore de statistiques : elles se calculent chaque heure, pour les capteurs avec une unité.";
    }
    return;
  }
  drawConsumption(points, since, until);
}

/** Un compteur (énergie, eau…) : la consommation de chaque jour, en barres. */
function drawConsumption(points, since, until) {
  const unit = entities.get(historyView.entityId)?.attributes.unit_of_measurement ?? "";
  const x = (time) => CHART.left + ((time - since) / Math.max(1, until - since)) * (CHART.width - CHART.left - CHART.right);
  const chart = svg("svg", { viewBox: `0 0 ${CHART.width} ${CHART.height}`, role: "img", "aria-label": `Consommation de ${$("#history-title").textContent}` });
  for (const [time, anchor] of [[since, "start"], [(since + until) / 2, "middle"], [until, "end"]]) {
    const label = svg("text", { x: x(time), y: CHART.height - 8, class: "axis", "text-anchor": anchor });
    label.textContent = shortDay.format(new Date(time));
    chart.append(label);
  }
  const high = Math.max(...points.map((p) => p.change ?? 0), 1e-9) * 1.1;
  const y = (value) => CHART.top + ((high - value) / high) * (CHART.height - CHART.top - CHART.bottom);
  for (const value of [high / 1.1, high / 2.2]) {
    chart.append(svg("line", { x1: CHART.left, x2: CHART.width - CHART.right, y1: y(value), y2: y(value), class: "grid" }));
    const label = svg("text", { x: CHART.left - 6, y: y(value) + 4, class: "axis", "text-anchor": "end" });
    label.textContent = `${number.format(value)}${unit ? ` ${unit}` : ""}`;
    chart.append(label);
  }
  const width = Math.max(1, ((CHART.width - CHART.left - CHART.right) * 86400000) / Math.max(1, until - since) * 0.75);
  for (const p of points) {
    const value = p.change ?? 0;
    const bar = svg("rect", { x: x(Date.parse(p.at)), y: y(value), width, height: Math.max(0, y(0) - y(value)), class: "bar", rx: 2 });
    const title = svg("title");
    title.textContent = `${dayFormat.format(new Date(p.at))} : ${number.format(value)} ${unit}`;
    bar.append(title);
    chart.append(bar);
  }
  $("#history-chart").replaceChildren(chart);
  const total = points.reduce((sum, p) => sum + (p.change ?? 0), 0);
  $("#history-summary").textContent =
    `${number.format(total)} ${unit} au total · ${number.format(total / Math.max(1, points.length))} ${unit} par jour en moyenne`;
  $("#history-changes").replaceChildren();
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
  const format = historyView.hours > LONG_TERM_HOURS ? shortDay : historyView.hours > 24 ? dayTime : shortTime;
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
  const agreed = (text) => (state ? agree(state, text) : text);
  if (domain === "cover") return agreed(lookup(COVER_LABELS, value) ?? value);
  if (domain === "binary_sensor") {
    const labels = lookup(BINARY_LABELS, state?.attributes.device_class);
    if (labels && (value === "on" || value === "off")) return agreed(value === "on" ? labels[0] : labels[1]);
  }
  if (unit && Number.isFinite(Number(value))) return `${number.format(Number(value))} ${unit}`;
  return agreed(lookup(STATUS_LABELS, value) ?? value);
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
  drawing_set: "dessin de la maison réglé",
  drawing_image: "image du dessin envoyée",
  drawing_image_delete: "image du dessin retirée",
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

// ---------------------------------------------------------------- Journal de bord
// Ce qui s'est passé, en phrases, et pourquoi (GET /api/logbook).
const logbookView = { hours: 24, entityId: "", timer: null };
const dayFormat = new Intl.DateTimeFormat("fr", { weekday: "long", day: "numeric", month: "long" });

function openLogbook(entityId = "") {
  logbookView.entityId = entityId;
  const select = $("#logbook-entity");
  const known = [...entities.values()].sort((a, b) => nameOf(a).localeCompare(nameOf(b), "fr"));
  select.replaceChildren(
    el("option", { value: "", text: "Tous les appareils" }),
    ...known.map((state) => el("option", { value: state.entity_id, text: nameOf(state) })),
  );
  // Un appareil supprimé depuis : on le garde dans la liste.
  if (entityId && !entities.has(entityId)) select.append(el("option", { value: entityId, text: entityId }));
  select.value = entityId;
  $("#logbook").showModal();
  loadLogbook();
}

/** Le jour, en toutes lettres : « aujourd'hui », « hier », « lundi 12 octobre ». */
function dayLabel(date) {
  const startOfDay = (d) => new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
  const days = Math.round((startOfDay(new Date()) - startOfDay(date)) / 86400000);
  return days === 0 ? "Aujourd'hui" : days === 1 ? "Hier" : dayFormat.format(date);
}

async function loadLogbook() {
  for (const button of document.querySelectorAll("[data-logbook-hours]")) {
    button.setAttribute("aria-pressed", String(Number(button.dataset.logbookHours) === logbookView.hours));
  }
  const list = $("#logbook-list");
  const query = new URLSearchParams({ hours: String(logbookView.hours) });
  if (logbookView.entityId) query.set("entity_id", logbookView.entityId);
  // Une session expirée est gérée par `api` (retour à la connexion).
  const { status, data } = await api("GET", `/api/logbook?${query}`).catch(() => ({ status: 0 }));
  if (status !== 200) {
    list.replaceChildren(el("p", { class: "error", text: "Journal de bord indisponible." }));
    return;
  }
  const entries = Array.isArray(data.entries) ? data.entries : [];
  if (entries.length === 0) {
    list.replaceChildren(el("p", { class: "journal-hint", text: "Rien de noté sur cette période." }));
    return;
  }
  const groups = [];
  for (const entry of entries) {
    const at = new Date(entry.at);
    const day = dayLabel(at);
    if (groups.at(-1)?.day !== day) groups.push({ day, items: [] });
    const domain = typeof entry.entity_id === "string" ? entry.entity_id.split(".")[0] : "";
    const item = el(
      "li",
      { class: domain === "" ? "logbook-system" : "" },
      el("time", { datetime: entry.at, text: shortTime.format(at) }),
      el("span", {}, el("strong", { text: entry.name }), " ", entry.message),
    );
    if (typeof entry.cause === "string") item.append(el("small", { text: entry.cause }));
    groups.at(-1).items.push(item);
  }
  list.replaceChildren(
    ...groups.map(({ day, items }) => el("section", {}, el("h3", { text: day }), el("ol", { class: "journal-list" }, ...items))),
  );
}

// Pendant que le journal est ouvert, il se met à jour (une fois par seconde au plus).
function refreshLogbookSoon() {
  if (!$("#logbook").open || logbookView.timer) return;
  logbookView.timer = setTimeout(() => {
    logbookView.timer = null;
    if ($("#logbook").open) loadLogbook();
  }, 1000);
}

$("#open-logbook").addEventListener("click", () => openLogbook());
$("#logbook-close").addEventListener("click", () => $("#logbook").close());
$("#logbook-entity").addEventListener("change", (event) => {
  logbookView.entityId = event.currentTarget.value;
  loadLogbook();
});
for (const button of document.querySelectorAll("[data-logbook-hours]")) {
  button.addEventListener("click", () => {
    logbookView.hours = Number(button.dataset.logbookHours);
    loadLogbook();
  });
}

// ---------------------------------------------------------------- Notifications
// Les entités persistent_notification.* : celles des automatisations, les
// alertes de réparation du serveur (kind = "repair"), et les ordres retenus
// par la garde, à valider (kind = "approval").

function noticeStates() {
  return [...entities.values()].filter((state) => domainOf(state) === "persistent_notification");
}

function renderNotices() {
  const all = noticeStates();
  const active = all
    .filter((state) => state.state === "notifying")
    .sort((a, b) => {
      const rank = (s) => {
        if (s.attributes.kind === "approval") return 0;
        return s.attributes.kind === "repair" ? (lookup({ error: 1, warning: 2 }, s.attributes.severity) ?? 3) : 4;
      };
      return rank(a) - rank(b) || String(b.attributes.created_at).localeCompare(String(a.attributes.created_at));
    });
  const bell = $("#open-notices");
  bell.hidden = active.length === 0;
  bell.classList.toggle("urgent", active.some((s) => s.attributes.kind === "approval" || (s.attributes.kind === "repair" && s.attributes.severity === "error")));
  $("#notice-count").textContent = active.length > 9 ? "9+" : String(active.length);
  bell.setAttribute("aria-label", `Notifications (${active.length})`);
  if (!$("#notices").open) return;
  const list = $("#notice-list");
  list.replaceChildren(...active.map(noticeItem));
  if (active.length === 0) list.replaceChildren(el("li", { class: "auto-empty", text: "Rien à signaler." }));
  const ignored = all.filter((state) => state.state === "ignored").length;
  $("#notice-ignored").hidden = ignored === 0;
  $("#notice-ignored").textContent = ignored === 1
    ? "1 alerte ignorée : elle disparaîtra quand le problème sera réglé."
    : `${ignored} alertes ignorées : elles disparaîtront quand les problèmes seront réglés.`;
}

function noticeItem(state) {
  const repair = state.attributes.kind === "repair";
  const approval = state.attributes.kind === "approval";
  const severity = repair ? String(state.attributes.severity ?? "warning") : approval ? "approval" : "info";
  const head = el("div", { class: "notice-head" }, el("strong", { text: String(state.attributes.title ?? "Notification") }));
  if (repair) head.append(el("span", { class: `badge ${severity === "error" ? "bad" : ""}`, text: "à réparer" }));
  const item = el("li", { class: `notice-item sev-${severity}` }, head, el("p", { class: "notice-message", text: String(state.attributes.message ?? "") }));
  const meta = el("div", { class: "notice-meta" });
  if (typeof state.attributes.created_at === "string") {
    meta.append(el("small", { text: `il y a ${ago(state.attributes.created_at)}` }));
  }
  if (approval && Number.isSafeInteger(state.attributes.approval_id)) {
    meta.append(approvalButtons(state.attributes.approval_id));
  } else if (canControl(state)) {
    const dismiss = el("button", { type: "button", class: "ghost small", text: repair ? "Ignorer" : "Retirer" });
    dismiss.addEventListener("click", () => {
      dismiss.disabled = true;
      callService("persistent_notification", "dismiss", { entity_id: state.entity_id });
    });
    meta.append(dismiss);
  }
  item.append(meta);
  return item;
}

/** « Valider » / « Refuser » un ordre retenu par la garde. */
function approvalButtons(id) {
  const approve = el("button", { type: "button", class: "primary small", text: "Valider" });
  const deny = el("button", { type: "button", class: "ghost small", text: "Refuser" });
  const answer = async (yes) => {
    approve.disabled = true;
    deny.disabled = true;
    const { status, data } = await api("POST", `/api/approvals/${id}`, { approve: yes });
    if (status === 404) flash("Cet ordre n'attend plus : déjà traité, ou oublié.");
    else if (status !== 200) flash(typeof data.error === "string" ? `${data.error}.` : "Réponse impossible.");
    else flash(yes ? "Ordre validé : il est parti." : "Ordre refusé.");
  };
  approve.addEventListener("click", () => answer(true));
  deny.addEventListener("click", () => answer(false));
  return el("span", { class: "approval-actions" }, deny, approve);
}

$("#open-notices").addEventListener("click", () => {
  $("#notices").showModal();
  renderNotices();
});
$("#notices-close").addEventListener("click", () => $("#notices").close());

// ---------------------------------------------------------------- Énergie
// Le bilan vient de GET /api/energy (statistiques long terme, par jour et
// par mois). La puissance et la période tarifaire viennent du Linky, en direct.

const energyView = { range: "30", report: null };
const kwhFormat = new Intl.NumberFormat("fr", { maximumFractionDigits: 1 });
const axisFormat = new Intl.NumberFormat("fr", { maximumFractionDigits: 0 });
const euros = new Intl.NumberFormat("fr", { style: "currency", currency: "EUR" });
const monthName = new Intl.DateTimeFormat("fr", { month: "short" });
/** Couleurs des compteurs : 6 au plus (au-delà, on recommence). */
const SERIES = 6;

async function openEnergy() {
  $("#energy").showModal();
  await loadEnergy();
}

async function loadEnergy() {
  for (const button of document.querySelectorAll("#energy .period")) {
    button.setAttribute("aria-pressed", String(button.dataset.range === energyView.range));
  }
  const days = energyView.range === "year" ? 1 : Number(energyView.range);
  const { status, data } = await api("GET", `/api/energy?days=${days}`);
  if (status !== 200) {
    $("#energy-chart").replaceChildren(el("p", { class: "auto-empty", text: "Bilan indisponible." }));
    return;
  }
  energyView.report = data;
  renderEnergy();
}

for (const button of document.querySelectorAll("#energy .period")) {
  button.addEventListener("click", () => {
    energyView.range = button.dataset.range;
    loadEnergy();
  });
}
$("#open-energy").addEventListener("click", () => openEnergy());
$("#energy-close").addEventListener("click", () => $("#energy").close());

/** La puissance et la période tarifaire du moment (si le Linky est branché). */
function renderEnergyNow() {
  const power = entities.get("sensor.linky_puissance");
  const period = entities.get("sensor.linky_periode");
  const box = $("#energy-now");
  const parts = [];
  if (power && Number.isFinite(Number(power.state))) {
    parts.push(el("span", { class: "energy-power" }, el("strong", { text: kwhFormat.format(Number(power.state)) }), " VA en ce moment"));
  }
  if (period && period.state !== "unavailable") parts.push(el("span", { class: "badge", text: period.state }));
  const tempo = entities.get(TEMPO_TODAY);
  if (tempo && lookup(TEMPO_COLORS, tempo.state)) parts.push(tempoBadge(tempo.state));
  const tomorrow = entities.get(TEMPO_TOMORROW);
  if (tempo && tomorrow) parts.push(el("span", { class: "sub", text: tempoTomorrowText(tomorrow) }));
  box.hidden = parts.length === 0;
  box.replaceChildren(...parts);
}

/** Les périodes affichées : chaque jour (ou mois), même sans donnée. */
function energyPeriods(report) {
  const byDate = new Map();
  for (const row of energyView.range === "year" ? report.months : report.days) byDate.set(row.date, row.kwh);
  const periods = [];
  const today = new Date();
  if (energyView.range === "year") {
    for (let month = 0; month <= today.getMonth(); month++) {
      const date = `${today.getFullYear()}-${String(month + 1).padStart(2, "0")}`;
      periods.push({ date, label: monthName.format(new Date(today.getFullYear(), month, 1)), kwh: byDate.get(date) ?? [] });
    }
  } else {
    for (let back = Number(energyView.range) - 1; back >= 0; back--) {
      const day = new Date(today.getFullYear(), today.getMonth(), today.getDate() - back);
      const date = `${day.getFullYear()}-${String(day.getMonth() + 1).padStart(2, "0")}-${String(day.getDate()).padStart(2, "0")}`;
      periods.push({ date, label: shortDay.format(day), kwh: byDate.get(date) ?? [] });
    }
  }
  return periods;
}

const known = (value) => value !== null && value !== undefined;

function totalTile(title, total) {
  const tile = el("div", { class: "energy-total" }, el("span", { class: "label", text: title }),
    el("strong", { text: `${kwhFormat.format(total.consumed_kwh)} kWh` }));
  if (known(total.cost)) tile.append(el("small", { text: `≈ ${euros.format(total.cost)}` }));
  if (total.produced_kwh > 0) {
    const revenue = known(total.revenue) ? ` · ${euros.format(total.revenue)}` : "";
    tile.append(el("small", { text: `${kwhFormat.format(total.produced_kwh)} kWh produits${revenue}` }));
  }
  return tile;
}

function renderEnergy() {
  const report = energyView.report;
  renderEnergyNow();
  const sources = report.sources ?? [];
  const empty = sources.length === 0;
  for (const id of ["#energy-totals", "#energy-legend", "#energy-details"]) $(id).hidden = empty;
  if (empty) {
    $("#energy-chart").replaceChildren(
      el("p", { class: "auto-empty", text: "Aucun compteur d'énergie pour l'instant. Branche le Linky (HA_TELEINFO), ou indique tes compteurs et leurs prix dans energy.yaml." }),
    );
    $("#energy-note").textContent = "";
    return;
  }
  $("#energy-totals").replaceChildren(
    totalTile("Aujourd'hui", report.today),
    totalTile("Ce mois-ci", report.month),
    totalTile("Cette année", report.year),
  );

  const periods = energyPeriods(report);
  // Le graphique prend la largeur de l'écran : le texte reste lisible sur téléphone.
  const C = { ...CHART, left: 64, width: Math.max(320, Math.min(640, Math.round($("#energy-chart").clientWidth) || 640)) };
  const value = (period, index) => period.kwh[index] ?? 0;
  const consumption = sources.map((s, i) => [s, i]).filter(([s]) => s.kind === "consumption");
  const production = sources.map((s, i) => [s, i]).filter(([s]) => s.kind === "production");
  const sum = (period, list) => list.reduce((total, [, i]) => total + value(period, i), 0);
  const high = Math.max(1e-9, ...periods.map((p) => sum(p, consumption)));
  const low = Math.max(0, ...periods.map((p) => sum(p, production)));
  const top = high * 1.1;
  const bottom = low * 1.1;
  const plot = C.height - C.top - C.bottom;
  const y = (v) => C.top + ((top - v) / (top + bottom)) * plot;
  const step = (C.width - C.left - C.right) / periods.length;
  const width = Math.max(2, step * 0.7);
  const x = (index) => C.left + index * step + (step - width) / 2;
  const chart = svg("svg", { viewBox: `0 0 ${C.width} ${C.height}`, role: "img", "aria-label": "Consommation par période, en kWh" });

  for (const v of [high, high / 2, ...(low > 0 ? [-low] : [])]) {
    chart.append(svg("line", { x1: C.left, x2: C.width - C.right, y1: y(v), y2: y(v), class: "grid" }));
    const label = svg("text", { x: C.left - 6, y: y(v) + 4, class: "axis", "text-anchor": "end" });
    label.textContent = `${axisFormat.format(Math.abs(v))} kWh`;
    chart.append(label);
  }
  chart.append(svg("line", { x1: C.left, x2: C.width - C.right, y1: y(0), y2: y(0), class: "baseline" }));
  const labelAt = new Set([0, Math.floor((periods.length - 1) / 2), periods.length - 1]);
  periods.forEach((period, index) => {
    if (labelAt.has(index)) {
      const anchor = index === 0 ? "start" : index === periods.length - 1 ? "end" : "middle";
      const at = anchor === "start" ? x(index) : anchor === "end" ? x(index) + width : x(index) + width / 2;
      const label = svg("text", { x: at, y: C.height - 8, class: "axis", "text-anchor": anchor });
      label.textContent = period.label;
      chart.append(label);
    }
    // Les compteurs empilés, avec 2 px d'écart entre eux ; la production vers le bas.
    for (const [list, direction] of [[consumption, 1], [production, -1]]) {
      let base = 0;
      for (const [source, i] of list) {
        const v = value(period, i);
        if (v <= 0) continue;
        const from = y(direction * base);
        const to = y(direction * (base + v));
        const gap = base > 0 ? 2 : 0;
        const rect = svg("rect", {
          x: x(index),
          width,
          y: direction > 0 ? Math.min(from, to) : Math.min(from, to) + gap,
          height: Math.max(1, Math.abs(to - from) - gap),
          rx: Math.min(3, width / 2),
          class: `energy-bar series-${(i % SERIES) + 1}`,
        });
        const title = svg("title");
        title.textContent = `${period.label} · ${source.name} : ${kwhFormat.format(v)} kWh`;
        rect.append(title);
        chart.append(rect);
        base += v;
      }
    }
    // Une zone de survol plus grande que la barre, avec le total.
    const hit = svg("rect", { x: C.left + index * step, y: C.top, width: step, height: plot, class: "hit" });
    const title = svg("title");
    const parts = [`${kwhFormat.format(sum(period, consumption))} kWh consommés`];
    if (production.length) parts.push(`${kwhFormat.format(sum(period, production))} kWh produits`);
    title.textContent = `${period.label} : ${parts.join(", ")}`;
    hit.append(title);
    chart.append(hit);
  });
  $("#energy-chart").replaceChildren(chart);

  // Légende : chaque compteur, son total sur la période et son coût.
  $("#energy-legend").replaceChildren(
    ...sources.map((source, i) => {
      const total = periods.reduce((t, p) => t + value(p, i), 0);
      const cost = known(source.price) ? ` · ${euros.format(total * source.price)}` : "";
      return el("li", {},
        el("span", { class: `swatch series-${(i % SERIES) + 1}`, "aria-hidden": "true" }),
        el("span", { text: source.name }),
        el("small", { text: `${kwhFormat.format(total)} kWh${source.kind === "production" ? " produits" : ""}${cost}` }),
      );
    }),
  );

  // Le tableau des chiffres (pour lire sans le graphique).
  const head = el("tr", {}, el("th", { text: energyView.range === "year" ? "Mois" : "Jour" }), ...sources.map((s) => el("th", { text: s.name })));
  const rows = [...periods].reverse().map((period) =>
    el("tr", {}, el("th", { text: period.label }), ...sources.map((_, i) => el("td", { text: known(period.kwh[i]) ? kwhFormat.format(period.kwh[i]) : "–" }))),
  );
  $("#energy-table").replaceChildren(el("thead", {}, head), el("tbody", {}, ...rows));

  const notes = ["Les chiffres sont calculés à la fin de chaque heure : l'heure en cours n'est pas encore comptée."];
  if (known(report.subscription)) notes.push(`Abonnement : ${euros.format(report.subscription)} par mois, en plus.`);
  if (!report.configured) notes.push("Pour ajouter les prix, voir energy.yaml.");
  $("#energy-note").textContent = notes.join(" ");
}
