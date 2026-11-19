// Accueil v4 : les pages Pièce et « Toute la maison » (maquette « Hub maison »).
//
// En v4, ouvrir une pièce (« Ouvrir la pièce », la ligne d'un chauffage…)
// montre une page plein écran au lieu de la fenêtre de la pièce : le dessin
// zoomé sur elle en fond, les lumières et les volets à gauche, le chauffage de
// sa zone et ce qui s'y est passé à droite. En haut, les autres pièces de
// l'étage, l'autre étage et « Toute la maison » (les pièces d'un coup d'œil,
// le chauffage, l'eau chaude, les prises, la sécurité).
//
// Chargé après hub-v4.js (cadre, lampes, volets). Classes Tailwind, pas
// d'innerHTML, les positions passent par des variables CSS (CSP).
"use strict";

// La page ouverte : `name` est une pièce, ou HUB_V4_ALL pour toute la maison.
const HUB_V4_ALL = "*";
const hubV4Pg = { dialog: null, name: null, back: null, refresh: null, place: null, timer: null };

// Une tuile : le verre sombre de la maquette.
const HUB_V4_TILE = "flex flex-col gap-3 p-4 rounded-[24px] bg-[rgb(16_18_42/0.72)] backdrop-blur-[18px] backdrop-saturate-150 "
  + "shadow-[inset_0_1px_0_rgb(255_255_255/0.08),inset_0_0_0_1px_rgb(255_255_255/0.06),0_18px_40px_-22px_rgb(0_0_0/0.7)]";
const HUB_V4_TILE_HEAD = "flex items-center gap-2.5 min-h-9 [&>svg]:size-5 [&>svg]:shrink-0";
const HUB_V4_TILE_TITLE = "flex-1 min-w-0 text-[15px] font-semibold truncate";
// Une pastille de la barre du haut (pièce, étage).
const HUB_V4_PILL = "flex items-center gap-1 min-h-10 px-3.5 rounded-full border-0 bg-transparent text-[14px] font-medium "
  + `text-inherit whitespace-nowrap cursor-pointer transition-colors hover:bg-white/10 ${HUB_V4_MUTE} `
  + "aria-[current=page]:bg-white/14 aria-[current=page]:text-[#f4f2fb] [&_svg]:size-4 " + HUB_V4_FOCUS;
// La part de l'écran qu'occupe la pièce zoomée (largeur, hauteur).
const HUB_V4_ZOOM = { w: 0.34, h: 0.42, y: 0.5 };
const HUB_V4_LOG_HOURS = 12;
const HUB_V4_LOG_EVERY = 60_000;

// ------------------------------------------------------- Calculs (testés)

/** Les pièces d'un onglet du dessin, de gauche à droite (une fois chacune). */
function hubV4FloorRooms(page) {
  const best = new Map();
  for (const zone of page?.zones ?? []) {
    if (!zone.area) continue;
    const seen = best.get(zone.area);
    if (!seen || zone.w * zone.h > seen.w * seen.h) best.set(zone.area, zone);
  }
  return [...best.values()]
    .sort((a, b) => (a.x - b.x) || (a.y - b.y))
    .map((zone) => zone.area);
}

/** L'onglet qui dessine la pièce, ou null. */
function hubV4PageOf(pages, name) {
  return pages.find((page) => page.zones.some((zone) => zone.area === name)) ?? null;
}

/** « Rez-de-chaussée » → « tout le rez-de-chaussée » ; « Étage » → « tout l'étage ». */
function hubV4AllOf(floor) {
  if (!floor) return "toute la maison";
  const lower = `${floor.charAt(0).toLowerCase()}${floor.slice(1)}`;
  return /^[aeiouyéèêàâîôûh]/i.test(lower) ? `tout l'${lower}` : `tout le ${lower}`;
}

/** Sans accents ni majuscules, pour comparer des noms. */
function hubV4Fold(text) {
  return text.normalize("NFD").replace(/[̀-ͯ]/g, "").toLowerCase();
}

/**
 * Le chauffage d'une pièce : le sien, sinon celui de son étage (rangé dans
 * une pièce du même onglet, ou dont le nom dit l'étage : « Chauffage rez »),
 * sinon le seul de la maison. `zones` : les climate hors eau chaude, chacun
 * avec `area` (sa pièce ou null) ; `floorRooms` : les pièces de l'étage.
 */
function hubV4HeatFor(zones, room, floor, floorRooms) {
  const own = zones.find((z) => z.area === room);
  if (own) return { zone: own, shared: false };
  const sameFloor = zones.find((z) => z.area && floorRooms.includes(z.area));
  if (sameFloor) return { zone: sameFloor, shared: true };
  if (floor) {
    const word = hubV4Fold(floor).split(/[^a-z]+/).find(Boolean) ?? "";
    const named = word ? zones.find((z) => !z.area && hubV4Fold(z.name).split(/[^a-z]+/).includes(word)) : undefined;
    if (named) return { zone: named, shared: true };
  }
  const free = zones.filter((z) => !z.area);
  if (zones.length === 1 && free.length === 1) return { zone: free[0], shared: true };
  return null;
}

/**
 * Le dessin zoomé sur une pièce : taille et position (px) de l'image pour que
 * la zone (en % de l'image) tienne au centre de la scène. `ratio` : hauteur
 * sur largeur de l'image.
 */
