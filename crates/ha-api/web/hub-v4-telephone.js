// Accueil v4 sur le téléphone : la vue étroite de la maquette « Hub maison ».
//
// Sur un écran étroit, quand la v4 est choisie (Mon compte → Style de
// l'accueil), l'accueil v4 s'affiche aussi, mais en colonne, comme la planche
// « Téléphone » de la maquette : la date et l'heure, « En direct » et l'avatar,
// le salut en serif, le résumé, l'ordre qui attend, le dessin de la maison sous
// la lune (le soleil le jour), ses onglets (vue de dehors, étages, « Toute la
// maison »), les lentilles, les réglages de la lentille choisie (chauffage,
// volets, lumières), quatre cartes (météo, chauffage, Tempo, agenda), la
// musique dans le flux, et cinq onglets fixés en bas. Le dessin n'est pas
// recopié : c'est le même #hub-house que sur la tablette, déplacé ici puis
// remis à sa place. Toucher une pièce, une carte ou la musique ouvre les mêmes
// incrustations que sur la tablette, qui montent du bas de l'écran (hub-v4.js).
//
// Styles en classes Tailwind, couleurs par les jetons du thème (nuit en
// Sombre, jour en Clair). Chargé après hub-v4-sonnette.js. Pas d'innerHTML.
"use strict";

// Plus étroit que ça : un téléphone (le seuil « sm » de Tailwind, comme les
// incrustations qui montent du bas).
const hubV4TelMedia = matchMedia("(max-width: 639px)");
const hubV4Tel = { ui: null, placed: false };
// Le verre des cartes : celui de la tablette (indigo la nuit, crème le jour).
const HUB_V4_TEL_GLASS = "bg-v-glass backdrop-blur-[18px] backdrop-saturate-150 shadow-[var(--v3-glass-shadow)]";
// « dim. 1 nov. » : le jour d'un rendez-vous plus lointain.
const hubV4TelDay = new Intl.DateTimeFormat("fr", { weekday: "short", day: "numeric", month: "short" });

/** L'accueil v4 en colonne, sur ce téléphone ? */
function hubV4Phone() {
  return hubV4On() && hubV4TelMedia.matches;
}

/** « 1 ordre attend ta validation », « 2 ordres attendent… » ("" sans ordre). */
function hubV4TelApproval(count) {
  if (!(count > 0)) return "";
  return count === 1 ? "1 ordre attend ta validation" : `${count} ordres attendent ta validation`;
}

/** Le nom court d'une zone de chauffage : « Chauffage étage » → « étage ». */
function hubV4TelZoneName(name) {
  const rest = String(name ?? "").replace(/^(chauffage|radiateurs?|thermostat)\s+/i, "").trim();
  return rest || String(name ?? "");
}

/** La carte du chauffage : la température de deux zones au plus.
 *  `zones` : [{ name, temperature }]. */
function hubV4TelHeat(zones) {
  const shown = zones.filter((z) => Number.isFinite(z.temperature)).slice(0, 2);
  if (shown.length === 0) return null;
  return {
    value: shown.map((z) => `${Math.round(z.temperature)}°`).join(" · "),
    sub: shown.map((z) => hubV4TelZoneName(z.name)).join(" · "),
  };
}

/** La carte de l'agenda, en deux lignes comme la maquette : aujourd'hui
 *  (« Rien aujourd'hui », « 17:30 · Dentiste », « En cours · Réunion ») et le
 *  rendez-vous suivant (« Piscine demain 09:05 », « Toussaint dim. 1 nov. »,
 *  "" sans rien). `events` : [{ summary, startDate, endDate, allDay }]. */
function hubV4TelAgenda(events, now) {
  const midnight = new Date(now);
  midnight.setHours(24, 0, 0, 0);
  const tomorrow = new Date(now);
  tomorrow.setHours(48, 0, 0, 0);
  const coming = events
    .filter((e) => e.endDate > now)
    .sort((a, b) => a.startDate - b.startDate || Number(b.allDay) - Number(a.allDay));
  const todays = coming.filter((e) => e.startDate < midnight);
  const timed = todays.find((e) => !e.allDay);
  const day = todays.find((e) => e.allDay);
  let today = "Rien aujourd'hui";
  if (timed) today = `${timed.startDate <= now ? "En cours" : hubV4Clock(timed.startDate)} · ${timed.summary}`;
  else if (day) today = `${day.summary} aujourd'hui`;
  const next = coming.find((e) => e.startDate >= midnight);
  let later = "";
  if (next) {
    const date = next.startDate < tomorrow ? "demain" : hubV4TelDay.format(next.startDate);
    later = next.allDay ? `${next.summary} ${date}` : `${next.summary} ${date} ${hubV4Clock(next.startDate)}`;
  }
  return { today, later };
}

