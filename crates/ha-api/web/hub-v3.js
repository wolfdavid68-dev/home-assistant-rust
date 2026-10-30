// Accueil v3 (opt-in, dans Mon compte) : la maison en vedette.
//
// Quand le réglage « v3 » est choisi sur cet appareil, le tableau de bord
// (#hub) est réarrangé : un salut, un résumé, la musique, le chauffage, et une
// barre de « lentilles » qui met en valeur un type d'appareil sur le dessin.
// « Actuel » remet tout à sa place.
//
// Chargé en dernier : il utilise les fonctions des autres fichiers.
// Pas d'innerHTML ici non plus.
"use strict";

const HUB_V3_LENSES = [
  { id: "vivant", label: "Vivant" },
  { id: "lumieres", label: "Lumières" },
  { id: "volets", label: "Volets" },
  { id: "chauffage", label: "Chauffage" },
  { id: "musique", label: "Musique" },
  { id: "securite", label: "Sécurité" },
];
const HUB_V3_KEY = "ha-hub-style";
const HUB_V3_OPENINGS = ["door", "window", "garage_door", "opening"];
// Les capteurs d'alerte : fumée, fuite d'eau, gaz…
const HUB_V3_ALERTS = ["smoke", "moisture", "gas", "carbon_monoxide", "safety", "tamper"];
// Le mot de chaque alerte.
const HUB_V3_ALERT_WORDS = {
  smoke: "fumée", moisture: "fuite d'eau", gas: "gaz", carbon_monoxide: "monoxyde de carbone",
  safety: "danger", tamper: "sabotage",
};
// Les capteurs qui comptent pour la sécurité : les ouvertures, les alertes, les mouvements.
const HUB_V3_SECURITY = [...HUB_V3_OPENINGS, ...HUB_V3_ALERTS, "motion", "occupancy", "presence"];
// Les portes de garage et les portails sont des « volets » pour HA, mais comptent pour la sécurité.
const HUB_V3_GATES = ["garage", "gate", "door"];
// Ce qui cloche pour une serrure.
const HUB_V3_LOCK_ISSUES = { unlocked: "déverrouillé", open: "ouvert", jammed: "bloqué" };
// L'eau chaude : sa température n'est pas celle d'une pièce.
const HUB_V3_WATER = "climate.eau_chaude";
const HUB_V3_NUMBER = new Intl.NumberFormat("fr", { maximumFractionDigits: 1 });
// Les lentilles qui posent une pastille par pièce : l'icône (cherchée dans ICONS au rendu)
// et le nom des deux groupes pour le lecteur d'écran.
const HUB_V3_PIN_LENSES = {
  lumieres: { icon: "bulb", pins: "Lumières par pièce", rooms: "Lumières des pièces hors du dessin" },
  volets: { icon: "blinds", pins: "Volets par pièce", rooms: "Volets des pièces hors du dessin" },
  chauffage: { icon: "temperature", pins: "Températures par pièce", rooms: "Températures des pièces hors du dessin" },
  musique: { icon: "music", pins: "Musique par pièce", rooms: "Musique des pièces hors du dessin" },
  securite: { icon: "shield", pins: "Sécurité par pièce", rooms: "Sécurité des pièces hors du dessin" },
};
// L'icône devant la phrase de chaque lentille.
const HUB_V3_READ_ICONS = { lumieres: "bulb", volets: "blinds", chauffage: "flame", musique: "music", securite: "shield" };
const hubV3 = { placed: false, lens: "vivant", ui: null, pins: new Map(), chips: new Map(), pinLens: "" };
// Les pastilles entrent l'une après l'autre ; au-delà de 10, elles arrivent ensemble.
const HUB_V3_STAGGER_MAX = 10;
// La minuterie de chaque bouton qui attend une réponse.
const hubV3Waits = new WeakMap();

/** Le salut selon l'heure. */
function hubV3Hello(hour) {
  return hour >= 5 && hour < 18 ? "Bonjour." : "Bonsoir.";
}

/** Ce qui demande l'attention : rien, une chose, ou le compte. */
function hubV3Attention(count) {
  if (count === 0) return "Tout est calme.";
  if (count === 1) return "Une chose demande ton attention.";
  return `${count} choses demandent ton attention.`;
}

/** « 2 lumières allumées » : le bon singulier ou pluriel. */
function hubV3Count(n, one, many) {
  return `${n} ${n === 1 ? one : many}`;
}

/** La phrase des volets : tous fermés, tous ouverts, ou le compte de chaque. */
function hubV3CoversText(open, total) {
  if (total === 0) return "Aucun volet";
  if (open === 0) return total === 1 ? "Le volet est fermé" : `Les ${total} volets sont fermés`;
  if (open === total) return total === 1 ? "Le volet est ouvert" : `Les ${total} volets sont ouverts`;
  return `${hubV3Count(open, "ouvert", "ouverts")} · ${hubV3Count(total - open, "fermé", "fermés")}`;
}

/** Les volets d'une pièce qui en a plusieurs : « Fermés », « Ouverts » ou « 1 ouvert sur 3 ». */
function hubV3CoverSummary(open, total) {
  if (open === 0) return "Fermés";
  if (open === total) return "Ouverts";
  return `${hubV3Count(open, "ouvert", "ouverts")} sur ${total}`;
}

/** La phrase de la sécurité : en ordre, ce qui cloche (deux choses au plus), ou le compte. */
function hubV3SecurityText(issues) {
  if (issues.length === 0) return "Tout est en ordre";
  if (issues.length <= 2) return issues.join(" · ");
  return hubV3Count(issues.length, "chose à vérifier", "choses à vérifier");
}

/** Une température à la française : « 21,2° ». */
function hubV3Temp(value) {
  return `${HUB_V3_NUMBER.format(value)}°`;
}

/** Une chaîne nettoyée, ou "" si ce n'en est pas une. */
function hubV3Clean(value) {
  return typeof value === "string" ? value.trim() : "";
}

