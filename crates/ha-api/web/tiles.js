// Tableau de bord — l'accueil et les tuiles de chaque sorte d'appareil.
//
// Un des fichiers du tableau de bord, chargés dans l'ordre par index.html
// (utils.js, app.js, tiles.js, dialogs.js, editor.js, account.js, start.js,
// puis canvas.js, house.js, agenda.js). Ils partagent leurs déclarations :
// une fonction d'un fichier peut appeler celle d'un autre, une fois la page
// chargée.
"use strict";

// ---------------------------------------------------------------- Accueil
// La salutation, le résumé, la météo, le plan de la maison et les scènes.
// Redessinés à chaque changement d'état (au plus une fois par image).
let homePending = false;
function scheduleHome() {
  if (homePending) return;
  homePending = true;
  requestAnimationFrame(() => {
    homePending = false;
    renderHome();
  });
}

const fullDate = new Intl.DateTimeFormat("fr", { weekday: "long", day: "numeric", month: "long" });
const ALERT_COVERS = ["garage", "gate", "door"];
const ALERT_SENSORS = ["smoke", "moisture", "gas", "carbon_monoxide", "problem", "safety"];

/** Ce qui demande de l'attention : portail ou garage ouvert, fumée, fuite… */
function alertText(state) {
  const domain = domainOf(state);
  const dc = state.attributes.device_class;
  if (domain === "cover" && ALERT_COVERS.includes(dc) && ["open", "opening"].includes(state.state)) return agree(state, "ouvert");
  if (domain === "binary_sensor" && dc === "garage_door" && state.state === "on") return agree(state, "ouvert");
  if (domain === "binary_sensor" && ALERT_SENSORS.includes(dc) && state.state === "on") return lookup(BINARY_LABELS, dc)?.[0] ?? "alerte";
  if (domain === "sensor" && dc === "alarm" && state.state === "triggered") return "alarme déclenchée";
  if (domain === "lock" && state.state === "jammed") return agree(state, "bloqué");
  if (domain === "vacuum" && state.state === "error") return "en erreur";
  if (domain === "siren" && state.state === "on") return "sonne";
  if (domain === "alarm_control_panel" && state.state === "triggered") return "intrusion";
  if (state.entity_id === VIGILANCE_TODAY && vigilanceRank(state.state) >= 2) return vigilanceSummary(state);
  // Un jour rouge Tempo : l'électricité coûte bien plus cher en heures pleines.
  if ((state.entity_id === TEMPO_TODAY || state.entity_id === TEMPO_TOMORROW) && state.state === "Rouge") return "jour rouge";
  // Un rappel en cours (étape 69), sauf si ce qu'il surveille est déjà
  // signalé (le garage ouvert) : on ne le compte pas deux fois.
  if (domain === "alert" && state.state === "on") {
    const watched = entities.get(state.attributes.surveille);
    return watched !== undefined && domainOf(watched) !== "alert" && alertText(watched) !== null ? null : "rappel en cours";
  }
  return null;
}

function alerts() {
  return [...entities.values()]
    .map((state) => ({ state, text: alertText(state) }))
    .filter((alert) => alert.text !== null);
}

function joinNames(names) {
  return names.length <= 1 ? names.join("") : `${names.slice(0, -1).join(", ")} et ${names.at(-1)}`;
}

function renderHome() {
  const now = new Date();
  setText($("#today"), fullDate.format(now));
  setText($("#clock"), hourMinute.format(now));
  renderNotices();
  if ($("#energy").open) renderEnergyNow();
  // Sous la vue hub, le tableau de bord ne se voit pas : on ne redessine que
  // ce qui peut se voir (une pièce ouverte, le dessin de la maison), puis la
  // vue hub. Le tableau est redessiné en quittant la vue hub (étape 88).
  if (typeof hubShouldShow === "function" && hubShouldShow()) {
    if (typeof roomShown === "function" && roomShown()) renderRoomHead();
    if (typeof house !== "undefined" && house.loading === null && token.get()) loadHouse();
    updateHub();
    return;
  }

  // La salutation, et ce qui demande de l'attention.
  const hour = now.getHours();
  const hello = hour >= 5 && hour < 18 ? "Bonjour" : "Bonsoir";
  const who = account.user ? ` ${account.user.charAt(0).toUpperCase()}${account.user.slice(1)}` : "";
  const found = alerts();
  const greeting = $("#greeting");
  // Un rendez-vous dans l'heure (agenda.js, étape 75).
  const soon = typeof agendaSoon === "function" ? agendaSoon() : null;
  if (found.length === 0) {
    swap(greeting, `${hello}${who}. Tout est calme.`, ...(soon ? [" ", soon, "."] : []));
  } else if (found.length === 1) {
    // Demain est dans la tuile de Tempo aujourd'hui.
    const target = found[0].state.entity_id === TEMPO_TOMORROW ? TEMPO_TODAY : found[0].state.entity_id;
    const link = el("a", { href: `#${tileId(target)}`, text: nameOf(found[0].state) });
    link.addEventListener("click", (event) => {
      event.preventDefault();
      if (view.room !== null) showRoom(null);
      const tile = document.getElementById(tileId(target));
      tile?.scrollIntoView({ behavior: "smooth", block: "center" });
      tile?.querySelector("button")?.focus({ preventScroll: true });
    });
    swap(greeting, `${hello}${who}. Tout est calme, sauf `, link, ".");
  } else {
    swap(greeting, `${hello}${who}. ${found.length} choses demandent ton attention.`);
  }

  // Qui est là.
  const zone = entities.get("zone.home");
  const people = Array.isArray(zone?.attributes.persons) ? zone.attributes.persons.filter((p) => typeof p === "string") : null;
  const parts = [];
  if (found.length > 0) parts.push(found.map((a) => `${nameOf(a.state)} : ${a.text}`).join(" · "));
  if (people) parts.push(people.length === 0 ? "Personne à la maison" : `${joinNames(people)} ${people.length > 1 ? "sont" : "est"} à la maison`);
  // L'agenda de la maison d'abord (les agendas en ligne sont en plus).
  const agenda = entities.get("calendar.maison") ?? [...entities.values()].find((s) => domainOf(s) === "calendar");
  if (agenda) {
    const next = calendarEvents(agenda).find((e) => e.endDate > now && !e.all_day);
    if (next && next.startDate <= now) parts.push(`En ce moment : ${next.summary}`);
    else if (next && agendaDay(next.startDate) === "Aujourd'hui") parts.push(`${next.summary} à ${hourMinute.format(next.startDate)}`);
  }
  setText($("#summary"), parts.join(" · "));

  // La météo, en une pastille.
  const weather = [...entities.values()].find((s) => domainOf(s) === "weather");
  const pill = $("#weather-pill");
  const pillParts = [];
  if (weather) {
    const label = lookup(WEATHER_LABELS, weather.state)?.[0] ?? "";
    const temp = finite(weather.attributes.temperature) ? `${number.format(weather.attributes.temperature)}°` : "";
    pillParts.push(svgIcon(weatherIcon(weather.state)), el("strong", { text: temp }), label);
  }
  // La vigilance, dès le jaune : dans la pastille (même sans la météo).
  const vigilance = entities.get(VIGILANCE_TODAY);
  if (vigilance && vigilanceRank(vigilance.state) >= 1) pillParts.push(vigilanceBadge(vigilance.state));
  pill.hidden = pillParts.length === 0;
  swap(pill, ...pillParts);

  renderPlan();
  renderScenes();
  // Le dessin de la maison, les repères et le cartouche (house.js).
  if (typeof renderHouse === "function") renderHouse();
  // L'agenda, s'il est ouvert (agenda.js).
  if (typeof renderAgendaView === "function") renderAgendaView();
  // La vue hub de la tablette murale (hub.js).
  if (typeof updateHub === "function") updateHub();
}

/** Ce qu'on sait d'une pièce : lumières, température, alertes. */
function roomInfo(area) {
  const states = area.entities.map((e) => entities.get(e.entity_id)).filter((s) => s !== undefined);
  const lights = states.filter((s) => isLight(s) && s.state === "on").length;
  const temperature = states
    .map((s) => (domainOf(s) === "climate" ? s.attributes.current_temperature
      : s.attributes.device_class === "temperature" ? Number(s.state) : NaN))
    .find((t) => finite(t) || (typeof t === "number" && Number.isFinite(t)));
  const alert = states.map(alertText).find((text) => text !== null);
  const open = states.filter((s) => domainOf(s) === "binary_sensor"
    && ["door", "window", "opening"].includes(s.attributes.device_class) && s.state === "on").length;
  const heating = states.some((s) => domainOf(s) === "climate" && s.attributes.hvac_action === "heating");
  const humidity = states
    .map((s) => (domainOf(s) === "climate" ? s.attributes.current_humidity
      : s.attributes.device_class === "humidity" ? Number(s.state) : NaN))
    .find((h) => finite(h));
  return { count: states.length, lights, temperature, humidity, alert, open, heating };
}