function hubV4Zoom(zone, stage, ratio) {
  const zw = Math.max(zone.w, 1) / 100;
  const zh = Math.max(zone.h, 1) / 100;
  const width = Math.min(stage.w * HUB_V4_ZOOM.w / zw, stage.h * HUB_V4_ZOOM.h / (zh * ratio));
  const height = width * ratio;
  const left = stage.w / 2 - (zone.x / 100 + zw / 2) * width;
  const top = stage.h * HUB_V4_ZOOM.y - (zone.y / 100 + zh / 2) * height;
  return {
    width: Math.round(width), height: Math.round(height), left: Math.round(left), top: Math.round(top),
    ring: {
      left: Math.round(left + (zone.x / 100) * width), top: Math.round(top + (zone.y / 100) * height),
      width: Math.round(zw * width), height: Math.round(zh * height),
    },
  };
}

/** La ligne sous le nom : « 3 sur 5 allumées · volet fermé ». */
function hubV4RoomLine(lit, total, coverWord, covers) {
  const parts = [];
  if (total === 1) parts.push(lit === 1 ? "lumière allumée" : "lumière éteinte");
  else if (total > 1) parts.push(hubV4LightsText(lit, total).toLowerCase());
  if (coverWord) parts.push(`${covers > 1 ? "volets" : "volet"} ${coverWord.toLowerCase()}`);
  const line = parts.join(" · ");
  return line ? `${line.charAt(0).toUpperCase()}${line.slice(1)}` : "";
}

/** Le compte d'une pièce dans « Les pièces » : « 2/3 », ou « – » sans lampe. */
function hubV4RoomCount(lit, total) {
  return total === 0 ? "–" : `${lit}/${total}`;
}

/** Le résumé de toute la maison : « 23 sur 29 allumées · 7 volets fermés · tout est calme ». */
function hubV4HouseLine(counts, issues) {
  const parts = [];
  if (counts.lights > 0) {
    parts.push(counts.lit === 0 ? "Toutes les lumières éteintes" : `${counts.lit} sur ${counts.lights} ${counts.lit === 1 ? "allumée" : "allumées"}`);
  }
  const closed = counts.covers - counts.open;
  if (counts.covers > 0) parts.push(closed === 0 ? "volets ouverts" : `${closed} ${closed === 1 ? "volet fermé" : "volets fermés"}`);
  parts.push(issues === 0 ? "tout est calme" : `${issues} ${issues === 1 ? "chose à voir" : "choses à voir"}`);
  const line = parts.join(" · ");
  return `${line.charAt(0).toUpperCase()}${line.slice(1)}`;
}

/** Un capteur muet : « depuis 14:05 », ou « depuis hier » s'il s'est tu avant aujourd'hui. */
function hubV4SilentSince(changed, now) {
  const at = new Date(changed);
  if (Number.isNaN(at.getTime())) return "";
  const sameDay = at.toDateString() === now.toDateString();
  if (sameDay) return `depuis ${shortTime.format(at)}`;
  const yesterday = new Date(now.getFullYear(), now.getMonth(), now.getDate() - 1);
  return at.toDateString() === yesterday.toDateString() ? "depuis hier" : "depuis plusieurs jours";
}

// --------------------------------------------------------- Les morceaux

function hubV4Tile(icon, tone, title, ...rest) {
  const titleNode = el("b", { class: HUB_V4_TILE_TITLE, text: title });
  const head = el("header", { class: HUB_V4_TILE_HEAD }, el("span", {
    class: "text-[#ffbf5e] data-[tone=sky]:text-[#9ad8ff] data-[tone=heat]:text-[#ff9a6b] "
      + "data-[tone=violet]:text-[#b9a3ff] data-[tone=alert]:text-[#ff8b8b] [&_svg]:size-5",
    "data-tone": tone, "aria-hidden": "true",
  }, svgIcon(icon)), titleNode, ...rest);
  return { node: el("section", { class: HUB_V4_TILE }, head), head, titleNode };
}

/** Les climate de la maison (hors eau chaude), chacun avec sa pièce. */
function hubV4HeatZones() {
  return heatingStates(null)
    .filter((s) => s.entity_id !== HUB_V3_WATER)
    .map((s) => ({ id: s.entity_id, name: nameOf(s), area: hubV4AreaOf(s.entity_id) }));
}

