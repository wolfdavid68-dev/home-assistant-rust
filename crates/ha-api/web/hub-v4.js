// Accueil v4 (opt-in, dans Mon compte) : toute la maquette « Hub maison ».
//
// La v4 reprend l'accueil v3 (hub-v3.js pose data-hub-style="v3" et
// data-hub-v4 sur <html>) et y ajoute ses écrans. Ici, le cadre commun des
// incrustations : une fenêtre s'ouvre par-dessus l'écran, le reste s'efface.
// On la ferme avec la croix, Échap, ou en touchant à côté. Toutes ont la même
// forme : en-tête avec icône, réglages, un lien pour aller plus loin.
//
// Premier usage : toucher une pièce sur le dessin. La pièce s'entoure de
// lumière, une petite fenêtre à côté règle ses lumières et son volet.
//
// Styles en classes Tailwind (compilées dans style.css par scripts/tailwind.sh),
// animations dans css/app.css. Chargé après hub-v3.js. Pas d'innerHTML.
"use strict";

const HUB_V4_ICONS = {
  close: ["M6 6l12 12", "M18 6L6 18"],
  go: ["M9 18l6-6-6-6"],
};
// La fenêtre se pose à 16 px de la pièce, à 8 px au moins du bord de l'écran.
const HUB_V4_GAP = 16;
const HUB_V4_MARGIN = 8;
const hubV4 = { dialog: null, refresh: null };

// Les classes communes. Les couleurs sont celles de l'accueil v3 (hub-v3.css).
const HUB_V4_FOCUS = "focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[#9ad8ff]";
const HUB_V4_BTN = "flex items-center justify-center gap-1.5 min-h-11 px-4 rounded-[14px] border-0 "
  + "bg-white/8 text-[14px] font-medium text-inherit cursor-pointer transition-colors "
  + "hover:bg-white/14 disabled:opacity-40 disabled:cursor-default [&.pending]:opacity-60 [&_svg]:size-4 "
  + HUB_V4_FOCUS;
const HUB_V4_MUTE = "text-[rgb(226_224_244/0.66)]";

/** Le réglage v4 est-il choisi sur cet appareil ? */
function hubV4On() {
  return document.documentElement.dataset.hubV4 !== undefined;
}

/** Ce que disent les lumières d'une pièce : « 3 sur 5 allumées ». */
function hubV4LightsText(lit, total) {
  if (total === 1) return lit === 1 ? "Allumée" : "Éteinte";
  if (lit === 0) return "Toutes éteintes";
  if (lit === total) return "Toutes allumées";
  return `${lit} sur ${total} ${lit === 1 ? "allumée" : "allumées"}`;
}

/** Le nom d'une lampe sans celui de la pièce : « Salon plafond » → « Plafond ». */
function hubV4ShortName(name, room) {
  const prefix = `${room.toLowerCase()} `;
  if (!name.toLowerCase().startsWith(prefix) || name.length <= prefix.length) return name;
  const rest = name.slice(prefix.length).trim();
  return rest ? `${rest.charAt(0).toUpperCase()}${rest.slice(1)}` : name;
}

/** La ligne sous le nom de la pièce : « Rez-de-chaussée · 21,3° ». */
function hubV4RoomSub(floor, temperature) {
  const parts = [];
  if (floor) parts.push(floor);
  if (typeof temperature === "number" && Number.isFinite(temperature)) parts.push(hubV3Temp(temperature));
  return parts.join(" · ");
}

/** L'état des volets d'une pièce, en un mot. */
function hubV4CoverWord(states) {
  const known = states.filter((s) => s.state !== "unavailable" && s.state !== "unknown");
  if (known.length === 0) return states.length > 0 ? "Indisponible" : "";
  if (known.some((s) => s.state === "opening" || s.state === "closing")) return "En mouvement";
  const many = states.length > 1;
  if (known.every((s) => s.state === "closed")) return many ? "Fermés" : "Fermé";
  if (known.every((s) => s.state === "open")) {
    const position = known.length === 1 ? Number(known[0].attributes.current_position) : NaN;
    if (Number.isFinite(position) && position > 0 && position < 100) return `Ouvert à ${position} %`;
    return many ? "Ouverts" : "Ouvert";
  }
  return "Ouverts en partie";
}