/** « Bleu aujourd'hui » : l'adjectif de Tempo, avec sa majuscule. */
function hubV4TelTempo(adjective, when) {
  const word = String(adjective ?? "");
  return word ? `${word.charAt(0).toUpperCase()}${word.slice(1)} ${when}` : "";
}

/** L'onglet où se touchent les pièces : celui affiché s'il a des zones de
 *  pièce, sinon le premier qui en a (une vue de dehors n'en a pas), sinon null. */
function hubV4TelRoomsPage(pages, current) {
  const rooms = (page) => (page?.zones ?? []).some((zone) => typeof zone.area === "string" && zone.area !== "");
  const shown = pages.find((page) => page.id === current);
  if (rooms(shown)) return shown.id;
  return pages.find(rooms)?.id ?? null;
}

/** Les appareils rangés pièce par pièce (dans l'ordre des pièces), puis par
 *  nom ; ceux d'aucune pièce à la fin. `items` : [{ id, name, area }]. */
function hubV4TelOrder(items, rooms) {
  const rank = (area) => {
    const i = rooms.indexOf(area);
    return i === -1 ? rooms.length : i;
  };
  return [...items]
    .sort((a, b) => rank(a.area) - rank(b.area) || a.name.localeCompare(b.name, "fr"))
    .map((item) => item.id);
}

/** Les lentilles qui ont leurs réglages sous le dessin. */
const HUB_V4_TEL_PANELS = ["lumieres", "volets", "chauffage"];

// ---------------------------------------------------------------- Les éléments
// Un bouton d'ici n'a pas sa propre taille de texte : une règle de la page
// (`[&_button]:[font:inherit]`, plus précise qu'une classe seule) la remet à
// celle du parent. La taille se pose donc sur le texte, dans un <span>.

/** Une carte de la grille : petit titre avec son icône (teintée par `tone`),
 *  puis son contenu, en bas. */
function hubV4TelCard(icon, tone, title, open) {
  const body = el("span", { class: "flex flex-col items-start gap-1 min-w-0 max-w-full" });
  const node = el("button", {
    type: "button",
    class: "flex flex-col items-stretch justify-between gap-2.5 min-h-24 min-w-0 p-3.5 rounded-panneau border-0 text-left "
      + `text-v-ink cursor-pointer ${HUB_V4_TEL_GLASS} ${HUB_V4_FOCUS}`,
  }, el("span", { class: `flex items-center gap-1.5 text-legende font-medium ${HUB_V4_MUTE}` },
    el("span", { class: `grid shrink-0 ${tone} [&_svg]:size-3.5` }, svgIcon(icon)), title), body);
  node.addEventListener("click", open);
  return { node, body };
}

/** Un grand chiffre fin (« 11,7° », « 18° · 19° »). */
function hubV4TelBig() {
  return el("b", { class: "font-chiffres text-titre-l font-extralight leading-none tracking-[-0.02em] whitespace-nowrap" });
}

/** Une ligne de texte d'une carte. */
function hubV4TelLine(tone) {
  return el("span", { class: `block max-w-full text-legende leading-snug truncate ${tone}` });
}

/** Un onglet du bas. */
function hubV4TelTab(icon, label, run) {
  const node = el("button", {
    type: "button",
    class: `flex flex-col items-center justify-center gap-1 min-w-0 min-h-12 p-0 border-0 bg-transparent ${HUB_V4_MUTE} `
      + `cursor-pointer aria-[current=page]:text-v-amber [&_svg]:size-[22px] ${HUB_V4_FOCUS}`,
  }, svgIcon(icon), el("span", { class: "max-w-full text-legende font-medium leading-none truncate", text: label }));
  node.addEventListener("click", run);
  return node;
}

/** Quitte l'accueil pour une autre vue ; le bouton Maison du bas y ramène. */
function hubV4TelLeave(open) {
  return () => {
    hubV4Close();
    leaveHub();
    open?.();
  };
}

