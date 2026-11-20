// Accueil v4 sur le téléphone : la vue étroite de la maquette « Hub maison ».
//
// Sur un écran étroit, quand la v4 est choisie (Mon compte → Style de
// l'accueil), l'accueil v4 s'affiche aussi, mais en colonne : la date et
// l'heure, le salut, l'ordre qui attend, le dessin de la maison et ses
// lentilles (avec ses onglets : sur la tablette, #hub-views, caché ici), les
// réglages de la lentille choisie (chauffage, volets, lumières), quatre cartes (météo, chauffage, Tempo, agenda), la musique en
// bas, et cinq onglets. Le dessin n'est pas recopié : c'est le même #hub-house
// que sur la tablette, déplacé ici puis remis à sa place. Toucher une pièce,
// une carte ou la musique ouvre les mêmes incrustations que sur la tablette,
// qui montent du bas de l'écran (hub-v4.js).
//
// Styles en classes Tailwind. Chargé après hub-v4-sonnette.js. Pas d'innerHTML.
"use strict";

// Plus étroit que ça : un téléphone (le seuil « sm » de Tailwind, comme les
// incrustations qui montent du bas).
const hubV4TelMedia = matchMedia("(max-width: 639px)");
const hubV4Tel = { ui: null, placed: false };
const HUB_V4_TEL_GLASS = "bg-scrim/52 backdrop-blur-[18px] backdrop-saturate-150 "
  + "shadow-[inset_0_1px_0_color-mix(in_srgb,var(--v3-voile)_9%,transparent),inset_0_0_0_1px_color-mix(in_srgb,var(--v3-voile)_6%,transparent)]";

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

/** La carte de l'agenda : le rendez-vous en cours ou le prochain, d'ici
 *  demain soir. `events` : [{ summary, startDate, endDate, allDay }]. */
