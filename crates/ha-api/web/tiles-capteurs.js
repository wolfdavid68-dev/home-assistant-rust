// Tableau de bord — les tuiles des valeurs : le thermostat, les entrées
// (inputs.yaml), les alertes, les minuteurs, le soleil, l'alarme, les
// capteurs (et l'âge de leur mesure), les machines et les dates.
//
// Un des fichiers de tiles.js, chargé après tiles-meteo.js par index.html.
// Les scripts partagent leurs déclarations. Pas d'innerHTML ici non plus.
"use strict";

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
    tileHead(state, ICONS.toggle, on ? "Activé" : state.state === "off" ? "Désactivé" : statusText(state), knob()),
  );
  toggle.addEventListener("click", () => inputCommand(state, "toggle"));
  return el("div", { id: tileId(state.entity_id), class: `${tileClass(state, "switch")} has-knob` }, toggle);
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
      // Un `number` MQTT n'a pas increment / decrement : on envoie la valeur.
      button.addEventListener("click", () => (domainOf(state) === "number"
        ? inputCommand(state, "set_value", { value: Math.min(max, Math.max(min, (Number.isFinite(value) ? value : min) + (service === "increment" ? step : -step))) })
        : inputCommand(state, service)));
      return button;
    };
    children.push(el("div", { class: "setpoint" }, adjust("decrement", "−", "Diminuer"), shown, adjust("increment", "+", "Augmenter")));
  } else {
    const slider = el("input", {
      type: "range", min: String(min), max: String(max), step: String(step),
      value: String(Number.isFinite(value) ? value : min), "aria-label": `Valeur de ${nameOf(state)}`,
    });
    fillSlider(slider);
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
  const pressed = !Number.isNaN(Date.parse(state.state));
  const head = tileHead(state, ICONS.button, pressed ? "" : "Jamais appuyé");
  if (pressed) markAgo(head.querySelector(".sub"), state.state, "Appuyé il y a ");
  const button = el("button", { type: "button", class: "action-button", text: "Appuyer" });
  button.addEventListener("click", () => inputCommand(state, "press"));
  return el("div", { id: tileId(state.entity_id), class: "tile action" }, head, button);
}

/** Un rappel insistant (étape 69) : calme, en cours (avec « Compris »), ou
 *  compris (avec « Reprendre »). */
function alertTile(state) {
  const a = state.attributes;
  const sent = Number.isInteger(a.rappels) ? a.rappels : 0;
  const next = typeof a.prochain_rappel === "string" ? Date.parse(a.prochain_rappel) : NaN;
  let subtitle;
  let button = null;
  if (state.state === "on") {
    const parts = ["En cours"];
    if (sent > 0) parts.push(`${sent} message${sent > 1 ? "s" : ""}`);
    if (!Number.isNaN(next)) parts.push(`${sent === 0 ? "premier message" : "prochain"} à ${hourMinute.format(new Date(next))}`);
    subtitle = parts.join(" · ");
    if (a.acquittable !== false) {
      button = el("button", { type: "button", class: "action-button", text: "Compris" });
      button.addEventListener("click", () => inputCommand(state, "turn_off"));
    }
  } else if (state.state === "off") {
    subtitle = typeof a.compris_par === "string" ? `Compris par ${a.compris_par}` : "Compris";
    button = el("button", { type: "button", class: "action-button", text: "Reprendre" });
    button.addEventListener("click", () => inputCommand(state, "turn_on"));
  } else {
    subtitle = state.state === "idle" ? "Rien à signaler" : statusText(state);
  }
  return el(
    "div",
    { id: tileId(state.entity_id), class: `${tileClass(state, "reminder")} action${state.state === "on" ? " alert" : ""}` },
    tileHead(state, ICONS.bell, subtitle),
    ...(button ? [button] : []),
  );
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
  if (active) {
    remaining.dataset.finishesAt = a.finishes_at;
    startCountdowns();
  }
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
  const total = seconds(a.duration);
  const left = active ? (Date.parse(a.finishes_at) - Date.now()) / 1000 : seconds(a.remaining) ?? total;
  return el(
    "div",
    { id: tileId(state.entity_id), class: `tile sensor${state.state === "active" ? " on" : ""}` },
    tileHead(state, ICONS.timer, subtitle),
    el("div", { class: "timer-row" }, ring(total, left ?? 0, active ? a.finishes_at : null), remaining),
    el("div", { class: "presets", role: "group", "aria-label": `Commandes de ${nameOf(state)}` }, ...buttons),
  );
}