/** Le chauffage d'une zone : Confort/Éco, la consigne avec − et +, la mesure. */
function hubV4HeatTile(entityId, title) {
  const tile = hubV4Tile(ICONS.flame, "heat", title);
  const modes = [["confort", "Confort"], ["eco", "Éco"]].map(([preset, text]) => {
    const node = el("button", {
      type: "button", class: `${HUB_V4_BTN} flex-1 aria-pressed:bg-[rgb(255_154_107/0.24)] aria-pressed:text-[#ffc3a4]`,
      "aria-pressed": "false", text,
    });
    node.dataset.preset = preset;
    node.addEventListener("click", () => {
      const state = entities.get(entityId);
      if (!state || !hubV4Ready(state)) return;
      markPending(entityId);
      callService("climate", "set_preset_mode", { entity_id: entityId, preset_mode: preset });
      hubV3Wait(node);
    });
    return node;
  });
  const modeRow = el("div", { class: "flex gap-2", role: "group", "aria-label": "Mode" }, ...modes);
  const nudge = (direction) => {
    const node = el("button", {
      type: "button", class: `grid place-items-center size-14 shrink-0 p-0 rounded-full border-0 bg-white/8 text-[26px] `
        + `font-light text-inherit cursor-pointer transition-colors hover:bg-white/14 disabled:opacity-35 disabled:cursor-default ${HUB_V4_FOCUS}`,
      "aria-label": direction > 0 ? "Monter la consigne" : "Baisser la consigne",
      text: direction > 0 ? "+" : "−",
    });
    node.addEventListener("click", () => nudgeHeating(entityId, direction));
    return node;
  };
  const minus = nudge(-1);
  const plus = nudge(1);
  const target = el("b", { class: "font-titre text-[52px] font-light leading-none tracking-tight" });
  const now = el("span", { class: `text-[13px] ${HUB_V4_MUTE}` });
  const mode = el("span", { class: `text-[13px] ${HUB_V4_MUTE}` });
  tile.head.append(mode);
  tile.node.append(modeRow, el("div", { class: "flex items-center justify-between gap-3" },
    minus, el("div", { class: "flex flex-col items-center gap-1", "aria-live": "polite" }, target, now), plus));
  const refresh = () => {
    const state = entities.get(entityId);
    if (!state) return false;
    const on = state.state === "heat";
    const value = on ? heatTarget(state) : null;
    setText(target, value === null ? (state.state === "off" ? "Arrêt" : "—") : `${number.format(value)}°`);
    const current = state.attributes.current_temperature;
    setText(now, finite(current) ? `il fait ${number.format(current)}°` : "");
    setText(mode, heatMode(state));
    const { step, min, max } = heatLimits(state);
    const ready = hubV4Ready(state);
    minus.disabled = !ready || value === null || value - step < min - 1e-9;
    plus.disabled = !ready || value === null || value + step > max + 1e-9;
    const presets = Array.isArray(state.attributes.preset_modes) ? state.attributes.preset_modes : [];
    modeRow.hidden = !modes.some((node) => presets.includes(node.dataset.preset));
    for (const node of modes) {
      node.hidden = !presets.includes(node.dataset.preset);
      node.disabled = !ready;
      setAttr(node, "aria-pressed", String(state.attributes.preset_mode === node.dataset.preset));
    }
    return true;
  };
  return { node: tile.node, refresh };
}

/** Une lampe de la grille : la toucher l'allume ou l'éteint. */
function hubV4LampButton(entityId, room) {
  const name = el("b", { class: "text-[14px] font-semibold truncate" });
  const word = el("span", { class: `text-[12px] ${HUB_V4_MUTE} group-aria-pressed:text-[#ffd79a]` });
  const node = el("button", {
    type: "button", "aria-pressed": "false",
    class: "group flex items-center gap-2.5 min-h-14 px-3 rounded-[16px] border-0 bg-white/6 text-left text-inherit "
      + "cursor-pointer transition-colors hover:bg-white/10 aria-pressed:bg-[rgb(255_191_94/0.16)] "
      + `disabled:opacity-40 disabled:cursor-default [&.pending]:opacity-60 ${HUB_V4_FOCUS}`,
  },
  el("span", {
    class: `grid place-items-center size-9 shrink-0 rounded-full bg-white/8 ${HUB_V4_MUTE} [&_svg]:size-[18px] `
      + "group-aria-pressed:bg-[#ffbf5e] group-aria-pressed:text-[#2a1a00] group-aria-pressed:shadow-[0_0_18px_rgb(255_191_94/0.6)]",
    "aria-hidden": "true",
  }, svgIcon(ICONS.bulb)),
  el("span", { class: "flex flex-col min-w-0" }, name, word));
  node.addEventListener("click", () => {
    const state = entities.get(entityId);
    if (!state || !hubV4Ready(state)) return;
    markPending(entityId);
    callService("light", state.state === "on" ? "turn_off" : "turn_on", { entity_id: entityId });
  });
  // Un appui long (ou clic droit) : la fenêtre de la lampe, pour l'intensité.
  node.addEventListener("contextmenu", (event) => {
    event.preventDefault();
    hubV4Lamp(entityId);
  });
  const refresh = () => {
    const state = entities.get(entityId);
    if (!state) return;
    setText(name, hubV4ShortName(nameOf(state), room));
    const on = state.state === "on";
    const percent = on ? hubV4Percent(state) : null;
    setText(word, on ? (percent !== null && percent < 100 ? `Allumée · ${percent} %` : "Allumée")
      : state.state === "off" ? "Éteinte" : statusText(state));
    setAttr(node, "aria-pressed", String(on));
    node.disabled = !hubV4Ready(state);
    if (node.dataset.state !== state.state) {
      node.dataset.state = state.state;
      node.classList.remove("pending");
    }
  };
  return { node, refresh };
}

/** Un volet de la liste : son état et Ouvrir / Mi-hauteur / Fermer. */
function hubV4CoverRow(entityId, room) {
  const name = el("b", { class: "flex-1 min-w-0 text-[14px] font-semibold truncate" });
  const word = el("span", { class: `text-[13px] ${HUB_V4_MUTE}` });
  const send = (service, data, node) => {
    const state = entities.get(entityId);
    if (!state || !hubV4Ready(state)) return;
    markPending(entityId);
    callService("cover", service, { entity_id: entityId, ...data });
    hubV3Wait(node);
  };
  const buttons = [["Ouvrir", "open_cover", {}], ["Mi-hauteur", "set_cover_position", { position: 50 }], ["Fermer", "close_cover", {}]]
    .map(([text, service, data]) => {
      const node = el("button", { type: "button", class: `${HUB_V4_BTN} flex-1 px-2 text-[13px]`, text });
      node.dataset.service = service;
      node.addEventListener("click", () => send(service, data, node));
      return node;
    });
  const node = el("div", { class: "flex flex-col gap-2" },
    el("div", { class: "flex items-baseline gap-2" }, name, word),
    el("div", { class: "flex gap-2", role: "group" }, ...buttons));
  const refresh = () => {
    const state = entities.get(entityId);
    if (!state) return;
    setText(name, hubV4ShortName(nameOf(state), room));
    setText(word, hubV4CoverWord([state]));
    const ready = hubV4Ready(state);
    const positioned = state.attributes.current_position != null;
    for (const button of buttons) {
      button.hidden = button.dataset.service === "set_cover_position" && !positioned;
      button.disabled = !ready;
      if (button.dataset.state !== state.state) {
        button.dataset.state = state.state;
        button.classList.remove("pending");
      }
    }
    setAttr(buttons[0].parentElement, "aria-label", nameOf(state));
  };
  return { node, refresh };
}