function renderPlan() {
  const card = $("#plan");
  const listed = shownAreas();
  // Mise en page « tableau » : les pièces sont dans leurs onglets, en haut
  // (étape 90) ; la colonne de gauche ne garde que l'essentiel.
  card.hidden = listed.length === 0 || isBoard();
  if (card.hidden) return;
  let lit = 0;
  let warnings = 0;
  const temps = [];
  const rooms = listed.map((area) => {
    const info = roomInfo(area);
    lit += info.lights;
    if (info.alert) warnings += 1;
    if (Number.isFinite(info.temperature)) temps.push(info.temperature);
    const detail = info.alert ? info.alert
      : info.lights > 0 ? `${info.lights} lumière${info.lights > 1 ? "s" : ""}`
      : info.open > 0 ? `${info.open} ouverture${info.open > 1 ? "s" : ""}`
      : info.heating ? "chauffe" : "";
    const classes = ["room"];
    if (info.count >= 4) classes.push("big");
    if (info.alert) classes.push("alert");
    else if (info.lights > 0) classes.push("lit");
    else if (info.heating) classes.push("heat");
    const room = el(
      "button",
      { type: "button", class: classes.join(" "), "aria-label": [area.name, detail, Number.isFinite(info.temperature) ? `${number.format(info.temperature)} degrés` : ""].filter(Boolean).join(", ") },
      el("span", { class: "room-name", text: area.name }),
      el("span", { class: "room-foot" },
        el("span", { text: detail }),
        Number.isFinite(info.temperature) ? el("span", { class: "room-temp", text: `${number.format(info.temperature)}°` }) : el("span")),
    );
    // L'humidité : affichée par le style « plan » (« 25,1° | 49 % »).
    if (finite(info.humidity)) room.querySelector(".room-temp")?.append(el("span", { class: "room-hum", text: ` | ${number.format(Math.round(info.humidity))} %` }));
    // La mise en page « tableau » : une ligne, avec l'icône de la pièce.
    const measures = [
      Number.isFinite(info.temperature) ? `${number.format(info.temperature)} °C` : "",
      finite(info.humidity) ? `${number.format(info.humidity)} %` : "",
    ].filter(Boolean).join(" | ");
    room.append(el("span", { class: "room-line" },
      el("span", { class: "room-icon" }, svgIcon(roomIcon(area.name))),
      el("span", { class: "room-text" },
        el("span", { class: "room-line-name", text: area.name }),
        el("span", { class: "room-line-info", text: [measures, detail].filter(Boolean).join(" · ") || "—" }))));
    // Mise en page « tableau » : la pièce s'ouvre en fenêtre, par-dessus la
    // maquette (house.js) ; sinon, seule, à la place des sections.
    room.addEventListener("click", () => (isBoard() ? openRoom(area.name) : showRoom(area.name)));
    return room;
  });
  const legend = el("div", { class: "legend" });
  if (lit > 0) legend.append(el("span", {}, el("i"), `${lit} lumière${lit > 1 ? "s" : ""} allumée${lit > 1 ? "s" : ""}`));
  if (warnings > 0) legend.append(el("span", {}, el("i", { class: "alert" }), `${warnings} alerte${warnings > 1 ? "s" : ""}`));
  if (temps.length > 0) {
    const mean = temps.reduce((sum, t) => sum + t, 0) / temps.length;
    legend.append(el("span", {}, el("b", { text: `${number.format(mean)}°` }), " en moyenne"));
  }
  swap(card,
    el("div", { class: "plan-head" }, el("h2", { text: "La maison" })),
    el("div", { class: "plan" }, ...rooms),
    legend,
  );
}

/** L'icône d'une pièce, d'après son nom. */
function roomIcon(name) {
  const plain = name.toLowerCase().normalize("NFD").replace(/[\u0300-\u036f]/g, "");
  const rules = [
    [/chambre|dortoir/, ICONS.bed], [/salon|sejour|living|tv/, ICONS.sofa], [/cuisine/, ICONS.pot],
    [/bain|douche|sdb/, ICONS.bath], [/escalier/, ICONS.stairs], [/wc|toilette/, ICONS.toilet],
    [/garage|parking|abri/, ICONS.car], [/bureau|lab/, ICONS.desk], [/buanderie|lingerie|cellier/, ICONS.washer],
    [/jardin|terrasse|piscine|exterieur|cour/, ICONS.tree], [/entree|couloir|hall|palier/, ICONS.door],
  ];
  return rules.find(([pattern]) => pattern.test(plain))?.[1] ?? ICONS.home;
}

function renderScenes() {
  const list = [...entities.values()]
    .filter((s) => ["scene", "script"].includes(domainOf(s)) && canControl(s))
    .sort((a, b) => nameOf(a).localeCompare(nameOf(b), "fr"));
  const nav = $("#scenes");
  nav.hidden = list.length === 0;
  swap(nav, ...list.map((state) => {
    const isScript = domainOf(state) === "script";
    const running = isScript && state.state === "on";
    const button = el(
      "button",
      { type: "button", id: `scene-${state.entity_id}`, class: running ? "scene on" : "scene", "aria-pressed": isScript ? String(running) : "false" },
      svgIcon(isScript ? (running ? ICONS.stop : ICONS.play) : ICONS.sparkle),
      el("span", { text: nameOf(state) }),
    );
    button.addEventListener("click", () => {
      button.classList.add("pending");
      setTimeout(() => button.classList.remove("pending"), 5000);
      callService(domainOf(state), running ? "turn_off" : "turn_on", { entity_id: state.entity_id });
    });
    return button;
  }));
}

// L'heure, chaque minute (et la salutation avec).
setInterval(() => { if (!$("#dashboard-view").hidden) renderHome(); }, 30_000);

function renderTile(state) {
  const tile = renderTileContent(state);
  if (!canControl(state)) makeReadOnly(tile);
  if (account.permissions.history !== false) tile.append(historyButton(state));
  return tile;
}

/** Ce compte peut-il commander cet appareil ? Les règles viennent du serveur
 *  (GET /api/, `permissions.controls`, `null` = tout) ; il les vérifie de
 *  toute façon à chaque commande. */
function canControl(state) {
  const controls = account.permissions.controls;
  if (controls === null || typeof controls !== "object") return true;
  const domain = domainOf(state);
  if (!Array.isArray(controls.domains) || !controls.domains.includes(domain)) return false;
  const forbidden = Array.isArray(controls.forbidden_cover_classes) ? controls.forbidden_cover_classes : [];
  if (domain === "cover" && forbidden.includes(state.attributes.device_class)) return false;
  // Un groupe : chacun de ses membres aussi (comme le vérifie le serveur).
  const members = Array.isArray(state.attributes.entity_id) ? state.attributes.entity_id : [];
  return members.every((id) => {
    const member = entities.get(id);
    return member !== undefined && member !== state && canControl(member);
  });
}

/** Une tuile en lecture seule : l'état reste lisible, les commandes
 *  disparaissent (plutôt qu'un refus au moment d'appuyer). */
function makeReadOnly(tile) {
  tile.classList.add("readonly");
  for (const control of tile.querySelectorAll("button, input, select")) control.disabled = true;
}