/** Le cadran d'un thermostat : l'arc jusqu'à la consigne, un point pour la mesure. */
function dial(target, measured, min, max) {
  const r = 52;
  const sweep = 2 * Math.PI * r * 0.75;
  const full = 2 * Math.PI * r;
  const part = (value) => Math.min(1, Math.max(0, (value - min) / Math.max(1e-9, max - min)));
  const picture = svg("svg", { viewBox: "0 0 132 132", role: "img", "aria-label": target === null ? "Consigne inconnue" : `Consigne ${number.format(target)} degrés` });
  picture.append(svg("circle", { cx: 66, cy: 66, r, class: "track", "stroke-dasharray": `${sweep} ${full}`, transform: "rotate(135 66 66)" }));
  if (target !== null) {
    picture.append(svg("circle", { cx: 66, cy: 66, r, class: "arc", "stroke-dasharray": `${sweep * part(target)} ${full}`, transform: "rotate(135 66 66)" }));
  }
  if (measured !== null) {
    const angle = ((135 + 270 * part(measured)) * Math.PI) / 180;
    picture.append(svg("circle", { cx: 66 + r * Math.cos(angle), cy: 66 + r * Math.sin(angle), r: 5, class: "now" }));
  }
  return el("div", { class: "dial" }, picture,
    el("div", { class: "value", text: target === null ? "—" : `${number.format(target)}°` }));
}

/** `1:30:00` → 5400 secondes. */
function seconds(hms) {
  const match = typeof hms === "string" ? /^(\d+):(\d{2}):(\d{2})$/.exec(hms) : null;
  return match ? Number(match[1]) * 3600 + Number(match[2]) * 60 + Number(match[3]) : null;
}

/** Un anneau qui se vide pendant que le minuteur tourne. */
function ring(totalSeconds, leftSeconds, finishesAt) {
  const full = 2 * Math.PI * 20;
  const picture = svg("svg", { viewBox: "0 0 48 48", class: "ring", "aria-hidden": "true" });
  picture.append(svg("circle", { cx: 24, cy: 24, r: 20, class: "track" }));
  const arc = svg("circle", { cx: 24, cy: 24, r: 20, class: "arc", "stroke-dasharray": full });
  const share = totalSeconds ? Math.min(1, Math.max(0, leftSeconds / totalSeconds)) : 0;
  arc.setAttribute("stroke-dashoffset", String(full * (1 - share)));
  if (finishesAt && totalSeconds) {
    arc.dataset.ringFinishes = finishesAt;
    arc.dataset.ringTotal = String(totalSeconds);
  }
  picture.append(arc);
  return picture;
}

// Le compte à rebours ne tourne que si un minuteur en marche est affiché :
// `timerTile` le lance, et il s'arrête de lui-même quand il n'y a plus rien
// à faire avancer. Sans minuteur, la page n'est plus fouillée chaque seconde.
let countdownTimer = 0;

function startCountdowns() {
  if (countdownTimer === 0) countdownTimer = setInterval(tickCountdowns, 1000);
}

function tickCountdowns() {
  const full = 2 * Math.PI * 20;
  const arcs = document.querySelectorAll("[data-ring-finishes]");
  const texts = document.querySelectorAll("[data-finishes-at]");
  if (arcs.length === 0 && texts.length === 0) {
    clearInterval(countdownTimer);
    countdownTimer = 0;
    return;
  }
  for (const arc of arcs) {
    const left = Math.max(0, (Date.parse(arc.dataset.ringFinishes) - Date.now()) / 1000);
    arc.setAttribute("stroke-dashoffset", String(full * (1 - Math.min(1, left / Number(arc.dataset.ringTotal)))));
  }
  for (const node of texts) {
    node.textContent = timerRemaining(node.dataset.finishesAt);
  }
}