/** Une liste qui suit des appareils : garde les nœuds, en ajoute, en retire. */
function hubV4Keyed(box, make) {
  const items = new Map();
  return (ids) => {
    for (const [id, item] of items) {
      if (!ids.includes(id)) {
        item.node.remove();
        items.delete(id);
      }
    }
    const nodes = ids.map((id) => {
      if (!items.has(id)) items.set(id, make(id));
      const item = items.get(id);
      item.refresh();
      return item.node;
    });
    if (nodes.some((node, i) => box.children[i] !== node)) box.replaceChildren(...nodes);
  };
}

// ------------------------------------------------------------- La page

/** Le fond d'un onglet du dessin à l'heure qu'il est, ou null. */
function hubV4PageImage(page) {
  if (!page) return null;
  const name = typeof backgroundFor === "function" && typeof ambianceOf === "function"
    ? backgroundFor(page, ambianceOf(page)) : page.background;
  return houseUrl(name) ?? houseUrl(page.background);
}

/** La barre du haut : retour, les pièces de l'étage, l'autre étage, toute la maison. */
function hubV4PageNav(name) {
  const pages = house.pages ?? [];
  const back = el("button", { type: "button", class: `${HUB_V4_PILL} bg-white/8 text-[#f4f2fb] pl-2.5` },
    el("span", { class: "rotate-180 [&_svg]:size-4", "aria-hidden": "true" }, svgIcon(HUB_V4_ICONS.go)), "Maison");
  back.addEventListener("click", () => hubV4PageClose());
  const pill = (text, current, run, arrow = false) => {
    const node = el("button", { type: "button", class: HUB_V4_PILL }, text, arrow ? svgIcon(HUB_V4_ICONS.go) : "");
    if (current) node.setAttribute("aria-current", "page");
    node.addEventListener("click", run);
    return node;
  };
  const links = [];
  if (name === HUB_V4_ALL) {
    for (const page of pages) {
      const first = hubV4FloorRooms(page).find((room) => areas.some((a) => a.name === room));
      if (first) links.push(pill(page.name, false, () => hubV4Page(first)));
    }
  } else {
    const page = hubV4PageOf(pages, name);
    const rooms = page ? hubV4FloorRooms(page) : areas.map((a) => a.name);
    for (const room of rooms.filter((r) => areas.some((a) => a.name === r))) {
      links.push(pill(room, room === name, () => hubV4Page(room)));
    }
    // Les autres étages : leur première pièce.
    for (const other of pages.filter((p) => p !== page)) {
      const first = hubV4FloorRooms(other).find((room) => areas.some((a) => a.name === room));
      if (!first) continue;
      if (links.length > 0 && !links.some((l) => l.dataset.sep)) {
        const sep = el("span", { class: "w-px h-6 mx-1 bg-white/14", "aria-hidden": "true" });
        sep.dataset.sep = "";
        links.push(sep);
      }
      links.push(pill(other.name, false, () => hubV4Page(first), true));
    }
  }
  links.push(pill("Toute la maison", name === HUB_V4_ALL, () => hubV4Page(HUB_V4_ALL)));
  // Depuis « Toute la maison » : le tableau de bord complet, comme avant.
  if (name === HUB_V4_ALL && typeof leaveHub === "function") {
    links.push(pill("Tableau complet", false, () => {
      hubV4PageClose(false);
      leaveHub();
    }, true));
  }
  return el("nav", {
    class: "absolute inset-x-0 top-0 z-20 flex items-center gap-3 px-5 pt-4 pb-2 overflow-x-auto [scrollbar-width:none]",
    "aria-label": "Pièces",
  }, back, el("div", { class: "flex items-center gap-1 p-1 rounded-full bg-[rgb(16_18_42/0.6)] backdrop-blur-[14px]" }, ...links));
}

