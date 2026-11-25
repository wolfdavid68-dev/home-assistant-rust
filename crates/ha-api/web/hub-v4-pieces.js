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

// La page ouverte : `name` est une pièce, HUB_V4_ALL pour toute la maison, ou
// HUB_V4_ENERGY pour l'énergie (son contenu est dans hub-v4-energie.js).
const HUB_V4_ALL = "*";
const HUB_V4_ENERGY = "*énergie";
const hubV4Pg = { dialog: null, name: null, back: null, refresh: null, place: null, timer: null };

// Une tuile : le verre sombre de la maquette.
const HUB_V4_TILE = "flex flex-col gap-3 p-4 rounded-[24px] bg-scrim/72 backdrop-blur-[18px] backdrop-saturate-150 "
  + "shadow-[inset_0_1px_0_color-mix(in_srgb,var(--v3-voile)_8%,transparent),inset_0_0_0_1px_color-mix(in_srgb,var(--v3-voile)_6%,transparent),0_18px_40px_-22px_rgb(0_0_0/0.7)]";
const HUB_V4_TILE_HEAD = "flex items-center gap-2.5 min-h-9 [&>svg]:size-5 [&>svg]:shrink-0";
const HUB_V4_TILE_TITLE = "flex-1 min-w-0 text-courant font-semibold truncate";
// Une pastille de la barre du haut (pièce, étage).
const HUB_V4_PILL = "flex items-center gap-1 min-h-9 px-3 rounded-full border-0 bg-transparent text-petit font-medium "
  + `text-inherit whitespace-nowrap cursor-pointer transition-colors hover:bg-voile/10 ${HUB_V4_MUTE} `
  + "aria-[current=page]:bg-voile/14 aria-[current=page]:text-v-ink [&_svg]:size-3.5 " + HUB_V4_FOCUS;