const PRESET_LABELS = { confort: "Confort", eco: "Éco", absent: "Absent", hors_gel: "Hors-gel", nuit: "Nuit", programme: "Programme", boost: "Boost" };

function climateTile(state) {
  const a = state.attributes;
  const heating = a.hvac_action === "heating";
  const on = state.state === "heat";
  const target = finite(a.temperature) ? a.temperature : null;
  const step = finite(a.target_temp_step) && a.target_temp_step > 0 ? a.target_temp_step : 0.5;
  const measured = finite(a.current_temperature) ? `${degrees(a.current_temperature)} mesurés` : "température inconnue";
  // Sans « hvac_action » (le chauffage Atlantic ne le dit pas), juste « Allumé ».
  const activity = typeof a.hvac_action === "string" ? (heating ? "Chauffe" : "En veille") : "Allumé";
  // Le ballon d'eau chaude ne donne pas sa température : rien à dire alors.
  const subtitle = [on ? activity : "Arrêté", Object.hasOwn(a, "current_temperature") ? measured : null].filter(Boolean).join(" · ");
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
  const min = finite(a.min_temp) ? a.min_temp : 5;
  const max = finite(a.max_temp) ? a.max_temp : 30;
  const children = [
    tileHead(state, ICONS.flame, subtitle),
    el("div", { class: "setpoint" },
      adjust(-step, "Baisser"),
      dial(target, finite(a.current_temperature) ? a.current_temperature : null, min, max),
      adjust(step, "Monter")),
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

// L'âge des données (étape 101) : sous une valeur, l'heure du relevé ; quand
// un capteur qui change souvent se tait trop longtemps, « dernière donnée il y
// a 40 minutes », en ambre. Un capteur dont la valeur ne bouge pas ne renvoie
// rien : les délais sont donc larges, pour ne pas crier au loup la nuit.
const STALE_FAST_MS = 15 * 60_000;
const STALE_SLOW_MS = 4 * 3_600_000;

/** Au-delà de ce délai sans nouvelle, la valeur est vieille ; 0 : jamais. */
function staleAfter(state) {
  const unit = state.attributes.unit_of_measurement ?? "";
  const kind = state.attributes.device_class ?? "";
  if (["power", "apparent_power", "current", "voltage"].includes(kind) || /^(W|kW|VA|A|V)$/.test(unit)) return STALE_FAST_MS;
  if (["temperature", "humidity"].includes(kind) || unit === "°C") return STALE_SLOW_MS;
  return 0;
}

/** « à 08:15 » aujourd'hui, « le 15/10 à 08:15 » un autre jour. */
function readingTime(iso) {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return "";
  if (new Date().toDateString() === date.toDateString()) return `à ${hourMinute.format(date)}`;
  return `le ${date.toLocaleDateString("fr", { day: "2-digit", month: "2-digit" })} à ${hourMinute.format(date)}`;
}

/** La ligne sous la valeur, et si elle est vieille. */
function freshness(iso, limit) {
  if (limit > 0 && Date.now() - Date.parse(iso) > limit) return { text: `dernière donnée il y a ${ago(iso)}`, stale: true };
  const when = readingTime(iso);
  return { text: when ? `relevé ${when}` : "", stale: false };
}

/** Les tuiles ne sont redessinées qu'à un changement : la minute qui passe
 *  fait vieillir celles qui se taisent. */
function refreshFreshness() {
  for (const sub of document.querySelectorAll(".sub[data-reading]")) {
    const { text, stale } = freshness(sub.dataset.reading, Number(sub.dataset.staleAfter));
    setText(sub, text);
    sub.classList.toggle("is-stale", stale);
  }
}
setInterval(refreshFreshness, 60_000);

function sensorTile(state) {
  if (typeof state.attributes.appareil === "string") return machineTile(state);
  if (state.attributes.device_class === "timestamp") return timestampTile(state);
  if (state.attributes.device_class === "date") return dateTile(state);
  const unit = state.attributes.unit_of_measurement;
  // « 22.2 » s'affiche « 22,2 » (à la française).
  const numeric = /^-?\d+(\.\d+)?$/.test(state.state) ? number.format(Number(state.state)) : state.state;
  const value = el("div", { class: "value", text: numeric });
  if (unit) value.append(el("small", { text: unit }));
  const reading = state.last_updated ?? state.last_changed;
  const limit = staleAfter(state);
  const fresh = freshness(reading, limit);
  const head = tileHead(state, lookup(ICONS, state.attributes.device_class) ?? ICONS.gauge, fresh.text);
  const sub = head.querySelector(".sub");
  sub.dataset.reading = reading;
  sub.dataset.staleAfter = String(limit);
  sub.classList.toggle("is-stale", fresh.stale);
  return el("div", { id: tileId(state.entity_id), class: tileClass(state, "sensor") }, head, value);
}

// Une machine suivie par SmartThings (étape 86) : « En marche », le programme,
// l'heure de fin, la progression ; à la fin, la durée du dernier cycle.
function machineTile(state) {
  const a = state.attributes;
  const running = state.state === "En marche" || state.state === "En pause";
  const parts = [];
  if (running && typeof a.cycle === "string") parts.push(a.cycle);
  if (running && typeof a.fin_heure === "string") parts.push(`fin vers ${a.fin_heure}`);
  if (!running && Number.isFinite(a.dernier_cycle_minutes)) parts.push(`dernier cycle : ${a.dernier_cycle_minutes} min`);
  const head = tileHead(state, ICONS.machine, parts.join(" · "));
  if (!running && parts.length === 0) markAgo(head.querySelector(".sub"), state.last_changed, "il y a ");
  const tile = el(
    "div",
    { id: tileId(state.entity_id), class: `${tileClass(state, "sensor")}${running ? " on" : ""}` },
    head,
    el("div", { class: "value small-value", text: running && Number.isFinite(a.reste_minutes) ? `${state.state} · ${a.reste_minutes} min` : state.state }),
  );
  if (running && Number.isFinite(a.progression)) {
    const bar = el("div", { class: "machine-progress", role: "progressbar", "aria-valuenow": String(a.progression), "aria-valuemin": "0", "aria-valuemax": "100" }, el("span"));
    bar.style.setProperty("--progress", `${Math.min(100, Math.max(0, a.progression))}%`);
    tile.append(bar);
  }
  return tile;
}

// Un jour (prochain jour férié, étape 56) : « Toussaint », « dim. 1 nov., dans 16 jours ».
function dateTile(state) {
  const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(state.state);
  const day = match ? new Date(Number(match[1]), Number(match[2]) - 1, Number(match[3])) : null;
  const name = typeof state.attributes.nom === "string" ? state.attributes.nom : "";
  const left = Number(state.attributes.dans_jours);
  const when = Number.isInteger(left) ? (left === 0 ? "aujourd'hui" : left === 1 ? "demain" : `dans ${left} jours`) : "";
  const shown = day ? day.toLocaleDateString("fr-FR", { weekday: "short", day: "numeric", month: "short" }) : statusText(state);
  return el(
    "div",
    { id: tileId(state.entity_id), class: tileClass(state, "sensor") },
    tileHead(state, ICONS.calendar, [shown, when].filter(Boolean).join(", ")),
    el("div", { class: "value small-value", text: name || shown }),
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
    valid ? markAgo(el("div", { class: "value small-value" }), state.state, "il y a ") : el("div", { class: "value small-value", text: "—" }),
  );
  if (failure) tile.append(el("p", { class: "error climate-info", text: `Dernier essai échoué : ${failure}` }));
  return tile;
}