/**
 * Où poser la fenêtre (px dans l'écran) : à côté de la pièce, du côté où il y
 * a le plus de place, ou au centre sans pièce. `side` dit de quel côté de la
 * pièce elle se trouve ("" : pas de flèche), `ay` la hauteur de la flèche.
 */
function hubV4Place(room, pop, screen) {
  const clampY = (y) => Math.max(HUB_V4_MARGIN, Math.min(y, screen.h - pop.h - HUB_V4_MARGIN));
  if (!room) {
    return { x: Math.round(Math.max(HUB_V4_MARGIN, (screen.w - pop.w) / 2)), y: Math.round(clampY((screen.h - pop.h) / 2)), side: "", ay: 0 };
  }
  const cx = room.left + room.width / 2;
  const cy = room.top + room.height / 2;
  const spots = {
    right: room.left + room.width + HUB_V4_GAP,
    left: room.left - HUB_V4_GAP - pop.w,
  };
  const fits = (side) => (side === "right"
    ? spots.right + pop.w <= screen.w - HUB_V4_MARGIN
    : spots.left >= HUB_V4_MARGIN);
  const order = cx <= screen.w / 2 ? ["right", "left"] : ["left", "right"];
  const side = order.find(fits) ?? "";
  const x = side ? spots[side]
    : Math.max(HUB_V4_MARGIN, Math.min(screen.w - pop.w - HUB_V4_MARGIN, cx - pop.w / 2));
  const y = clampY(cy - pop.h / 2);
  const ay = side ? Math.max(28, Math.min(pop.h - 28, cy - y)) : 0;
  return { x: Math.round(x), y: Math.round(y), side, ay: Math.round(ay) };
}

// ---------------------------------------------------------------- Le cadre

/**
 * Ouvre une incrustation. `sub` : l'élément sous le titre. `spot` : le
 * rectangle (écran) à entourer de lumière, ou null. `more` : { label, run }
 * pour aller plus loin. `refresh` remet les réglages à jour : appelé avant
 * l'ouverture, puis à chaque rendu de l'accueil.
 */