/** Le fond : le dessin de l'étage, zoomé sur la pièce (ou entier pour toute la maison). */
function hubV4PageStage(name) {
  const pages = house.pages ?? [];
  const page = name === HUB_V4_ALL ? pages[0] : hubV4PageOf(pages, name);
  const url = hubV4PageImage(page);
  const stage = el("div", { class: "absolute inset-0 overflow-hidden pointer-events-none", "aria-hidden": "true" });
  if (!url) return { node: stage, place: () => {} };
  const image = el("img", {
    src: url, alt: "",
    class: "absolute max-w-none left-(--l) top-(--t) w-(--w) h-(--h) opacity-0 transition-opacity duration-500 "
      + "data-ready:opacity-60 motion-reduce:transition-none",
  });
  stage.append(image);
  let ring = null;
  const zone = name === HUB_V4_ALL ? null
    : page.zones.filter((z) => z.area === name).sort((a, b) => b.w * b.h - a.w * a.h)[0];
  if (zone) {
    ring = el("div", {
      class: "absolute left-(--rl) top-(--rt) w-(--rw) h-(--rh) rounded-[14px] border-2 border-[#ffbf5e] "
        + "shadow-[0_0_36px_rgb(255_191_94/0.5),inset_0_0_26px_rgb(255_191_94/0.22)] animate-v4-breathe motion-reduce:animate-none",
    });
    ring.hidden = true;
    stage.append(ring);
  }
  // Les colonnes restent lisibles : le dessin s'assombrit sur les côtés.
  stage.append(el("div", {
    class: "absolute inset-0 bg-[linear-gradient(90deg,#0b0d22_0%,rgb(11_13_34/0.7)_28%,rgb(11_13_34/0.1)_44%,"
      + "rgb(11_13_34/0.1)_58%,rgb(11_13_34/0.7)_74%,#0b0d22_100%)] max-lg:bg-[rgb(11_13_34/0.72)]",
  }));
  const place = () => {
    const box = stage.getBoundingClientRect();
    const ratio = image.naturalWidth > 0 ? image.naturalHeight / image.naturalWidth : 0;
    if (box.width === 0 || ratio === 0) return;
    const at = zone ? hubV4Zoom(zone, { w: box.width, h: box.height }, ratio)
      : hubV4Zoom({ x: 0, y: 0, w: 100, h: 100 }, { w: box.width, h: box.height }, ratio);
    if (!zone) {
      // Toute la maison : le dessin entier, un peu plus grand que le milieu.
      const width = Math.min(box.width * 0.62, box.height * 0.86 / ratio);
      Object.assign(at, { width, height: width * ratio, left: (box.width - width) / 2, top: (box.height - width * ratio) / 2 });
    }
    image.style.setProperty("--l", `${at.left}px`);
    image.style.setProperty("--t", `${at.top}px`);
    image.style.setProperty("--w", `${at.width}px`);
    image.style.setProperty("--h", `${at.height}px`);
    image.dataset.ready = "";
    if (ring) {
      ring.style.setProperty("--rl", `${at.ring.left}px`);
      ring.style.setProperty("--rt", `${at.ring.top}px`);
      ring.style.setProperty("--rw", `${at.ring.width}px`);
      ring.style.setProperty("--rh", `${at.ring.height}px`);
      ring.hidden = false;
    }
  };
  image.addEventListener("load", place);
  return { node: stage, place };
}

/** Le nom d'un appareil du journal, sans celui de la pièce : « Plafond s'est allumé ». */
function hubV4LogName(entry, room) {
  const state = entities.get(entry.entity_id);
  const name = state ? nameOf(state) : entry.name || entry.entity_id;
  return hubV4ShortName(name, room);
}

/** Ce qui s'est passé dans la pièce (journal de bord), si le compte y a droit. */
async function hubV4LoadLog(ids) {
  if (typeof account === "undefined" || !account.permissions?.history) return null;
  const query = new URLSearchParams({ hours: String(HUB_V4_LOG_HOURS) });
  const { status, data } = await api("GET", `/api/logbook?${query}`).catch(() => ({ status: 0 }));
  if (status !== 200 || !Array.isArray(data.entries)) return null;
  return data.entries
    .filter((entry) => ids.has(entry.entity_id))
    .sort((a, b) => Date.parse(b.at) - Date.parse(a.at))
    .slice(0, 4);
}