/** Les éléments de la vue téléphone, créés une seule fois, avec leurs gestes. */
function hubV4TelBuild() {
  if (hubV4Tel.ui) return hubV4Tel.ui;
  const button = (attributes, ...children) => el("button", { type: "button", ...attributes }, ...children);
  const ui = {
    when: el("span", { class: "flex-1 min-w-0 text-legende font-medium text-v-ink2 truncate" }),
    live: el("span", {
      class: "flex items-center gap-1.5 shrink-0 h-7 px-2.5 rounded-full bg-v-ok/10 ring-1 ring-inset ring-v-ok/25 "
        + "text-legende font-medium text-v-ink data-off:bg-v-alert/10 data-off:ring-v-alert/25",
    }),
    bell: button({
      class: `relative grid place-items-center size-11 shrink-0 p-0 rounded-full border-0 bg-transparent text-v-ink `
        + `cursor-pointer [&_svg]:size-5 ${HUB_V4_FOCUS}`,
      "aria-label": "Notifications",
    }, svgIcon(ICONS.bell)),
    badge: el("span", {
      class: "absolute top-0.5 right-0.5 grid place-items-center min-w-5 h-5 px-1 rounded-full bg-v-amber text-micro font-bold text-v-inverse",
    }),
    // L'avatar : la lettre du compte sur un disque ambre-rose, le point vert
    // quand quelqu'un est à la maison. Le toucher montre qui est là.
    avatar: button({
      class: `grid place-items-center size-11 shrink-0 -mr-1.5 p-0 rounded-full border-0 bg-transparent cursor-pointer ${HUB_V4_FOCUS}`,
      "aria-label": "Qui est là",
    }),
    avatarDisc: el("span", {
      class: "relative grid place-items-center size-8 rounded-full bg-[linear-gradient(140deg,var(--v3-amber),var(--v3-pink))] "
        + "text-corps font-semibold text-v-inverse [&_svg]:size-4",
    }),
    avatarDot: el("span", {
      class: "absolute -right-0.5 -bottom-0.5 size-3 rounded-full bg-v-ok ring-2 ring-scrim",
      "aria-hidden": "true",
    }),
    avatarLetter: el("span", { "aria-hidden": "true" }),
    hello: el("h1", { class: "m-0 font-recit text-affiche font-normal leading-[1.05] tracking-[-0.01em] text-v-ink" }),
    calm: el("em", { class: "block italic text-v-amber" }),
    summary: el("p", { class: "m-0 mt-2 text-corps leading-snug text-v-ink2" }),
    approval: el("div", { class: "flex px-5", role: "status" }),
    approvalText: el("span", { class: "min-w-0 text-legende font-medium text-v-ink2 truncate" }),
    approvalGo: button({
      class: "flex items-center gap-2.5 max-w-full min-h-11 pl-3.5 pr-4 rounded-champ border-0 bg-v-amber/12 "
        + "ring-1 ring-inset ring-v-amber/25 backdrop-blur-[12px] text-left text-v-ink cursor-pointer "
        + `[&_svg]:size-4 ${HUB_V4_FOCUS}`,
    }),
    houseSlot: el("div", { class: "relative shrink-0" }),
    // La lune (le soleil le jour) au-dessus de la maison, derrière elle ;
    // voilée par les nuages, absente sous la pluie, comme sur la tablette.
    sky: el("span", {
      class: "pointer-events-none absolute top-[6%] right-[8%] -z-1 size-9 rounded-full bg-[image:var(--v4-moon)] "
        + "shadow-[var(--v4-moon-halo)] [[data-theme=light]_&]:size-10 [[data-theme=light]_&]:bg-[image:var(--v4-sun)] "
        + "[[data-theme=light]_&]:shadow-[var(--v4-sun-halo)] [[data-meteo=eclaircies]_&]:opacity-80 "
        + "[:is([data-meteo=nuageux],[data-meteo=vent],[data-meteo=brouillard])_&]:opacity-20 "
        + "[:is([data-meteo=nuageux],[data-meteo=vent],[data-meteo=brouillard])_&]:blur-[3px] "
        + "[:is([data-meteo=pluie],[data-meteo=averses],[data-meteo=orage],[data-meteo=grele],[data-meteo=neige],[data-meteo=pluie-neige])_&]:hidden",
      "aria-hidden": "true",
    }),
    // Les onglets du dessin (vue de dehors, étages) et « Toute la maison »,
    // posés en haut du dessin.
    pages: el("nav", {
      class: "absolute top-0 inset-x-0 z-[2] flex gap-0.5 px-3 overflow-x-auto [scrollbar-width:none] [&::-webkit-scrollbar]:hidden",
      "aria-label": "Vue",
    }),
    pagesMark: "",
    lens: el("nav", {
      class: "flex gap-2 px-4 overflow-x-auto scroll-px-4 [scrollbar-width:none] [&::-webkit-scrollbar]:hidden",
      "aria-label": "Ce que montre la maison",
    }),
    cards: el("div", { class: "grid grid-cols-2 gap-2.5 px-4" }),
    // La musique, dans le flux sous les cartes (comme la maquette) : rien
    // n'est caché sous elle.
    music: el("section", {
      class: `flex items-center gap-1 mx-4 p-2 pr-1 rounded-panneau text-v-ink ${HUB_V4_TEL_GLASS}`,
      "aria-label": "Musique",
    }),
    musicOpen: button({ class: `flex flex-1 items-center gap-3 min-w-0 min-h-12 p-0 border-0 bg-transparent text-left text-inherit cursor-pointer rounded-champ ${HUB_V4_FOCUS}` }),
    musicImg: el("img", { class: "absolute inset-0 size-full object-cover", alt: "", decoding: "async", draggable: "false" }),
    musicTitle: el("b", { class: "text-corps font-semibold leading-snug truncate" }),
    musicSub: el("small", { class: `text-legende leading-snug ${HUB_V4_MUTE} truncate` }),
    play: button({ class: `grid place-items-center size-11 shrink-0 p-0 rounded-full border-0 bg-transparent text-inherit cursor-pointer disabled:opacity-40 [&_svg]:size-5 ${HUB_V4_FOCUS}` }),
    next: button({ class: `grid place-items-center size-11 shrink-0 p-0 rounded-full border-0 bg-transparent text-inherit cursor-pointer disabled:opacity-40 [&_svg]:size-5 ${HUB_V4_FOCUS}`, "aria-label": "Morceau suivant" },
      svgIcon(ICONS.next)),
  };
  ui.musicImg.hidden = true;
  ui.bell.append(ui.badge);
  ui.bell.addEventListener("click", () => hubV4Bell());
  ui.avatarDisc.append(ui.avatarLetter, ui.avatarDot);
  ui.avatar.append(ui.avatarDisc);
  ui.avatar.addEventListener("click", () => hubV4People());
  ui.approvalGo.append(el("span", { class: "grid shrink-0 text-v-amber" }, svgIcon(ICONS.list)), ui.approvalText,
    el("span", { class: "shrink-0 text-petit font-semibold text-v-amber2", text: "Voir" }));
  ui.approval.append(ui.approvalGo);
  ui.approvalGo.addEventListener("click", () => hubV4Approval());
  ui.houseSlot.append(ui.sky, ui.pages);

  // Les lentilles : les mêmes que sur la tablette (hub-v3.js), une pastille
  // « • Vivant », l'ambre sur celle choisie.
  ui.segs = HUB_V3_LENSES.map((lens) => {
    const seg = button({
      class: "flex items-center gap-2 shrink-0 min-h-11 px-4 rounded-full border-0 bg-voile/8 text-v-ink cursor-pointer "
        + `transition-colors aria-pressed:bg-v-amber aria-pressed:text-v-inverse ${HUB_V4_FOCUS}`,
      "data-lens": lens.id,
      "aria-pressed": "false",
    }, el("span", { class: "size-1.5 rounded-full bg-current opacity-60", "aria-hidden": "true" }),
    el("span", { class: "text-petit font-medium", text: lens.label }));
    seg.addEventListener("click", () => {
      hubV3.lens = lens.id;
      // Une lentille montre les pièces : depuis la vue de dehors, l'étage d'abord.
      hubV4TelRoomsShown();
      hubV3RenderLens();
      hubV4TelRenderLens();
      hubV4TelRenderPanel();
    });
    return seg;
  });
  ui.lens.append(...ui.segs);
  hubV4TelPanelBuild(ui);

  ui.weather = hubV4TelCard(ICONS.cloud, HUB_V4_MUTE, "Météo", () => hubV4Weather());
  ui.weatherBig = hubV4TelBig();
  ui.weatherSky = el("span", { class: "min-w-0 text-legende text-v-ink2 truncate" });
  ui.weather.body.append(el("span", { class: "flex items-baseline gap-1.5 min-w-0 max-w-full" }, ui.weatherBig, ui.weatherSky));

  ui.heat = hubV4TelCard(ICONS.temperature, "text-v-heat", "Chauffage", () => {
    // Toutes les zones de chauffage, réglables : la page « Toute la maison ».
    if (typeof hubV4Page === "function" && hubV4Page(HUB_V4_ALL)) return;
    // Sans elle : la lentille du chauffage, sur un étage qui a ses pièces.
    hubV3.lens = "chauffage";
    hubV4TelRoomsShown();
    hubV3RenderLens();
    hubV4TelRenderLens();
    hubV4Tel.ui.houseSlot.scrollIntoView({ block: "center", behavior: hubV4TelMotion() });
  });
  ui.heatBig = hubV4TelBig();
  ui.heatSub = hubV4TelLine("text-v-ink2");
  ui.heat.body.append(ui.heatBig, ui.heatSub);

  ui.tempo = hubV4TelCard(ICONS.bolt, HUB_V4_MUTE, "Tempo", () => hubV4Tempo());
  ui.tempoDot = el("span", {
    class: "inline-block shrink-0 size-2.5 rounded-full bg-voile/40 data-[c=blue]:bg-v-sky "
      + "data-[c=white]:bg-white data-[c=white]:ring-1 data-[c=white]:ring-voile/30 data-[c=red]:bg-v-alert",
    "aria-hidden": "true",
  });
  ui.tempoToday = el("span", { class: "min-w-0 truncate" });
  ui.tempoSub = hubV4TelLine(HUB_V4_MUTE);
  ui.tempo.body.append(el("span", { class: "flex items-center gap-2 max-w-full text-petit font-medium text-v-ink" },
    ui.tempoDot, ui.tempoToday), ui.tempoSub);

  ui.agenda = hubV4TelCard(ICONS.calendar, HUB_V4_MUTE, "Agenda", () => hubV4Agenda());
  ui.agendaToday = el("span", { class: "block max-w-full text-petit font-medium leading-snug text-v-ink truncate" });
  ui.agendaLater = hubV4TelLine("text-v-ink2");
  ui.agenda.body.append(ui.agendaToday, ui.agendaLater);
  ui.cards.append(ui.weather.node, ui.heat.node, ui.tempo.node, ui.agenda.node);

  const art = el("span", {
    class: "relative grid place-items-center size-11 shrink-0 overflow-hidden rounded-[12px] "
      + "bg-[linear-gradient(140deg,var(--v3-violet),var(--v3-pink)_55%,var(--v3-amber))] text-v-inverse [&_svg]:size-5",
    "aria-hidden": "true",
  }, svgIcon(ICONS.music), ui.musicImg);
  ui.musicOpen.append(art, el("span", { class: "flex flex-col min-w-0" }, ui.musicTitle, ui.musicSub));
  ui.music.append(ui.musicOpen, ui.play, ui.next);
  ui.musicOpen.addEventListener("click", () => hubV4Music());
  const act = (handler) => () => {
    const state = musicPlayer();
    if (state && canControl(state)) handler(state);
  };
  ui.play.addEventListener("click", act((state) => musicCommand(state,
    state.state === "playing" ? "media_pause" : "media_play")));
  ui.next.addEventListener("click", act((state) => musicCommand(state, "media_next_track")));

  ui.home = hubV4TelTab(ICONS.home, "Maison", () => {
    hubV4Close();
    $("#hub").scrollTo({ top: 0, behavior: hubV4TelMotion() });
  });
  ui.home.setAttribute("aria-current", "page");
  ui.tabs = el("nav", {
    class: "fixed inset-x-0 bottom-0 z-[1] grid grid-cols-5 px-1 pt-1.5 pb-[max(8px,env(safe-area-inset-bottom))] "
      + "bg-scrim/80 backdrop-blur-[20px] backdrop-saturate-150 border-0 border-t border-solid border-voile/8",
    "aria-label": "Navigation",
  },
  ui.home,
  hubV4TelTab(ICONS.calendar, "Agenda", hubV4TelLeave(() => openAgenda())),
  hubV4TelTab(ICONS.music, "Musique", hubV4TelLeave(() => openMusic())),
  hubV4TelTab(ICONS.bolt, "Énergie", () => {
    if (typeof hubV4Page === "function" && hubV4Page(HUB_V4_ENERGY)) return;
    hubV4TelLeave(() => openEnergy())();
  }),
  hubV4TelTab(HUB_ICONS.grid, "Plus", hubV4TelLeave(null)));

  // En bas : la place des onglets fixés, et un peu d'air sous la musique.
  ui.root = el("div", {
    id: "hub-tel",
    class: "hub-v4-tel hidden [[data-hub-tel]_&]:flex flex-col gap-3.5 min-h-full "
      + "pt-[max(10px,env(safe-area-inset-top))] pb-[calc(92px+env(safe-area-inset-bottom))] "
      + "text-v-ink [font-variant-numeric:tabular-nums]",
  },
  el("header", { class: "flex items-center gap-2 pl-5 pr-4" }, ui.when, ui.live, ui.bell, ui.avatar),
  el("div", { class: "px-5 pt-1" }, ui.hello, ui.summary),
  ui.approval, ui.houseSlot, ui.lens, ui.panel, ui.cards, ui.music, ui.tabs);

  // En colonne, l'accueil défile ; seule la vue téléphone s'y affiche.
  $("#hub").classList.add("[[data-hub-tel]_&]:block!", "[[data-hub-tel]_&]:p-0!", "[[data-hub-tel]_&]:overflow-y-auto!",
    "[[data-hub-tel]_&]:overscroll-contain", "[[data-hub-tel]_&>:not(.hub-v4-tel)]:hidden!");
  // Le dessin, dans la colonne : une hauteur fixe à la place du centre de la tablette.
  $("#hub-house").classList.add("[[data-hub-tel]_&]:relative!", "[[data-hub-tel]_&]:inset-auto!",
    "[[data-hub-tel]_&]:h-[clamp(230px,34dvh,320px)]!", "[[data-hub-tel]_&]:mx-2",
    // La carte du lave-linge, plus petite : le dessin est bas, ses pièces restent à toucher.
    "[[data-hub-tel]_&_.machine-live]:scale-75", "[[data-hub-tel]_&_.machine-live]:origin-bottom-left");
  $("#hub").prepend(ui.root);
  hubV4Tel.ui = ui;
  return ui;
}