function hubV4Overlay({ icon, title, sub, body, more, spot, refresh }) {
  hubV4Close();
  const dialog = el("dialog", {
    class: "hub-v4-ovs fixed inset-0 m-0 p-0 w-screen h-dvh max-w-none max-h-none border-0 bg-transparent "
      + "overflow-hidden text-[#f4f2fb] font-sans [color-scheme:dark] [font-variant-numeric:tabular-nums] "
      + "backdrop:bg-transparent",
    "aria-labelledby": "hub-v4-title",
  });
  const veil = el("div", {
    class: "absolute inset-0 animate-v4-fade motion-reduce:animate-none "
      + "bg-[radial-gradient(ellipse_at_center,rgb(7_10_28/0.35),rgb(7_10_28/0.78))] data-spot:bg-none",
  });
  const parts = [veil];
  if (spot) {
    veil.dataset.spot = "";
    // Le reste s'efface autour de la pièce (une ombre immense), qui respire en ambre.
    const halo = el("div", {
      class: "pointer-events-none absolute left-(--sx) top-(--sy) w-(--sw) h-(--sh) rounded-[18px] "
        + "shadow-[0_0_0_200vmax_rgb(7_10_28/0.72)] animate-v4-fade motion-reduce:animate-none",
    }, el("div", {
      class: "absolute -inset-1 rounded-[22px] border-2 border-[#ffbf5e] "
        + "shadow-[0_0_28px_rgb(255_191_94/0.55),inset_0_0_22px_rgb(255_191_94/0.25)] "
        + "animate-v4-breathe motion-reduce:animate-none",
    }));
    halo.style.setProperty("--sx", `${spot.left}px`);
    halo.style.setProperty("--sy", `${spot.top}px`);
    halo.style.setProperty("--sw", `${spot.width}px`);
    halo.style.setProperty("--sh", `${spot.height}px`);
    parts.push(halo);
  }
  const close = el("button", {
    type: "button", class: `grid place-items-center size-11 shrink-0 p-0 rounded-[14px] border-0 bg-white/8 text-inherit `
      + `cursor-pointer transition-colors hover:bg-white/14 [&_svg]:size-5 ${HUB_V4_FOCUS}`,
    "aria-label": "Fermer",
  }, svgIcon(HUB_V4_ICONS.close));
  close.addEventListener("click", () => hubV4Close());
  const head = el("header", { class: "flex items-center gap-3 min-h-12" },
    el("span", {
      class: "grid place-items-center size-11 shrink-0 rounded-2xl bg-[rgb(255_191_94/0.16)] text-[#ffbf5e] [&_svg]:size-[22px]",
      "aria-hidden": "true",
    }, svgIcon(icon)),
    el("div", { class: "flex flex-col flex-1 min-w-0" },
      el("b", { id: "hub-v4-title", class: "text-[17px] font-semibold leading-tight truncate", text: title }), sub),
    close);
  const pop = el("section", {
    class: "hub-v4-pop absolute left-(--x) top-(--y) w-[320px] max-h-[calc(100dvh-16px)] overflow-auto "
      + "flex flex-col gap-2.5 p-3 rounded-[28px] bg-[rgb(16_18_42/0.8)] backdrop-blur-[20px] backdrop-saturate-150 "
      + "shadow-[inset_0_1px_0_rgb(255_255_255/0.09),inset_0_0_0_1px_rgb(255_255_255/0.06),0_18px_40px_-18px_rgb(0_0_0/0.65)] "
      + "animate-v4-pop motion-reduce:animate-none "
      + "max-sm:left-2 max-sm:right-2 max-sm:top-auto max-sm:bottom-2 max-sm:w-auto",
  }, head, ...body);
  if (more) {
    const go = el("button", { type: "button", class: `${HUB_V4_BTN} justify-between` },
      el("span", { text: more.label }), svgIcon(HUB_V4_ICONS.go));
    go.addEventListener("click", () => {
      hubV4Close();
      more.run();
    });
    pop.append(go);
  }
  // La flèche vers la pièce : un carré tourné, sur le bord qui lui fait face.
  const arrow = el("span", {
    class: "absolute top-(--ay) -mt-[7px] size-3.5 rotate-45 bg-[rgb(16_18_42/0.8)] "
      + "data-[side=right]:-left-[7px] data-[side=left]:-right-[7px] max-sm:hidden",
    "aria-hidden": "true",
  });
  arrow.hidden = true;
  pop.append(arrow);
  parts.push(pop);
  dialog.append(...parts);

  veil.addEventListener("click", () => hubV4Close());
  dialog.addEventListener("click", (event) => {
    if (event.target === dialog) hubV4Close();
  });
  const place = () => {
    const box = pop.getBoundingClientRect();
    const at = hubV4Place(spot, { w: box.width, h: box.height }, { w: innerWidth, h: innerHeight });
    pop.style.setProperty("--x", `${at.x}px`);
    pop.style.setProperty("--y", `${at.y}px`);
    arrow.hidden = at.side === "";
    if (at.side) {
      arrow.dataset.side = at.side;
      arrow.style.setProperty("--ay", `${at.ay}px`);
    }
  };
  // Un écran qui change de taille : la pièce n'est plus au même endroit.
  const resize = () => hubV4Close();
  dialog.addEventListener("close", () => {
    removeEventListener("resize", resize);
    dialog.remove();
    if (hubV4.dialog === dialog) {
      hubV4.dialog = null;
      hubV4.refresh = null;
    }
    if (back?.isConnected && !document.querySelector("dialog[open]")) back.focus();
  });
  // Le focus revient où il était, à la fermeture.
  const back = document.activeElement;
  hubV4.dialog = dialog;
  hubV4.refresh = refresh;
  refresh();
  document.body.append(dialog);
  dialog.showModal();
  // Mesurée une fois remplie et affichée : la fenêtre se pose à côté de la pièce.
  place();
  addEventListener("resize", resize);
}

/** Ferme l'incrustation ouverte, s'il y en a une. */
function hubV4Close() {
  if (hubV4.dialog?.open) hubV4.dialog.close();
}