/** La page d'une pièce : colonnes de réglages par-dessus le dessin zoomé. */
function hubV4RoomBody(name) {
  const area = () => areas.find((a) => a.name === name);
  const pages = house.pages ?? [];
  const page = hubV4PageOf(pages, name);
  const floor = pages.length > 1 && page ? page.name : "";
  const floorRooms = page ? hubV4FloorRooms(page) : [];
  const paints = [];

  // À gauche : l'étage, le nom, l'état ; les lumières ; les volets.
  const line = el("p", { class: `m-0 text-[15px] ${HUB_V4_MUTE}` });
  const left = [el("div", { class: "flex flex-col gap-1.5 px-1 pb-2" },
    floor ? el("p", { class: `m-0 text-[12px] font-semibold uppercase tracking-[0.14em] ${HUB_V4_MUTE}`, text: floor }) : "",
    el("h1", { id: "hub-v4-page-title", class: "m-0 font-titre text-[46px] font-normal leading-[1.05] tracking-tight", text: name }),
    line)];

  const lightsTile = hubV4Tile(ICONS.bulb, "amber", "Lumières");
  const lightsCount = el("span", { class: `text-[13px] ${HUB_V4_MUTE}` });
  const allButton = el("button", { type: "button", class: `${HUB_V4_BTN} min-h-9 px-3 text-[13px]` });
  allButton.addEventListener("click", () => {
    if (hubV3ToggleRoom(name)) hubV3Wait(allButton);
  });
  lightsTile.head.append(lightsCount, allButton);
  const lampGrid = el("div", { class: "grid grid-cols-2 gap-2" });
  lightsTile.node.append(lampGrid);
  const syncLamps = hubV4Keyed(lampGrid, (id) => hubV4LampButton(id, name));
  left.push(lightsTile.node);

  const coversTile = hubV4Tile(ICONS.blinds, "sky", "Volets");
  const coverList = el("div", { class: "flex flex-col gap-3.5" });
  coversTile.node.append(coverList);
  const syncCovers = hubV4Keyed(coverList, (id) => hubV4CoverRow(id, name));
  left.push(coversTile.node);

  paints.push(() => {
    const lights = hubV3RoomLights(area());
    const covers = hubV3Members("volets", area());
    const lit = lights.filter((s) => s.state === "on").length;
    setText(line, hubV4RoomLine(lit, lights.length, hubV4CoverWord(covers), covers.length));
    lightsTile.node.hidden = lights.length === 0;
    setText(lightsCount, `${lit} sur ${lights.length}`);
    setText(allButton, lit > 0 ? "Tout éteindre" : "Tout allumer");
    allButton.disabled = !lights.some(hubV4Ready);
    if (allButton.dataset.lit !== String(lit)) {
      allButton.dataset.lit = String(lit);
      allButton.classList.remove("pending");
    }
    syncLamps(lights.map((s) => s.entity_id));
    coversTile.node.hidden = covers.length === 0;
    setText(coversTile.titleNode, covers.length > 1 ? `Volets (${covers.length})` : "Volet");
    syncCovers(covers.map((s) => s.entity_id));
  });

  // À droite : le chauffage de la zone, ce qui cloche, ce qui s'est passé.
  const right = [];
  const heat = hubV4HeatFor(hubV4HeatZones(), name, floor, floorRooms);
  if (heat) {
    const short = heat.zone.area === name ? name : floor ? floor.split(/[-\s]/)[0] : heat.zone.name;
    const tile = hubV4HeatTile(heat.zone.id, `Chauffage · ${short}`);
    if (heat.shared) {
      tile.node.append(el("p", { class: `m-0 text-[13px] ${HUB_V4_MUTE}`, text: `Ce réglage vaut pour ${hubV4AllOf(floor)}.` }));
    }
    right.push(tile.node);
    paints.push(() => {
      tile.node.hidden = !tile.refresh();
    });
  }

  const alertTile = hubV4Tile(ICONS.bell, "alert", "À voir");
  const alertList = el("ul", { class: "m-0 p-0 list-none flex flex-col gap-2.5" });
  alertTile.node.append(alertList);
  right.push(alertTile.node);
  paints.push(() => {
    const now = new Date();
    const rows = [];
    for (const state of roomStates(area())) {
      const text = alertText(state);
      if (text !== null) rows.push([state.entity_id, `${nameOf(state)} : ${text}`]);
      else if (state.state === "unavailable" && domainOf(state) === "sensor") {
        rows.push([state.entity_id, `${nameOf(state)} ne répond plus ${hubV4SilentSince(state.last_changed, now)}`.trim()]);
      }
    }
    alertTile.node.hidden = rows.length === 0;
    const mark = rows.map((r) => r[1]).join("|");
    if (alertList.dataset.mark === mark) return;
    alertList.dataset.mark = mark;
    alertList.replaceChildren(...rows.map(([id, text]) => {
      const item = el("li", { class: "flex items-center gap-3 text-[14px]" }, el("span", { class: "flex-1 min-w-0", text }));
      if (typeof account !== "undefined" && account.permissions?.history) {
        const see = el("button", { type: "button", class: `${HUB_V4_BTN} min-h-9 px-3 text-[13px]`, text: "Voir" });
        see.addEventListener("click", () => openLogbook(id));
        item.append(see);
      }
      return item;
    }));
  });

  const logTile = hubV4Tile(ICONS.timer, "violet", "Récemment");
  const logList = el("ol", { class: "m-0 p-0 list-none flex flex-col gap-2" });
  logTile.node.append(logList);
  logTile.node.hidden = true;
  right.push(logTile.node);
  const loadLog = async () => {
    const ids = new Set(area()?.entities.map((e) => e.entity_id) ?? []);
    const entries = await hubV4LoadLog(ids);
    if (hubV4Pg.name !== name) return;
    logTile.node.hidden = !entries || entries.length === 0;
    if (!entries) return;
    logList.replaceChildren(...entries.map((entry) => el("li", { class: "flex gap-3 text-[14px]" },
      el("time", { class: `shrink-0 w-12 ${HUB_V4_MUTE}`, datetime: entry.at, text: shortTime.format(new Date(entry.at)) }),
      el("span", { class: "min-w-0" }, el("b", { class: "font-semibold", text: hubV4LogName(entry, name) }), ` ${entry.message}`))));
  };
  loadLog();
  hubV4Pg.timer = setInterval(loadLog, HUB_V4_LOG_EVERY);

  return { left, right, refresh: () => paints.forEach((paint) => paint()) };
}

