// Tableau de bord — les tuiles des appareils : lumière, volet, prise,
// serrure, vanne, aspirateur, enceinte, compteur, ventilateur, alarme,
// caméra, listes, agenda, humidificateur, chauffe-eau, détecteurs, présence,
// zones, actions et notifications.
//
// Un des fichiers de tiles.js, chargé juste après lui par index.html. Les
// scripts partagent leurs déclarations : `el`, `entities`, `tileHead`…
// viennent de utils.js, app.js et tiles-outils.js. Pas d'innerHTML ici non plus.
"use strict";

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
    ...(typeof mediaNowPlaying === "function" ? [mediaNowPlaying(state)] : []),
    el("div", { class: "cover-buttons" },
      commandButton(state, "media_previous_track", "Morceau précédent", { icon: ICONS.previous }),
      playing
        ? commandButton(state, "media_pause", "Mettre en pause", { icon: ICONS.pause })
        : commandButton(state, "media_play", "Lancer la lecture", { icon: ICONS.play }),
      commandButton(state, "media_next_track", "Morceau suivant", { icon: ICONS.next })),
    el("div", { class: "media-volume" }, mute, slider, el("span", { class: "media-volume-value", text: `${volume}\u202f%` })),
    ...mediaFavorites(state),
  );
}

// Au-del\u00e0, les favoris passent dans une liste d\u00e9roulante.
const MAX_FAVORITE_BUTTONS = 8;

/** Les favoris Sonos (\u00e9tape 91) : un toucher les lance sur l'enceinte. */
function mediaFavorites(state) {
  const a = state.attributes;
  const names = (Array.isArray(a.source_list) ? a.source_list : []).filter((n) => typeof n === "string" && n !== "");
  if (names.length === 0 || state.state === "unavailable") return [];
  const launch = (source) => {
    markPending(state.entity_id);
    callService("media_player", "select_source", { entity_id: state.entity_id, source });
  };
  if (names.length > MAX_FAVORITE_BUTTONS) {
    const list = el("select", { class: "tile-select", "aria-label": `Lancer un favori sur ${nameOf(state)}` });
    list.append(el("option", { value: "", text: "Lancer un favori\u2026" }));
    for (const name of names) list.append(el("option", { value: name, text: name }));
    list.addEventListener("change", () => {
      if (list.value !== "") launch(list.value);
    });
    return [list];
  }
  const buttons = names.map((name) => {
    const button = el("button", {
      type: "button", class: "action-button media-favorite", title: name,
      "aria-label": `Lancer ${name} sur ${nameOf(state)}`,
    }, name);
    button.setAttribute("aria-pressed", String(a.source === name));
    button.addEventListener("click", () => launch(name));
    return button;
  });
  return [el("div", { class: "media-favorites", role: "group", "aria-label": "Favoris" }, ...buttons)];
}

// Le texte en cours d'écriture survit au rafraîchissement de la tuile.
const announceDraft = { message: "", target: "", volume: "" };

function announceTile(state) {
  const a = state.attributes;
  const speakers = (Array.isArray(a.enceintes) ? a.enceintes : []).filter((id) => entities.has(id));
  const voice = typeof a.voix === "string" ? a.voix : "";
  const hasVoice = voice !== "" && !voice.startsWith("aucune");
  const said = typeof a.dernier_message === "string" && !Number.isNaN(Date.parse(state.state));
  const last = said ? "" : hasVoice ? `Voix : ${voice}` : "Pas de voix installée : carillon seulement";
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
    return callService("tts", "speak", data);
  };
  const form = el("form", { class: "add-form announce-form" }, input,
    el("button", { type: "submit", class: "action-button", text: "Annoncer" }));
  form.querySelector("button").disabled = !hasVoice;
  form.addEventListener("submit", async (event) => {
    event.preventDefault();
    if (input.value.trim() === "" || speakers.length === 0) return;
    const message = input.value;
    // Le message reste en place si l'annonce n'est pas partie.
    if (!(await send(true))) return;
    const field = liveField(input);
    if (field.value === message) field.value = "";
    if (announceDraft.message === message) announceDraft.message = "";
  });
  const chime = el("button", { type: "button", class: "ghost small", text: "Carillon seul" });
  chime.addEventListener("click", () => { if (speakers.length > 0) send(false); });
  const head = tileHead(state, ICONS.megaphone, last);
  if (said) markAgo(head.querySelector(".sub"), state.state, "Dernière il y a ", ` : « ${a.dernier_message} »`);
  return el(
    "div",
    { id: tileId(state.entity_id), class: "tile sensor announce" },
    head,
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
  const head = tileHead(state, ICONS.button, kind ? `« ${kind} »` : "Aucun appui reçu");
  if (kind && !Number.isNaN(Date.parse(state.state))) markAgo(head.querySelector(".sub"), state.state, `« ${kind} » · il y a `);
  return el("div", { id: tileId(state.entity_id), class: tileClass(state, "binary") }, head);
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

/** Un petit formulaire « texte + bouton » (ajouter un élément…). Le texte
 *  n'est effacé que si `onSubmit` ne renvoie pas false : un ordre qui n'est
 *  pas parti laisse le texte en place, pour réessayer. */
function addForm(placeholder, label, onSubmit, extra = []) {
  const input = el("input", { type: "text", maxlength: "200", placeholder, "aria-label": placeholder, required: "" });
  const form = el("form", { class: "add-form" }, input, ...extra, el("button", { type: "submit", class: "action-button", text: label }));
  form.addEventListener("submit", async (event) => {
    event.preventDefault();
    const text = input.value.trim();
    if (!text) return;
    const sent = await onSubmit(text, form);
    // (Sauf si on a déjà retapé autre chose entre-temps.)
    const field = liveField(input);
    if (sent !== false && field.value.trim() === text) field.value = "";
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
  const form = addForm("Nouveau rendez-vous…", "Ajouter", async (summary) => {
    if (!day.value) return false;
    const data = time.value
      ? { start_date_time: `${day.value} ${time.value}` }
      : { start_date: day.value };
    const sent = await callService("calendar", "create_event", { entity_id: state.entity_id, summary, ...data });
    if (sent) time.value = "";
    return sent;
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
    : Number.isNaN(Date.parse(state.state)) ? "Jamais activée" : "";
  const head = tileHead(state, isScript ? ICONS.play : ICONS.sparkle, subtitle);
  if (!isScript && subtitle === "") markAgo(head.querySelector(".sub"), state.state, "Activée il y a ");
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
    head,
    button,
  );
}

// Le téléphone : date de la dernière notification, et un bouton pour tester.
function notifyTile(state) {
  const sent = !Number.isNaN(Date.parse(state.state));
  const head = tileHead(state, ICONS.bell, sent ? "" : "Aucune notification envoyée");
  if (sent) markAgo(head.querySelector(".sub"), state.state, "Dernière notification il y a ");
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
    head,
    button,
  );
}