/** Les réglages de la lentille, sous le dessin : sur le téléphone, toucher une
 *  pastille du dessin réduit n'est pas évident. Chauffage : chaque zone avec
 *  Confort/Éco et − / +. Volets : chacun avec Ouvrir / Fermer, et « Tout
 *  fermer ». Lumières : les pièces, qui s'ouvrent en incrustation. */
function hubV4TelPanelBuild(ui) {
  ui.panel = el("section", { class: "flex flex-col gap-2.5 px-4" });
  ui.panelLens = "";

  ui.heatList = el("div", { class: "flex flex-col gap-2.5" });
  ui.syncHeat = hubV4Keyed(ui.heatList, (id) => hubV4HeatTile(id, nameOf(entities.get(id))));

  const covers = hubV4Tile(ICONS.blinds, "sky", "Volets");
  ui.coverAll = el("button", { type: "button", class: HUB_V4_BTN });
  ui.coverAll.addEventListener("click", () => {
    hubV3ActionRun("volets", ui.coverAll);
  });
  covers.head.append(ui.coverAll);
  const coverList = el("div", { class: "flex flex-col gap-4" });
  covers.node.append(coverList);
  ui.covers = covers.node;
  ui.syncCovers = hubV4Keyed(coverList, (id) => hubV4CoverRow(id, ""));

  const lights = hubV4Tile(ICONS.bulb, "amber", "Lumières");
  ui.lightsOff = el("button", { type: "button", class: HUB_V4_BTN, text: "Tout éteindre" });
  ui.lightsOff.addEventListener("click", () => {
    hubV3ActionRun("lumieres", ui.lightsOff);
  });
  lights.head.append(ui.lightsOff);
  const roomGrid = el("div", { class: "grid grid-cols-2 gap-2" });
  lights.node.append(roomGrid);
  ui.lights = lights.node;
  ui.syncRooms = hubV4Keyed(roomGrid, (room) => {
    const count = el("span", { class: `text-petit ${HUB_V4_MUTE} group-data-full:text-v-amber2` });
    const node = el("button", {
      type: "button",
      class: "group flex items-center justify-between gap-2 min-h-12 px-3.5 rounded-champ border-0 bg-voile/6 text-left "
        + "text-corps font-semibold text-inherit cursor-pointer transition-colors hover:bg-voile/10 "
        + `data-full:bg-v-amber/16 ${HUB_V4_FOCUS}`,
    }, el("span", { class: "truncate", text: room }), count);
    node.addEventListener("click", () => hubV4Room(room));
    return {
      node,
      refresh: () => {
        const area = areas.find((a) => a.name === room);
        const list = area ? hubV3RoomLights(area) : [];
        const lit = list.filter((s) => s.state === "on").length;
        setText(count, hubV4RoomCount(lit, list.length));
        node.toggleAttribute("data-full", list.length > 0 && lit === list.length);
        setAttr(node, "aria-label", `${room} : ${hubV4LightsText(lit, list.length).toLowerCase()}`);
      },
    };
  });

  ui.panel.append(ui.heatList, ui.covers, ui.lights);
}