function renderTileContent(state) {
  if (state.entity_id === VIGILANCE_TODAY) return vigilanceTile(state);
  if (state.entity_id === TEMPO_TODAY) return tempoTile(state);
  switch (domainOf(state)) {
    case "light": return isOutlet(state) ? switchTile(state) : lightTile(state);
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
    case "alert": return alertTile(state);
    case "switch":
      if (state.entity_id === SHEDDING) return switchTile(state, ICONS.bolt, sheddingLabels(state));
      if (state.entity_id === SHADE) return switchTile(state, ICONS.blinds, shadeLabels(state));
      return switchTile(state);
    case "siren": return switchTile(state, ICONS.siren, { on: "Sonne !", off: "Au repos" });
    case "humidifier": return humidifierTile(state);
    case "lock": return lockTile(state);
    case "valve": return valveTile(state);
    case "vacuum": return vacuumTile(state);
    case "media_player": return mediaTile(state);
    case "tts": return announceTile(state);
    case "water_heater": return waterHeaterTile(state);
    case "todo": return todoTile(state);
    case "camera": return cameraTile(state);
    case "fan": return fanTile(state);
    case "counter": return counterTile(state);
    case "input_text": return textTile(state);
    case "input_datetime": return dateTimeTile(state);
    case "schedule": return scheduleTile(state);
    case "button": return inputButtonTile(state);
    case "number": return inputNumberTile(state);
    case "select": return inputSelectTile(state);
    case "text": return textTile(state);
    case "event": return eventTile(state);
    case "alarm_control_panel": return alarmPanelTile(state);
    case "calendar": return calendarTile(state);
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
  fillSlider(slider);
  slider.addEventListener("change", () => {
    markPending(state.entity_id);
    callService("light", "turn_on", { entity_id: state.entity_id, brightness: Number(slider.value) });
  });
  const toggle = el(
    "button",
    { class: "tile-toggle", type: "button", "aria-pressed": String(on) },
    tileHead(state, ICONS.bulb, on ? agree(state, `Allumé · ${Math.round((brightness / 255) * 100)}\u202f%`) : statusText(state), knob()),
  );
  toggle.addEventListener("click", () => {
    markPending(state.entity_id);
    callService("light", "toggle", { entity_id: state.entity_id });
  });
  return el(
    "div",
    { id: tileId(state.entity_id), class: `${tileClass(state, "light")} has-knob` },
    toggle,
    on ? slider : el("span"),
  );
}

function coverTile(state) {
  const position = state.attributes.current_position;
  const subtitle =
    typeof position === "number"
      ? agree(state, position >= 100 ? "Ouvert" : position <= 0 ? "Fermé" : `Ouvert à ${Math.round(position)}\u202f%`)
      : agree(state, lookup(COVER_LABELS, state.state) ?? statusText(state));
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
    fillSlider(slider);
    slider.addEventListener("change", () => command("set_cover_position", { position: Number(slider.value) }));
    children.push(slider);
  }
  return el("div", { id: tileId(state.entity_id), class: tileClass(state, "cover") }, ...children);
}

function switchTile(state, icon = ICONS.plug, labels = null) {
  const on = state.state === "on";
  const text = (labels && lookup(labels, state.state)) ?? statusText(state);
  // Le bouton est à l'intérieur de la tuile (et non la tuile entière), pour
  // que le bouton « historique » puisse être à côté : un bouton ne peut pas
  // en contenir un autre.
  const toggle = el(
    "button",
    { class: "tile-toggle", type: "button", "aria-pressed": String(on) },
    tileHead(state, icon, text, knob()),
  );
  toggle.addEventListener("click", () => {
    markPending(state.entity_id);
    callService(domainOf(state), "toggle", { entity_id: state.entity_id });
  });
  return el("div", { id: tileId(state.entity_id), class: `${tileClass(state, "switch")} has-knob` }, toggle);
}

// Le délestage (étape 62) : ce qu'il a coupé, sous son nom.
const SHEDDING = "switch.delestage";
function sheddingLabels(state) {
  const cut = Array.isArray(state.attributes.coupes) ? state.attributes.coupes : [];
  const names = cut
    .map((item) => (typeof item?.entity_id === "string" ? entities.get(item.entity_id) : undefined))
    .filter((device) => device !== undefined)
    .map((device) => nameOf(device));
  let on = "Armé, rien de coupé";
  if (typeof state.attributes.erreur === "string") on = "Armé, limite inconnue";
  else if (names.length > 0) on = `Coupé : ${joinNames(names)}`;
  return { on, off: "Désarmé" };
}

// La protection solaire (étape 70) : les volets qu'elle a baissés, et
// pourquoi elle attend.
const SHADE = "switch.protection_solaire";
function shadeLabels(state) {
  const a = state.attributes;
  const names = (Array.isArray(a.baisses) ? a.baisses : [])
    .map((item) => (typeof item?.entity_id === "string" ? entities.get(item.entity_id) : undefined))
    .filter((device) => device !== undefined)
    .map((device) => nameOf(device));
  let on = "Armée, soleil couché";
  if (names.length > 0) on = `Baissé : ${joinNames(names)}`;
  else if (typeof a.soleil_hauteur === "number" && a.soleil_hauteur > 0) {
    if (a.ensoleille === false) on = "Armée, pas de soleil";
    else if (a.chaleur === false) on = "Armée, pas assez chaud";
    else if (Array.isArray(a.soleil_sur) && a.soleil_sur.length === 0) on = "Armée, soleil sur aucune façade";
    else on = "Armée";
  }
  // Les volets bougés à la main, laissés tranquilles un moment.
  const paused = (Array.isArray(a.en_pause) ? a.en_pause : [])
    .map((id) => entities.get(id))
    .filter((device) => device !== undefined)
    .map((device) => nameOf(device));
  if (paused.length > 0) on = `${on} · à la main : ${joinNames(paused)}`;
  return { on, off: "Désarmée" };
}

// ----- Serrure, vanne, aspirateur, humidificateur, chauffe-eau (MQTT)

/** Un bouton qui envoie une commande à l'appareil de la tuile. */
function commandButton(state, service, label, options = {}) {
  const button = el(
    "button",
    { type: "button", class: options.icon ? "round" : "action-button", "aria-label": `${label} ${nameOf(state)}` },
    ...(options.icon ? [svgIcon(options.icon)] : [label]),
  );
  if (options.pressed !== undefined) button.setAttribute("aria-pressed", String(options.pressed));
  button.addEventListener("click", () => {
    markPending(state.entity_id);
    callService(domainOf(state), service, { entity_id: state.entity_id, ...options.data });
  });
  return button;
}

function lockTile(state) {
  const locked = state.state === "locked";
  const text = agree(state, lookup(LOCK_LABELS, state.state) ?? statusText(state));
  const buttons = [locked
    // Déverrouiller : le serveur redemande le mot de passe (voir callService).
    ? commandButton(state, "unlock", "Déverrouiller")
    : commandButton(state, "lock", "Verrouiller")];
  if (state.attributes.supports_open === true) buttons.push(commandButton(state, "open", "Ouvrir la porte"));
  const classes = [tileClass(state, "lock")];
  if (locked) classes.push("on");
  if (state.state === "jammed") classes.push("alert");
  return el(
    "div",
    { id: tileId(state.entity_id), class: classes.join(" ") },
    tileHead(state, locked ? ICONS.lock : ICONS.unlock, text),
    el("div", { class: "tile-actions" }, ...buttons),
  );
}

function valveTile(state) {
  const text = agree(state, lookup(COVER_LABELS, state.state) ?? statusText(state));
  return el(
    "div",
    { id: tileId(state.entity_id), class: tileClass(state, "cover") + (state.state === "open" ? " on" : "") },
    tileHead(state, ICONS.valve, text),
    el("div", { class: "cover-buttons" },
      commandButton(state, "open_valve", "Ouvrir", { icon: ICONS.up }),
      commandButton(state, "close_valve", "Fermer", { icon: ICONS.down })),
  );
}

function vacuumTile(state) {
  const a = state.attributes;
  const battery = finite(a.battery_level) ? `${Math.round(a.battery_level)}\u202f%` : "";
  const text = [lookup(VACUUM_LABELS, state.state) ?? statusText(state), battery].filter(Boolean).join(" · ");
  const cleaning = state.state === "cleaning";
  const classes = [tileClass(state, "vacuum")];
  if (cleaning) classes.push("on");
  if (state.state === "error") classes.push("alert");
  return el(
    "div",
    { id: tileId(state.entity_id), class: classes.join(" ") },
    tileHead(state, ICONS.vacuum, text),
    el("div", { class: "cover-buttons" },
      cleaning
        ? commandButton(state, "pause", "Mettre en pause", { icon: ICONS.pause })
        : commandButton(state, "start", "Lancer le ménage", { icon: ICONS.play }),
      commandButton(state, "stop", "Arrêter", { icon: ICONS.stop }),
      commandButton(state, "return_to_base", "Renvoyer à sa base", { icon: ICONS.home })),
  );
}

// ----- Enceintes et annonces (étape 73)

const MEDIA_LABELS = { playing: "Lecture", paused: "En pause", idle: "À l'arrêt" };