/** Appelé après chaque rendu de l'accueil (hub-v3.js) : l'incrustation suit la maison. */
function renderHubV4() {
  if (!hubV4.dialog) return;
  if (!hubV4On() || !hub.on || hub.away) {
    hubV4Close();
    return;
  }
  hubV4.refresh?.();
}

// ---------------------------------------------------------------- Une pièce

/** Le rectangle (écran) de la plus grande zone de la pièce sur le dessin, ou null. */
function hubV4RoomSpot(name) {
  let best = null;
  for (const { zone, node } of house.view?.zones ?? []) {
    if (zone.area !== name || !node.isConnected) continue;
    if (!best || zone.w * zone.h > best.zone.w * best.zone.h) best = { zone, node };
  }
  if (!best) return null;
  const box = best.node.getBoundingClientRect();
  if (box.width === 0 || box.height === 0) return null;
  return { left: box.left, top: box.top, width: box.width, height: box.height };
}

/** L'étage de la pièce (le nom de l'onglet qui la dessine), s'il y en a plusieurs. */
function hubV4Floor(name) {
  const pages = house.pages ?? [];
  if (pages.length < 2) return "";
  return pages.find((page) => page.zones.some((zone) => zone.area === name))?.name ?? "";
}

/** Un appareil qu'on peut commander maintenant. */
function hubV4Ready(state) {
  return canControl(state) && state.state !== "unavailable" && state.state !== "unknown";
}

/**
 * Toucher une pièce en v4 : la pièce s'entoure de lumière, une fenêtre règle
 * ses lumières (toutes ou une à une) et son volet. Faux si la v4 n'est pas
 * là, ou s'il n'y a rien à régler (la pièce s'ouvre alors en entier).
 */