/** Le titre en cours d'une enceinte (morceau, radio ou source), ou "" à l'arrêt. */
function hubV3MusicTitle(state) {
  if (state.state !== "playing" && state.state !== "paused") return "";
  const a = state.attributes;
  return hubV3Clean(a.media_title) || hubV3Clean(a.media_channel) || hubV3Clean(a.source);
}

/** Le résumé d'une pièce en Sécurité : « En ordre » ou « 2 à vérifier ». */
function hubV3SecurityShort(n) {
  return n === 0 ? "En ordre" : `${n} à vérifier`;
}

/** Ce que dit la pastille des volets d'une pièce (lecteur d'écran). */
function hubV3CoverLabel(name, open, total, canMove = true) {
  const said = open > 0 && open < total ? `${hubV3Count(open, "volet ouvert", "volets ouverts")} sur ${total}`
    : hubV3CoversText(open, total).toLowerCase();
  const hint = !canMove ? "" : open > 0 ? " Toucher pour fermer." : " Toucher pour ouvrir.";
  return `${name} : ${said}.${hint}`;
}

/** Un volet (pas une porte de garage ni un portail). */
function hubV3IsShutter(state) {
  return domainOf(state) === "cover" && !HUB_V3_GATES.includes(state.attributes.device_class);
}

/** Un appareil qui compte pour la sécurité : ouverture, alerte, serrure, alarme, sirène, portail. */
function hubV3IsSecurity(state) {
  const domain = domainOf(state);
  const dc = state.attributes.device_class;
  if (domain === "binary_sensor") return HUB_V3_OPENINGS.includes(dc) || HUB_V3_ALERTS.includes(dc);
  if (domain === "cover") return HUB_V3_GATES.includes(dc);
  return ["lock", "alarm_control_panel", "siren"].includes(domain);
}

/** La lentille d'un appareil, ou "" s'il n'en a pas. */
function hubV3Kind(domain, deviceClass) {
  if (domain === "light") return deviceClass === "outlet" ? "" : "lumieres";
  if (domain === "cover") return HUB_V3_GATES.includes(deviceClass) ? "securite" : "volets";
  if (domain === "climate" || domain === "water_heater") return "chauffage";
  if (domain === "sensor") return deviceClass === "temperature" || deviceClass === "humidity" ? "chauffage" : "";
  if (domain === "media_player") return "musique";
  if (domain === "binary_sensor") return HUB_V3_SECURITY.includes(deviceClass) ? "securite" : "";
  if (["lock", "alarm_control_panel", "camera", "siren"].includes(domain)) return "securite";
  return "";
}

/** Le réglage v3 est-il choisi sur cet appareil ? */
function hubV3On() {
  return document.documentElement.dataset.hubStyle === "v3";
}

/** Pose ou retire l'attribut qui active le style v3 (CSS). */
function hubV3Apply(on) {
  if (on) document.documentElement.dataset.hubStyle = "v3";
  else delete document.documentElement.dataset.hubStyle;
}

/** Au chargement : le réglage enregistré, puis le choix dans Mon compte. */
function hubV3Init() {
  let saved = null;
  try {
    saved = localStorage.getItem(HUB_V3_KEY);
  } catch {
    // Stockage bloqué : le réglage par défaut.
  }
  hubV3Apply(saved === "v3");
  const pick = $("#hub-style");
  if (pick) {
    pick.value = hubV3On() ? "v3" : "";
    pick.addEventListener("change", () => {
      try {
        if (pick.value === "v3") localStorage.setItem(HUB_V3_KEY, "v3");
        else localStorage.removeItem(HUB_V3_KEY);
      } catch {
        // Stockage bloqué : le choix ne durera que le temps de la page.
      }
      hubV3Apply(pick.value === "v3");
      renderHubV3();
      updateHub();
    });
  }
  // Un nouveau dessin (changement de page, redessin) : ses appareils se
  // retrouvent à leur lentille, et les pastilles des pièces reviennent.
  const stage = $("#house-stage");
  if (stage) {
    const observer = new MutationObserver(() => {
      if (!hubV3On() || !hubV3.placed) return;
      hubV3Tag();
      hubV3RenderPins();
    });
    observer.observe(stage, { childList: true });
  }
  renderHubV3();
}

