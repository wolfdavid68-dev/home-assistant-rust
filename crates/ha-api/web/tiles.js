// Tableau de bord — l'accueil, et le choix de la tuile de chaque appareil.
//
// Un des fichiers du tableau de bord, chargés dans l'ordre par index.html
// (utils.js, app.js, tiles.js, tiles-appareils.js, tiles-meteo.js,
// tiles-capteurs.js, tiles-outils.js, dialogs.js, editor.js, account.js,
// start.js, puis canvas.js, house.js, agenda.js). Ils partagent leurs
// déclarations : une fonction d'un fichier peut appeler celle d'un autre,
// une fois la page chargée.
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

// L'heure, chaque minute (et la salutation avec). Les panneaux du tableau
// sont refaits en entier : ce qui suit l'heure avance (la courbe qui glisse,
// le prochain rendez-vous), et les courbes relisent leur historique.
setInterval(() => {
  if ($("#dashboard-view").hidden) return;
  forgetPanels();
  renderHome();
}, 30_000);

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