function mediaTile(state) {
  const a = state.attributes;
  const playing = state.state === "playing";
  const known = ["playing", "paused", "idle"].includes(state.state);
  const what = [a.media_title, a.media_artist].filter((t) => typeof t === "string" && t !== "").join(" — ");
  const channel = typeof a.media_channel === "string" && a.media_channel !== "" ? a.media_channel : "";
  const group = (Array.isArray(a.group_members) ? a.group_members : [])
    .map((id) => entities.get(id))
    .filter(Boolean)
    .map(nameOf);
  const text = [
    known ? MEDIA_LABELS[state.state] : statusText(state),
    state.state !== "idle" ? [channel, what].filter(Boolean).join(" · ") : "",
    group.length > 0 ? `avec ${joinNames(group)}` : "",
  ].filter(Boolean).join(" · ");
  const volume = finite(a.volume_level) ? Math.round(a.volume_level * 100) : 0;
  const muted = a.is_volume_muted === true;
  const slider = el("input", {
    type: "range", min: "0", max: "100", value: String(volume),
    "aria-label": `Volume de ${nameOf(state)}`,
  });
  fillSlider(slider);
  slider.addEventListener("change", () => {
    markPending(state.entity_id);
    callService("media_player", "volume_set", { entity_id: state.entity_id, volume_level: Number(slider.value) / 100 });
  });
  const mute = commandButton(state, "volume_mute", muted ? "Remettre le son" : "Couper le son", {
    icon: muted ? ICONS.muted : ICONS.speaker, pressed: muted, data: { is_volume_muted: !muted },
  });
  const classes = [tileClass(state, "media")];
  if (playing) classes.push("on");
  return el(
    "div",
    { id: tileId(state.entity_id), class: classes.join(" ") },
    tileHead(state, ICONS.speaker, text),
    el("div", { class: "cover-buttons" },
      commandButton(state, "media_previous_track", "Morceau précédent", { icon: ICONS.previous }),
      playing
        ? commandButton(state, "media_pause", "Mettre en pause", { icon: ICONS.pause })
        : commandButton(state, "media_play", "Lancer la lecture", { icon: ICONS.play }),
      commandButton(state, "media_next_track", "Morceau suivant", { icon: ICONS.next })),
    el("div", { class: "media-volume" }, mute, slider, el("span", { class: "media-volume-value", text: `${volume}\u202f%` })),
  );
}

// Le texte en cours d'écriture survit au rafraîchissement de la tuile.
const announceDraft = { message: "", target: "", volume: "" };

function announceTile(state) {
  const a = state.attributes;
  const speakers = (Array.isArray(a.enceintes) ? a.enceintes : []).filter((id) => entities.has(id));
  const voice = typeof a.voix === "string" ? a.voix : "";
  const hasVoice = voice !== "" && !voice.startsWith("aucune");
  const last = typeof a.dernier_message === "string" && !Number.isNaN(Date.parse(state.state))
    ? `Dernière il y a ${ago(state.state)} : « ${a.dernier_message} »`
    : hasVoice ? `Voix : ${voice}` : "Pas de voix installée : carillon seulement";
  const input = el("input", {
    type: "text", maxlength: "500", autocomplete: "off", value: announceDraft.message,
    placeholder: hasVoice ? "Le repas est prêt" : "Installe Piper pour parler (docs/sonos.md)",
    "aria-label": "Message à annoncer",
  });
  input.disabled = !hasVoice;
  input.addEventListener("input", () => { announceDraft.message = input.value; });
  const target = el("select", { "aria-label": "Enceintes" });
  target.append(el("option", { value: "", text: "Partout" }));
  for (const id of speakers) target.append(el("option", { value: id, text: nameOf(entities.get(id)) }));
  target.value = speakers.includes(announceDraft.target) ? announceDraft.target : "";
  target.addEventListener("change", () => { announceDraft.target = target.value; });
  const volume = el("select", { "aria-label": "Volume de l'annonce" });
  for (const [value, text] of [["", "Volume actuel"], ["30", "30\u202f%"], ["50", "50\u202f%"], ["70", "70\u202f%"]]) {
    volume.append(el("option", { value, text }));
  }
  volume.value = announceDraft.volume;
  volume.addEventListener("change", () => { announceDraft.volume = volume.value; });
  const send = (withMessage) => {
    const data = {
      entity_id: state.entity_id,
      media_player_entity_id: target.value === "" ? speakers : target.value,
    };
    if (withMessage) data.message = input.value;
    if (volume.value !== "") data.volume = Number(volume.value);
    markPending(state.entity_id);
    callService("tts", "speak", data);
  };
  const form = el("form", { class: "add-form announce-form" }, input,
    el("button", { type: "submit", class: "action-button", text: "Annoncer" }));
  form.querySelector("button").disabled = !hasVoice;
  form.addEventListener("submit", (event) => {
    event.preventDefault();
    if (input.value.trim() === "" || speakers.length === 0) return;
    send(true);
    input.value = "";
    announceDraft.message = "";
  });
  const chime = el("button", { type: "button", class: "ghost small", text: "Carillon seul" });
  chime.addEventListener("click", () => { if (speakers.length > 0) send(false); });
  return el(
    "div",
    { id: tileId(state.entity_id), class: "tile sensor announce" },
    tileHead(state, ICONS.megaphone, last),
    form,
    el("div", { class: "announce-options" }, target, volume, chime),
  );
}

// ----- Compteur, date et heure, plage horaire (étape 30)

function counterTile(state) {
  const a = state.attributes;
  const value = Number(state.state);
  const shown = el("div", { class: "setpoint" });
  const adjust = (service, text, label) => {
    const button = el("button", { type: "button", class: "round", "aria-label": `${label} ${nameOf(state)}`, text });
    button.addEventListener("click", () => inputCommand(state, service));
    return button;
  };
  shown.append(adjust("decrement", "−", "Diminuer"), el("span", { class: "setpoint-value", text: Number.isFinite(value) ? number.format(value) : "–" }), adjust("increment", "+", "Augmenter"));
  const reset = el("button", { type: "button", class: "ghost small", text: "Remettre à zéro" });
  reset.addEventListener("click", () => inputCommand(state, "reset"));
  const bounds = [Number.isFinite(a.minimum) && `min ${a.minimum}`, Number.isFinite(a.maximum) && `max ${a.maximum}`].filter(Boolean).join(" · ");
  return el("div", { id: tileId(state.entity_id), class: "tile sensor" }, tileHead(state, ICONS.sliders, bounds || "Compteur"), shown, reset);
}

function dateTimeTile(state) {
  const a = state.attributes;
  const hasDate = a.has_date === true;
  const hasTime = a.has_time === true;
  const known = state.state !== "unknown";
  const [datePart, timePart] = hasDate && hasTime ? state.state.split(" ") : hasDate ? [state.state, ""] : ["", state.state];
  const inputs = [];
  const date = el("input", { type: "date", "aria-label": `Date de ${nameOf(state)}` });
  const time = el("input", { type: "time", "aria-label": `Heure de ${nameOf(state)}` });
  if (hasDate) { if (known) date.value = datePart; inputs.push(date); }
  if (hasTime) { if (known) time.value = (timePart ?? "").slice(0, 5); inputs.push(time); }
  const send = () => {
    const data = {};
    if (hasDate && date.value) data.date = date.value;
    if (hasTime && time.value) data.time = time.value;
    if (Object.keys(data).length > 0) inputCommand(state, "set_datetime", data);
  };
  for (const input of inputs) input.addEventListener("change", send);
  const shown = !known ? "Pas encore réglé"
    : hasDate && hasTime ? `${shortDate.format(new Date(`${datePart}T12:00:00`))} à ${timePart.slice(0, 5)}`
    : hasDate ? shortDate.format(new Date(`${datePart}T12:00:00`))
    : state.state.slice(0, 5);
  return el("div", { id: tileId(state.entity_id), class: "tile sensor" }, tileHead(state, ICONS.timer, shown), el("div", { class: "add-when" }, ...inputs));
}

function scheduleTile(state) {
  const on = state.state === "on";
  const next = typeof state.attributes.next_event === "string" ? new Date(state.attributes.next_event) : null;
  const when = next && !Number.isNaN(next.getTime())
    ? ` · ${on ? "jusqu'à" : "reprend"} ${agendaDay(next) === "Aujourd'hui" ? "" : `${agendaDay(next).toLowerCase()} `}${hourMinute.format(next)}`
    : "";
  return el(
    "div",
    { id: tileId(state.entity_id), class: tileClass(state, "binary") },
    tileHead(state, ICONS.calendar, `${on ? "En cours" : "En pause"}${when}`),
  );
}

// ----- Ventilateur, texte, télécommande, alarme (MQTT)