/** Les éléments de l'accueil v3, créés une seule fois, avec leurs gestes. */
function hubV3Build() {
  if (hubV3.ui) return hubV3.ui;
  const button = (attributes, ...children) => el("button", { type: "button", ...attributes }, ...children);
  const musicImg = el("img", { alt: "", decoding: "async", draggable: "false" });
  musicImg.hidden = true;
  const ui = {
    hello: el("h2", { id: "hub-v3-hello", class: "hub-v3-hello" }),
    summary: el("p", { id: "hub-v3-summary", class: "hub-v3-summary" }),
    music: el("section", { id: "hub-v3-music", class: "hub-v3-music", "aria-label": "Musique" }),
    musicOpen: button({ class: "hub-v3-music-open" }),
    musicImg,
    musicTitle: el("b", { class: "hub-v3-music-title" }),
    musicSub: el("small", { class: "hub-v3-music-sub" }),
    play: button({ class: "hub-v3-play" }),
    next: button({ class: "hub-v3-next", "aria-label": "Morceau suivant" }, svgIcon(ICONS.next)),
    tempo: el("div", { id: "hub-v3-tempo", class: "hub-v3-tempo" }),
    heating: el("section", { id: "hub-v3-heating", class: "hub-v3-heating", "aria-label": "Chauffage" }),
    actions: el("div", { id: "hub-v3-actions", class: "hub-v3-actions" }),
    bell: button({ id: "hub-v3-bell", class: "hub-v3-ib", "aria-label": "Notifications" }, svgIcon(ICONS.bell)),
    badge: el("span", { class: "hub-v3-badge" }),
    assist: button({ id: "hub-v3-assist", class: "hub-v3-ib", "aria-label": "Assist : demander en français" },
      svgIcon(ICONS.sparkle)),
    account: button({ id: "hub-v3-account", class: "hub-v3-ib hub-v3-avatar", "aria-label": "Mon compte" }),
    lens: el("nav", { id: "hub-v3-lens", class: "hub-v3-lens", "aria-label": "Ce que montre la maison" }),
    readIcon: el("span", { class: "hub-v3-read-icon", "aria-hidden": "true", "data-icon": "bulb" },
      svgIcon(ICONS.bulb)),
    readout: el("p", { id: "hub-v3-readout", class: "hub-v3-readout", role: "status" }),
    allOff: button({ id: "hub-v3-alloff", class: "hub-v3-alloff" }, "Tout éteindre"),
    pins: el("div", { class: "hub-v3-pins", role: "group", "aria-label": "Lumières par pièce" }),
    rooms: el("div", { id: "hub-v3-lit-rooms", class: "hub-v3-lit-rooms", role: "group",
      "aria-label": "Lumières des pièces hors du dessin" }),
    musicBtn: button({ id: "hub-v3-music-btn" }, svgIcon(ICONS.music), el("span", { text: "Musique" })),
  };
  ui.music.hidden = true;
  ui.tempo.hidden = true;
  ui.heating.hidden = true;
  ui.read = el("div", { id: "hub-v3-read", class: "hub-v3-read" }, ui.readIcon, ui.readout, ui.allOff);
  ui.read.hidden = true;
  ui.readIcon.hidden = true;
  ui.allOff.hidden = true;
  ui.rooms.hidden = true;
  ui.musicBtn.hidden = true;
  ui.badge.hidden = true;

  ui.musicOpen.append(
    el("span", { class: "hub-v3-art" }, svgIcon(ICONS.music), musicImg),
    el("span", { class: "hub-v3-music-text" }, ui.musicTitle, ui.musicSub),
  );
  ui.music.append(ui.musicOpen, el("div", { class: "hub-v3-music-ctl" }, ui.play, ui.next));
  ui.bell.append(ui.badge);
  ui.actions.append(ui.bell, ui.assist, ui.account);

  ui.segs = HUB_V3_LENSES.map((lens) => {
    const seg = button({ class: "hub-v3-seg", "data-lens": lens.id, "aria-pressed": "false" },
      el("span", { class: "hub-v3-sd", "aria-hidden": "true" }), lens.label);
    seg.addEventListener("click", () => {
      hubV3.lens = lens.id;
      hubV3RenderLens();
    });
    return seg;
  });
  ui.lens.append(el("span", { class: "hub-v3-seg-ind", "aria-hidden": "true" }), ...ui.segs);

  // Ouvrir le lecteur quitte l'accueil : la musique le recouvrirait.
  ui.musicOpen.addEventListener("click", () => {
    leaveHub();
    openMusic();
  });
  ui.musicBtn.addEventListener("click", () => {
    leaveHub();
    openMusic();
  });
  const act = (handler) => () => {
    const state = musicPlayer();
    if (state && canControl(state)) handler(state);
  };
  ui.play.addEventListener("click", act((state) => musicCommand(state,
    state.state === "playing" ? "media_pause" : "media_play")));
  ui.next.addEventListener("click", act((state) => musicCommand(state, "media_next_track")));
  // Les boutons de l'en-tête restent les vrais : ceux-ci les déclenchent.
  ui.bell.addEventListener("click", () => $("#open-notices")?.click());
  ui.assist.addEventListener("click", () => $("#open-assist")?.click());
  ui.account.addEventListener("click", () => $("#open-account")?.click());
  // Le bouton de la phrase suit la lentille (voir hubV3Action).
  ui.allOff.addEventListener("click", () => {
    if (hubV3ActionRun(hubV3.lens)) hubV3Wait(ui.allOff);
  });

  hubV3.ui = ui;
  return ui;
}

/** Range les blocs dans la vue v3. Une seule fois, tant que le réglage dure. */
function hubV3Place() {
  const ui = hubV3Build();
  const root = $("#hub");
  root.querySelector(".hub-left").append(root.querySelector(".hub-time"), ui.hello, ui.summary, $("#hub-scenes"), ui.music);
  root.querySelector(".hub-right").append($("#hub-weather"), ui.tempo, $("#hub-agenda"), ui.heating, $("#hub-rooms"));
  root.querySelector(".hub-top").append(ui.actions);
  root.querySelector(".hub-center").prepend(ui.lens, ui.read);
  root.querySelector(".hub-center").append(ui.rooms);
  $("#hub-open-agenda").after(ui.musicBtn);
  hubV3.placed = true;
}

/** Remet les blocs à leur place d'origine et retire ce qui a été ajouté. */
function hubV3Restore() {
  const ui = hubV3.ui;
  const root = $("#hub");
  root.querySelector(".hub-left").append(root.querySelector(".hub-time"), $("#hub-weather"), $("#hub-agenda"));
  root.querySelector(".hub-right").append($("#hub-scenes"), $("#hub-rooms"));
  const added = [ui.hello, ui.summary, ui.music, ui.tempo, ui.heating, ui.actions, ui.lens, ui.read, ui.rooms, ui.pins,
    ui.musicBtn];
  for (const node of added) node.remove();
  const box = $("#hub-house");
  if (box) delete box.dataset.lens;
  hubV3.placed = false;
}

/** Appelé après chaque rendu du tableau de bord (hub.js). */
function renderHubV3() {
  if (!hubV3On()) {
    if (hubV3.placed) hubV3Restore();
    return;
  }
  if (!hub.on) return;
  if (!hubV3.placed) hubV3Place();
  hubV3RenderHello();
  hubV3RenderSummary();
  hubV3RenderActions();
  hubV3RenderMusic();
  hubV3RenderTempo();
  hubV3RenderHeating();
  hubV3RenderLens();
}