function hubV4TelNext(events, now) {
  const midnight = new Date(now);
  midnight.setHours(24, 0, 0, 0);
  const timed = events
    .filter((e) => !e.allDay && e.endDate > now)
    .sort((a, b) => a.startDate - b.startDate);
  const first = timed[0];
  if (first) {
    const value = first.startDate <= now ? "En cours"
      : first.startDate < midnight ? hubV4Clock(first.startDate) : `Demain ${hubV4Clock(first.startDate)}`;
    return { value, sub: first.summary };
  }
  const day = events.find((e) => e.allDay && e.startDate < midnight);
  return day ? { value: "Aujourd'hui", sub: day.summary } : null;
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

/** Une carte de la grille : petit titre, valeur, ligne dessous. */
function hubV4TelCard(icon, title, open) {
  const value = el("b", { class: "max-w-full text-[20px] font-semibold leading-tight truncate" });
  const sub = el("small", { class: `max-w-full text-[12px] ${HUB_V4_MUTE} truncate` });
  const node = el("button", {
    type: "button",
    class: `flex flex-col items-start justify-between gap-1 min-h-20 min-w-0 p-3.5 rounded-[20px] border-0 text-left `
      + `text-inherit cursor-pointer ${HUB_V4_TEL_GLASS} ${HUB_V4_FOCUS}`,
  }, el("span", { class: `flex items-center gap-1.5 text-[12px] font-medium ${HUB_V4_MUTE} [&_svg]:size-3.5` },
    svgIcon(icon), title), value, sub);
  node.addEventListener("click", open);
  return { node, value, sub };
}

/** Un onglet du bas. */
function hubV4TelTab(icon, label, run) {
  const node = el("button", {
    type: "button",
    class: `flex flex-col items-center gap-1 min-w-0 pt-1.5 border-0 bg-transparent text-[11px] font-medium `
      + `${HUB_V4_MUTE} cursor-pointer aria-[current=page]:text-v-amber2 [&_svg]:size-6 ${HUB_V4_FOCUS}`,
  }, svgIcon(icon), el("span", { text: label }));
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
    when: el("span", { class: "flex-1 min-w-0 truncate" }),
    live: el("span", { class: `flex items-center gap-1.5 shrink-0 ${HUB_V4_MUTE}` }),
    bell: button({
      class: `relative grid place-items-center size-10 shrink-0 p-0 rounded-full border-0 bg-voile/8 text-inherit `
        + `cursor-pointer [&_svg]:size-5 ${HUB_V4_FOCUS}`,
      "aria-label": "Notifications",
    }, svgIcon(ICONS.bell)),
    badge: el("span", {
      class: "absolute -top-1 -right-1 grid place-items-center min-w-5 h-5 px-1 rounded-full bg-[#ffbf5e] text-[11px] font-bold text-[#1a1530]",
    }),
    avatar: button({
      class: `grid place-items-center size-10 shrink-0 p-0 rounded-full border-0 bg-[#b9a3ff] text-[15px] font-bold `
        + `text-[#1a1530] cursor-pointer ${HUB_V4_FOCUS}`,
      "aria-label": "Qui est là",
    }),
    hello: el("h1", { class: "m-0 font-titre text-[32px] font-semibold leading-[1.08] tracking-[-0.03em]" }),
    calm: el("em", { class: "block not-italic text-v-sky" }),
    summary: el("p", { class: `m-0 mt-2 text-[14px] leading-snug ${HUB_V4_MUTE}` }),
    approval: el("div", {
      class: "flex items-center gap-3 mx-4 pl-4 pr-1.5 py-1.5 rounded-full bg-v-amber/14 text-v-amber2 text-[14px] font-medium",
      role: "status",
    }),
    approvalText: el("span", { class: "flex-1 min-w-0 truncate" }),
    approvalGo: button({
      class: `min-h-9 px-4 rounded-full border-0 bg-[#ffbf5e] text-[13px] font-semibold text-[#1a1530] cursor-pointer ${HUB_V4_FOCUS}`,
    }, "Voir"),
    houseSlot: el("div", { class: "relative shrink-0" }),
    // Les onglets du dessin (vue de dehors, étages) et « Toute la maison ».
    pages: el("nav", {
      class: "flex gap-2 px-4 overflow-x-auto [scrollbar-width:none] [&::-webkit-scrollbar]:hidden",
      "aria-label": "Vue",
    }),
    pagesMark: "",
    lens: el("nav", {
      class: "flex gap-2 px-4 overflow-x-auto [scrollbar-width:none] [&::-webkit-scrollbar]:hidden",
      "aria-label": "Ce que montre la maison",
    }),
    cards: el("div", { class: "grid grid-cols-2 gap-2.5 px-4" }),
    music: el("section", {
      class: "fixed left-3 right-3 bottom-[calc(76px+env(safe-area-inset-bottom))] z-[1] flex items-center gap-3 p-2 pr-2.5 "
        + "rounded-[22px] bg-scrim/82 backdrop-blur-[20px] backdrop-saturate-150 "
        + "shadow-[inset_0_1px_0_color-mix(in_srgb,var(--v3-voile)_9%,transparent),inset_0_0_0_1px_color-mix(in_srgb,var(--v3-voile)_6%,transparent),0_18px_40px_-18px_rgb(0_0_0/0.65)]",
      "aria-label": "Musique",
    }),
    musicOpen: button({ class: `flex flex-1 items-center gap-3 min-w-0 p-0 border-0 bg-transparent text-left text-inherit cursor-pointer ${HUB_V4_FOCUS}` }),
    musicImg: el("img", { class: "absolute inset-0 size-full object-cover", alt: "", decoding: "async", draggable: "false" }),
    musicTitle: el("b", { class: "text-[14px] font-semibold truncate" }),
    musicSub: el("small", { class: `text-[12px] ${HUB_V4_MUTE} truncate` }),
    play: button({ class: `grid place-items-center size-11 shrink-0 p-0 rounded-full border-0 bg-voile/10 text-inherit cursor-pointer disabled:opacity-40 [&_svg]:size-5 ${HUB_V4_FOCUS}` }),
    next: button({ class: `grid place-items-center size-11 shrink-0 p-0 rounded-full border-0 bg-transparent text-inherit cursor-pointer disabled:opacity-40 [&_svg]:size-5 ${HUB_V4_FOCUS}`, "aria-label": "Morceau suivant" },
      svgIcon(ICONS.next)),
  };
  ui.musicImg.hidden = true;
  ui.bell.append(ui.badge);
  ui.bell.addEventListener("click", () => hubV4Bell());
  ui.avatar.addEventListener("click", () => hubV4People());
  ui.approval.append(ui.approvalText, ui.approvalGo);
  ui.approvalGo.addEventListener("click", () => hubV4Approval());

  // Les lentilles : les mêmes que sur la tablette (hub-v3.js).
  ui.segs = HUB_V3_LENSES.map((lens) => {
    const seg = button({
      class: `shrink-0 min-h-9 px-3.5 rounded-full border-0 bg-voile/8 text-[13px] font-medium text-inherit cursor-pointer `
        + `transition-colors aria-pressed:bg-[#ffbf5e] aria-pressed:text-[#1a1530] ${HUB_V4_FOCUS}`,
      "data-lens": lens.id,
      "aria-pressed": "false",
    }, lens.label);
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

  ui.weather = hubV4TelCard(ICONS.cloud, "Météo", () => hubV4Weather());
  ui.heat = hubV4TelCard(ICONS.flame, "Chauffage", () => {
    // Toutes les zones de chauffage, réglables : la page « Toute la maison ».
    if (typeof hubV4Page === "function" && hubV4Page(HUB_V4_ALL)) return;
    // Sans elle : la lentille du chauffage, sur un étage qui a ses pièces.
    hubV3.lens = "chauffage";
    hubV4TelRoomsShown();
    hubV3RenderLens();
    hubV4TelRenderLens();
    hubV4Tel.ui.houseSlot.scrollIntoView({ block: "center", behavior: hubV4TelMotion() });
  });
  ui.tempo = hubV4TelCard(ICONS.bolt, "Tempo", () => hubV4Tempo());
  ui.agenda = hubV4TelCard(ICONS.calendar, "Agenda", () => hubV4Agenda());
  ui.tempoDot = el("span", {
    class: "inline-block size-2.5 mr-2 rounded-full align-middle bg-voile/40 data-[c=blue]:bg-[#5b8cff] "
      + "data-[c=white]:bg-white data-[c=white]:ring-1 data-[c=white]:ring-voile/30 data-[c=red]:bg-[#ff6b6b]",
    "aria-hidden": "true",
  });
  ui.cards.append(ui.weather.node, ui.heat.node, ui.tempo.node, ui.agenda.node);

  const art = el("span", {
    class: "relative grid place-items-center size-11 shrink-0 overflow-hidden rounded-xl "
      + "bg-[linear-gradient(135deg,#ff9bd2,#b9a3ff_55%,#9ad8ff)] text-[#1a1530] [&_svg]:size-5",
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
    class: "fixed inset-x-0 bottom-0 z-[1] grid grid-cols-5 pt-1 pb-[max(10px,env(safe-area-inset-bottom))] "
      + "bg-scrim/72 backdrop-blur-[20px] backdrop-saturate-150 border-0 border-t border-solid border-voile/8",
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

  ui.root = el("div", {
    id: "hub-tel",
    class: "hub-v4-tel hidden [[data-hub-tel]_&]:flex flex-col gap-4 min-h-full "
      + "pt-[max(12px,env(safe-area-inset-top))] pb-[calc(152px+env(safe-area-inset-bottom))] "
      + "text-v-ink [font-variant-numeric:tabular-nums]",
  },
  el("header", { class: "flex items-center gap-2.5 px-5 text-[13px] font-medium" }, ui.when, ui.live, ui.bell, ui.avatar),
  el("div", { class: "px-5" }, ui.hello, ui.summary),
  ui.approval, ui.pages, ui.houseSlot, ui.lens, ui.panel, ui.cards, ui.music, ui.tabs);

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
  ui.coverAll = el("button", { type: "button", class: `${HUB_V4_BTN} min-h-9 px-3 text-[13px]` });
  ui.coverAll.addEventListener("click", () => {
    if (hubV3ActionRun("volets")) hubV3Wait(ui.coverAll);
  });
  covers.head.append(ui.coverAll);
  const coverList = el("div", { class: "flex flex-col gap-4" });
  covers.node.append(coverList);
  ui.covers = covers.node;
  ui.syncCovers = hubV4Keyed(coverList, (id) => hubV4CoverRow(id, ""));

  const lights = hubV4Tile(ICONS.bulb, "amber", "Lumières");
  ui.lightsOff = el("button", { type: "button", class: `${HUB_V4_BTN} min-h-9 px-3 text-[13px]`, text: "Tout éteindre" });
  ui.lightsOff.addEventListener("click", () => {
    if (hubV3ActionRun("lumieres")) hubV3Wait(ui.lightsOff);
  });
  lights.head.append(ui.lightsOff);
  const roomGrid = el("div", { class: "grid grid-cols-2 gap-2" });
  lights.node.append(roomGrid);
  ui.lights = lights.node;
  ui.syncRooms = hubV4Keyed(roomGrid, (room) => {
    const count = el("span", { class: `text-[13px] ${HUB_V4_MUTE} group-data-full:text-v-amber2` });
    const node = el("button", {
      type: "button",
      class: "group flex items-center justify-between gap-2 min-h-12 px-3.5 rounded-[14px] border-0 bg-voile/6 text-left "
        + "text-[14px] font-semibold text-inherit cursor-pointer transition-colors hover:bg-voile/10 "
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
  const pill = (text, pressed, run) => {
    const node = el("button", {
      type: "button",
      class: `shrink-0 min-h-9 px-3.5 rounded-full border-0 bg-voile/8 text-[13px] font-medium text-inherit cursor-pointer `
        + `transition-colors aria-pressed:bg-v-ink aria-pressed:text-v-inverse ${HUB_V4_FOCUS}`,
      "aria-pressed": String(pressed),
    }, text);
    node.addEventListener("click", run);
    return node;
  };
  const tabs = pages.length < 2 ? [] : pages.map((page) => pill(page.name, page.id === current, () => {
    if (page.id === house.config?.id) return;
    selectPage(page.id);
    hubV4TelRenderPages();
  }));
  const whole = all ? [pill("Toute la maison", false, () => hubV4Page(HUB_V4_ALL))] : [];
  swap(ui.pages, ...tabs, ...whole);
}

function hubV4TelRenderLens() {
  const ui = hubV4Tel.ui;
  for (const seg of ui.segs) setAttr(seg, "aria-pressed", String(seg.dataset.lens === hubV3.lens));
}

function hubV4TelRenderTop(now) {
  const ui = hubV4Tel.ui;
  const day = fullDate.format(now);
  setText(ui.when, `${day.charAt(0).toUpperCase()}${day.slice(1)} · ${hourMinute.format(now)}`);
  const live = $("#connection")?.classList.contains("live");
  swap(ui.live, el("i", { class: live ? "size-2 rounded-full bg-v-ok" : "size-2 rounded-full bg-v-alert" }),
    live ? "En direct" : "Hors ligne");
  const notices = $("#open-notices");
  ui.bell.hidden = !notices || notices.hidden;
  const count = $("#notice-count");
  const n = count && !count.hidden ? count.textContent.trim() : "";
  setText(ui.badge, n);
  ui.badge.hidden = n === "";
  setAttr(ui.bell, "aria-label", n ? `Notifications : ${n}` : "Notifications");
  const account = $("#open-account");
  setText(ui.avatar, account ? account.textContent.trim().charAt(0).toUpperCase() : "");
}

function hubV4TelRenderHello(now) {
  const ui = hubV4Tel.ui;
  setText(ui.calm, hubV3Attention(alerts().length));
  swap(ui.hello, hubV3Hello(now.getHours()), " ", ui.calm);
  // La phrase du résumé est celle de la tablette (hub-v3.js), déjà à jour.
  const summary = hubV3.ui?.summary.textContent ?? "";
  setText(ui.summary, summary);
  ui.summary.hidden = summary === "";
  const text = hubV4TelApproval(hubV4Approvals().length);
  setText(ui.approvalText, text);
  ui.approval.hidden = text === "";
}

function hubV4TelRenderCards(now) {
  const ui = hubV4Tel.ui;
  const weather = [...entities.values()].find((s) => domainOf(s) === "weather");
  ui.weather.node.hidden = !weather;
  if (weather) {
    const t = weather.attributes.temperature;
    setText(ui.weather.value, finite(t) ? `${number.format(t)}°` : "—");
    setText(ui.weather.sub, lookup(WEATHER_LABELS, weather.state)?.[0] ?? "");
  }

  const zones = heatingStates(null)
    .filter((s) => s.entity_id !== HUB_V3_WATER)
    .map((s) => ({ name: nameOf(s), temperature: Number(s.attributes.current_temperature) }));
  const heat = hubV4TelHeat(zones);
  ui.heat.node.hidden = heat === null;
  if (heat) {
    setText(ui.heat.value, heat.value);
    setText(ui.heat.sub, heat.sub);
  }

  const today = entities.get(TEMPO_TODAY);
  ui.tempo.node.hidden = !today;
  if (today) {
    const [css, adjective] = lookup(TEMPO_COLORS, today.state) ?? ["", ""];
    setAttr(ui.tempoDot, "data-c", css);
    swap(ui.tempo.value, ui.tempoDot, adjective ? `Jour ${adjective}` : statusText(today));
    const tomorrow = entities.get(TEMPO_TOMORROW);
    setText(ui.tempo.sub, tomorrow ? tempoTomorrowText(tomorrow) : "");
  }

  const calendars = typeof agendaCalendars === "function" ? agendaCalendars() : [];
  ui.agenda.node.hidden = calendars.length === 0;
  if (calendars.length > 0) {
    const end = new Date(now);
    end.setHours(48, 0, 0, 0);
    const next = hubV4TelNext(agendaEvents(now, end, calendars), now);
    setText(ui.agenda.value, next ? next.value : "Rien");
    setText(ui.agenda.sub, next ? next.sub : "d'ici demain soir");
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
// Le bouton Maison de la barre du bas (tableau complet, agenda, musique) ramène
// à l'accueil v4.
document.querySelector?.("#dock-home")?.addEventListener("click", () => {
  if (hubV4Phone()) hubBack();
});