// Un fond de verre pour la barre du haut (retour, pastilles).
const HUB_V4_BAR = "bg-scrim/60 backdrop-blur-[14px] ring-1 ring-inset ring-voile/8";
// La part de l'écran qu'occupe la pièce zoomée (largeur, hauteur), et le
// zoom le plus fort, en fois le dessin entier : la maison reste reconnaissable.
const HUB_V4_ZOOM = { w: 0.34, h: 0.42, y: 0.5, max: 1.6 };
// Le cadran du chauffage : un arc de 270° ouvert en bas (repère SVG 200 × 200).
const HUB_V4_DIAL = { c: 100, r: 80, from: 135, sweep: 270 };
const HUB_V4_PG_ICONS = { warn: ["M12 3l10 18H2z", "M12 10v4", "M12 17.5v.5"] };
let hubV4DialCount = 0;
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
 * la zone (en % de l'image) tienne au milieu de la scène. `ratio` : hauteur
 * sur largeur de l'image ; `options.max` : la largeur à ne pas dépasser ;
 * `options.cx` : le milieu de l'espace libre (sinon celui de la scène).
 */
function hubV4Zoom(zone, stage, ratio, options = {}) {
  const zw = Math.max(zone.w, 1) / 100;
  const zh = Math.max(zone.h, 1) / 100;
  const width = Math.min(options.max ?? Infinity, stage.w * HUB_V4_ZOOM.w / zw, stage.h * HUB_V4_ZOOM.h / (zh * ratio));
  const height = width * ratio;
  const left = (options.cx ?? stage.w / 2) - (zone.x / 100 + zw / 2) * width;
  let top = stage.h * HUB_V4_ZOOM.y - (zone.y / 100 + zh / 2) * height;
  // Plus haut que la scène : ni vide au-dessus ni vide au-dessous.
  if (height >= stage.h) top = Math.min(0, Math.max(top, stage.h - height));
  return {
    width: Math.round(width), height: Math.round(height), left: Math.round(left), top: Math.round(top),
    ring: {
      left: Math.round(left + (zone.x / 100) * width), top: Math.round(top + (zone.y / 100) * height),
      width: Math.round(zw * width), height: Math.round(zh * height),
    },
  };
}

/** Où en est une valeur entre min et max, de 0 à 1 (0 sans valeur). */
function hubV4DialPart(value, min, max) {
  if (!Number.isFinite(value) || !(max > min)) return 0;
  return Math.min(1, Math.max(0, (value - min) / (max - min)));
}

/** L'arc du cadran jusqu'à `part` (0 à 1) : son tracé SVG et son extrémité. */
function hubV4DialArc(part) {
  const { c, r, from, sweep } = HUB_V4_DIAL;
  const k = Math.min(1, Math.max(0, part));
  const at = (u) => {
    const angle = (from + sweep * u) * Math.PI / 180;
    return [Math.round((c + r * Math.cos(angle)) * 100) / 100, Math.round((c + r * Math.sin(angle)) * 100) / 100];
  };
  const [x0, y0] = at(0);
  const [x, y] = at(k);
  const d = k === 0 ? `M${x0} ${y0}` : `M${x0} ${y0}A${r} ${r} 0 ${sweep * k > 180 ? 1 : 0} 1 ${x} ${y}`;
  return { d, x, y };
}

/**
 * Le bouton allumé du volet (Ouvrir, Mi-hauteur, Fermer) d'après son état, ou
 * null entre deux. `positioned` : le volet sait aller à mi-hauteur.
 */
function hubV4CoverPick(state, positioned) {
  const pos = state.attributes?.current_position;
  const known = Number.isFinite(pos);
  if (state.state === "closed" || (known && pos === 0)) return "close_cover";
  if (known && pos > 0 && pos < 100) return positioned && Math.abs(pos - 50) <= 15 ? "set_cover_position" : null;
  return state.state === "open" ? "open_cover" : null;
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

/** Le fil de « Toute la maison » : « Mardi 27 octobre · 21:56 » (`parts` : formatToParts). */
function hubV4DateLine(parts) {
  const get = (type) => parts.find((p) => p.type === type)?.value ?? "";
  const day = [get("weekday"), get("day"), get("month")].filter(Boolean).join(" ");
  const time = get("hour") && get("minute") ? `${get("hour")}:${get("minute")}` : "";
  const text = [day, time].filter(Boolean).join(" · ");
  return `${text.charAt(0).toUpperCase()}${text.slice(1)}`;
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
    class: "text-v-amber data-[tone=sky]:text-v-sky data-[tone=heat]:text-v-heat "
      + "data-[tone=violet]:text-v-violet data-[tone=alert]:text-v-alert [&_svg]:size-5",
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

/** Un élément SVG (le cadran). */
function hubV4Svg(tag, attributes = {}, ...children) {
  const node = document.createElementNS("http://www.w3.org/2000/svg", tag);
  for (const [name, value] of Object.entries(attributes)) node.setAttribute(name, value);
  node.append(...children);
  return node;
}

/** L'en-tête d'une page : le fil (icône maison + étage ou date), le titre en serif, la ligne d'état. */
function hubV4PageHead(crumb, title, line) {
  return el("div", { class: "flex flex-col gap-1 px-1 pb-2" },
    el("p", { class: `flex items-center gap-1.5 m-0 text-legende font-medium ${HUB_V4_MUTE} [&>svg]:size-3.5 [&>svg]:shrink-0` },
      svgIcon(ICONS.home), crumb),
    el("h1", {
      id: "hub-v4-page-title",
      class: "m-0 font-recit text-geant font-normal leading-[1.02] tracking-[-0.01em] text-balance", text: title,
    }),
    line);
}

/**
 * Le chauffage d'une zone : Confort/Éco en haut à droite, le cadran (arc de
 * min à max, la consigne au milieu, la mesure en dessous) entre − et +.
 */
function hubV4HeatTile(entityId, title) {
  const tile = hubV4Tile(ICONS.flame, "heat", title);
  const modes = [["confort", "Confort"], ["eco", "Éco"]].map(([preset, text]) => {
    const node = el("button", {
      type: "button", "aria-pressed": "false", text,
      class: "min-h-8 px-2.5 rounded-[9px] border-0 bg-transparent text-petit font-medium text-inherit cursor-pointer "
        + "transition-colors hover:bg-voile/10 aria-pressed:bg-v-heat/24 aria-pressed:text-v-heat "
        + `disabled:opacity-40 disabled:cursor-default [&.pending]:opacity-60 ${HUB_V4_FOCUS}`,
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
  const modeRow = el("div", { class: "flex gap-0.5 p-0.5 rounded-[11px] bg-voile/8", role: "group", "aria-label": "Mode" }, ...modes);
  const nudge = (direction) => {
    const node = el("button", {
      type: "button",
      class: "grid place-items-center size-11 shrink-0 p-0 rounded-full border-0 text-titre font-light cursor-pointer "
        + "transition-colors disabled:opacity-35 disabled:cursor-default "
        + (direction > 0 ? "bg-v-heat/18 text-v-heat hover:bg-v-heat/28 " : "bg-v-sky/14 text-v-sky hover:bg-v-sky/24 ")
        + HUB_V4_FOCUS,
      "aria-label": direction > 0 ? "Monter la consigne" : "Baisser la consigne",
      text: direction > 0 ? "+" : "−",
    });
    node.addEventListener("click", () => nudgeHeating(entityId, direction));
    return node;
  };
  const minus = nudge(-1);
  const plus = nudge(1);

  // Le cadran : la piste, la part chauffée (dégradé chaleur → ambre), la mesure, la consigne.
  const gradient = `hub-v4-dial-${++hubV4DialCount}`;
  const { c, r } = HUB_V4_DIAL;
  const track = hubV4Svg("path", {
    d: hubV4DialArc(1).d, fill: "none", "stroke-width": "11", "stroke-linecap": "round", class: "stroke-voile/12",
  });
  const arc = hubV4Svg("path", { fill: "none", stroke: `url(#${gradient})`, "stroke-width": "11", "stroke-linecap": "round" });
  const nowDot = hubV4Svg("circle", { r: "3.5", class: "fill-v-ink" });
  const knob = hubV4Svg("circle", { r: "9", "stroke-width": "3", class: "fill-v-ink stroke-scrim" });
  const svg = hubV4Svg("svg", { viewBox: `0 0 ${2 * c} 172`, class: "block w-full h-auto overflow-visible", "aria-hidden": "true" },
    hubV4Svg("defs", {}, hubV4Svg("linearGradient", { id: gradient, gradientUnits: "userSpaceOnUse", x1: "20", y1: "160", x2: "180", y2: "20" },
      hubV4Svg("stop", { offset: "0", class: "[stop-color:var(--v3-heat)]" }),
      hubV4Svg("stop", { offset: "1", class: "[stop-color:var(--v3-amber)]" }))),
    track, arc, nowDot, knob);
  const target = el("b", { class: "font-recit text-geant font-normal leading-none tracking-[-0.02em]" });
  const nowText = el("span", {});
  const now = el("span", { class: `flex items-center gap-1.5 text-legende ${HUB_V4_MUTE}` },
    el("span", { class: "size-1.5 rounded-full bg-v-ink", "aria-hidden": "true" }), nowText);
  const low = el("span", {});
  const high = el("span", {});
  const dial = el("div", { class: "relative flex-1 min-w-0 max-w-[200px]" }, svg,
    el("div", { class: "absolute inset-x-0 top-[31%] flex flex-col items-center gap-1.5", "aria-live": "polite" }, target, now),
    el("div", { class: `flex justify-between px-[13%] -mt-2 text-legende ${HUB_V4_MUTE}`, "aria-hidden": "true" }, low, high));
  // Le mode en mots quand Confort/Éco n'existe pas (ou en dérogation).
  const mode = el("span", { class: `text-petit ${HUB_V4_MUTE}` });
  tile.head.append(mode, modeRow);
  tile.node.append(el("div", { class: "flex items-center justify-between gap-2" }, minus, dial, plus));
  const show = (node, on) => setAttr(node, "visibility", on ? "visible" : "hidden");
  const refresh = () => {
    const state = entities.get(entityId);
    if (!state) return false;
    const attrs = state.attributes;
    const on = state.state === "heat";
    const value = on ? heatTarget(state) : null;
    setText(target, value === null ? (state.state === "off" ? "Arrêt" : "—") : `${number.format(value)}°`);
    const current = attrs.current_temperature;
    setText(nowText, finite(current) ? `il fait ${number.format(current)}°` : "");
    now.hidden = !finite(current);
    const { step, min, max } = heatLimits(state);
    // L'arc va du minimum au maximum de l'appareil (7 à 28° s'il ne les dit pas).
    const lo = finite(attrs.min_temp) ? attrs.min_temp : 7;
    const hi = finite(attrs.max_temp) ? attrs.max_temp : 28;
    setText(low, `${number.format(lo)}°`);
    setText(high, `${number.format(hi)}°`);
    const end = hubV4DialArc(hubV4DialPart(value, lo, hi));
    setAttr(arc, "d", end.d);
    show(arc, value !== null);
    setAttr(knob, "cx", String(end.x));
    setAttr(knob, "cy", String(end.y));
    show(knob, value !== null);
    const mark = hubV4DialArc(hubV4DialPart(current, lo, hi));
    setAttr(nowDot, "cx", String(mark.x));
    setAttr(nowDot, "cy", String(mark.y));
    show(nowDot, finite(current));
    const ready = hubV4Ready(state);
    minus.disabled = !ready || value === null || value - step < min - 1e-9;
    plus.disabled = !ready || value === null || value + step > max + 1e-9;
    const presets = Array.isArray(attrs.preset_modes) ? attrs.preset_modes : [];
    modeRow.hidden = !modes.some((node) => presets.includes(node.dataset.preset));
    for (const node of modes) {
      node.hidden = !presets.includes(node.dataset.preset);
      node.disabled = !ready;
      setAttr(node, "aria-pressed", String(attrs.preset_mode === node.dataset.preset));
    }
    setText(mode, heatMode(state));
    mode.hidden = !modeRow.hidden && attrs.derogation !== true;
    return true;
  };
  return { node: tile.node, refresh };
}

/** Une lampe de la grille : la toucher l'allume ou l'éteint. */
function hubV4LampButton(entityId, room) {
  const name = el("b", { class: "text-corps font-semibold truncate" });
  const word = el("span", { class: `text-legende ${HUB_V4_MUTE} group-aria-pressed:text-v-amber2` });
  const node = el("button", {
    type: "button", "aria-pressed": "false",
    class: "group flex items-center gap-2.5 min-h-14 px-2.5 rounded-[14px] border-0 bg-voile/6 text-left text-inherit "
      + "cursor-pointer transition-colors hover:bg-voile/10 aria-pressed:bg-v-amber/14 "
      + "aria-pressed:ring-1 aria-pressed:ring-inset aria-pressed:ring-v-amber/35 "
      + `disabled:opacity-40 disabled:cursor-default [&.pending]:opacity-60 ${HUB_V4_FOCUS}`,
  },
  el("span", {
    class: `grid place-items-center size-9 shrink-0 rounded-full bg-voile/8 ${HUB_V4_MUTE} [&_svg]:size-[18px] `
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

/**
 * Un volet de la liste : son nom et son état (quand la pièce en a plusieurs),
 * puis Ouvrir / Mi-hauteur / Fermer en contrôle segmenté, l'état présent allumé.
 */
function hubV4CoverRow(entityId, room) {
  const name = el("b", { class: "flex-1 min-w-0 text-corps font-semibold truncate" });
  const word = el("span", { class: `text-petit ${HUB_V4_MUTE}` });
  const send = (service, data, node) => {
    const state = entities.get(entityId);
    if (!state || !hubV4Ready(state)) return;
    markPending(entityId);
    callService("cover", service, { entity_id: entityId, ...data });
    hubV3Wait(node);
  };
  const buttons = [["Ouvrir", "open_cover", {}], ["Mi-hauteur", "set_cover_position", { position: 50 }], ["Fermer", "close_cover", {}]]
    .map(([text, service, data]) => {
      const node = el("button", {
        type: "button", "aria-pressed": "false", text,
        class: "flex-1 min-h-10 px-2 rounded-[11px] border-0 bg-transparent text-petit font-medium text-inherit cursor-pointer "
          + "transition-colors hover:bg-voile/10 aria-pressed:bg-v-sky/16 aria-pressed:ring-1 aria-pressed:ring-inset "
          + "aria-pressed:ring-v-sky/45 aria-pressed:font-semibold disabled:opacity-40 disabled:cursor-default "
          + `[&.pending]:opacity-60 ${HUB_V4_FOCUS}`,
      });
      node.dataset.service = service;
      node.addEventListener("click", () => send(service, data, node));
      return node;
    });
  // Seul dans la pièce, le volet a son état dans l'en-tête de la carte : la ligne du nom s'efface.
  const node = el("div", { class: "flex flex-col gap-2" },
    el("div", { class: "flex items-baseline gap-2 group-data-single/covers:hidden" }, name, word),
    el("div", { class: "flex gap-1 p-1 rounded-[14px] bg-voile/6", role: "group" }, ...buttons));
  const refresh = () => {
    const state = entities.get(entityId);
    if (!state) return;
    setText(name, hubV4ShortName(nameOf(state), room));
    setText(word, hubV4CoverWord([state]));
    const ready = hubV4Ready(state);
    const positioned = hubV4CanPosition(state);
    const pick = hubV4CoverPick(state, positioned);
    for (const button of buttons) {
      button.hidden = button.dataset.service === "set_cover_position" && !positioned;
      button.disabled = !ready;
      setAttr(button, "aria-pressed", String(button.dataset.service === pick));
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

/** Une page sans pièce : toute la maison ou l'énergie. */
function hubV4Whole(name) {
  return name === HUB_V4_ALL || name === HUB_V4_ENERGY;
}

/**
 * La barre du haut : « ‹ Maison » à gauche ; au milieu, les pièces de l'étage
 * (la pièce ouverte surlignée), les autres étages (« Étage › »), puis toute
 * la maison. Sans étages connus : toutes les pièces.
 */
function hubV4PageNav(name) {
  const pages = house.pages ?? [];
  const back = el("button", {
    // Le focus d'ouverture va au retour (sinon à la barre qui défile, sur un téléphone).
    // Sur un écran étroit, il reste collé à gauche quand la barre défile.
    type: "button", autofocus: "",
    class: `${HUB_V4_PILL} ${HUB_V4_BAR} rounded-[14px] min-h-10 pl-2.5 pr-3.5 text-v-ink shrink-0 max-lg:sticky max-lg:left-0 max-lg:z-10 max-lg:bg-scrim lg:justify-self-start`,
  }, el("span", { class: "rotate-180 [&_svg]:size-3.5", "aria-hidden": "true" }, svgIcon(HUB_V4_ICONS.go)), "Maison");
  back.addEventListener("click", () => hubV4PageClose());
  const pill = (text, current, run, arrow = false) => {
    const node = el("button", { type: "button", class: HUB_V4_PILL }, text, arrow ? svgIcon(HUB_V4_ICONS.go) : "");
    if (current) node.setAttribute("aria-current", "page");
    node.addEventListener("click", run);
    return node;
  };
  const firstOf = (page) => hubV4FloorRooms(page).find((room) => areas.some((a) => a.name === room));
  const links = [];
  if (name === HUB_V4_ENERGY) {
    links.push(pill("Énergie", true, () => {}));
  } else if (name === HUB_V4_ALL) {
    for (const page of pages) {
      const first = firstOf(page);
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
      const first = firstOf(other);
      if (first) links.push(pill(other.name, false, () => hubV4Page(first), true));
    }
  }
  links.push(pill("Toute la maison", name === HUB_V4_ALL, () => hubV4Page(HUB_V4_ALL)));
  // Depuis l'énergie : le bilan détaillé (graphique jour par jour), comme avant.
  if (name === HUB_V4_ENERGY && typeof openEnergy === "function") {
    links.push(pill("Bilan détaillé", false, () => openEnergy(), true));
  }
  // Depuis « Toute la maison » : le tableau de bord complet, comme avant.
  if (name === HUB_V4_ALL && typeof leaveHub === "function") {
    links.push(pill("Tableau complet", false, () => {
      hubV4PageClose(false);
      leaveHub();
    }, true));
  }
  return el("nav", {
    class: "absolute inset-x-0 top-0 z-20 flex items-center gap-3 px-5 pt-4 pb-2 overflow-x-auto [scrollbar-width:none] "
      + "lg:grid lg:grid-cols-[1fr_auto_1fr] max-lg:bg-scrim/88 max-lg:backdrop-blur-[16px]",
    "aria-label": "Pièces",
  }, back, el("div", { class: `flex items-center gap-0.5 p-1 rounded-full ${HUB_V4_BAR}` }, ...links));
}

/**
 * Le fond : le dessin de l'étage (le « Dessin de la maison » du propriétaire),
 * zoomé sur la pièce, le ciel autour ; ou entier pour toute la maison.
 * `place(free)` le pose ; `free` : le milieu (px) de l'espace entre les colonnes.
 */
function hubV4PageStage(name) {
  const pages = house.pages ?? [];
  const page = hubV4Whole(name) ? pages[0] : hubV4PageOf(pages, name);
  const url = hubV4PageImage(page);
  const stage = el("div", { class: "absolute inset-0 overflow-hidden pointer-events-none", "aria-hidden": "true" });
  if (!url) return { node: stage, place: () => {} };
  // Les bords du dessin se fondent dans le ciel de la page.
  const image = el("img", {
    src: url, alt: "",
    class: "absolute max-w-none left-(--l) top-(--t) w-(--w) h-(--h) opacity-0 transition-opacity duration-500 "
      + "[mask-image:radial-gradient(closest-side,black_68%,transparent)] "
      + (hubV4Whole(name) ? "data-ready:opacity-60 " : "data-ready:opacity-100 ") + "motion-reduce:transition-none",
  });
  stage.append(image);
  let ring = null;
  const zone = hubV4Whole(name) ? null
    : page.zones.filter((z) => z.area === name).sort((a, b) => b.w * b.h - a.w * a.h)[0];
  if (zone) {
    ring = el("div", {
      class: "absolute left-(--rl) top-(--rt) w-(--rw) h-(--rh) rounded-[14px] border-2 border-v-amber/80 "
        + "shadow-[0_0_36px_color-mix(in_srgb,var(--v3-amber)_45%,transparent),inset_0_0_26px_color-mix(in_srgb,var(--v3-amber)_20%,transparent)] animate-v4-breathe motion-reduce:animate-none",
    });
    ring.hidden = true;
    stage.append(ring);
  }
  // Les colonnes restent lisibles : un voile léger sur les côtés (plein sur un écran étroit).
  stage.append(el("div", {
    class: "absolute inset-0 bg-linear-to-r from-scrim/55 via-transparent via-50% to-scrim/55 max-lg:bg-scrim/72 max-lg:bg-none",
  }));
  const place = (free = null) => {
    const box = stage.getBoundingClientRect();
    const ratio = image.naturalWidth > 0 ? image.naturalHeight / image.naturalWidth : 0;
    if (box.width === 0 || ratio === 0) return;
    const size = { w: box.width, h: box.height };
    // Le dessin entier, un peu plus grand que le milieu.
    const whole = Math.min(box.width * 0.62, box.height * 0.86 / ratio);
    let at;
    if (zone) {
      at = hubV4Zoom(zone, size, ratio, { max: whole * HUB_V4_ZOOM.max, cx: free ?? box.width / 2 });
    } else {
      at = { width: whole, height: whole * ratio, left: (box.width - whole) / 2, top: (box.height - whole * ratio) / 2 };
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
  image.addEventListener("load", () => hubV4PagePlace());
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

  // À gauche : le fil (l'étage), le nom, l'état ; les lumières ; les volets.
  const line = el("p", { class: `m-0 text-corps ${HUB_V4_MUTE}` });
  const left = [hubV4PageHead(floor || "Maison", name, line)];

  const lightsTile = hubV4Tile(ICONS.bulb, "amber", "Lumières");
  // « Lumières 3 sur 5 » côte à côte, « Tout éteindre » au bout.
  lightsTile.titleNode.className = "min-w-0 text-courant font-semibold truncate";
  const lightsCount = el("span", { class: `flex-1 text-petit ${HUB_V4_MUTE}` });
  const allButton = el("button", { type: "button", class: `${HUB_V4_BTN} min-h-9 px-3 rounded-[11px] text-petit` });
  allButton.addEventListener("click", () => {
    if (hubV3ToggleRoom(name)) hubV3Wait(allButton);
  });
  lightsTile.head.append(lightsCount, allButton);
  const lampGrid = el("div", { class: "grid grid-cols-2 gap-2" });
  lightsTile.node.append(lampGrid);
  const syncLamps = hubV4Keyed(lampGrid, (id) => hubV4LampButton(id, name));
  left.push(lightsTile.node);

  const coversTile = hubV4Tile(ICONS.blinds, "sky", "Volets");
  const coversWord = el("span", { class: `text-petit ${HUB_V4_MUTE}` });
  coversTile.head.append(coversWord);
  const coverList = el("div", { class: "group/covers flex flex-col gap-3.5" });
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
    // Un seul volet : son état à droite de l'en-tête, comme la maquette.
    coverList.toggleAttribute("data-single", covers.length === 1);
    setText(coversWord, covers.length === 1 ? hubV4CoverWord(covers) : "");
    syncCovers(covers.map((s) => s.entity_id));
  });

  // À droite : le chauffage de la zone, ce qui cloche, ce qui s'est passé.
  const right = [];
  const heat = hubV4HeatFor(hubV4HeatZones(), name, floor, floorRooms);
  if (heat) {
    const short = heat.zone.area === name ? name : floor ? floor.split(/[-\s]/)[0] : heat.zone.name;
    const tile = hubV4HeatTile(heat.zone.id, /^chauffage/i.test(short) ? short : `Chauffage · ${short}`);
    if (heat.shared) {
      tile.node.append(el("p", { class: `flex items-start gap-1.5 m-0 text-legende ${HUB_V4_MUTE} [&>svg]:size-3.5 [&>svg]:shrink-0 [&>svg]:mt-px [&>svg]:text-v-heat` },
        svgIcon(ICONS.flame), `Ce réglage vaut pour ${hubV4AllOf(floor)}.`));
    }
    right.push(tile.node);
    paints.push(() => {
      tile.node.hidden = !tile.refresh();
    });
  }

  // Ce qui cloche : une carte par appareil (un capteur muet, une porte ouverte…).
  const alertBox = el("div", { class: "flex flex-col gap-3" });
  right.push(alertBox);
  const syncAlerts = hubV4Keyed(alertBox, (id) => {
    const tile = hubV4Tile(HUB_V4_PG_ICONS.warn, "alert", "");
    const text = el("p", { class: `m-0 text-petit leading-snug ${HUB_V4_MUTE}` });
    tile.node.append(text);
    let see = null;
    if (typeof account !== "undefined" && account.permissions?.history) {
      see = el("button", {
        type: "button",
        class: "self-end min-h-9 -my-1 px-2 rounded-[10px] border-0 bg-transparent text-petit font-semibold text-v-amber "
          + `cursor-pointer hover:underline ${HUB_V4_FOCUS}`,
      });
      see.addEventListener("click", () => openLogbook(id));
      tile.node.append(see);
    }
    return {
      node: tile.node,
      refresh: () => {
        const state = entities.get(id);
        if (!state) return;
        setText(tile.titleNode, nameOf(state));
        const said = alertText(state);
        const sensor = domainOf(state) === "sensor";
        if (said === null) {
          const since = hubV4SilentSince(state.last_changed, new Date());
          setText(text, `Le capteur ne répond plus${since ? ` ${since}` : ""}. Sa dernière valeur est masquée pour ne pas tromper.`);
        } else {
          setText(text, `${said.charAt(0).toUpperCase()}${said.slice(1)}.`);
        }
        if (see) setText(see, sensor ? "Voir le capteur" : "Voir l'historique");
      },
    };
  });
  paints.push(() => {
    const ids = [];
    for (const state of roomStates(area())) {
      if (alertText(state) !== null || (state.state === "unavailable" && domainOf(state) === "sensor")) ids.push(state.entity_id);
    }
    alertBox.hidden = ids.length === 0;
    syncAlerts(ids.slice(0, 3));
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
    logList.replaceChildren(...entries.map((entry) => el("li", { class: "flex gap-3 text-petit" },
      el("time", { class: "shrink-0 w-11 font-semibold", datetime: entry.at, text: shortTime.format(new Date(entry.at)) }),
      el("span", { class: "min-w-0" }, hubV4LogName(entry, name), ` ${entry.message}`))));
  };
  loadLog();
  hubV4Pg.timer = setInterval(loadLog, HUB_V4_LOG_EVERY);

  return { left, right, refresh: () => paints.forEach((paint) => paint()) };
}

/** « Toute la maison » : les pièces d'un coup d'œil, chauffage, eau chaude, prises, sécurité. */
function hubV4HouseBody() {
  const paints = [];
  const when = new Intl.DateTimeFormat("fr", { weekday: "long", day: "numeric", month: "long", hour: "2-digit", minute: "2-digit" });
  const date = el("span", {});
  const line = el("p", { class: `m-0 text-corps ${HUB_V4_MUTE}` });
  const left = [hubV4PageHead(date, "Toute la maison", line)];
  paints.push(() => {
    setText(date, hubV4DateLine(when.formatToParts(new Date())));
    setText(line, hubV4HouseLine(hubV3Counts(), hubV3Issues().length));
  });

  // Les pièces : « Salon 2/3 », ambrées quand tout est allumé.
  const roomsTile = hubV4Tile(ICONS.home, "amber", "Les pièces");
  const grid = el("div", { class: "grid grid-cols-2 gap-2" });
  roomsTile.node.append(grid);
  left.push(roomsTile.node);
  const syncRooms = hubV4Keyed(grid, (room) => {
    const count = el("span", { class: `text-petit ${HUB_V4_MUTE} group-data-full:text-v-amber2` });
    const node = el("button", {
      type: "button",
      class: "group flex items-center justify-between gap-2 min-h-12 px-3.5 rounded-[14px] border-0 bg-voile/6 text-left "
        + "text-corps font-semibold text-inherit cursor-pointer transition-colors hover:bg-voile/10 "
        + `data-full:bg-v-amber/16 ${HUB_V4_FOCUS}`,
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
  const waterBig = el("b", { class: "font-titre text-affiche font-light leading-none" });
  const waterSub = el("span", { class: `text-petit ${HUB_V4_MUTE}` });
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
  const plugsCount = el("span", { class: `text-petit ${HUB_V4_MUTE}` });
  const plugsAll = el("button", { type: "button", class: `${HUB_V4_BTN} min-h-9 px-3 text-petit` });
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
    const name = el("b", { class: "text-corps font-semibold truncate" });
    const word = el("span", { class: `text-legende ${HUB_V4_MUTE}` });
    const node = el("button", {
      type: "button", "aria-pressed": "false",
      class: "flex flex-col items-start min-h-14 px-3 py-2 rounded-[16px] border-0 bg-voile/6 text-left text-inherit "
        + "cursor-pointer transition-colors hover:bg-voile/10 aria-pressed:bg-v-violet/18 "
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
    safeList.replaceChildren(...rows.map((row) => el("li", { class: "flex items-baseline justify-between gap-3 text-corps" },
      el("span", { class: "truncate", text: row.name }),
      el("span", { class: row.bad ? "font-semibold text-v-alert" : HUB_V4_MUTE, text: row.word }))));
  });

  return { left, right, refresh: () => paints.forEach((paint) => paint()) };
}

/**
 * Ouvre la page v4 d'une pièce (ou de toute la maison avec HUB_V4_ALL). Faux
 * si la v4 n'est pas là : la pièce s'ouvre alors comme avant.
 */
function hubV4Page(name) {
  if (!hubV4On() || !hub.on || hub.away) return false;
  if (name === HUB_V4_ENERGY ? typeof hubV4EnergyBody !== "function"
    : name !== HUB_V4_ALL && !areas.some((a) => a.name === name)) return false;
  hubV4Close();
  const back = hubV4Pg.dialog ? hubV4Pg.back : document.activeElement;
  hubV4PageClose(false);

  const body = name === HUB_V4_ENERGY ? hubV4EnergyBody()
    : name === HUB_V4_ALL ? hubV4HouseBody() : hubV4RoomBody(name);
  const stage = hubV4PageStage(name);
  const room = !hubV4Whole(name);
  const column = (nodes, side) => el("div", {
    // Sur la tablette, chaque colonne défile ; plus étroit, c'est la page entière.
    class: `flex flex-col gap-3 lg:min-h-0 lg:overflow-y-auto lg:pb-6 [scrollbar-width:thin] ${side === "right" ? "lg:col-start-3 lg:row-start-1" : ""}`,
  }, ...nodes);
  const nav = hubV4PageNav(name);
  const left = column(body.left, "left");
  const right = column(body.right, "right");
  // Au milieu, sous le dessin : une phrase (l'énergie), s'il y en a une.
  const middle = body.middle ? [el("div", {
    class: "hidden lg:flex lg:col-start-2 lg:row-start-1 flex-col justify-end items-center pb-24 min-h-0 pointer-events-none",
  }, ...body.middle)] : [];
  const dialog = el("dialog", {
    class: "hub-v4-page fixed inset-0 m-0 p-0 w-screen h-dvh max-w-none max-h-none border-0 bg-scrim bg-(image:--v3-sky-bg) overflow-hidden "
      + "text-v-ink font-sans [color-scheme:dark] in-data-[theme=light]:[color-scheme:light] [font-variant-numeric:tabular-nums] "
      + "backdrop:bg-scrim animate-v4-fade motion-reduce:animate-none",
    "aria-labelledby": "hub-v4-page-title",
  },
  stage.node,
  nav,
  el("div", {
    class: "relative z-10 grid h-full gap-5 px-6 pt-[76px] grid-cols-1 max-lg:content-start overflow-y-auto "
      + "max-lg:px-4 max-lg:pb-[max(24px,env(safe-area-inset-bottom))] lg:overflow-hidden "
      // Une pièce : des colonnes plus étroites, la pièce zoomée au milieu.
      + (room ? "lg:grid-cols-[minmax(0,340px)_minmax(0,1fr)_minmax(0,310px)]"
        : "lg:grid-cols-[minmax(0,390px)_minmax(0,1fr)_minmax(0,350px)]"),
  }, left, ...middle, right));
  dialog.addEventListener("close", () => hubV4PageClose(true, dialog));

  // Le milieu de l'espace libre entre les colonnes (côte à côte), sinon de l'écran.
  const free = () => {
    const a = left.getBoundingClientRect();
    const b = right.getBoundingClientRect();
    return a.width > 0 && b.width > 0 && a.right < b.left ? (a.right + b.left) / 2 : null;
  };
  hubV4Pg.dialog = dialog;
  hubV4Pg.name = name;
  hubV4Pg.back = back;
  hubV4Pg.refresh = body.refresh;
  hubV4Pg.place = () => stage.place(free());
  body.refresh();
  document.body.append(dialog);
  dialog.showModal();
  hubV4Pg.place();
  // Sur un écran étroit, la barre défile : la pièce ouverte se montre.
  nav.querySelector("[aria-current=page]")?.scrollIntoView?.({ block: "nearest", inline: "center" });
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
  if (!hubV4Whole(hubV4Pg.name) && !areas.some((a) => a.name === hubV4Pg.name)) {
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