/** Les compteurs de la maison : lumières et volets. */
function hubV3Counts() {
  const states = [...entities.values()];
  const covers = states.filter(hubV3IsShutter);
  return {
    lights: states.filter(isLight).length,
    lit: states.filter((s) => isLight(s) && s.state === "on").length,
    covers: covers.length,
    open: covers.filter((s) => s.state === "open" || s.state === "opening").length,
  };
}

/** La pompe à chaleur, si elle répond. */
function hubV3Pump() {
  const pump = entities.get("sensor.pompe_a_chaleur");
  return pump && pump.state !== "unknown" && pump.state !== "unavailable" ? pump : null;
}

/** Les machines en marche (ou finies depuis peu), comme dans le salut du tableau. */
function hubV3Machines() {
  return [...entities.values()]
    .filter((s) => typeof s.attributes.appareil === "string")
    .filter((s) => s.state === "En marche"
      || (s.state === "Terminé" && Date.now() - Date.parse(s.last_changed) < MACHINE_DONE_SHOWN))
    .map((s) => (s.state === "Terminé" ? `${nameOf(s)} a fini`
      : typeof s.attributes.fin_heure === "string" ? `${nameOf(s)} : fin vers ${s.attributes.fin_heure}`
        : `${nameOf(s)} en marche`));
}

function hubV3RenderHello() {
  const ui = hubV3.ui;
  const count = alerts().length;
  const soon = count === 0 ? agendaSoon() : null;
  const em = soon ? el("em", {}, soon, ".") : el("em", { text: hubV3Attention(count) });
  swap(ui.hello, hubV3Hello(new Date().getHours()), " ", em);
}

/** La phrase du résumé : lumières, volets, température, machines, colis. */
function hubV3RenderSummary() {
  const ui = hubV3.ui;
  const counts = hubV3Counts();
  const parts = [];
  if (counts.lights > 0) {
    parts.push(counts.lit > 0 ? hubV3Count(counts.lit, "lumière allumée", "lumières allumées") : "Lumières éteintes");
  }
  if (counts.covers > 0) {
    parts.push(counts.open > 0 ? hubV3Count(counts.open, "volet ouvert", "volets ouverts") : "Volets fermés");
  }
  const temps = shownAreas().map((area) => roomInfo(area).temperature).filter(finite);
  if (temps.length > 0) {
    const average = temps.reduce((sum, t) => sum + t, 0) / temps.length;
    parts.push(`${number.format(Math.round(average * 10) / 10)}° dedans`);
  }
  parts.push(...hubV3Machines(), parcelGreeting());
  const text = parts.filter(Boolean).join(" · ");
  setText(ui.summary, text);
  ui.summary.hidden = text === "";
}

function hubV3RenderActions() {
  const ui = hubV3.ui;
  const notices = $("#open-notices");
  ui.bell.hidden = !notices || notices.hidden;
  const count = $("#notice-count");
  const n = count && !count.hidden ? count.textContent.trim() : "";
  setText(ui.badge, n);
  ui.badge.hidden = n === "";
  setAttr(ui.bell, "aria-label", n ? `Notifications : ${n}` : "Notifications");
  const assist = $("#open-assist");
  ui.assist.hidden = !assist || assist.hidden;
  const account = $("#open-account");
  ui.account.hidden = !account || account.hidden;
  setText(ui.account, account ? account.textContent.trim() : "");
}

/** La musique de l'enceinte affichée, comme le tableau (sans le volume). */
function hubV3RenderMusic() {
  const ui = hubV3.ui;
  const state = musicPlayer();
  ui.music.hidden = state === null;
  ui.musicBtn.hidden = musicPlayers().length === 0;
  if (state === null) return;
  const a = state.attributes;
  const available = !["unavailable", "unknown"].includes(state.state);
  const playing = state.state === "playing";
  const active = playing || state.state === "paused";
  const allowed = canControl(state) && available;
  const title = hubV3MusicTitle(state);
  const status = available ? lookup(MEDIA_LABELS, state.state) ?? statusText(state) : "Injoignable";
  setText(ui.musicTitle, title || "Musique");
  setText(ui.musicSub, [active ? hubV3Clean(a.media_artist) || status : status, nameOf(state)].filter(Boolean).join(" · "));
  showCover(ui.musicImg, active ? a.entity_picture : null);
  ui.music.classList.toggle("playing", playing);
  setAttr(ui.musicOpen, "aria-label", `Ouvrir le lecteur${title ? ` : ${title}` : ""}`);
  swap(ui.play, svgIcon(playing ? ICONS.pause : ICONS.play));
  setAttr(ui.play, "aria-label", playing ? "Mettre en pause" : "Lancer la lecture");
  ui.play.disabled = !allowed;
  ui.next.disabled = !allowed || !active;
}

/** Le tempo du jour (et du lendemain), si le Linky le donne. */
function hubV3RenderTempo() {
  const ui = hubV3.ui;
  const today = entities.get(TEMPO_TODAY);
  ui.tempo.hidden = !today;
  if (!today) return;
  const parts = [el("span", { class: "hub-v3-tempo-label", text: "Tempo" }), tempoBadge(today.state)];
  const tomorrow = entities.get(TEMPO_TOMORROW);
  if (tomorrow) parts.push(el("span", { class: "hub-v3-tempo-next", text: tempoTomorrowText(tomorrow) }));
  swap(ui.tempo, ...parts);
}