function fanTile(state) {
  const a = state.attributes;
  const on = state.state === "on";
  const speed = finite(a.percentage) && on ? ` · ${Math.round(a.percentage)}\u202f%` : "";
  const toggle = el(
    "button",
    { class: "tile-toggle", type: "button", "aria-pressed": String(on) },
    tileHead(state, ICONS.fan, `${statusText(state)}${speed}`, knob()),
  );
  toggle.addEventListener("click", () => inputCommand(state, "toggle"));
  const children = [toggle];
  if (finite(a.percentage)) {
    const slider = el("input", { type: "range", min: "0", max: "100", step: "1", value: String(Math.round(a.percentage)), "aria-label": `Vitesse de ${nameOf(state)}` });
    fillSlider(slider);
    slider.addEventListener("change", () => inputCommand(state, "set_percentage", { percentage: Number(slider.value) }));
    children.push(slider);
  }
  const presets = (Array.isArray(a.preset_modes) ? a.preset_modes : []).filter((p) => typeof p === "string");
  if (presets.length > 0) {
    children.push(el("div", { class: "presets", role: "group", "aria-label": "Modes" }, ...presets.map((preset) => {
      const label = lookup(PRESET_LABELS, preset) ?? preset.replaceAll("_", " ");
      const button = el("button", { type: "button", class: "period", text: label.charAt(0).toUpperCase() + label.slice(1), "aria-pressed": String(a.preset_mode === preset) });
      button.addEventListener("click", () => inputCommand(state, "set_preset_mode", { preset_mode: preset }));
      return button;
    })));
  }
  if (typeof a.oscillating === "boolean") {
    const oscillate = el("button", { type: "button", class: "ghost small", "aria-pressed": String(a.oscillating), text: a.oscillating ? "Oscillation : oui" : "Oscillation : non" });
    oscillate.addEventListener("click", () => inputCommand(state, "oscillate", { oscillating: !a.oscillating }));
    children.push(oscillate);
  }
  return el("div", { id: tileId(state.entity_id), class: `${tileClass(state, "switch")} has-knob` }, ...children);
}

function textTile(state) {
  const a = state.attributes;
  const secret = a.mode === "password";
  const max = Number.isInteger(a.max) ? a.max : 255;
  const input = el("input", { type: secret ? "password" : "text", maxlength: String(max), "aria-label": `Nouvelle valeur de ${nameOf(state)}`, autocomplete: "off" });
  if (!secret && !["unknown", "unavailable"].includes(state.state)) input.value = state.state;
  const form = el("form", { class: "add-form" }, input, el("button", { type: "submit", class: "action-button", text: "Envoyer" }));
  form.addEventListener("submit", (event) => {
    event.preventDefault();
    inputCommand(state, "set_value", { value: input.value });
    if (secret) input.value = "";
  });
  // Un texte « mot de passe » n'est jamais affiché.
  const shown = secret ? "••••••" : ["unknown", "unavailable"].includes(state.state) ? statusText(state) : state.state;
  return el("div", { id: tileId(state.entity_id), class: "tile sensor" }, tileHead(state, ICONS.list, shown), form);
}

function eventTile(state) {
  const kind = typeof state.attributes.event_type === "string" ? state.attributes.event_type : null;
  const when = Number.isNaN(Date.parse(state.state)) ? "" : ` · il y a ${ago(state.state)}`;
  return el(
    "div",
    { id: tileId(state.entity_id), class: tileClass(state, "binary") },
    tileHead(state, ICONS.button, kind ? `« ${kind} »${when}` : "Aucun appui reçu"),
  );
}

const ALARM_PANEL_LABELS = {
  disarmed: "Désarmée", armed_home: "Armée (présence)", armed_away: "Armée (absence)",
  armed_night: "Armée (nuit)", armed_vacation: "Armée (vacances)", armed_custom_bypass: "Armée (personnalisé)",
  pending: "Délai d'entrée…", triggered: "Intrusion !", arming: "Armement…", disarming: "Désarmement…",
};
const ALARM_PANEL_ACTIONS = [
  ["arm_away", "alarm_arm_away", "Absence"], ["arm_home", "alarm_arm_home", "Présence"],
  ["arm_night", "alarm_arm_night", "Nuit"], ["arm_vacation", "alarm_arm_vacation", "Vacances"],
];

function alarmPanelTile(state) {
  const features = Array.isArray(state.attributes.supported_features) ? state.attributes.supported_features : [];
  const armed = state.state.startsWith("armed") || state.state === "triggered" || state.state === "pending";
  const buttons = [];
  if (state.state !== "disarmed") {
    // Désarmer : le serveur redemande le mot de passe (voir callService).
    buttons.push(commandButton(state, "alarm_disarm", "Désarmer"));
  }
  for (const [feature, service, label] of ALARM_PANEL_ACTIONS) {
    if (features.includes(feature) && !state.state.startsWith("armed")) buttons.push(commandButton(state, service, label));
  }
  const classes = [tileClass(state, "lock")];
  if (armed) classes.push("on");
  if (state.state === "triggered") classes.push("alert");
  return el(
    "div",
    { id: tileId(state.entity_id), class: classes.join(" ") },
    tileHead(state, ICONS.shield, lookup(ALARM_PANEL_LABELS, state.state) ?? statusText(state)),
    el("div", { class: "tile-actions" }, ...buttons),
  );
}

// ----- Listes et agenda

/** Un petit formulaire « texte + bouton » (ajouter un élément…). */
function addForm(placeholder, label, onSubmit, extra = []) {
  const input = el("input", { type: "text", maxlength: "200", placeholder, "aria-label": placeholder, required: "" });
  const form = el("form", { class: "add-form" }, input, ...extra, el("button", { type: "submit", class: "action-button", text: label }));
  form.addEventListener("submit", (event) => {
    event.preventDefault();
    const text = input.value.trim();
    if (!text) return;
    onSubmit(text, form);
    input.value = "";
  });
  return form;
}

/** Une caméra (la sonnette, étape 82) : sa dernière photo. */
function cameraTile(state) {
  const img = el("img", { class: "camera-image", alt: `Photo : ${nameOf(state)}`, hidden: "" });
  const caption = el("span", { text: "" });
  const tile = el("div", { id: tileId(state.entity_id), class: "tile wide-tile camera-tile" },
    tileHead(state, ICONS.camera, ""), el("div", { class: "camera-frame" }, img), el("p", { class: "muted camera-caption" }, caption));
  if (typeof showCameraImage === "function") showCameraImage(state, img, caption);
  return tile;
}

/** Les lignes d'une liste : la case, le texte, la date ou (Iglou, étape 81)
 *  la quantité, et « × » quand l'élément peut être retiré. Pour Iglou, un
 *  titre par frigo ou congélateur. */
function todoRows(state, items) {
  const iglou = state.attributes.source === "iglou";
  const command = (service, data = {}) => callService("todo", service, { entity_id: state.entity_id, ...data });
  const rows = [];
  let group = null;
  for (const item of items) {
    if (iglou && typeof item.group === "string" && item.group !== group) {
      group = item.group;
      rows.push(el("li", { class: "todo-group", text: group }));
    }
    const done = item.status === "completed";
    const box = el("input", { type: "checkbox", "aria-label": `${item.summary} : ${iglou ? "dans le panier" : "fait"}` });
    box.checked = done;
    box.addEventListener("change", () => command("update_item", { item: item.uid, status: box.checked ? "completed" : "needs_action" }));
    let extra = "";
    if (iglou) {
      const text = [item.out === true ? "épuisé" : "", typeof item.quantity === "string" ? item.quantity : ""].filter(Boolean).join(" · ");
      extra = el("span", { class: item.out === true ? "item-due out" : "item-due", text });
    } else if (typeof item.due === "string") {
      extra = el("span", { class: "item-due", text: shortDate.format(new Date(`${item.due}T12:00:00`)) });
    }
    let remove = "";
    if (!iglou || item.removable === true) {
      remove = el("button", { type: "button", class: "item-remove", "aria-label": `Retirer ${item.summary}`, text: "×" });
      remove.addEventListener("click", () => command("remove_item", { item: item.uid }));
    }
    rows.push(el("li", { class: done ? "done" : "" }, el("label", {}, box, el("span", { text: item.summary }), extra), remove));
  }
  return rows;
}

function todoItems(state) {
  return (Array.isArray(state.attributes.items) ? state.attributes.items : [])
    .filter((item) => item && typeof item.summary === "string" && typeof item.uid === "string");
}

function todoTile(state) {
  const items = todoItems(state);
  const iglou = state.attributes.source === "iglou";
  const left = items.filter((item) => item.status !== "completed").length;
  const command = (service, data = {}) => callService("todo", service, { entity_id: state.entity_id, ...data });
  const status = iglou
    ? (left === 0 ? "Rien à acheter · Iglou" : `${left} à acheter · Iglou`)
    : (left === 0 ? "Rien à faire" : `${left} à faire`);
  const children = [
    tileHead(state, ICONS.list, status),
    items.length > 0 ? el("ul", { class: "todo-list" }, ...todoRows(state, items)) : el("p", { class: "muted", text: "La liste est vide." }),
    addForm("Ajouter…", "Ajouter", (text) => command("add_item", { item: text })),
  ];
  // Avec Iglou, les courses se rangent dans l'appli (elle remet le frigo à plein).
  if (items.length > left && !iglou) {
    const clear = el("button", { type: "button", class: "ghost small", text: "Retirer ce qui est fait" });
    clear.addEventListener("click", () => command("remove_completed_items"));
    children.push(clear);
  }
  return el("div", { id: tileId(state.entity_id), class: "tile wide-tile" }, ...children);
}