/** Les réglages de la lentille choisie (rien pour Vivant, Musique, Sécurité). */
function hubV4TelRenderPanel() {
  const ui = hubV4Tel.ui;
  const lens = hubV3.lens;
  const rooms = areas.filter((a) => !a.hidden).map((a) => a.name);
  const shown = HUB_V4_TEL_PANELS.includes(lens);
  ui.panel.hidden = !shown;
  ui.heatList.hidden = lens !== "chauffage";
  ui.covers.hidden = lens !== "volets";
  ui.lights.hidden = lens !== "lumieres";
  setAttr(ui.panel, "aria-label", shown ? `Réglages : ${HUB_V3_LENSES.find((l) => l.id === lens)?.label}` : "");
  if (lens === "chauffage") {
    // Dans l'ordre de la carte Chauffage : les zones de la maison d'abord.
    ui.syncHeat(hubV4HeatZones().map((zone) => zone.id));
  } else if (lens === "volets") {
    const list = [...entities.values()].filter(hubV3IsShutter)
      .map((s) => ({ id: s.entity_id, name: nameOf(s), area: hubV4AreaOf(s.entity_id) }));
    ui.covers.hidden = list.length === 0;
    ui.syncCovers(hubV4TelOrder(list, rooms));
    const action = hubV3Action("volets");
    ui.coverAll.hidden = action === null;
    if (action !== null && ui.coverAll.textContent !== action) {
      ui.coverAll.classList.remove("pending");
      setText(ui.coverAll, action);
    }
  } else if (lens === "lumieres") {
    const withLights = areas.filter((a) => !a.hidden && hubV3RoomLights(a).length > 0).map((a) => a.name);
    ui.lights.hidden = withLights.length === 0;
    ui.syncRooms(withLights);
    ui.lightsOff.hidden = hubV3Action("lumieres") === null;
  }
  // Une autre lentille : ses réglages se montrent sous la rangée des lentilles.
  if (shown && ui.panelLens !== lens && ui.panelLens !== "") {
    ui.lens.scrollIntoView({ block: "start", behavior: hubV4TelMotion() });
  }
  ui.panelLens = lens;
}