/** Le chauffage : une ligne par zone, et la pompe à chaleur en tête. */
function hubV3RenderHeating() {
  const ui = hubV3.ui;
  const states = heatingStates(null);
  ui.heating.hidden = states.length === 0;
  if (states.length === 0) return;
  const chip = [];
  const pump = hubV3Pump();
  if (pump) {
    const code = pump.attributes.code_erreur;
    if (finite(code)) chip.push(el("span", { class: "hub-v3-heat-chip alert", text: `PAC · erreur ${code}` }));
    else if (pump.state === "chauffage") chip.push(el("span", { class: "hub-v3-heat-chip running", text: "PAC en marche" }));
    else chip.push(el("span", { class: "hub-v3-heat-chip", text: `PAC · ${pump.state}` }));
  }
  const head = el("div", { class: "hub-v3-heat-head" }, svgIcon(ICONS.flame), el("h3", { text: "Chauffage" }), ...chip);
  const rows = states.map((state) => {
    const area = areas.find((a) => a.entities.some((e) => e.entity_id === state.entity_id));
    const current = state.attributes.current_temperature;
    const mode = heatMode(state);
    const target = state.state !== "off" && state.state !== "unavailable" ? heatTarget(state) : null;
    const classes = ["hub-v3-zone"];
    if (state.entity_id === HUB_V3_WATER) classes.push("water");
    if (state.attributes.hvac_action === "heating") classes.push("heating");
    const children = [el("span", { class: "hub-v3-zone-text" }, el("b", { text: nameOf(state) }),
      el("small", { text: finite(current) ? `il fait ${number.format(current)}° · ${mode}` : mode }))];
    if (target !== null) children.push(el("span", { class: "hub-v3-zone-target", text: `${number.format(target)}°` }));
    if (!area) return el("div", { class: classes.join(" ") }, ...children);
    const row = el("button", { type: "button", class: classes.join(" "), title: `Ouvrir ${area.name}` }, ...children);
    row.addEventListener("click", () => openRoom(area.name));
    return row;
  });
  swap(ui.heating, head, ...rows);
}

/** Le texte sous la barre de lentilles : ce que montre la lentille choisie. */
function hubV3Readout(lens) {
  const counts = hubV3Counts();
  switch (lens) {
    case "lumieres":
      return counts.lit > 0 ? hubV3Count(counts.lit, "lumière allumée", "lumières allumées")
        : "Toutes les lumières sont éteintes";
    case "volets":
      return hubV3CoversText(counts.open, counts.covers);
    case "chauffage": {
      const heating = heatingStates(null).filter((s) => s.attributes.hvac_action === "heating").length;
      const parts = [heating > 0 ? hubV3Count(heating, "zone chauffe", "zones chauffent") : "Rien ne chauffe"];
      const pump = hubV3Pump();
      if (pump) parts.push(pump.state === "chauffage" ? "PAC en marche" : `PAC · ${pump.state}`);
      const outside = entities.get("sensor.temperature_exterieure");
      if (outside && Number.isFinite(Number(outside.state))) parts.push(`Dehors ${number.format(Number(outside.state))}°`);
      return parts.join(" · ");
    }
    case "musique": {
      const player = musicPlayer();
      if (player && (player.state === "playing" || player.state === "paused")) {
        const playing = player.state === "playing";
        return [hubV3MusicTitle(player) || "Musique", playing ? hubV3Clean(player.attributes.media_artist) : "En pause",
          nameOf(player)].filter(Boolean).join(" · ");
      }
      return "Pas de musique en ce moment";
    }
    case "securite":
      return hubV3SecurityText(hubV3Issues());
    default:
      return "";
  }
}

/** Met en valeur la lentille choisie : barre, dessin, texte. */
function hubV3RenderLens() {
  const ui = hubV3.ui;
  const index = Math.max(0, HUB_V3_LENSES.findIndex((l) => l.id === hubV3.lens));
  ui.lens.style.setProperty("--i", String(index));
  setAttr(ui.lens, "data-lens", hubV3.lens);
  for (const seg of ui.segs) setAttr(seg, "aria-pressed", String(seg.dataset.lens === hubV3.lens));
  const box = $("#hub-house");
  if (box) setAttr(box, "data-lens", hubV3.lens);
  const text = hubV3Readout(hubV3.lens);
  setText(ui.readout, text);
  ui.read.hidden = text === "";
  const lens = hubV3.lens;
  setAttr(ui.read, "data-lens", lens);
  // L'icône de la lentille, et son bouton (« Tout éteindre », « Tout fermer »…) s'il y a de quoi faire.
  const icon = lookup(HUB_V3_READ_ICONS, lens) ?? "";
  ui.readIcon.hidden = icon === "";
  if (icon !== "" && ui.readIcon.dataset.icon !== icon) {
    ui.readIcon.dataset.icon = icon;
    ui.readIcon.replaceChildren(svgIcon(ICONS[icon]));
  }
  const action = hubV3Action(lens);
  if (action === null || ui.allOff.textContent !== action) ui.allOff.classList.remove("pending");
  ui.allOff.hidden = action === null;
  if (action !== null) setText(ui.allOff, action);
  ui.read.classList.toggle("alert", lens === "securite" && hubV3Issues().length > 0);
  hubV3Tag();
  hubV3RenderPins();
}

/** La lentille d'une entité, d'après son identifiant ("" si inconnue). */
function hubV3KindOf(entityId) {
  if (typeof entityId !== "string") return "";
  return hubV3Kind(entityId.split(".")[0], entities.get(entityId)?.attributes.device_class);
}

/** Marque chaque élément du dessin (étiquette, zone, calque) de sa lentille. */
function hubV3Tag() {
  if (!house.view) return;
  const mark = (node, kind) => {
    if (node.dataset.kind !== kind) node.dataset.kind = kind;
  };
  for (const { label, node } of house.view.labels ?? []) mark(node, hubV3KindOf(label.entity_id));
  for (const { zone, node } of house.view.zones ?? []) mark(node, zone.area ? "lumieres" : hubV3KindOf(zone.entity_id));
  for (const { layer, img } of house.view.layers ?? []) mark(img, layer.area ? "lumieres" : hubV3KindOf(layer.entity_id));
}

/** Le centre d'une zone du dessin, en % : là où se pose sa pastille. */
function hubV3ZoneCenter(zone) {
  return { x: zone.x + zone.w / 2, y: zone.y + zone.h / 2 };
}