/** « Toute la maison » : les pièces d'un coup d'œil, chauffage, eau chaude, prises, sécurité. */
function hubV4HouseBody() {
  const paints = [];
  const when = new Intl.DateTimeFormat("fr", { weekday: "long", day: "numeric", month: "long", hour: "2-digit", minute: "2-digit" });
  const date = el("p", { class: `m-0 text-[12px] font-semibold uppercase tracking-[0.14em] ${HUB_V4_MUTE}` });
  const line = el("p", { class: `m-0 text-[15px] ${HUB_V4_MUTE}` });
  const left = [el("div", { class: "flex flex-col gap-1.5 px-1 pb-2" }, date,
    el("h1", { id: "hub-v4-page-title", class: "m-0 font-titre text-[46px] font-normal leading-[1.05] tracking-tight", text: "Toute la maison" }),
    line)];
  paints.push(() => {
    setText(date, when.format(new Date()));
    setText(line, hubV4HouseLine(hubV3Counts(), hubV3Issues().length));
  });

  // Les pièces : « Salon 2/3 », ambrées quand tout est allumé.
  const roomsTile = hubV4Tile(ICONS.home, "amber", "Les pièces");
  const grid = el("div", { class: "grid grid-cols-2 gap-2" });
  roomsTile.node.append(grid);
  left.push(roomsTile.node);
  const syncRooms = hubV4Keyed(grid, (room) => {
    const count = el("span", { class: `text-[13px] ${HUB_V4_MUTE} group-data-full:text-[#ffd79a]` });
    const node = el("button", {
      type: "button",
      class: "group flex items-center justify-between gap-2 min-h-12 px-3.5 rounded-[14px] border-0 bg-white/6 text-left "
        + "text-[14px] font-semibold text-inherit cursor-pointer transition-colors hover:bg-white/10 "
        + `data-full:bg-[rgb(255_191_94/0.16)] ${HUB_V4_FOCUS}`,
    }, el("span", { class: "truncate", text: room }), count);
    node.addEventListener("click", () => hubV4Page(room));
    return {
      node,
      refresh: () => {
        const area = areas.find((a) => a.name === room);
        const lights = area ? hubV3RoomLights(area) : [];
        const lit = lights.filter((s) => s.state === "on").length;
        setText(count, hubV4RoomCount(lit, lights.length));
        node.toggleAttribute("data-full", lights.length > 0 && lit === lights.length);
        setAttr(node, "aria-label", `${room} : ${lights.length === 0 ? "pas de lumière" : hubV4LightsText(lit, lights.length).toLowerCase()}`);
      },
    };
  });
  paints.push(() => syncRooms(areas.filter((a) => !a.hidden).map((a) => a.name)));

  const right = [];
  for (const zone of hubV4HeatZones()) {
    const tile = hubV4HeatTile(zone.id, zone.name);
    right.push(tile.node);
    paints.push(() => {
      tile.node.hidden = !tile.refresh();
    });
  }

  // L'eau chaude : la température du ballon, et ce que fait la pompe à chaleur.
  const waterTile = hubV4Tile(ICONS.humidity, "sky", "Eau chaude");
  const waterBig = el("b", { class: "font-titre text-[34px] font-light leading-none" });
  const waterSub = el("span", { class: `text-[13px] ${HUB_V4_MUTE}` });
  waterTile.node.append(el("div", { class: "flex items-baseline gap-3" }, waterBig, waterSub));
  right.push(waterTile.node);
  paints.push(() => {
    const water = entities.get(HUB_V3_WATER);
    waterTile.node.hidden = !water;
    if (!water) return;
    const current = water.attributes.current_temperature;
    setText(waterBig, finite(current) ? `${number.format(current)}°` : "—");
    const pump = hubV3Pump();
    setText(waterSub, [finite(current) ? "dans le ballon" : heatMode(water), pump ? `Pompe à chaleur : ${pump.state}` : ""]
      .filter(Boolean).join(" · "));
  });

  // Les prises rangées dans les pièces.
  const plugsTile = hubV4Tile(ICONS.plug, "violet", "Prises");
  const plugsCount = el("span", { class: `text-[13px] ${HUB_V4_MUTE}` });
  const plugsAll = el("button", { type: "button", class: `${HUB_V4_BTN} min-h-9 px-3 text-[13px]` });
  plugsTile.head.append(plugsCount, plugsAll);
  const plugGrid = el("div", { class: "grid grid-cols-2 gap-2" });
  plugsTile.node.append(plugGrid);
  right.push(plugsTile.node);
  const plugs = () => {
    const placed = new Set(areas.flatMap((a) => a.entities.map((e) => e.entity_id)));
    return [...entities.values()].filter((s) => domainOf(s) === "switch" && placed.has(s.entity_id))
      .sort((a, b) => nameOf(a).localeCompare(nameOf(b), "fr"));
  };
  plugsAll.addEventListener("click", () => {
    const list = plugs().filter(hubV4Ready);
    const on = list.filter((s) => s.state === "on");
    const targets = on.length > 0 ? on : list;
    for (const state of targets) {
      markPending(state.entity_id);
      callService("switch", on.length > 0 ? "turn_off" : "turn_on", { entity_id: state.entity_id });
    }
    if (targets.length > 0) hubV3Wait(plugsAll);
  });
  const syncPlugs = hubV4Keyed(plugGrid, (id) => {
    const name = el("b", { class: "text-[14px] font-semibold truncate" });
    const word = el("span", { class: `text-[12px] ${HUB_V4_MUTE}` });
    const node = el("button", {
      type: "button", "aria-pressed": "false",
      class: "flex flex-col items-start min-h-14 px-3 py-2 rounded-[16px] border-0 bg-white/6 text-left text-inherit "
        + "cursor-pointer transition-colors hover:bg-white/10 aria-pressed:bg-[rgb(185_163_255/0.18)] "
        + `disabled:opacity-40 disabled:cursor-default [&.pending]:opacity-60 ${HUB_V4_FOCUS}`,
    }, name, word);
    node.addEventListener("click", () => {
      const state = entities.get(id);
      if (!state || !hubV4Ready(state)) return;
      markPending(id);
      callService("switch", state.state === "on" ? "turn_off" : "turn_on", { entity_id: id });
    });
    return {
      node,
      refresh: () => {
        const state = entities.get(id);
        if (!state) return;
        setText(name, nameOf(state));
        setText(word, state.state === "on" ? "Allumée" : state.state === "off" ? "Éteinte" : statusText(state));
        setAttr(node, "aria-pressed", String(state.state === "on"));
        node.disabled = !hubV4Ready(state);
        if (node.dataset.state !== state.state) {
          node.dataset.state = state.state;
          node.classList.remove("pending");
        }
      },
    };
  });
  paints.push(() => {
    const list = plugs();
    plugsTile.node.hidden = list.length === 0;
    const on = list.filter((s) => s.state === "on").length;
    setText(plugsCount, `${on} sur ${list.length}`);
    setText(plugsAll, on > 0 ? "Tout éteindre" : "Tout allumer");
    plugsAll.disabled = !list.some(hubV4Ready);
    if (plugsAll.dataset.on !== String(on)) {
      plugsAll.dataset.on = String(on);
      plugsAll.classList.remove("pending");
    }
    syncPlugs(list.map((s) => s.entity_id));
  });

  // La sécurité : un mot par appareil, ce qui cloche en premier.
  const safeTile = hubV4Tile(ICONS.shield, "sky", "Sécurité");
  const safeList = el("ul", { class: "m-0 p-0 list-none flex flex-col gap-2" });
  safeTile.node.append(safeList);
  right.push(safeTile.node);
  paints.push(() => {
    const states = [...entities.values()].filter(hubV3IsSecurity);
    safeTile.node.hidden = states.length === 0;
    const rows = states
      .map((s) => ({ name: nameOf(s), word: hubV3SecurityWord(s), bad: hubV3SecurityIssue(s) !== null }))
      .sort((a, b) => (b.bad - a.bad) || a.name.localeCompare(b.name, "fr"))
      .slice(0, 6);
    const mark = rows.map((r) => `${r.name}:${r.word}:${r.bad}`).join("|");
    if (safeList.dataset.mark === mark) return;
    safeList.dataset.mark = mark;
    safeList.replaceChildren(...rows.map((row) => el("li", { class: "flex items-baseline justify-between gap-3 text-[14px]" },
      el("span", { class: "truncate", text: row.name }),
      el("span", { class: row.bad ? "font-semibold text-[#ff8b8b]" : HUB_V4_MUTE, text: row.word }))));
  });

  return { left, right, refresh: () => paints.forEach((paint) => paint()) };
}