function hubV4TelMotion() {
  return matchMedia("(prefers-reduced-motion: reduce)").matches ? "auto" : "smooth";
}

/** Passe en colonne (ou en revient) : le dessin change de place. */
function hubV4TelSync() {
  const want = hubV4Phone();
  const root = document.documentElement;
  if (want) {
    const ui = hubV4TelBuild();
    root.dataset.hubTel = "";
    if (!hubV4Tel.placed) {
      ui.houseSlot.append($("#hub-house"));
      hubV4Tel.placed = true;
    }
  } else {
    delete root.dataset.hubTel;
    if (hubV4Tel.placed) {
      $("#hub-views").after($("#hub-house"));
      hubV4Tel.placed = false;
    }
  }
  return want;
}

// ---------------------------------------------------------------- Le rendu

/** Affiche un étage qui a ses pièces, si le dessin montre la maison de dehors. */
function hubV4TelRoomsShown() {
  const pages = house.pages ?? [];
  const id = hubV4TelRoomsPage(pages, house.config?.id);
  if (id === null || id === house.config?.id) return;
  selectPage(id);
  hubV4TelRenderPages();
}

/** Les onglets du dessin : refaits seulement s'ils changent (un bouton refait
 *  sous le doigt perd le toucher). */
function hubV4TelRenderPages() {
  const ui = hubV4Tel.ui;
  const pages = house.pages ?? [];
  const all = typeof hubV4Page === "function";
  const current = house.config?.id;
  const mark = JSON.stringify([all, current, ...pages.map((page) => [page.id, page.name])]);
  if (mark === ui.pagesMark) return;
  ui.pagesMark = mark;
  ui.pages.hidden = pages.length < 2 && !all;
  // Une pastille de verre, dans une cible de 44 px ; `pressed` null : un
  // lien (« Toute la maison »), pas un onglet à choisir.
  const pill = (text, pressed, run, ...more) => {
    const node = el("button", {
      type: "button",
      class: `group grid place-items-center shrink-0 min-h-11 px-0.5 border-0 bg-transparent text-v-ink cursor-pointer ${HUB_V4_FOCUS}`,
      ...(pressed === null ? {} : { "aria-pressed": String(pressed) }),
    }, el("span", {
      class: "flex items-center gap-1 px-3 py-1.5 rounded-full bg-scrim/60 backdrop-blur-[10px] ring-1 ring-inset ring-voile/10 "
        + "text-legende font-medium transition-colors group-aria-pressed:bg-v-ink group-aria-pressed:text-v-inverse [&_svg]:size-3.5",
    }, text, ...more));
    node.addEventListener("click", run);
    return node;
  };
  const tabs = pages.length < 2 ? [] : pages.map((page) => pill(page.name, page.id === current, () => {
    if (page.id === house.config?.id) return;
    selectPage(page.id);
    hubV4TelRenderPages();
  }));
  const whole = all ? [pill("Toute la maison", null, () => hubV4Page(HUB_V4_ALL), svgIcon(HUB_V4_ICONS.go))] : [];
  swap(ui.pages, ...tabs, ...whole);
}