/** Ce que dit la pastille d'une pièce (lecteur d'écran). */
function hubV3PinLabel(name, lit, total, canToggle = true) {
  let said;
  if (lit === 0) said = "tout est éteint";
  else if (total === 1) said = "la lumière est allumée";
  else if (lit === total) said = `les ${total} lumières sont allumées`;
  else said = `${hubV3Count(lit, "lumière allumée", "lumières allumées")} sur ${total}`;
  const hint = !canToggle ? "" : lit === 0 ? " Toucher pour allumer." : " Toucher pour éteindre.";
  return `${name} : ${said}.${hint}`;
}

/** Un bouton qui attend la réponse : grisé 5 s au plus. */
function hubV3Wait(node) {
  node.classList.add("pending");
  clearTimeout(hubV3Waits.get(node));
  hubV3Waits.set(node, setTimeout(() => node.classList.remove("pending"), 5000));
}

/** Les lumières d'une pièce (sans les prises). */
function hubV3RoomLights(area) {
  return area.entities.map((e) => entities.get(e.entity_id)).filter((s) => s !== undefined && isLight(s));
}

/** Une pièce éclairée s'éteint ; une pièce éteinte s'allume. Faux si rien n'est parti. */
function hubV3ToggleRoom(name) {
  const area = areas.find((a) => a.name === name);
  if (!area) return false;
  const lights = hubV3RoomLights(area).filter(canControl);
  const lit = lights.filter((s) => s.state === "on");
  const service = lit.length > 0 ? "turn_off" : "turn_on";
  const targets = lit.length > 0 ? lit : lights.filter((s) => s.state === "off");
  for (const state of targets) {
    markPending(state.entity_id);
    callService("light", service, { entity_id: state.entity_id });
  }
  return targets.length > 0;
}

/** Un bouton de pièce : l'icône de la lentille, le nom (dans la rangée) et ce qu'elle montre. */
function hubV3RoomButton(name, cls, withName, lens) {
  const count = el("span", { class: "hub-v3-pin-n" });
  const parts = [svgIcon(ICONS[HUB_V3_PIN_LENSES[lens].icon])];
  if (withName) parts.push(el("span", { class: "hub-v3-chip-name", text: name }));
  let sub = null;
  // Sur le dessin, hors Lumières : une deuxième ligne possible (consigne, artiste).
  if (!withName && lens !== "lumieres") {
    sub = el("span", { class: "hub-v3-pin-sub" });
    sub.hidden = true;
    parts.push(el("span", { class: "hub-v3-pin-text" }, count, sub));
  } else {
    parts.push(count);
  }
  const attributes = { type: "button", class: cls };
  if (!withName) attributes.title = name;
  const node = el("button", attributes, ...parts);
  node.addEventListener("click", () => {
    if (hubV3Tap(lens, name)) hubV3Wait(node);
  });
  return { node, count, sub };
}

/** Les appareils d'une pièce que montre une lentille. */
function hubV3Members(lens, area) {
  if (lens === "lumieres") return hubV3RoomLights(area);
  const states = area.entities.map((e) => entities.get(e.entity_id)).filter((s) => s !== undefined);
  switch (lens) {
    case "volets":
      return states.filter(hubV3IsShutter);
    case "chauffage":
      return states.filter((s) => (domainOf(s) === "climate" && s.entity_id !== HUB_V3_WATER)
        || (domainOf(s) === "sensor" && s.attributes.device_class === "temperature"
          && Number.isFinite(Number(s.state))));
    case "musique":
      return states.filter((s) => domainOf(s) === "media_player");
    case "securite":
      return states.filter(hubV3IsSecurity);
    default:
      return [];
  }
}

/** Le mot d'un appareil de sécurité : « Fermée », « Verrouillé », « Rien à signaler »… */
function hubV3SecurityWord(state) {
  const domain = domainOf(state);
  const s = state.state;
  if (s === "unavailable" || s === "unknown") return statusText(state);
  if (domain === "binary_sensor") {
    const dc = state.attributes.device_class;
    if (HUB_V3_OPENINGS.includes(dc)) return agree(state, s === "on" ? "Ouvert" : "Fermé");
    if (s !== "on") return "Rien à signaler";
    const word = lookup(HUB_V3_ALERT_WORDS, dc) ?? "alerte";
    return `${word.charAt(0).toUpperCase()}${word.slice(1)} !`;
  }
  if (domain === "lock") return agree(state, lookup(LOCK_LABELS, s) ?? statusText(state));
  if (domain === "cover") return coverText(state);
  if (domain === "alarm_control_panel") {
    const known = lookup(ALARM_LABELS, s);
    if (known) return known[0];
    if (s === "triggered") return "Intrusion !";
    return s.startsWith("armed") ? "Activée" : statusText(state);
  }
  if (domain === "siren") return s === "on" ? "Sonne !" : statusText(state);
  return statusText(state);
}

/** Ce qui cloche chez un appareil de sécurité, ou null. */
function hubV3SecurityIssue(state) {
  const domain = domainOf(state);
  const dc = state.attributes.device_class;
  const s = state.state;
  const name = nameOf(state);
  if (domain === "binary_sensor" && s === "on") {
    if (HUB_V3_OPENINGS.includes(dc)) return `${name} ${agree(state, "ouvert")}`;
    if (HUB_V3_ALERTS.includes(dc)) return `${name} : ${lookup(HUB_V3_ALERT_WORDS, dc) ?? "alerte"}`;
  }
  const lockIssue = domain === "lock" ? lookup(HUB_V3_LOCK_ISSUES, s) : undefined;
  if (lockIssue) return `${name} ${agree(state, lockIssue)}`;
  if (domain === "cover") {
    const text = alertText(state);
    return text === null ? null : `${name} ${text}`;
  }
  if (domain === "alarm_control_panel") {
    if (s === "triggered") return `${name} : intrusion`;
    if (s === "detection" || s === "sos") return `${name} : ${lookup(ALARM_LABELS, s)[0].toLowerCase()}`;
  }
  if (domain === "siren" && s === "on") return `${name} sonne`;
  return null;
}

/** Tout ce qui cloche dans la maison, côté sécurité. */
function hubV3Issues() {
  return [...entities.values()].filter(hubV3IsSecurity).map(hubV3SecurityIssue).filter(Boolean);
}

