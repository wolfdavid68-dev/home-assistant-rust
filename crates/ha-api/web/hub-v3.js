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
// Les capteurs qui comptent pour la sécurité (une porte ouverte, une fumée…).
const HUB_V3_SECURITY = ["door", "window", "garage_door", "opening", "motion", "occupancy", "presence", "smoke",
  "moisture", "gas", "safety", "tamper"];
const HUB_V3_OPENINGS = ["door", "window", "garage_door", "opening"];
const hubV3 = { placed: false, lens: "vivant", ui: null };

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

/** La lentille d'un appareil, ou "" s'il n'en a pas. */
function hubV3Kind(domain, deviceClass) {
  if (domain === "light") return deviceClass === "outlet" ? "" : "lumieres";
  if (domain === "cover") return "volets";
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
  // retrouvent à leur lentille.
  const stage = $("#house-stage");
  if (stage) {
    const observer = new MutationObserver(() => {
      if (hubV3On() && hubV3.placed) hubV3Tag();
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
    readout: el("p", { id: "hub-v3-readout", class: "hub-v3-readout", role: "status" }),
    musicBtn: button({ id: "hub-v3-music-btn" }, svgIcon(ICONS.music), el("span", { text: "Musique" })),
  };
  ui.music.hidden = true;
  ui.tempo.hidden = true;
  ui.heating.hidden = true;
  ui.readout.hidden = true;
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
  root.querySelector(".hub-center").prepend(ui.lens, ui.readout);
  $("#hub-open-agenda").after(ui.musicBtn);
  hubV3.placed = true;
}

/** Remet les blocs à leur place d'origine et retire ce qui a été ajouté. */
function hubV3Restore() {
  const ui = hubV3.ui;
  const root = $("#hub");
  root.querySelector(".hub-left").append(root.querySelector(".hub-time"), $("#hub-weather"), $("#hub-agenda"));
  root.querySelector(".hub-right").append($("#hub-scenes"), $("#hub-rooms"));
  for (const node of [ui.hello, ui.summary, ui.music, ui.tempo, ui.heating, ui.actions, ui.lens, ui.readout, ui.musicBtn]) {
    node.remove();
  }
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

/** Les compteurs de la maison : lumières, volets, ouvertures. */
function hubV3Counts() {
  const states = [...entities.values()];
  const covers = states.filter((s) => domainOf(s) === "cover");
  return {
    lights: states.filter(isLight).length,
    lit: states.filter((s) => isLight(s) && s.state === "on").length,
    covers: covers.length,
    open: covers.filter((s) => s.state === "open").length,
    openings: states.filter((s) => domainOf(s) === "binary_sensor"
      && HUB_V3_OPENINGS.includes(s.attributes.device_class) && s.state === "on").length,
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
  const text = (value) => (typeof value === "string" && value.trim() !== "" ? value.trim() : "");
  const title = active ? text(a.media_title) || text(a.media_channel) || text(a.source) : "";
  const status = available ? lookup(MEDIA_LABELS, state.state) ?? statusText(state) : "Injoignable";
  setText(ui.musicTitle, title || "Musique");
  setText(ui.musicSub, [active ? text(a.media_artist) || status : status, nameOf(state)].filter(Boolean).join(" · "));
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
    if (state.entity_id === "climate.eau_chaude") classes.push("water");
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
      return counts.open > 0 ? hubV3Count(counts.open, "volet ouvert", "volets ouverts") : "Tous les volets sont fermés";
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
      if (player && player.state === "playing") {
        const title = typeof player.attributes.media_title === "string" ? player.attributes.media_title.trim() : "";
        return `${title || "Musique"} · ${nameOf(player)}`;
      }
      return "Pas de musique en ce moment";
    }
    case "securite":
      if (counts.openings === 0) return "Tout est fermé";
      if (counts.openings === 1) return "1 porte ou fenêtre ouverte";
      return `${counts.openings} portes ou fenêtres ouvertes`;
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
  ui.readout.hidden = text === "";
  hubV3Tag();
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

document.addEventListener("DOMContentLoaded", hubV3Init);