function hubV4Room(name) {
  if (!hubV4On() || !hub.on || hub.away) return false;
  const area = () => areas.find((a) => a.name === name);
  if (!area()) return false;
  const lights = () => (area() ? hubV3RoomLights(area()) : []);
  const covers = () => (area() ? hubV3Members("volets", area()) : []);
  if (lights().length === 0 && covers().length === 0) return false;

  const body = [];
  const paints = [];

  if (lights().length > 0) {
    const tgSub = el("span", { class: `text-[13px] ${HUB_V4_MUTE}` });
    const toggle = el("button", {
      type: "button", role: "switch", "aria-label": `Lumières : ${name}`,
      class: "group flex items-center gap-3 w-full min-h-14 px-3 rounded-[18px] border-0 bg-[rgb(255_191_94/0.1)] "
        + "text-left text-inherit cursor-pointer disabled:opacity-40 disabled:cursor-default [&.pending]:opacity-60 "
        + HUB_V4_FOCUS,
    },
    el("span", { class: "text-[#ffbf5e] [&_svg]:size-5", "aria-hidden": "true" }, svgIcon(ICONS.bulb)),
    el("span", { class: "flex flex-col flex-1 min-w-0" },
      el("b", { class: "text-[15px] font-semibold", text: "Lumières" }), tgSub),
    el("span", {
      class: "relative shrink-0 w-[52px] h-8 rounded-full bg-white/16 transition-colors group-aria-checked:bg-[#ffbf5e] "
        + "after:absolute after:top-1 after:left-1 after:size-6 after:rounded-full after:bg-white after:transition-transform "
        + "group-aria-checked:after:translate-x-5 motion-reduce:after:transition-none",
      "aria-hidden": "true",
    }));
    toggle.addEventListener("click", () => {
      if (hubV3ToggleRoom(name)) hubV3Wait(toggle);
    });
    body.push(toggle);

    const lamps = el("div", {
      class: "grid grid-cols-[repeat(auto-fill,minmax(88px,1fr))] gap-2", role: "group", "aria-label": "Lampe par lampe",
    });
    const lampNodes = new Map();
    const makeLamp = (state) => {
      const node = el("button", {
        type: "button",
        class: "flex flex-col items-center justify-center gap-1 min-h-14 px-2 py-1.5 rounded-[14px] border-0 "
          + `bg-white/6 text-[12px] ${HUB_V4_MUTE} cursor-pointer transition-colors `
          + "aria-pressed:bg-[rgb(255_191_94/0.14)] aria-pressed:text-[#ffd9a0] "
          + "disabled:opacity-40 disabled:cursor-default [&.pending]:opacity-60 [&_svg]:size-[18px] " + HUB_V4_FOCUS,
      }, svgIcon(ICONS.bulb), el("span", { class: "max-w-full truncate", text: hubV4ShortName(nameOf(state), name) }));
      node.addEventListener("click", () => {
        const now = entities.get(state.entity_id);
        if (!now || !hubV4Ready(now)) return;
        markPending(now.entity_id);
        callService("light", now.state === "on" ? "turn_off" : "turn_on", { entity_id: now.entity_id });
        hubV3Wait(node);
      });
      return node;
    };
    body.push(lamps);

    paints.push(() => {
      const states = lights();
      const lit = states.filter((s) => s.state === "on").length;
      setText(tgSub, hubV4LightsText(lit, states.length));
      setAttr(toggle, "aria-checked", String(lit > 0));
      toggle.disabled = !states.some(hubV4Ready);
      if (toggle.dataset.lit !== String(lit)) {
        toggle.dataset.lit = String(lit);
        toggle.classList.remove("pending");
      }
      // Lampe par lampe : seulement s'il y en a plusieurs.
      lamps.hidden = states.length < 2;
      const ids = states.map((s) => s.entity_id);
      if (ids.join("|") !== [...lampNodes.keys()].join("|")) {
        lampNodes.clear();
        for (const state of states) lampNodes.set(state.entity_id, makeLamp(state));
        lamps.replaceChildren(...lampNodes.values());
      }
      for (const state of states) {
        const node = lampNodes.get(state.entity_id);
        const on = state.state === "on";
        setAttr(node, "aria-pressed", String(on));
        setAttr(node, "aria-label", `${nameOf(state)} : ${on ? "allumée" : "éteinte"}`);
        node.disabled = !hubV4Ready(state);
        if (node.dataset.state !== state.state) {
          node.dataset.state = state.state;
          node.classList.remove("pending");
        }
      }
    });
  }

  if (covers().length > 0) {
    const label = el("b", { class: "font-semibold" });
    const word = el("span", { class: HUB_V4_MUTE });
    body.push(el("div", { class: "flex items-baseline justify-between gap-2 px-1 pt-1 text-[13px]" }, label, word));
    const move = (service) => {
      const targets = covers().filter(hubV4Ready);
      for (const state of targets) {
        markPending(state.entity_id);
        callService("cover", service, { entity_id: state.entity_id });
      }
      return targets.length > 0;
    };
    const buttons = [["Monter", "up", "open_cover"], ["Arrêter", "stop", "stop_cover"], ["Descendre", "down", "close_cover"]]
      .map(([text, icon, service]) => {
        const node = el("button", { type: "button", class: `${HUB_V4_BTN} px-2 text-[13px]` },
          svgIcon(ICONS[icon]), el("span", { text }));
        node.addEventListener("click", () => {
          if (move(service)) hubV3Wait(node);
        });
        return node;
      });
    body.push(el("div", { class: "grid grid-cols-3 gap-2", role: "group", "aria-label": "Volets" }, ...buttons));
    paints.push(() => {
      const states = covers();
      setText(label, states.length > 1 ? `Volets (${states.length})` : "Volet");
      setText(word, hubV4CoverWord(states));
      const mark = states.map((s) => s.state).join("|");
      const ready = states.some(hubV4Ready);
      for (const node of buttons) {
        node.disabled = !ready;
        if (node.dataset.state !== mark) {
          node.dataset.state = mark;
          node.classList.remove("pending");
        }
      }
    });
  }

  const sub = el("span", { class: `text-[13px] ${HUB_V4_MUTE} truncate` });
  const refresh = () => {
    if (!area()) {
      hubV4Close();
      return;
    }
    const text = hubV4RoomSub(hubV4Floor(name), roomInfo(area()).temperature);
    setText(sub, text);
    sub.hidden = text === "";
    for (const paint of paints) paint();
  };
  hubV4Overlay({
    icon: roomIcon(name),
    title: name,
    sub,
    body,
    more: { label: "Ouvrir la pièce", run: () => openRoom(name) },
    spot: hubV4RoomSpot(name),
    refresh,
  });
  return true;
}