/** Ce que montre une pastille hors Lumières : texte, deuxième ligne, état, étiquette. */
function hubV3Look(lens, name, members) {
  const available = (s) => s.state !== "unavailable" && s.state !== "unknown";
  switch (lens) {
    case "volets": {
      const open = members.filter((s) => s.state === "open" || s.state === "opening").length;
      const allowed = members.some((s) => canControl(s) && available(s));
      const text = members.length === 1 ? coverText(members[0]) : hubV3CoverSummary(open, members.length);
      return { text, sub: "", quiet: open === 0, alert: false, pressed: open > 0, disabled: !allowed,
        label: hubV3CoverLabel(name, open, members.length, allowed), key: `${text}|${open}` };
    }
    case "chauffage": {
      const climates = members.filter((s) => domainOf(s) === "climate");
      const climate = climates[0] ?? null;
      const sensor = roomThermometer(members);
      const ct = climate ? climate.attributes.current_temperature : null;
      const current = typeof ct === "number" && Number.isFinite(ct) ? ct : sensor ? Number(sensor.state) : null;
      const heating = climates.some((s) => s.attributes.hvac_action === "heating");
      const mode = climate ? heatMode(climate) : "";
      const target = climate && climate.state !== "off" && available(climate) ? heatTarget(climate) : null;
      const text = current !== null ? hubV3Temp(current) : mode;
      const sub = target !== null ? `consigne ${hubV3Temp(target)}` : "";
      const said = [current !== null ? `il fait ${hubV3Temp(current)}` : "", sub, mode.toLowerCase()]
        .filter(Boolean).join(", ");
      return { text, sub, quiet: !heating, alert: false, pressed: null, disabled: false,
        label: `${name} : ${said}. Toucher pour ouvrir la pièce.`, key: `${text}|${heating}` };
    }
    case "musique": {
      const player = members.find((s) => s.state === "playing") ?? members.find((s) => s.state === "paused")
        ?? members[0];
      const playing = player.state === "playing";
      const title = playing ? hubV3MusicTitle(player) : "";
      const status = available(player) ? lookup(MEDIA_LABELS, player.state) ?? statusText(player) : "Injoignable";
      const text = title || status;
      const sub = playing ? hubV3Clean(player.attributes.media_artist) : "";
      const said = title ? `${title}, en lecture` : status.toLowerCase();
      return { text, sub, quiet: !playing, alert: false, pressed: null, disabled: false,
        label: `${name} : ${said}. Toucher pour ouvrir le lecteur.`, key: text };
    }
    default: {
      const issues = members.map(hubV3SecurityIssue).filter(Boolean);
      const text = members.length === 1 ? hubV3SecurityWord(members[0]) : hubV3SecurityShort(issues.length);
      const said = issues.length === 0 ? "tout est en ordre" : issues.join(", ");
      return { text, sub: "", quiet: issues.length === 0, alert: issues.length > 0, pressed: null, disabled: false,
        label: `${name} : ${said}. Toucher pour ouvrir la pièce.`, key: text };
    }
  }
}

/** Met à jour un bouton de pièce d'après la lentille. Lumières garde son propre rendu. */
function hubV3Paint(lens, item, name, members) {
  if (lens === "lumieres") {
    hubV3PaintRoom(item, name, members);
    return;
  }
  const look = hubV3Look(lens, name, members);
  setText(item.count, look.text);
  if (item.sub) {
    setText(item.sub, look.sub);
    item.sub.hidden = look.sub === "";
  }
  item.node.classList.toggle("zero", look.quiet);
  item.node.classList.toggle("alert", look.alert);
  if (look.pressed !== null) setAttr(item.node, "aria-pressed", String(look.pressed));
  setAttr(item.node, "aria-label", look.label);
  item.node.disabled = look.disabled;
  if (item.node.dataset.seen !== look.key) {
    item.node.dataset.seen = look.key;
    item.node.classList.remove("pending");
  }
}

/** Ferme les volets ouverts ; s'ils sont tous fermés, les ouvre. Faux si rien n'est parti. */
function hubV3MoveCovers(covers) {
  const movable = covers.filter((s) => canControl(s) && s.state !== "unavailable" && s.state !== "unknown");
  const open = movable.filter((s) => s.state === "open" || s.state === "opening");
  const service = open.length > 0 ? "close_cover" : "open_cover";
  const targets = open.length > 0 ? open : movable;
  for (const state of targets) {
    markPending(state.entity_id);
    callService("cover", service, { entity_id: state.entity_id });
  }
  return targets.length > 0;
}

/** Toucher une pastille. Vrai si une commande est partie (la pastille attend la réponse).
 *  Chauffage et Sécurité ouvrent la pièce : jamais de déverrouillage d'un seul geste. */
function hubV3Tap(lens, name) {
  if (lens === "lumieres") return hubV3ToggleRoom(name);
  const area = areas.find((a) => a.name === name);
  if (!area) return false;
  if (lens === "volets") return hubV3MoveCovers(hubV3Members("volets", area));
  if (lens === "musique") {
    const players = hubV3Members("musique", area);
    const player = players.find((s) => s.state === "playing") ?? players[0];
    if (player) {
      leaveHub();
      openMusic(player.entity_id);
    }
    return false;
  }
  openRoom(name);
  return false;
}

/** Le bouton de la phrase sous la barre, ou null. */
function hubV3Action(lens) {
  const all = [...entities.values()];
  if (lens === "lumieres") {
    return all.some((s) => isLight(s) && s.state === "on" && canControl(s)) ? "Tout éteindre" : null;
  }
  if (lens === "volets") {
    const covers = all.filter((s) => hubV3IsShutter(s) && canControl(s)
      && s.state !== "unavailable" && s.state !== "unknown");
    if (covers.length === 0) return null;
    return covers.some((s) => s.state === "open" || s.state === "opening") ? "Tout fermer" : "Tout ouvrir";
  }
  if (lens === "musique") {
    const player = musicPlayer();
    if (!player || !canControl(player) || player.state === "unavailable" || player.state === "unknown") return null;
    return player.state === "playing" ? "Pause" : "Lecture";
  }
  return null;
}