const shortDate = new Intl.DateTimeFormat("fr", { weekday: "short", day: "numeric", month: "short" });

/** « Aujourd'hui », « Demain », « jeu. 22 oct. » (pour l'agenda ; le journal
 *  de bord a son propre `dayLabel`, tourné vers le passé). */
function agendaDay(date) {
  const today = new Date();
  const start = (d) => new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
  const days = Math.round((start(date) - start(today)) / 86_400_000);
  return days === 0 ? "Aujourd'hui" : days === 1 ? "Demain" : shortDate.format(date);
}

/** Les rendez-vous publiés par l'agenda, du plus tôt au plus tard. */
function calendarEvents(state) {
  return (Array.isArray(state.attributes.events) ? state.attributes.events : [])
    .filter((e) => e && typeof e.summary === "string" && typeof e.start === "string" && typeof e.end === "string")
    .map((e) => ({ ...e, startDate: new Date(e.start), endDate: new Date(e.end) }))
    .filter((e) => !Number.isNaN(e.startDate.getTime()) && !Number.isNaN(e.endDate.getTime()));
}

function calendarTile(state) {
  const now = Date.now();
  // Un agenda en ligne (étape 57) : en lecture seule.
  const readOnly = state.attributes.read_only === true;
  const coming = calendarEvents(state).filter((e) => e.endDate.getTime() > now).slice(0, 8);
  const current = state.state === "on";
  const failure = typeof state.attributes.erreur === "string" ? state.attributes.erreur : "";
  const subtitle = state.state === "unavailable"
    ? (failure ? `Injoignable : ${failure}` : "Pas encore chargé")
    : current && typeof state.attributes.message === "string"
      ? `En ce moment : ${state.attributes.message}`
      : coming.length === 0 ? "Rien de prévu" : `${coming.length} rendez-vous à venir`;
  const rows = [];
  let lastDay = "";
  for (const e of coming) {
    const day = agendaDay(e.startDate);
    if (day !== lastDay) rows.push(el("li", { class: "day", text: day }));
    lastDay = day;
    const when = e.all_day ? "Journée" : hourMinute.format(e.startDate);
    const ongoing = e.startDate.getTime() <= now;
    const row = el("li", { class: ongoing ? "event now" : "event" },
      el("span", { class: "event-time", text: when }),
      el("span", { class: "event-name", text: e.summary }));
    if (!readOnly) {
      const remove = el("button", { type: "button", class: "item-remove", "aria-label": `Supprimer ${e.summary}`, text: "×" });
      remove.addEventListener("click", () => {
        if (window.confirm(`Supprimer « ${e.summary} » ?`)) callService("calendar", "delete_event", { entity_id: state.entity_id, uid: e.uid });
      });
      row.append(remove);
    }
    rows.push(row);
  }
  if (readOnly) {
    const source = typeof state.attributes.source === "string" ? state.attributes.source : "";
    return el(
      "div",
      { id: tileId(state.entity_id), class: `tile wide-tile${current ? " on" : ""}` },
      tileHead(state, ICONS.calendar, subtitle),
      rows.length > 0 ? el("ol", { class: "agenda-list" }, ...rows) : el("p", { class: "muted", text: "Rien dans les deux mois qui viennent." }),
      ...(source ? [el("p", { class: "muted agenda-source", text: `Agenda en ligne (${source}), en lecture seule` })] : []),
    );
  }
  // Nouveau rendez-vous : un titre, un jour, une heure (sans heure : toute la journée).
  const day = el("input", { type: "date", "aria-label": "Jour", required: "" });
  day.value = new Date(now - new Date().getTimezoneOffset() * 60_000).toISOString().slice(0, 10);
  const time = el("input", { type: "time", "aria-label": "Heure (vide : toute la journée)" });
  const form = addForm("Nouveau rendez-vous…", "Ajouter", (summary) => {
    if (!day.value) return;
    const data = time.value
      ? { start_date_time: `${day.value} ${time.value}` }
      : { start_date: day.value };
    callService("calendar", "create_event", { entity_id: state.entity_id, summary, ...data });
    time.value = "";
  }, [el("div", { class: "add-when" }, day, time)]);
  return el(
    "div",
    { id: tileId(state.entity_id), class: `tile wide-tile${current ? " on" : ""}` },
    tileHead(state, ICONS.calendar, subtitle),
    rows.length > 0 ? el("ol", { class: "agenda-list" }, ...rows) : el("p", { class: "muted", text: "Aucun rendez-vous dans le mois qui vient." }),
    form,
  );
}

/** Une consigne réglable avec − et + (humidité, eau chaude). */
function stepper(state, { value, min, max, step, unit, service, field, label }) {
  const adjust = (delta, verb) => {
    const button = el("button", { type: "button", class: "round", "aria-label": `${verb} ${label} de ${nameOf(state)}`, text: delta > 0 ? "+" : "−" });
    button.disabled = value === null;
    button.addEventListener("click", () => {
      markPending(state.entity_id);
      const next = Math.min(max, Math.max(min, value + delta));
      callService(domainOf(state), service, { entity_id: state.entity_id, [field]: next });
    });
    return button;
  };
  return el("div", { class: "setpoint" },
    adjust(-step, "Baisser"),
    el("span", { class: "setpoint-value", text: value === null ? "–" : `${number.format(value)}${unit}` }),
    adjust(step, "Monter"));
}

function humidifierTile(state) {
  const a = state.attributes;
  const on = state.state === "on";
  const measured = finite(a.current_humidity) ? `${number.format(a.current_humidity)}\u202f% mesurés` : "";
  const toggle = el(
    "button",
    { class: "tile-toggle", type: "button", "aria-pressed": String(on) },
    tileHead(state, ICONS.humidity, [statusText(state), measured].filter(Boolean).join(" · "), knob()),
  );
  toggle.addEventListener("click", () => {
    markPending(state.entity_id);
    callService("humidifier", "toggle", { entity_id: state.entity_id });
  });
  return el(
    "div",
    { id: tileId(state.entity_id), class: `${tileClass(state, "switch")} has-knob` },
    toggle,
    stepper(state, {
      value: finite(a.humidity) ? a.humidity : null,
      min: finite(a.min_humidity) ? a.min_humidity : 0,
      max: finite(a.max_humidity) ? a.max_humidity : 100,
      step: 5, unit: "\u202f%", service: "set_humidity", field: "humidity", label: "l'humidité voulue",
    }),
  );
}

function waterHeaterTile(state) {
  const a = state.attributes;
  const measured = finite(a.current_temperature) ? `${degrees(a.current_temperature)} dans le ballon` : "";
  const mode = lookup(WATER_HEATER_LABELS, state.state) ?? statusText(state);
  const modes = (Array.isArray(a.operation_list) ? a.operation_list : [])
    .filter((m) => typeof m === "string")
    .map((m) => {
      const button = el("button", { type: "button", class: "period", text: lookup(WATER_HEATER_LABELS, m) ?? m.replaceAll("_", " "), "aria-pressed": String(state.state === m) });
      button.addEventListener("click", () => {
        markPending(state.entity_id);
        callService("water_heater", "set_operation_mode", { entity_id: state.entity_id, operation_mode: m });
      });
      return button;
    });
  const children = [
    tileHead(state, ICONS.flame, [mode, measured].filter(Boolean).join(" · ")),
    stepper(state, {
      value: finite(a.temperature) ? a.temperature : null,
      min: finite(a.min_temp) ? a.min_temp : 40,
      max: finite(a.max_temp) ? a.max_temp : 65,
      step: 1, unit: "°", service: "set_temperature", field: "temperature", label: "la température de l'eau",
    }),
  ];
  if (modes.length > 0) children.push(el("div", { class: "presets", role: "group", "aria-label": "Modes" }, ...modes));
  return el("div", { id: tileId(state.entity_id), class: `tile climate${state.state === "off" ? " stopped" : ""}` }, ...children);
}