/**
 * Ouvre la page v4 d'une pièce (ou de toute la maison avec HUB_V4_ALL). Faux
 * si la v4 n'est pas là : la pièce s'ouvre alors comme avant.
 */
function hubV4Page(name) {
  if (!hubV4On() || !hub.on || hub.away) return false;
  if (name !== HUB_V4_ALL && !areas.some((a) => a.name === name)) return false;
  hubV4Close();
  const back = hubV4Pg.dialog ? hubV4Pg.back : document.activeElement;
  hubV4PageClose(false);

  const body = name === HUB_V4_ALL ? hubV4HouseBody() : hubV4RoomBody(name);
  const stage = hubV4PageStage(name);
  const column = (nodes, side) => el("div", {
    class: `flex flex-col gap-3 min-h-0 overflow-y-auto pb-6 [scrollbar-width:thin] ${side === "right" ? "lg:col-start-3" : ""}`,
  }, ...nodes);
  const dialog = el("dialog", {
    class: "hub-v4-page fixed inset-0 m-0 p-0 w-screen h-dvh max-w-none max-h-none border-0 bg-[#0b0d22] overflow-hidden "
      + "text-[#f4f2fb] font-sans [color-scheme:dark] [font-variant-numeric:tabular-nums] backdrop:bg-[#0b0d22] "
      + "animate-v4-fade motion-reduce:animate-none",
    "aria-labelledby": "hub-v4-page-title",
  },
  stage.node,
  hubV4PageNav(name),
  el("div", {
    class: "relative z-10 grid h-full gap-5 px-6 pt-[76px] grid-cols-1 overflow-y-auto "
      + "lg:grid-cols-[minmax(0,390px)_minmax(0,1fr)_minmax(0,350px)] lg:overflow-hidden",
  }, column(body.left, "left"), column(body.right, "right")));
  dialog.addEventListener("close", () => hubV4PageClose(true, dialog));

  hubV4Pg.dialog = dialog;
  hubV4Pg.name = name;
  hubV4Pg.back = back;
  hubV4Pg.refresh = body.refresh;
  hubV4Pg.place = stage.place;
  body.refresh();
  document.body.append(dialog);
  dialog.showModal();
  stage.place();
  addEventListener("resize", hubV4PagePlace);
  return true;
}

function hubV4PagePlace() {
  hubV4Pg.place?.();
}

/** Ferme la page (`focus` : rendre le focus à ce qui l'avait avant). */
function hubV4PageClose(focus = true, only = null) {
  const dialog = hubV4Pg.dialog;
  if (!dialog || (only && only !== dialog)) return;
  hubV4Pg.dialog = null;
  hubV4Pg.name = null;
  hubV4Pg.refresh = null;
  hubV4Pg.place = null;
  clearInterval(hubV4Pg.timer);
  removeEventListener("resize", hubV4PagePlace);
  if (dialog.open) dialog.close();
  dialog.remove();
  if (focus && hubV4Pg.back?.isConnected && !document.querySelector("dialog[open]")) hubV4Pg.back.focus();
}

/** Appelé après chaque rendu de l'accueil (hub-v4.js) : la page suit la maison. */
function renderHubV4Page() {
  if (!hubV4Pg.dialog) return;
  if (!hubV4On() || !hub.on || hub.away) {
    hubV4PageClose();
    return;
  }
  if (hubV4Pg.name !== HUB_V4_ALL && !areas.some((a) => a.name === hubV4Pg.name)) {
    hubV4PageClose();
    return;
  }
  hubV4Pg.refresh?.();
}

// En v4, « Toute la maison » du dock ouvre sa page ; le tableau complet est
// à un toucher de là (« Tableau complet »).
document.querySelector?.("#hub-all")?.addEventListener("click", (event) => {
  if (hubV4Page(HUB_V4_ALL)) event.stopImmediatePropagation();
}, { capture: true });