/** Le geste du bouton de la phrase. Vrai si une commande est partie. */
function hubV3ActionRun(lens) {
  if (lens === "lumieres") {
    // Tout éteindre : une commande par lumière allumée que ce compte commande,
    // comme « Éteindre les lumières » dans la fiche d'une pièce.
    const lit = [...entities.values()].filter((s) => isLight(s) && s.state === "on" && canControl(s));
    for (const state of lit) {
      markPending(state.entity_id);
      callService("light", "turn_off", { entity_id: state.entity_id });
    }
    return lit.length > 0;
  }
  if (lens === "volets") return hubV3MoveCovers([...entities.values()].filter(hubV3IsShutter));
  if (lens === "musique") {
    const player = musicPlayer();
    if (!player || !canControl(player)) return false;
    musicCommand(player, player.state === "playing" ? "media_pause" : "media_play");
    return true;
  }
  return false;
}

/** Met à jour un bouton de pièce d'après ses lumières. */
function hubV3PaintRoom(item, name, lights) {
  const lit = lights.filter((s) => s.state === "on").length;
  const allowed = lights.some(canControl);
  setText(item.count, String(lit));
  item.node.classList.toggle("zero", lit === 0);
  setAttr(item.node, "aria-pressed", String(lit > 0));
  setAttr(item.node, "aria-label", hubV3PinLabel(name, lit, lights.length, allowed));
  item.node.disabled = !allowed;
  // Le nombre a changé : la réponse est arrivée, le bouton redevient vif.
  if (item.node.dataset.lit !== String(lit)) {
    item.node.dataset.lit = String(lit);
    item.node.classList.remove("pending");
  }
}

/** Range les enfants dans l'ordre voulu, sans toucher ceux déjà à leur place. */
function hubV3Order(parent, nodes) {
  nodes.forEach((node, index) => {
    if (parent.children[index] !== node) parent.insertBefore(node, parent.children[index] ?? null);
  });
}

/** Garde un bouton par pièce voulue : retire les autres, crée les manquants. */
function hubV3Sync(map, parent, wanted, make, lens) {
  const names = new Set(wanted.map((w) => w.name));
  for (const [name, item] of map) {
    if (!names.has(name)) {
      item.node.remove();
      map.delete(name);
    }
  }
  const items = wanted.map((w) => {
    let item = map.get(w.name);
    if (!item) {
      item = make(w.name);
      map.set(w.name, item);
    }
    hubV3Paint(lens, item, w.name, w.members);
    return item;
  });
  hubV3Order(parent, items.map((i) => i.node));
  return items;
}

/** Une pastille par pièce sur le dessin, une rangée pour les autres, selon la lentille. */
function hubV3RenderPins() {
  const ui = hubV3.ui;
  if (!ui || !hubV3.placed) return;
  const lens = hubV3.lens;
  // Nouvelle lentille : des pastilles neuves, qui entrent l'une après l'autre.
  if (hubV3.pinLens !== lens) {
    for (const map of [hubV3.pins, hubV3.chips]) {
      for (const item of map.values()) item.node.remove();
      map.clear();
    }
    hubV3.pinLens = lens;
  }
  const config = lookup(HUB_V3_PIN_LENSES, lens);
  if (!config) {
    ui.rooms.hidden = true;
    return;
  }
  setAttr(ui.pins, "data-lens", lens);
  setAttr(ui.pins, "aria-label", config.pins);
  setAttr(ui.rooms, "data-lens", lens);
  setAttr(ui.rooms, "aria-label", config.rooms);
  const view = house.view;
  // La plus grande zone de chaque pièce porte sa pastille.
  const largest = new Map();
  for (const { zone } of view?.zones ?? []) {
    if (!zone.area) continue;
    const known = largest.get(zone.area);
    if (!known || zone.w * zone.h > known.w * known.h) largest.set(zone.area, zone);
  }
  const onDrawing = [];
  for (const [name, zone] of largest) {
    const area = areas.find((a) => a.name === name);
    if (!area) continue;
    const members = hubV3Members(lens, area);
    if (members.length > 0) onDrawing.push({ name, members, at: hubV3ZoneCenter(zone) });
  }
  // De haut en bas puis de gauche à droite : l'ordre d'entrée et de lecture.
  onDrawing.sort((a, b) => a.at.y - b.at.y || a.at.x - b.at.x);
  if (view && ui.pins.parentNode !== view.stage) view.stage.append(ui.pins);
  const pins = hubV3Sync(hubV3.pins, ui.pins, onDrawing,
    (name) => hubV3RoomButton(name, "hub-v3-pin", false, lens), lens);
  pins.forEach((item, index) => {
    const { x, y } = onDrawing[index].at;
    if (item.node.style.left !== `${x}%`) item.node.style.left = `${x}%`;
    if (item.node.style.top !== `${y}%`) item.node.style.top = `${y}%`;
    item.node.style.setProperty("--d", String(Math.min(index, HUB_V3_STAGGER_MAX)));
  });
  // Les pièces qu'aucun onglet ne dessine : une rangée en bas.
  const drawn = new Set((house.pages ?? []).flatMap((page) => page.zones.map((zone) => zone.area)));
  for (const name of largest.keys()) drawn.add(name);
  const offDrawing = shownAreas()
    .filter((area) => !drawn.has(area.name))
    .map((area) => ({ name: area.name, members: hubV3Members(lens, area) }))
    .filter((w) => w.members.length > 0);
  hubV3Sync(hubV3.chips, ui.rooms, offDrawing,
    (name) => hubV3RoomButton(name, "hub-v3-room-chip", true, lens), lens);
  ui.rooms.hidden = offDrawing.length === 0;
}

document.addEventListener("DOMContentLoaded", hubV3Init);