function hubV4TelRenderLens() {
  const ui = hubV4Tel.ui;
  for (const seg of ui.segs) setAttr(seg, "aria-pressed", String(seg.dataset.lens === hubV3.lens));
}

function hubV4TelRenderTop(now) {
  const ui = hubV4Tel.ui;
  const live = $("#connection")?.classList.contains("live");
  swap(ui.live, el("i", { class: live ? "size-1.5 rounded-full bg-v-ok" : "size-1.5 rounded-full bg-v-alert" }),
    live ? "En direct" : "Hors ligne");
  ui.live.toggleAttribute("data-off", !live);
  const notices = $("#open-notices");
  const count = $("#notice-count");
  const n = count && !count.hidden ? count.textContent.trim() : "";
  // La cloche, seulement quand une notification attend (la maquette n'en a pas).
  ui.bell.hidden = !notices || notices.hidden || n === "";
  setText(ui.badge, n);
  ui.badge.hidden = n === "";
  setAttr(ui.bell, "aria-label", n ? `Notifications : ${n}` : "Notifications");
  // « Mercredi 25 novembre · 20:36 » ; avec la cloche, la place manque :
  // « Mer. 25 nov. · 20:36 ».
  const day = (ui.bell.hidden ? fullDate : hubV4TelDay).format(now);
  setText(ui.when, `${day.charAt(0).toUpperCase()}${day.slice(1)} · ${hourMinute.format(now)}`);
  const account = $("#open-account");
  const letter = account ? account.textContent.trim().charAt(0).toUpperCase() : "";
  if (letter) swap(ui.avatarLetter, letter);
  else swap(ui.avatarLetter, svgIcon(ICONS.person));
  const home = [...entities.values()].some((s) => domainOf(s) === "person" && s.state === "home");
  ui.avatarDot.hidden = !home;
}