function binaryTile(state) {
  // Jour ouvré (étape 56) : « Oui », « Non (Ascension) ».
  const workday = Array.isArray(state.attributes.jours_ouvres);
  const holiday = workday && typeof state.attributes.jour_ferie === "string" ? ` (${state.attributes.jour_ferie})` : "";
  const labels = workday ? ["Oui", `Non${holiday}`] : lookup(BINARY_LABELS, state.attributes.device_class) ?? ["Actif", "Inactif"];
  const text = workday
    ? (state.state === "on" ? labels[0] : state.state === "off" ? labels[1] : statusText(state))
    : agree(state, state.state === "on" ? labels[0] : state.state === "off" ? labels[1] : statusText(state));
  const icon = workday ? ICONS.calendar : lookup(BINARY_ICONS, state.attributes.device_class) ?? ICONS.door;
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
    finite(a.humidity) && `Humidité ${number.format(a.humidity)}\u202f%`,
    finite(a.wind_speed) && `Vent ${number.format(a.wind_speed)} km/h`,
  ].filter(Boolean).join(" · ");
  const days = (Array.isArray(a.forecast) ? a.forecast : [])
    .filter((day) => typeof day?.datetime === "string" && !Number.isNaN(Date.parse(day.datetime)))
    .slice(0, 5)
    .map((day) => {
      const [dayLabel] = lookup(WEATHER_LABELS, day.condition) ?? [""];
      const rain = finite(day.precipitation_probability) && day.precipitation_probability >= 30
        ? `${number.format(day.precipitation_probability)}\u202f%` : "";
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

// La vigilance Météo-France : le niveau du jour (sensor.vigilance_meteo),
// les phénomènes et leurs heures, et demain (sensor.vigilance_meteo_demain).
const VIGILANCE_TODAY = "sensor.vigilance_meteo";
const VIGILANCE_TOMORROW = "sensor.vigilance_meteo_demain";
// Le niveau, sa classe CSS et l'adjectif (« vigilance orange »).
const VIGILANCE_LEVELS = { Vert: ["green", "verte"], Jaune: ["yellow", "jaune"], Orange: ["orange", "orange"], Rouge: ["red", "rouge"] };
const vigilanceRank = (level) => ["Vert", "Jaune", "Orange", "Rouge"].indexOf(level);

function vigilanceBadge(level) {
  const [color, adjective] = lookup(VIGILANCE_LEVELS, level) ?? ["unknown", ""];
  // La couleur ne suffit pas : le niveau est toujours écrit.
  return el("span", { class: `vigilance-badge level-${color}`, text: `Vigilance ${adjective}` });
}

function vigilancePhenomena(state) {
  const list = Array.isArray(state.attributes.phenomenes) ? state.attributes.phenomenes : [];
  return list.filter((p) => typeof p?.phenomene === "string" && vigilanceRank(p.niveau) >= 0);
}

/** « orange : orages, vent violent » : pour le résumé de l'accueil. */
function vigilanceSummary(state) {
  const [, adjective] = lookup(VIGILANCE_LEVELS, state.state) ?? ["", state.state];
  const names = vigilancePhenomena(state)
    .filter((p) => p.niveau === state.state)
    .map((p) => p.phenomene.toLowerCase());
  return names.length > 0 ? `${adjective} (${joinNames(names)})` : adjective;
}

const hourText = (date) => (date.getMinutes() === 0 ? `${date.getHours()} h` : `${date.getHours()} h ${String(date.getMinutes()).padStart(2, "0")}`);

/** « de 14 h à 23 h », « à partir de 16 h », « toute la journée ». */
function vigilanceRangeText(range, periodStart, periodEnd) {
  const begin = new Date(range.debut);
  const end = new Date(range.fin);
  if (Number.isNaN(begin.getTime()) || Number.isNaN(end.getTime())) return "";
  const fromStart = periodStart !== null && begin <= periodStart;
  const toEnd = periodEnd !== null && end >= periodEnd;
  if (fromStart && toEnd) return "toute la journée";
  if (fromStart) return `jusqu'à ${hourText(end)}`;
  if (toEnd) return `à partir de ${hourText(begin)}`;
  return `de ${hourText(begin)} à ${hourText(end)}`;
}

function vigilanceDetails(state) {
  const a = state.attributes;
  const start = typeof a.debut === "string" ? new Date(a.debut) : null;
  const end = typeof a.fin === "string" ? new Date(a.fin) : null;
  return vigilancePhenomena(state).map((p) => {
    const ranges = (Array.isArray(p.plages) ? p.plages : []).filter((range) => vigilanceRank(range.niveau) > 0);
    // Les heures, couleur par couleur, la plus grave d'abord :
    // « orange de 16 h à 23 h · jaune de 9 h à 16 h et à partir de 23 h ».
    const levels = [...new Set([p.niveau, ...ranges.map((range) => range.niveau)])]
      .sort((x, y) => vigilanceRank(y) - vigilanceRank(x));
    const text = levels.map((level) => {
      const hours = ranges
        .filter((range) => range.niveau === level)
        .map((range) => vigilanceRangeText(range, start, end))
        .filter(Boolean);
      const adjective = lookup(VIGILANCE_LEVELS, level)[1];
      return hours.length > 0 ? `${adjective} ${joinNames(hours)}` : adjective;
    }).join(" · ");
    const where = p.littoral === true ? " (littoral)" : "";
    return el(
      "li",
      { class: `level-${lookup(VIGILANCE_LEVELS, p.niveau)[0]}` },
      el("strong", { text: `${p.phenomene}${where}` }),
      ` : ${text}`,
    );
  });
}

function vigilanceTile(state) {
  const a = state.attributes;
  const known = vigilanceRank(state.state) >= 0;
  const department = typeof a.departement === "string" ? `Département ${a.departement}` : "Météo-France";
  const items = known ? vigilanceDetails(state) : [];
  const parts = [
    tileHead(state, ICONS.shield, department),
    el("div", { class: "value small-value" }, known ? vigilanceBadge(state.state) : statusText(state)),
  ];
  if (known && items.length === 0) parts.push(el("p", { class: "sub", text: "Aucun phénomène dangereux prévu aujourd'hui." }));
  if (items.length > 0) parts.push(el("ul", { class: "vigilance-list", "aria-label": "Aujourd'hui" }, ...items));
  if (known && typeof a.niveau_actuel === "string" && a.niveau_actuel !== state.state) {
    parts.push(el("p", { class: "sub", text: `En ce moment : ${lookup(VIGILANCE_LEVELS, a.niveau_actuel)?.[1] ?? a.niveau_actuel}` }));
  }
  const tomorrow = entities.get(VIGILANCE_TOMORROW);
  if (tomorrow && known) {
    let text;
    if (vigilanceRank(tomorrow.state) >= 0) {
      const names = vigilancePhenomena(tomorrow).map((p) => p.phenomene.toLowerCase());
      text = `Demain : ${lookup(VIGILANCE_LEVELS, tomorrow.state)[1]}${names.length > 0 ? ` (${joinNames(names)})` : ""}`;
    } else {
      text = "Demain : publiée vers 6 h";
    }
    parts.push(el("p", { class: "vigilance-tomorrow", text }));
  }
  parts.push(el("small", { class: "attribution", text: typeof a.attribution === "string" ? a.attribution : "" }));
  const level = known ? lookup(VIGILANCE_LEVELS, state.state)[0] : "unknown";
  return el("div", { id: tileId(state.entity_id), class: `${tileClass(state, "vigilance")} wide level-${level}` }, ...parts);
}

// Tempo (étape 60) : la couleur du jour et du lendemain, lue sur le Linky.
const TEMPO_TODAY = "sensor.linky_tempo_aujourdhui";
const TEMPO_TOMORROW = "sensor.linky_tempo_demain";
// La couleur, sa classe CSS et l'adjectif (« jour rouge »).
const TEMPO_COLORS = { Bleu: ["blue", "bleu"], Blanc: ["white", "blanc"], Rouge: ["red", "rouge"] };

function tempoBadge(color) {
  const [css, adjective] = lookup(TEMPO_COLORS, color) ?? ["unknown", ""];
  // Comme pour la vigilance, la couleur est toujours écrite.
  return el("span", { class: `tempo-badge tempo-${css}`, text: `Jour ${adjective}` });
}

/** « Demain : jour rouge », ou pas encore annoncé. */
function tempoTomorrowText(tomorrow) {
  if (lookup(TEMPO_COLORS, tomorrow.state)) return `Demain : jour ${TEMPO_COLORS[tomorrow.state][1]}`;
  if (tomorrow.state === "unknown") return "Demain : pas encore annoncé";
  return `Demain : ${statusText(tomorrow)}`;
}

function tempoTile(state) {
  const color = lookup(TEMPO_COLORS, state.state);
  const parts = [
    tileHead(state, ICONS.bolt, "D'après le compteur"),
    el("div", { class: "value small-value" }, color ? tempoBadge(state.state) : statusText(state)),
  ];
  const tomorrow = entities.get(TEMPO_TOMORROW);
  if (tomorrow) {
    // Demain annoncé : sa couleur aussi, pour la voir d'un coup d'œil.
    const line = lookup(TEMPO_COLORS, tomorrow.state) ? ["Demain : ", tempoBadge(tomorrow.state)] : [tempoTomorrowText(tomorrow)];
    parts.push(el("p", { class: "vigilance-tomorrow" }, ...line));
  }
  // Le jour Tempo commence à 6 h : à 3 h du matin, c'est encore celui de la veille.
  parts.push(el("p", { class: "sub", text: "Un jour Tempo va de 6 h à 6 h." }));
  return el("div", { id: tileId(state.entity_id), class: `${tileClass(state, "tempo")} tempo-${color?.[0] ?? "unknown"}` }, ...parts);
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
  const subtitle = Number.isNaN(Date.parse(state.state)) ? "Jamais appuyé" : `Appuyé il y a ${ago(state.state)}`;
  const button = el("button", { type: "button", class: "action-button", text: "Appuyer" });
  button.addEventListener("click", () => inputCommand(state, "press"));
  return el("div", { id: tileId(state.entity_id), class: "tile action" }, tileHead(state, ICONS.button, subtitle), button);
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

function tickCountdowns() {
  const full = 2 * Math.PI * 20;
  for (const arc of document.querySelectorAll("[data-ring-finishes]")) {
    const left = Math.max(0, (Date.parse(arc.dataset.ringFinishes) - Date.now()) / 1000);
    arc.setAttribute("stroke-dashoffset", String(full * (1 - Math.min(1, left / Number(arc.dataset.ringTotal)))));
  }
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

function sensorTile(state) {
  if (typeof state.attributes.appareil === "string") return machineTile(state);
  if (state.attributes.device_class === "timestamp") return timestampTile(state);
  if (state.attributes.device_class === "date") return dateTile(state);
  const unit = state.attributes.unit_of_measurement;
  // « 22.2 » s'affiche « 22,2 » (à la française).
  const numeric = /^-?\d+(\.\d+)?$/.test(state.state) ? number.format(Number(state.state)) : state.state;
  const value = el("div", { class: "value", text: numeric });
  if (unit) value.append(el("small", { text: unit }));
  return el(
    "div",
    { id: tileId(state.entity_id), class: tileClass(state, "sensor") },
    tileHead(state, lookup(ICONS, state.attributes.device_class) ?? ICONS.gauge, `il y a ${ago(state.last_changed)}`),
    value,
  );
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
  if (!running && parts.length === 0) parts.push(`il y a ${ago(state.last_changed)}`);
  const tile = el(
    "div",
    { id: tileId(state.entity_id), class: `${tileClass(state, "sensor")}${running ? " on" : ""}` },
    tileHead(state, ICONS.machine, parts.join(" · ")),
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
    el("div", { class: "value small-value", text: valid ? `il y a ${ago(state.state)}` : "—" }),
  );
  if (failure) tile.append(el("p", { class: "error climate-info", text: `Dernier essai échoué : ${failure}` }));
  return tile;
}

function tileHead(state, icon, subtitle, extra) {
  const head = el(
    "div",
    { class: "tile-head" },
    el("span", { class: "icon" }, svgIcon(icon)),
    el("div", {}, el("div", { class: "name", text: nameOf(state), title: state.entity_id }), el("div", { class: "sub", text: subtitle })),
  );
  if (extra) head.append(extra);
  return head;
}

/** L'interrupteur dessiné à droite d'une tuile qu'on allume d'un toucher. */
const knob = () => el("span", { class: "knob", "aria-hidden": "true" });

/** Un curseur se remplit jusqu'à sa valeur (et suit le doigt). */
function fillSlider(slider) {
  const update = () => {
    const min = Number(slider.min) || 0;
    const max = Number(slider.max) || 100;
    const percent = ((Number(slider.value) - min) / Math.max(1e-9, max - min)) * 100;
    slider.style.setProperty("--fill", `${Math.min(100, Math.max(0, percent))}%`);
  };
  update();
  slider.addEventListener("input", update);
}

function tileClass(state, kind) {
  const classes = ["tile", kind];
  if (state.state === "on" || (kind === "cover" && state.state === "open")) classes.push("on");
  if (state.state === "unavailable" || state.state === "unknown") classes.push("unavailable");
  return classes.join(" ");
}

function statusText(state) {
  return agree(state, lookup(STATUS_LABELS, state.state) ?? state.state);
}

// Les noms féminins les plus courants dans une maison : sans eux, on lirait
// « Porte du garage : ouvert ». La même liste existe côté serveur, pour le
// journal de bord (crates/ha-logbook/src/describe.rs) : les garder pareilles.
const FEMININE = new Set([
  "alarme", "allée", "ampoule", "applique", "baie", "barrière", "bouilloire", "box", "buanderie",
  "cafetière", "caméra", "cave", "chambre", "chaudière", "climatisation", "clim", "cour",
  "cuisine", "électrovanne", "enceinte", "entrée", "fenêtre", "friteuse", "guirlande", "hotte",
  "imprimante", "lampe", "lucarne", "lumière", "machine", "multiprise", "piscine", "plaque",
  "pompe", "porte", "prise", "salle", "serrure", "sirène", "sonde", "sonnette", "station",
  "suspension", "télé", "télévision", "terrasse", "trappe", "vanne", "véranda", "ventilation",
  "verrière", "veilleuse", "vmc",
]);
const FEMININE_FORMS = {
  ouvert: "ouverte", fermé: "fermée", allumé: "allumée", éteint: "éteinte", connecté: "connectée",
  déconnecté: "déconnectée", actif: "active", inactif: "inactive", inconnu: "inconnue",
  verrouillé: "verrouillée", déverrouillé: "déverrouillée", bloqué: "bloquée",
};

/** Le nom de l'appareil est-il féminin ? On regarde son premier mot, ou
 *  l'article : « Porte du garage », « La cafetière », binary_sensor.porte_entree. */
function isFeminine(state) {
  const words = nameOf(state).toLowerCase().split(/[\s_'’-]+/u).filter(Boolean);
  const [first, second] = words;
  if (first === "la" || first === "une") return true;
  if (first === "le" || first === "un" || first === "l") return FEMININE.has(second);
  return FEMININE.has(first);
}

/** Accorde le premier mot d'un libellé : « Ouvert à 85 % » → « Ouverte à 85 % ». */
function agree(state, text) {
  if (!isFeminine(state)) return text;
  return text.replace(/^\p{L}+/u, (word) => {
    const feminine = lookup(FEMININE_FORMS, word.toLowerCase());
    if (feminine === undefined) return word;
    return word[0] === word[0].toLowerCase() ? feminine : feminine[0].toUpperCase() + feminine.slice(1);
  });
}

// Modes d'alarme Somfy : [texte affiché, activée ?]
const ALARM_LABELS = {
  off: ["Désactivée", false], disarmed: ["Désactivée", false],
  total: ["Activée (totale)", true], armed: ["Activée", true],
  partial: ["Activée (partielle)", true], partial1: ["Activée (partielle)", true], partial2: ["Activée (partielle)", true],
  detection: ["Intrusion détectée !", true], sos: ["Alerte SOS !", true],
};
const LOCK_LABELS = {
  locked: "Verrouillé", unlocked: "Déverrouillé", locking: "Verrouillage…", unlocking: "Déverrouillage…",
  jammed: "Bloqué !", open: "Ouvert", opening: "Ouverture…",
};
const VACUUM_LABELS = {
  cleaning: "Fait le ménage", docked: "Sur sa base", paused: "En pause", idle: "À l'arrêt",
  returning: "Rentre à sa base", error: "En erreur !",
};
const WATER_HEATER_LABELS = {
  off: "Arrêt", eco: "Éco", electric: "Électrique", gas: "Gaz", heat_pump: "Pompe à chaleur",
  high_demand: "Forte demande", performance: "Performance",
};
const COVER_LABELS = { open: "Ouvert", closed: "Fermé", opening: "Ouverture…", closing: "Fermeture…" };
const STATUS_LABELS = { on: "Allumé", off: "Éteint", unknown: "Inconnu", unavailable: "Indisponible" };
const BINARY_LABELS = {
  door: ["Ouvert", "Fermé"], window: ["Ouvert", "Fermé"], opening: ["Ouvert", "Fermé"], garage_door: ["Ouvert", "Fermé"],
  motion: ["Mouvement", "Calme"], occupancy: ["Présence", "Personne"],
  smoke: ["Fumée !", "Rien à signaler"],
  connectivity: ["Connecté", "Déconnecté"],
};