function hubV4TelRenderHello(now) {
  const ui = hubV4Tel.ui;
  // « Bonsoir. » seul quand tout est calme (comme la maquette) ; sinon, ce
  // qui demande l'attention, en italique ambre dessous.
  const count = alerts().length;
  setText(ui.calm, hubV3Attention(count));
  if (count > 0) swap(ui.hello, hubV3Hello(now.getHours()), " ", ui.calm);
  else swap(ui.hello, hubV3Hello(now.getHours()));
  // La phrase du résumé est celle de la tablette (hub-v3.js), déjà à jour.
  const summary = hubV3.ui?.summary.textContent ?? "";
  setText(ui.summary, summary);
  ui.summary.hidden = summary === "";
  const text = hubV4TelApproval(hubV4Approvals().length);
  setText(ui.approvalText, text);
  setAttr(ui.approvalGo, "aria-label", text ? `${text} : voir` : "");
  ui.approval.hidden = text === "";
}

function hubV4TelRenderCards(now) {
  const ui = hubV4Tel.ui;
  const weather = [...entities.values()].find((s) => domainOf(s) === "weather");
  ui.weather.node.hidden = !weather;
  if (weather) {
    const t = weather.attributes.temperature;
    setText(ui.weatherBig, finite(t) ? `${number.format(t)}°` : "—");
    setText(ui.weatherSky, lookup(WEATHER_LABELS, weather.state)?.[0] ?? "");
  }

  const zones = heatingStates(null)
    .filter((s) => s.entity_id !== HUB_V3_WATER)
    .map((s) => ({ name: nameOf(s), temperature: Number(s.attributes.current_temperature) }));
  const heat = hubV4TelHeat(zones);
  ui.heat.node.hidden = heat === null;
  if (heat) {
    setText(ui.heatBig, heat.value);
    setText(ui.heatSub, heat.sub);
  }

  const today = entities.get(TEMPO_TODAY);
  ui.tempo.node.hidden = !today;
  if (today) {
    const [css, adjective] = lookup(TEMPO_COLORS, today.state) ?? ["", ""];
    setAttr(ui.tempoDot, "data-c", css);
    setText(ui.tempoToday, hubV4TelTempo(adjective, "aujourd'hui") || statusText(today));
    const tomorrow = entities.get(TEMPO_TOMORROW);
    const next = tomorrow ? lookup(TEMPO_COLORS, tomorrow.state) : null;
    setText(ui.tempoSub, next ? hubV4TelTempo(next[1], "demain") : tomorrow ? tempoTomorrowText(tomorrow) : "");
    ui.tempoSub.hidden = !tomorrow;
  }

  const calendars = typeof agendaCalendars === "function" ? agendaCalendars() : [];
  ui.agenda.node.hidden = calendars.length === 0;
  if (calendars.length > 0) {
    // Aujourd'hui, puis le rendez-vous suivant dans la semaine.
    const end = new Date(now);
    end.setHours(24 * 8, 0, 0, 0);
    const lines = hubV4TelAgenda(agendaEvents(now, end, calendars), now);
    setText(ui.agendaToday, lines.today);
    setText(ui.agendaLater, lines.later);
    ui.agendaLater.hidden = lines.later === "";
  }
}

function hubV4TelRenderMusic() {
  const ui = hubV4Tel.ui;
  const state = musicPlayer();
  ui.music.hidden = state === null;
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
  setAttr(ui.musicOpen, "aria-label", `La musique${title ? ` : ${title}` : ""}`);
  swap(ui.play, svgIcon(playing ? ICONS.pause : ICONS.play));
  setAttr(ui.play, "aria-label", playing ? "Mettre en pause" : "Lancer la lecture");
  ui.play.disabled = !allowed;
  ui.next.disabled = !allowed || !active;
}

/** Appelé à chaque rendu de l'accueil v4 (hub-v4.js). */
function renderHubV4Tel() {
  if (!hubV4TelSync()) return;
  const now = new Date();
  hubV4TelRenderTop(now);
  hubV4TelRenderHello(now);
  hubV4TelRenderPages();
  hubV4TelRenderLens();
  hubV4TelRenderPanel();
  hubV4TelRenderCards(now);
  hubV4TelRenderMusic();
}

// Le téléphone tourné, la fenêtre élargie, le style changé : la vue suit.
hubV4TelMedia.addEventListener("change", () => {
  hubV4TelSync();
  updateHub();
});
document.querySelector?.("#hub-style")?.addEventListener("change", () => hubV4TelSync());
// Le bouton Maison de la barre (tableau complet, agenda, musique) ramène à
// l'accueil v4, sur le téléphone comme sur l'ordinateur ou la tablette.
document.querySelector?.("#dock-home")?.addEventListener("click", () => {
  if (hubV4On()) hubBack();
});
