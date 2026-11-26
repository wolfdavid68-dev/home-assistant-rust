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

// En v4, la maison suit le thème (houseNight : Sombre = nuit, Clair = l'heure) : quand le thème change
// (choix dans Apparence, ou le système qui passe en sombre), elle se redessine.
if (typeof MutationObserver === "function") new MutationObserver(() => {
  if (typeof house !== "undefined" && house.view && typeof updateHouse === "function") updateHouse(house.view);
}).observe(document.documentElement, { attributes: true, attributeFilter: ["data-theme", "data-hub-v4"] });

// Les classes communes. Les couleurs sont celles de l'accueil v3 (hub-v3.css).
const HUB_V4_FOCUS = "focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-v-amber";
const HUB_V4_BTN = "flex items-center justify-center gap-1.5 min-h-11 px-4 rounded-champ border-0 "
  + "bg-voile/8 text-corps font-medium text-inherit cursor-pointer transition-colors "
  + "hover:bg-voile/14 disabled:opacity-40 disabled:cursor-default [&.pending]:opacity-60 [&_svg]:size-4 "
  + HUB_V4_FOCUS;
const HUB_V4_MUTE = "text-v-mute";

/** Le réglage v4 est-il choisi sur cet appareil ? */
function hubV4On() {
  return document.documentElement.dataset.hubV4 !== undefined;
}

/**
 * Un volet se met-il à une position (« Mi-hauteur ») ? supported_features le
 * dit (le bit 4 de Home Assistant, ou une liste de noms) ; sans lui, un volet
 * qui donne sa position sait aussi la prendre (ha-mqtt annonce les deux ensemble).
 */
function hubV4CanPosition(state) {
  const features = state?.attributes?.supported_features;
  if (typeof features === "number") return (features & 4) !== 0;
  if (Array.isArray(features)) return features.includes("set_position") || features.includes("set_cover_position");
  return typeof state?.attributes?.current_position === "number";
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
 * l'ouverture, puis à chaque rendu de l'accueil. `tone` : la couleur de
 * l'icône (amber, sky, violet, green). `onClose` : appelé à la fermeture.
 */
function hubV4Overlay({ icon, tone = "amber", title, sub, body, more, spot, refresh, onClose }) {
  hubV4Close();
  const dialog = el("dialog", {
    class: "hub-v4-ovs fixed inset-0 m-0 p-0 w-screen h-dvh max-w-none max-h-none border-0 bg-transparent "
      + "overflow-hidden text-v-ink font-sans [color-scheme:var(--v4-scheme)] [font-variant-numeric:tabular-nums] "
      + "backdrop:bg-transparent",
    "aria-labelledby": "hub-v4-title",
  });
  const veil = el("div", {
    class: "absolute inset-0 animate-v4-fade motion-reduce:animate-none "
      + "bg-(image:--v4-veil) data-spot:bg-none",
  });
  const parts = [veil];
  if (spot) {
    veil.dataset.spot = "";
    // Le reste s'efface autour de la pièce (une ombre immense), qui respire en ambre.
    const halo = el("div", {
      class: "pointer-events-none absolute left-(--sx) top-(--sy) w-(--sw) h-(--sh) rounded-panneau "
        + "shadow-[0_0_0_200vmax_var(--v4-shade)] animate-v4-fade motion-reduce:animate-none",
    }, el("div", {
      class: "absolute -inset-1 rounded-carte border-2 border-v-amber "
        + "shadow-[0_0_28px_color-mix(in_srgb,var(--v3-amber)_55%,transparent),inset_0_0_22px_color-mix(in_srgb,var(--v3-amber)_25%,transparent)] "
        + "animate-v4-breathe motion-reduce:animate-none",
    }));
    halo.style.setProperty("--sx", `${spot.left}px`);
    halo.style.setProperty("--sy", `${spot.top}px`);
    halo.style.setProperty("--sw", `${spot.width}px`);
    halo.style.setProperty("--sh", `${spot.height}px`);
    parts.push(halo);
  }
  const close = el("button", {
    type: "button", class: `grid place-items-center size-11 shrink-0 p-0 rounded-champ border-0 bg-voile/8 text-inherit `
      + `cursor-pointer transition-colors hover:bg-voile/14 [&_svg]:size-5 ${HUB_V4_FOCUS}`,
    "aria-label": "Fermer",
  }, svgIcon(HUB_V4_ICONS.close));
  close.addEventListener("click", () => hubV4Close());
  // Sur le téléphone, l'en-tête sert aussi de poignée : il ne fait pas défiler la feuille.
  const head = el("header", { class: "flex items-center gap-3 min-h-12 max-sm:touch-none" },
    el("span", {
      class: "grid place-items-center size-11 shrink-0 rounded-2xl bg-v-amber/16 text-v-amber [&_svg]:size-[22px] "
        + "data-[tone=sky]:bg-v-sky/16 data-[tone=sky]:text-v-sky "
        + "data-[tone=violet]:bg-v-violet/16 data-[tone=violet]:text-v-violet "
        + "data-[tone=green]:bg-v-ok/16 data-[tone=green]:text-v-ok",
      "data-tone": tone,
      "aria-hidden": "true",
    }, svgIcon(icon)),
    el("div", { class: "flex flex-col flex-1 min-w-0" },
      el("b", { id: "hub-v4-title", class: "font-recit text-titre font-normal leading-[1.05] tracking-[-0.01em] truncate", text: title }), sub),
    close);
  const pop = el("section", {
    class: "hub-v4-pop absolute left-(--x) top-(--y) w-[320px] max-h-[calc(100dvh-16px)] overflow-auto "
      + "flex flex-col gap-2.5 p-3 rounded-[28px] bg-scrim/80 backdrop-blur-[20px] backdrop-saturate-150 "
      + "shadow-[inset_0_1px_0_color-mix(in_srgb,var(--v3-voile)_9%,transparent),inset_0_0_0_1px_color-mix(in_srgb,var(--v3-voile)_6%,transparent),0_18px_40px_-18px_rgb(0_0_0/0.65)] "
      + "animate-v4-pop motion-reduce:animate-none "
      // Sur le téléphone, une feuille qui monte du bas, qu'on tire vers le bas pour la fermer.
      + "max-sm:inset-x-0 max-sm:top-auto max-sm:bottom-0 max-sm:w-auto max-sm:max-h-[88dvh] max-sm:rounded-b-none "
      + "max-sm:pt-2 max-sm:pb-[max(12px,env(safe-area-inset-bottom))] max-sm:animate-v4-sheet translate-y-(--dy)",
  }, el("span", {
    // La barre à tirer : fine, avec une prise de 44 px de haut autour.
    class: "hidden max-sm:block relative shrink-0 mx-auto w-10 h-1.5 rounded-full bg-voile/25 touch-none "
      + "before:absolute before:-inset-x-6 before:-top-2.5 before:-bottom-3",
    "aria-hidden": "true",
  }), head, ...body);
  pop.style.setProperty("--dy", "0px");
  hubV4Drag(pop, [pop.firstElementChild, head]);
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
  // Hors de la fenêtre (qui défile) : dedans, elle ferait une barre de défilement.
  const arrow = el("span", {
    class: "absolute left-(--ax) top-(--ay) size-3.5 rotate-45 bg-scrim/80 max-sm:hidden",
    "aria-hidden": "true",
  });
  arrow.hidden = true;
  parts.push(pop, arrow);
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
      arrow.style.setProperty("--ax", `${(at.side === "right" ? at.x : at.x + box.width) - 7}px`);
      arrow.style.setProperty("--ay", `${at.y + at.ay - 7}px`);
    }
  };
  // Un écran qui change de largeur : la pièce n'est plus au même endroit. Sur le
  // téléphone, la barre d'adresse qui se replie change seulement la hauteur.
  const width = innerWidth;
  const resize = () => {
    if (innerWidth !== width) hubV4Close();
  };
  dialog.addEventListener("close", () => {
    removeEventListener("resize", resize);
    onClose?.();
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

/** La feuille du téléphone suit le doigt vers le bas ; assez loin, elle se ferme. */
function hubV4Drag(pop, grips) {
  let from = null;
  const move = (event) => {
    if (from === null) return;
    pop.style.setProperty("--dy", `${Math.max(0, event.clientY - from)}px`);
  };
  const end = (event) => {
    if (from === null) return;
    const far = event.type === "pointerup" && event.clientY - from > 90;
    from = null;
    if (far) hubV4Close();
    else pop.style.setProperty("--dy", "0px");
  };
  for (const grip of grips) {
    grip.addEventListener("pointerdown", (event) => {
      if (!hubV4TelSheet() || event.target.closest("button")) return;
      from = event.clientY;
      grip.setPointerCapture?.(event.pointerId);
    });
    grip.addEventListener("pointermove", move);
    grip.addEventListener("pointerup", end);
    grip.addEventListener("pointercancel", end);
  }
}

/** Les incrustations montent du bas (écran étroit, comme max-sm). */
function hubV4TelSheet() {
  return matchMedia("(max-width: 639px)").matches;
}

/** Ferme l'incrustation ouverte, s'il y en a une. */
function hubV4Close() {
  if (hubV4.dialog?.open) hubV4.dialog.close();
}

/** L'accueil s'éteint ou quitte la v4 : ses fenêtres (incrustation, page) se ferment. */
function hubV4Shut() {
  hubV4Close();
  if (typeof hubV4PageClose === "function") hubV4PageClose(false);
}

/** Appelé après chaque rendu de l'accueil (hub-v3.js) : l'incrustation suit la maison. */
function renderHubV4() {
  if (typeof renderHubV4Page === "function") renderHubV4Page();
  if (typeof renderHubV4Incr === "function") renderHubV4Incr();
  if (typeof renderHubV4Tel === "function") renderHubV4Tel();
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
    const tgSub = el("span", { class: `text-petit ${HUB_V4_MUTE}` });
    const toggle = el("button", {
      type: "button", role: "switch", "aria-label": `Lumières : ${name}`,
      class: "group flex items-center gap-3 w-full min-h-14 px-3 rounded-panneau border-0 bg-v-amber/10 "
        + "text-left text-inherit cursor-pointer disabled:opacity-40 disabled:cursor-default [&.pending]:opacity-60 "
        + HUB_V4_FOCUS,
    },
    el("span", { class: "flex flex-col flex-1 min-w-0" },
      el("b", { class: "text-courant font-semibold", text: "Lumières" }), tgSub),
    el("span", {
      class: "relative shrink-0 w-[52px] h-8 rounded-full bg-voile/16 transition-colors group-aria-checked:bg-v-amber "
        + "after:absolute after:top-1 after:left-1 after:size-6 after:rounded-full after:bg-white after:transition-transform "
        + "group-aria-checked:after:translate-x-5 motion-reduce:after:transition-none",
      "aria-hidden": "true",
    }));
    toggle.addEventListener("click", () => hubV3ToggleRoom(name, toggle));
    body.push(toggle);

    const lamps = el("div", {
      class: "grid grid-cols-[repeat(auto-fit,minmax(52px,1fr))] gap-1", role: "group", "aria-label": "Lampe par lampe",
    });
    const lampNodes = new Map();
    const makeLamp = (state) => {
      const node = el("button", {
        type: "button",
        class: "flex flex-col items-center justify-center gap-1 min-w-0 min-h-14 px-0.5 py-1.5 rounded-champ border-0 "
          + `bg-voile/6 text-legende ${HUB_V4_MUTE} cursor-pointer transition-colors `
          + "aria-pressed:bg-v-amber/14 aria-pressed:text-v-amber2 "
          + "disabled:opacity-40 disabled:cursor-default [&.pending]:opacity-60 [&_svg]:size-[18px] " + HUB_V4_FOCUS,
      }, svgIcon(ICONS.bulb), el("span", { class: "max-w-full truncate", text: hubV4ShortName(nameOf(state), name) }));
      node.addEventListener("click", () => {
        const now = entities.get(state.entity_id);
        if (!now || !hubV4Ready(now)) return;
        // Une lampe qui varie se règle dans sa propre fenêtre.
        if (domainOf(now) === "light" && hubV4Dimmable(now) && hubV4Lamp(now.entity_id, hubV4RoomSpot(name))) return;
        hubV3Send(node, "light", now.state === "on" ? "turn_off" : "turn_on", { entity_id: now.entity_id });
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
    const label = el("span", {});
    const word = el("span", {});
    const head = el("button", {
      type: "button",
      class: "flex items-baseline justify-between gap-2 w-full px-1 pt-1 rounded-puce border-0 bg-transparent "
        + `text-left text-petit ${HUB_V4_MUTE} cursor-pointer disabled:cursor-default enabled:hover:bg-voile/6 `
        + HUB_V4_FOCUS,
    }, label, word);
    // Un seul volet : sa ligne ouvre sa fenêtre (position, mi-hauteur).
    head.addEventListener("click", () => {
      const states = covers();
      if (states.length === 1) hubV4Cover(states[0].entity_id, hubV4RoomSpot(name));
    });
    body.push(head);
    // Plusieurs volets d'un coup : « Annuler » les remet comme avant.
    const move = (service, node) => {
      const targets = covers().filter(hubV4Ready);
      const done = { open_cover: "montent", close_cover: "descendent" }[service];
      return hubV3Orders(node, targets.map((s) => ["cover", service, { entity_id: s.entity_id }]),
        done ? `${name} : ${targets.length > 1 ? "les volets" : "le volet"} ${done}` : "");
    };
    // Des flèches seules, comme la maquette : le mot reste pour les lecteurs d'écran.
    const buttons = HUB_V4_MOVES.map(([text, icon, service]) => {
      const node = el("button", { type: "button", class: `${HUB_V4_BTN} px-2 [&_svg]:size-[18px]`, "aria-label": text },
        svgIcon(HUB_V4_MOVE_ICONS[icon]));
      node.addEventListener("click", () => move(service, node));
      return node;
    });
    body.push(el("div", { class: "grid grid-cols-3 gap-2", role: "group", "aria-label": "Volets" }, ...buttons));
    paints.push(() => {
      const states = covers();
      setText(label, states.length > 1 ? `Volets (${states.length})` : "Volet");
      setText(word, hubV4CoverWord(states));
      head.disabled = states.length !== 1;
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

  const sub = el("span", { class: `text-petit ${HUB_V4_MUTE} truncate` });
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

// ------------------------------------------------- Les morceaux communs

// Le bouton principal d'une incrustation (Éteindre, Lancer, Valider).
const HUB_V4_MAIN = "flex items-center justify-center gap-1.5 min-h-12 px-4 rounded-champ border-0 "
  + "bg-[#ffbf5e] text-courant font-semibold text-[#1f1305] cursor-pointer transition-colors "
  + "hover:bg-[#ffcb7d] disabled:opacity-40 disabled:cursor-default [&.pending]:opacity-60 [&_svg]:size-4 "
  + HUB_V4_FOCUS;
// Un curseur : la vraie glissière est invisible, posée sur le dessin.
const HUB_V4_RANGE = "absolute inset-0 size-full m-0 opacity-0 cursor-pointer disabled:cursor-default";
const HUB_V4_TRACK = "has-focus-visible:outline-2 has-focus-visible:outline-offset-2 has-focus-visible:outline-v-amber "
  + "has-disabled:opacity-40 [&.pending]:opacity-70";
// L'intitulé d'un groupe de réglages (« Lumière », « Où », « 4 jours ») : discret, comme la maquette.
const HUB_V4_LABEL = `px-1 pt-1 text-petit ${HUB_V4_MUTE}`;
// Monter, arrêter, descendre : des flèches et un rond (la maquette), pour la pièce et le volet.
const HUB_V4_MOVES = [["Monter", "up", "open_cover"], ["Arrêter", "stop", "stop_cover"], ["Descendre", "down", "close_cover"]];
const HUB_V4_MOVE_ICONS = {
  up: ["M12 19V5", "M6 11l6-6 6 6"],
  stop: ["M12 8.5a3.5 3.5 0 1 0 0 7 3.5 3.5 0 1 0 0-7z"],
  down: ["M12 5v14", "M6 13l6 6 6-6"],
};
// Allumer / éteindre, sur le bouton principal de la lampe ; une horloge pour les lignes d'information.
const HUB_V4_MORE_LINES = {
  power: ["M12 3v8", "M6.4 6.6a8 8 0 1 0 11.2 0"],
  clock: ["M12 3a9 9 0 1 0 0 18 9 9 0 1 0 0-18z", "M12 7.5V12l3 2"],
  cross: ["M7 7l10 10", "M17 7L7 17"],
};
// Une ligne d'information, discrète, avec sa petite icône (« Sans réponse… », « arrive vers 11 h »).
function hubV4Note(icon, text = "") {
  const words = el("span", { class: "min-w-0", text });
  const node = el("p", {
    class: `m-0 flex items-center gap-2.5 min-h-10 px-3 py-2 rounded-champ bg-voile/6 text-legende ${HUB_V4_MUTE} `
      + "[&>svg]:size-4 [&>svg]:shrink-0",
  }, svgIcon(icon), words);
  return { node, words };
}
// Pendant qu'on glisse, l'état reçu ne fait pas sauter le curseur.
const HUB_V4_HOLD = 2500;

/** Une rangée de choix (Veilleuse / 50 % / 100 %), le choix courant allumé. */
function hubV4Choices(label, tone, items) {
  const pressed = tone === "sky" ? "aria-pressed:bg-[#9ad8ff] aria-pressed:text-[#0b1a2b]"
    : "aria-pressed:bg-[#ffbf5e] aria-pressed:text-[#1f1305]";
  const nodes = items.map(([text, run]) => {
    const node = el("button", {
      type: "button", "aria-pressed": "false",
      class: "min-h-11 px-2 rounded-[12px] border-0 bg-transparent text-petit font-medium text-inherit cursor-pointer "
        + `transition-colors hover:bg-voile/8 ${pressed} disabled:opacity-40 disabled:cursor-default [&.pending]:opacity-60 ${HUB_V4_FOCUS}`,
      text,
    });
    node.addEventListener("click", run);
    return node;
  });
  const box = el("div", {
    class: "grid grid-flow-col auto-cols-fr gap-1 p-1 rounded-[16px] bg-voile/6", role: "group", "aria-label": label,
  }, ...nodes);
  return { box, nodes };
}

/** La pièce où se trouve un appareil, ou "". */
function hubV4AreaOf(entityId) {
  return areas.find((a) => a.entities.some((e) => e.entity_id === entityId))?.name ?? "";
}

/** Le lien « Toute la pièce », s'il y a une pièce. */
function hubV4RoomLink(room) {
  if (!room) return null;
  return { label: "Toute la pièce", run: () => hubV4Room(room) || openRoom(room) };
}

/** Le rectangle (écran) d'un élément, ou null s'il n'est pas affiché. */
function hubV4Rect(node) {
  const box = node?.getBoundingClientRect();
  if (!box || box.width === 0 || box.height === 0) return null;
  return { left: box.left, top: box.top, width: box.width, height: box.height };
}

/**
 * Toucher un appareil sur le dessin en v4 : une lampe qui varie ou un volet
 * s'ouvre en incrustation. Faux sinon (le toucher le commande directement).
 */
function hubV4Device(entityId, node) {
  const state = entities.get(entityId);
  if (!state || !hubV4Ready(state)) return false;
  if (domainOf(state) === "light" && hubV4Dimmable(state)) return hubV4Lamp(entityId, hubV4Rect(node));
  if (domainOf(state) === "cover") return hubV4Cover(entityId, hubV4Rect(node));
  return false;
}

// ---------------------------------------------------------------- Une lampe

/** La luminosité (1 à 255) en pour cent ; 0 pour une lampe éteinte. */
function hubV4Percent(state) {
  if (state.state !== "on") return 0;
  const brightness = Number(state.attributes.brightness);
  if (state.attributes.brightness == null || !Number.isFinite(brightness)) return 100;
  return Math.max(1, Math.min(100, Math.round((brightness / 255) * 100)));
}

/** Pour cent → luminosité (1 à 255). */
function hubV4Brightness(percent) {
  return Math.max(1, Math.min(255, Math.round(percent * 2.55)));
}

/** Une lampe qui varie (pas seulement allumée ou éteinte). */
function hubV4Dimmable(state) {
  const modes = state.attributes.supported_color_modes;
  if (Array.isArray(modes)) return modes.some((mode) => mode !== "onoff");
  return typeof state.attributes.brightness === "number";
}

/** Les bornes (kelvins) du blanc d'une lampe qui le règle, ou null. */
function hubV4WhiteRange(state) {
  const modes = state.attributes.supported_color_modes;
  if (!Array.isArray(modes) || !modes.includes("color_temp")) return null;
  const min = Number(state.attributes.min_color_temp_kelvin) || 2000;
  const max = Number(state.attributes.max_color_temp_kelvin) || 6500;
  return max > min ? { min, max } : null;
}

const HUB_V4_LEVELS = [["Veilleuse", 5], ["50 %", 50], ["100 %", 100]];

/** Une lampe : son intensité, la couleur de son blanc, l'éteindre. */
function hubV4Lamp(entityId, spot = null) {
  if (!hubV4On() || !hub.on || hub.away) return false;
  const get = () => entities.get(entityId);
  const first = get();
  if (!first || domainOf(first) !== "light") return false;
  const room = hubV4AreaOf(entityId);
  const body = [];
  const paints = [];
  const send = (data, node = null) => {
    const now = get();
    if (!now || !hubV4Ready(now)) return false;
    return hubV3Send(node, "light", "turn_on", { entity_id: entityId, ...data });
  };

  // Dans l'ordre de la maquette : l'intensité, le blanc, puis les niveaux tout prêts.
  let levels = null;
  if (hubV4Dimmable(first)) {
    let hold = 0;
    const value = el("b", { class: "relative text-courant font-semibold" });
    const range = el("input", { type: "range", min: "1", max: "100", step: "1", "aria-label": "Intensité", class: HUB_V4_RANGE });
    const bar = el("label", {
      class: `relative flex items-center justify-between h-14 px-4 text-v-ink rounded-panneau bg-voile/8 overflow-hidden ${HUB_V4_TRACK}`,
    },
    el("span", {
      class: "absolute inset-y-0 left-0 w-(--v) bg-linear-to-r from-v-amber/20 to-v-amber/45 "
        + "transition-[width] motion-reduce:transition-none "
        // La poignée : un trait au bout de la part allumée.
        + "after:absolute after:right-2 after:top-1/2 after:-translate-y-1/2 after:w-1 after:h-6 after:rounded-full after:bg-v-ink/45",
      "aria-hidden": "true",
    }),
    el("span", { class: "relative flex items-center gap-2 text-corps font-medium [&_svg]:size-4" },
      svgIcon(ICONS.illuminance), "Intensité"), value, range);
    const show = (percent) => {
      bar.style.setProperty("--v", `${percent}%`);
      setText(value, `${percent} %`);
    };
    range.addEventListener("input", () => {
      hold = Date.now() + HUB_V4_HOLD;
      show(Number(range.value));
    });
    range.addEventListener("change", () => {
      hold = Date.now() + HUB_V4_HOLD;
      send({ brightness: hubV4Brightness(Number(range.value)) }, bar);
    });
    levels = hubV4Choices("Niveaux", "amber", HUB_V4_LEVELS.map(([text, percent]) => [text, (event) => {
      if (!send({ brightness: hubV4Brightness(percent) }, event.currentTarget)) return;
      hold = Date.now() + HUB_V4_HOLD;
      range.value = String(percent);
      show(percent);
    }]));
    body.push(el("span", { class: HUB_V4_LABEL, text: "Lumière" }), bar);
    paints.push((now) => {
      const percent = hubV4Percent(now);
      const ready = hubV4Ready(now);
      range.disabled = !ready;
      for (const node of levels.nodes) node.disabled = !ready;
      if (Date.now() >= hold) {
        range.value = String(Math.max(1, percent));
        show(percent);
      }
      HUB_V4_LEVELS.forEach(([, level], i) => {
        setAttr(levels.nodes[i], "aria-pressed", String(percent > 0 && Math.abs(percent - level) <= 2));
      });
    });
  }

  const white = hubV4WhiteRange(first);
  if (white) {
    let hold = 0;
    const range = el("input", {
      type: "range", min: String(white.min), max: String(white.max), step: "50", "aria-label": "Couleur du blanc", class: HUB_V4_RANGE,
    });
    const track = el("label", {
      class: "relative block h-11 rounded-full bg-[linear-gradient(90deg,#ffb35c,#ffe7c2_48%,#e6f1ff_70%,#a9d4ff)] "
        + HUB_V4_TRACK,
    },
    el("span", {
      class: "absolute top-2 left-[calc((100%_-_44px)_*_var(--t,0.5)_+_8px)] size-7 rounded-full bg-white "
        + "ring-[3px] ring-scrim/70 shadow-[0_2px_8px_rgb(0_0_0/0.35)]",
      "aria-hidden": "true",
    }), range);
    const show = (kelvin) => {
      const t = Math.max(0, Math.min(1, (kelvin - white.min) / (white.max - white.min)));
      track.style.setProperty("--t", String(t));
    };
    range.addEventListener("input", () => {
      hold = Date.now() + HUB_V4_HOLD;
      show(Number(range.value));
    });
    range.addEventListener("change", () => {
      hold = Date.now() + HUB_V4_HOLD;
      send({ color_temp_kelvin: Number(range.value) }, track);
    });
    body.push(el("span", { class: HUB_V4_LABEL, text: "Couleur du blanc" }), track,
      el("div", { class: `flex justify-between px-1 text-legende ${HUB_V4_MUTE}` },
        el("span", { text: "Chaud" }), el("span", { text: "Froid" })));
    paints.push((now) => {
      range.disabled = !hubV4Ready(now);
      const kelvin = Number(now.attributes.color_temp_kelvin);
      if (Date.now() >= hold && now.attributes.color_temp_kelvin != null && Number.isFinite(kelvin)) {
        range.value = String(kelvin);
        show(kelvin);
      }
    });
  }

  if (levels) body.push(levels.box);

  const mainText = el("span");
  const main = el("button", { type: "button", class: HUB_V4_MAIN }, svgIcon(HUB_V4_MORE_LINES.power), mainText);
  main.addEventListener("click", () => {
    const now = get();
    if (!now || !hubV4Ready(now)) return;
    hubV3Send(main, "light", now.state === "on" ? "turn_off" : "turn_on", { entity_id: entityId });
  });
  body.push(main);
  paints.push((now) => {
    setText(mainText, now.state === "on" ? "Éteindre" : "Allumer");
    main.disabled = !hubV4Ready(now);
    if (main.dataset.state !== now.state) {
      main.dataset.state = now.state;
      main.classList.remove("pending");
    }
  });

  const sub = el("span", { class: `text-petit ${HUB_V4_MUTE} truncate` });
  hubV4Overlay({
    icon: ICONS.bulb,
    title: nameOf(first),
    sub,
    body,
    more: hubV4RoomLink(room),
    spot,
    refresh: () => {
      const now = get();
      if (!now) {
        hubV4Close();
        return;
      }
      const word = !hubV4Ready(now) ? "indisponible" : now.state === "on" ? "allumée" : "éteinte";
      setText(sub, room ? `${room} · ${word}` : word.charAt(0).toUpperCase() + word.slice(1));
      for (const paint of paints) paint(now);
    },
  });
  return true;
}

// ---------------------------------------------------------------- Un volet

/** La part fermée d'un volet, de 0 (ouvert) à 100 (fermé). */
function hubV4ClosedPart(state) {
  const position = Number(state.attributes.current_position);
  if (state.attributes.current_position != null && Number.isFinite(position)) {
    return 100 - Math.max(0, Math.min(100, position));
  }
  if (state.state === "closed") return 100;
  if (state.state === "open") return 0;
  return 50;
}

/** Ce qu'affiche la fenêtre : « 40 % » et « ouvert », ou « Fermé ». */
function hubV4CoverLabel(state) {
  const part = hubV4ClosedPart(state);
  if (part === 100) return { big: "Fermé", small: "" };
  if (part === 0) return { big: "Ouvert", small: "" };
  if (state.attributes.current_position == null) return { big: "", small: "" };
  return { big: `${100 - part} %`, small: "ouvert" };
}

/** Un volet : la fenêtre qui se ferme, monter, arrêter, descendre. */
function hubV4Cover(entityId, spot = null) {
  if (!hubV4On() || !hub.on || hub.away) return false;
  const get = () => entities.get(entityId);
  const first = get();
  if (!first || domainOf(first) !== "cover") return false;
  const room = hubV4AreaOf(entityId);
  const send = (service, data = {}, node = null) => {
    const now = get();
    if (!now || !hubV4Ready(now)) return false;
    return hubV3Send(node, "cover", service, { entity_id: entityId, ...data });
  };

  const big = el("b", { class: "font-chiffres text-titre-l font-extralight leading-none" });
  const small = el("span", { class: "text-legende text-v-ink2" });
  // La fenêtre : le dehors (teinté par le thème, nuit ou jour), les lames du volet par-dessus.
  const pane = el("div", {
    class: "relative w-[118px] h-[168px] shrink-0 rounded-champ overflow-hidden ring-4 ring-inset ring-voile/10 "
      + "bg-linear-to-b from-v-violet/30 via-v-violet/20 to-v-pink/25",
    "aria-hidden": "true",
  },
  el("span", {
    class: "absolute inset-x-0 top-0 h-(--c) bg-v-violet/45 "
      + "bg-[repeating-linear-gradient(180deg,transparent_0_9px,color-mix(in_srgb,var(--v3-scrim)_45%,transparent)_9px_11px)] "
      + "shadow-[0_2px_6px_rgb(0_0_0/0.3)] transition-[height] duration-700 motion-reduce:transition-none",
  }),
  el("span", { class: "absolute inset-x-0 bottom-3 flex flex-col items-center gap-1" }, big, small));
  const moves = HUB_V4_MOVES
    .map(([text, icon, service]) => {
      const node = el("button", {
        type: "button", class: `${HUB_V4_BTN} justify-start h-[52px] data-strong:bg-voile/16`,
      }, svgIcon(HUB_V4_MOVE_ICONS[icon]), el("span", { text }));
      if (service === "stop_cover") node.dataset.strong = "";
      node.addEventListener("click", () => send(service, {}, node));
      return node;
    });
  const positioned = hubV4CanPosition(first);
  const items = [["Ouvert", (event) => send("open_cover", {}, event.currentTarget)]];
  if (positioned) items.push(["Mi-hauteur", (event) => send("set_cover_position", { position: 50 }, event.currentTarget)]);
  items.push(["Fermé", (event) => send("close_cover", {}, event.currentTarget)]);
  const choices = hubV4Choices("Position", "sky", items);

  const sub = el("span", { class: `text-petit ${HUB_V4_MUTE} truncate` });
  hubV4Overlay({
    icon: ICONS.blinds,
    tone: "sky",
    title: nameOf(first),
    sub,
    body: [
      el("div", { class: "flex gap-3" }, pane, el("div", { class: "grid flex-1 content-start gap-2" }, ...moves)),
      choices.box,
    ],
    more: hubV4RoomLink(room),
    spot,
    refresh: () => {
      const now = get();
      if (!now) {
        hubV4Close();
        return;
      }
      const word = hubV4CoverWord([now]);
      setText(sub, room ? `${room} · ${word.toLowerCase()}` : word);
      const part = hubV4ClosedPart(now);
      pane.style.setProperty("--c", `${part}%`);
      const label = hubV4CoverLabel(now);
      setText(big, label.big);
      setText(small, label.small);
      small.hidden = label.small === "";
      const ready = hubV4Ready(now);
      for (const node of [...moves, ...choices.nodes]) node.disabled = !ready;
      const moving = now.state === "opening" || now.state === "closing";
      const wanted = positioned ? [0, 50, 100] : [0, 100];
      wanted.forEach((target, i) => {
        setAttr(choices.nodes[i], "aria-pressed", String(!moving && Math.abs(part - target) <= 5));
      });
      for (const node of moves) {
        if (node.dataset.state !== now.state) {
          node.dataset.state = now.state;
          node.classList.remove("pending");
        }
      }
    },
  });
  return true;
}

// ---------------------------------------------------------------- Un moment

const HUB_V4_GROUPS = [
  ["light", "Lumières", ICONS.bulb], ["switch", "Prises", ICONS.plug], ["cover", "Volets", ICONS.blinds],
  ["climate", "Chauffage", ICONS.flame], ["media_player", "Musique", ICONS.speaker],
];
// Un moment se lance seul au bout de 3 s, sauf si on l'annule.
const HUB_V4_COUNTDOWN = 3;

/**
 * Ce que va faire une scène, par sorte d'appareil : [{ domain, label, now,
 * then, done }]. `targets` : l'attribut de la scène (état visé de chaque
 * appareil), `get(id)` : l'état actuel d'un appareil.
 */
function hubV4MomentRows(targets, get) {
  const rows = [];
  for (const [domain, label] of HUB_V4_GROUPS) {
    const ids = Object.keys(targets ?? {}).filter((id) => id.startsWith(`${domain}.`));
    if (ids.length === 0) continue;
    const wanted = ids.map((id) => ({
      state: targets[id].state,
      attributes: { brightness: targets[id].brightness, current_position: targets[id].position },
    }));
    const current = ids.map(get).filter((s) => s !== undefined);
    let now = "";
    let then;
    if (domain === "climate") {
      // Le chauffage : le préréglage visé (« Éco »), sinon la consigne, sinon coupé.
      const row = hubV4MomentHeat(ids.map((id) => targets[id]), current);
      if (row) rows.push({ domain, label, ...row });
      continue;
    }
    if (domain === "media_player") {
      // La musique : seulement quand la scène l'arrête (l'attribut ne dit rien d'autre de sûr).
      if (!ids.every((id) => targets[id].state === "off")) continue;
      const done = current.length === ids.length && current.every((s) => ["off", "idle", "standby"].includes(s.state));
      rows.push({ domain, label, now: current.map(nameOf).join(", "), then: done ? "déjà arrêtée" : "s'arrête", done });
      continue;
    }
    if (domain === "cover") {
      now = hubV4CoverWord(current).toLowerCase();
      then = hubV4CoverWord(wanted).toLowerCase();
    } else {
      const lit = (list) => list.filter((s) => s.state === "on").length;
      if (current.length > 0) now = hubV4LightsText(lit(current), current.length).toLowerCase();
      then = hubV4LightsText(lit(wanted), wanted.length).toLowerCase();
      const levels = new Set(wanted.filter((s) => s.state === "on").map((s) => s.attributes.brightness));
      const [level] = levels;
      if (levels.size === 1 && typeof level === "number") {
        const percent = hubV4Percent({ state: "on", attributes: { brightness: level } });
        if (percent < 100) then += ` à ${percent} %`;
      }
    }
    const done = ids.every((id) => {
      const state = get(id);
      const target = targets[id];
      if (!state || state.state !== target.state) return false;
      if (typeof target.position === "number" && Number(state.attributes.current_position) !== target.position) return false;
      return typeof target.brightness !== "number" || Number(state.attributes.brightness) === target.brightness;
    });
    rows.push({ domain, label, now, then: done ? `déjà ${then}` : then, done });
  }
  return rows;
}

/**
 * La ligne « Chauffage » d'un moment : { now, then, done }, ou null si la
 * scène ne dit pas ce qu'elle règle. `wanted` : les cibles, `current` : les
 * états actuels des mêmes appareils.
 */
function hubV4MomentHeat(wanted, current) {
  const labels = typeof PRESET_LABELS === "object" ? PRESET_LABELS : {};
  const said = (preset) => lookup(labels, preset) ?? `${preset.charAt(0).toUpperCase()}${preset.slice(1)}`;
  const one = (list) => (new Set(list).size === 1 ? list[0] : undefined);
  const off = (t) => t.state === "off" || t.hvac_mode === "off";
  let then = "";
  let done = false;
  if (wanted.every(off)) {
    then = "coupé";
    done = current.length === wanted.length && current.every((s) => s.state === "off");
  } else if (typeof one(wanted.map((t) => t.preset_mode)) === "string") {
    const preset = wanted[0].preset_mode;
    then = said(preset);
    done = current.length === wanted.length && current.every((s) => s.attributes.preset_mode === preset);
  } else if (typeof one(wanted.map((t) => t.temperature)) === "number") {
    const degrees = wanted[0].temperature;
    then = `${hubV3Temp(degrees)}`;
    done = current.length === wanted.length && current.every((s) => Number(s.attributes.temperature) === degrees);
  } else {
    return null;
  }
  const presets = current.map((s) => s.attributes.preset_mode).filter((p) => typeof p === "string");
  const now = presets.length === current.length && one(presets) ? `en ${said(presets[0]).toLowerCase()}`
    : current.length > 1 ? `${current.length} chauffages` : "";
  return { now, then: done ? `déjà ${then.toLowerCase()}` : then, done };
}

/** L'anneau qui se vide pendant le compte à rebours. */
function hubV4Ring() {
  const NS = "http://www.w3.org/2000/svg";
  const svg = document.createElementNS(NS, "svg");
  svg.setAttribute("viewBox", "0 0 44 44");
  svg.setAttribute("class", "size-11 shrink-0 -rotate-90");
  svg.setAttribute("aria-hidden", "true");
  const rings = [
    ["stroke-voile/12", ""],
    ["stroke-v-violet [stroke-dasharray:100] animate-v4-count motion-reduce:animate-none", "100"],
  ];
  for (const [style, length] of rings) {
    const circle = document.createElementNS(NS, "circle");
    const attributes = { cx: "22", cy: "22", r: "19", fill: "none", "stroke-width": "4", "stroke-linecap": "round", class: style };
    for (const [name, value] of Object.entries(attributes)) circle.setAttribute(name, value);
    if (length) circle.setAttribute("pathLength", length);
    svg.append(circle);
  }
  return svg;
}

/**
 * Toucher un moment (une scène) en v4 : ce qui va se passer, puis il se lance
 * seul dans 3 s (Annuler l'arrête). Faux si la v4 n'est pas là ou si la scène
 * ne dit pas ce qu'elle règle (elle se lance alors directement).
 */
function hubV4Moment(state) {
  if (!hubV4On() || !hub.on || hub.away) return false;
  if (domainOf(state) !== "scene") return false;
  const entityId = state.entity_id;
  const targets = () => entities.get(entityId)?.attributes.targets;
  if (hubV4MomentRows(targets(), (id) => entities.get(id)).length === 0) return false;

  const list = el("ul", { class: "flex flex-col gap-1 m-0 p-0 list-none", "aria-label": "Ce qui va changer" });
  const count = el("b", { class: "text-courant font-semibold", "aria-live": "polite" });
  let left = HUB_V4_COUNTDOWN;
  let ticker = 0;
  const launch = () => {
    clearInterval(ticker);
    // Le moment touche plusieurs appareils : « Annuler » remet lumières, prises et volets comme avant.
    hubV3Orders(null, [["scene", "turn_on", { entity_id: entityId }]], `« ${nameOf(state)} » lancé`);
    hubV4Close();
  };
  const cancel = el("button", { type: "button", class: HUB_V4_BTN, text: "Annuler" });
  cancel.addEventListener("click", () => hubV4Close());
  const go = el("button", { type: "button", class: HUB_V4_MAIN, text: "Lancer" });
  go.addEventListener("click", launch);
  setText(count, `Se lance dans ${left} s`);

  hubV4Overlay({
    // L'icône du moment (la lune pour « Bonne nuit »), comme sur l'accueil.
    icon: typeof hubSceneLook === "function" ? hubSceneLook(state, 0).icon : ICONS.sparkle,
    tone: "violet",
    title: nameOf(state),
    sub: el("span", { class: `text-petit ${HUB_V4_MUTE}`, text: "Ce qui va se passer" }),
    body: [
      list,
      el("div", { class: "flex items-center gap-3 p-3 rounded-panneau bg-v-violet/12" },
        hubV4Ring(),
        el("div", { class: "flex flex-col" }, count,
          el("span", { class: `text-petit ${HUB_V4_MUTE}`, text: "Touche Annuler pour arrêter." }))),
      el("div", { class: "grid grid-cols-2 gap-2" }, cancel, go),
    ],
    spot: null,
    refresh: () => {
      if (!entities.has(entityId)) {
        hubV4Close();
        return;
      }
      const rows = hubV4MomentRows(targets(), (id) => entities.get(id));
      swap(list, ...rows.map((row) => el("li", { class: "flex items-center gap-3 min-h-12 px-3 py-1.5 rounded-[16px] bg-voile/6" },
        el("span", { class: `${HUB_V4_MUTE} [&_svg]:size-5`, "aria-hidden": "true" },
          svgIcon(HUB_V4_GROUPS.find(([domain]) => domain === row.domain)[2])),
        el("span", { class: "flex flex-col flex-1 min-w-0" },
          el("b", { class: "text-corps font-semibold", text: row.label }),
          el("span", { class: `text-legende ${HUB_V4_MUTE} truncate`, text: row.now })),
        el("span", {
          class: `text-petit text-right ${row.done ? "text-v-ok" : "text-v-ink"}`, text: row.then,
        }))));
    },
    onClose: () => clearInterval(ticker),
  });
  ticker = setInterval(() => {
    left -= 1;
    if (left <= 0) launch();
    else setText(count, `Se lance dans ${left} s`);
  }, 1000);
  return true;
}

// ------------------------------------------------------ Un ordre à valider

/** Les ordres qui attendent une validation, le plus ancien d'abord. */
function hubV4Approvals() {
  return noticeStates()
    .filter((s) => s.state === "notifying" && s.attributes.kind === "approval")
    .sort((a, b) => String(a.attributes.created_at).localeCompare(String(b.attributes.created_at)));
}

/** « Sans réponse, il est refusé dans 10 min. » (vide si l'heure manque). */
function hubV4Expiry(expiresAt, now) {
  const end = Date.parse(expiresAt);
  if (!Number.isFinite(end)) return "";
  const minutes = Math.ceil((end - now) / 60000);
  if (minutes <= 1) return "Sans réponse, il est refusé dans moins d'une minute.";
  return `Sans réponse, il est refusé dans ${minutes} min.`;
}

/** « Demandé à la voix (Alexa) · 21:42 » (l'heure locale de la demande). */
function hubV4AskedBy(origin, createdAt) {
  const date = new Date(createdAt);
  const time = Number.isNaN(date.getTime()) ? ""
    : `${String(date.getHours()).padStart(2, "0")}:${String(date.getMinutes()).padStart(2, "0")}`;
  if (origin) return time ? `Demandé ${origin} · ${time}` : `Demandé ${origin}`;
  return time ? `Demandé à ${time}` : "Demandé";
}

/**
 * L'ordre à valider en v4 : qui l'a demandé, l'ordre, pourquoi il est retenu,
 * Refuser ou Valider. Faux si la v4 n'est pas là ou si rien n'attend.
 */
function hubV4Approval() {
  if (!hubV4On() || !hub.on || hub.away) return false;
  if (hubV4Approvals().length === 0) return false;
  let shown = "";
  let busy = false;
  const sub = el("span", { class: `text-petit ${HUB_V4_MUTE}` });
  const who = el("span");
  const order = el("p", { class: "m-0 font-recit text-titre font-normal leading-snug" });
  const reason = el("p", { class: `m-0 text-petit ${HUB_V4_MUTE}` });
  // « Ne plus demander pour cet ordre » (maquette) : l'API ne prend que { approve } ; pas de case.
  const expiry = hubV4Note(HUB_V4_MORE_LINES.clock);
  const deny = el("button", { type: "button", class: HUB_V4_BTN }, svgIcon(HUB_V4_MORE_LINES.cross), el("span", { text: "Refuser" }));
  const approve = el("button", { type: "button", class: HUB_V4_MAIN, text: "Valider" });
  const answer = async (yes) => {
    if (!shown || busy) return;
    busy = true;
    deny.disabled = true;
    approve.disabled = true;
    // Sans réseau, api() lève une erreur : les boutons ne doivent pas rester bloqués.
    const { status, data } = await api("POST", `/api/approvals/${shown}`, { approve: yes })
      .catch(() => ({ status: 0, data: {} }));
    busy = false;
    deny.disabled = false;
    approve.disabled = false;
    if (status === 404) flash("Cet ordre n'attend plus : déjà traité, ou oublié.");
    else if (status !== 200) flash(typeof data.error === "string" ? `${data.error}.` : "Réponse impossible.");
    else flash(yes ? "Ordre validé : il est parti." : "Ordre refusé.");
    hubV4.refresh?.();
  };
  deny.addEventListener("click", () => answer(false));
  approve.addEventListener("click", () => answer(true));

  hubV4Overlay({
    icon: ICONS.list,
    title: "Ordre à valider",
    sub,
    body: [
      el("div", { class: "flex flex-col gap-2 p-3.5 rounded-panneau bg-voile/6" },
        el("span", { class: `flex items-center gap-2 text-petit ${HUB_V4_MUTE}` },
          el("span", { class: "text-v-sky [&_svg]:size-4", "aria-hidden": "true" }, svgIcon(ICONS.mic)), who),
        order, reason),
      el("div", { class: "grid grid-cols-2 gap-2" }, deny, approve),
      expiry.node,
    ],
    more: { label: "Tous les avis", run: () => $("#open-notices").click() },
    spot: null,
    refresh: () => {
      const waiting = hubV4Approvals();
      if (waiting.length === 0) {
        hubV4Close();
        return;
      }
      const state = waiting.find((s) => String(s.attributes.approval_id) === shown) ?? waiting[0];
      shown = String(state.attributes.approval_id ?? "");
      const a = state.attributes;
      setText(sub, waiting.length === 1 ? "1 en attente" : `${waiting.length} en attente`);
      setText(who, hubV4AskedBy(typeof a.origin === "string" ? a.origin : "", a.created_at));
      setText(order, typeof a.order === "string" ? `« ${a.order} »` : String(a.message ?? ""));
      setText(reason, typeof a.reason === "string" ? `Retenu parce que ${a.reason}.` : "");
      reason.hidden = typeof a.reason !== "string";
      const text = hubV4Expiry(a.expires_at, Date.now());
      setText(expiry.words, text);
      expiry.node.hidden = text === "";
    },
  });
  return true;
}

// Retour au style « Actuel » : rien de la v4 ne reste ouvert.
document.querySelector?.("#hub-style")?.addEventListener("change", () => {
  if (!hubV4On()) hubV4Shut();
});

// ---------------------------------------------------------------- L'accueil v4 : ses composants
// Ce que la v4 change sur l'accueil (hub-v3.js et hub.js les appellent quand
// hubV4On()) : la scène suggérée, Tempo en pastilles, la ligne des courses.
// La v3 garde son rendu. Le style propre aux blocs repris de la v3 est à la
// fin de hub-v3.css (« v4 accueil : composants »).

const HUB_V4_HOME_ICONS = {
  eye: ["M2 12s3.5-7 10-7 10 7 10 7-3.5 7-10 7S2 12 2 12z", "M12 9a3 3 0 1 0 0 6 3 3 0 0 0 0-6z"],
  cart: ["M3 4h2l2.4 11h10.2L20 8H6.2", "M9 20h.01", "M17 20h.01"],
  plus: ["M12 5v14", "M5 12h14"],
};
// La scène suggérée selon l'heure : [de, à (heures, 24 + n pour la nuit), nom].
const HUB_V4_SUGGEST = [
  [5, 11, /matin|r[ée]veil|lever|bonjour/i],
  [11, 17, /pars|d[ée]part|absen|quitt/i],
  [17, 21, /retour|arriv|rentr|bienvenue|soir|repas|d[îi]ner/i],
  [21, 29, /nuit|coucher|dodo/i],
];
// Le point de couleur d'une pastille Tempo (classes écrites en entier pour Tailwind).
const HUB_V4_TEMPO_DOTS = { Bleu: "bg-v-sky", Blanc: "bg-v-ink", Rouge: "bg-v-alert" };

/** L'index de la scène à mettre en avant à cette heure, ou -1. */
function hubV4Suggested(names, hour) {
  const h = hour < 5 ? hour + 24 : hour;
  const slot = HUB_V4_SUGGEST.find(([from, to]) => h >= from && h < to);
  return slot ? names.findIndex((name) => slot[2].test(String(name))) : -1;
}

/** Ce que dit une pastille Tempo : « Bleu aujourd'hui », « Demain : pas annoncé ». */
function hubV4TempoText(color, when) {
  if (lookup(HUB_V4_TEMPO_DOTS, color)) return `${color} ${when}`;
  const day = when.charAt(0).toUpperCase() + when.slice(1);
  if (color === "unavailable") return `${day} : indisponible`;
  return color === "unknown" || color === "" || color == null ? `${day} : pas encore annoncé` : `${day} : ${color}`;
}

/** Ce qui reste sur la liste des courses : « vides » ou le nombre. */
function hubV4CoursesText(left) {
  return left > 0 ? String(left) : "vides";
}

/** Une pastille Tempo : un point de la couleur du jour, puis le texte. */
function hubV4TempoPill(state, when) {
  const color = state ? state.state : "unknown";
  const dot = lookup(HUB_V4_TEMPO_DOTS, color) ?? "bg-v-mute";
  return el("span", { class: "inline-flex items-center gap-1.5 h-6 px-2.5 rounded-full bg-voile/8 text-legende font-medium text-v-ink2" },
    el("span", { class: `size-1.5 shrink-0 rounded-full ${dot}`, "aria-hidden": "true" }),
    hubV4TempoText(color, when));
}

/** La ligne « Courses et tâches » au bas de l'agenda, ou null sans liste. */
function hubV4CoursesRow() {
  const state = entities.get(HUB_LIST);
  if (!state) return null;
  const left = todoItems(state).filter((item) => item.status !== "completed").length;
  const row = el("button", {
    type: "button",
    class: `${HUB_HIT_Y2} flex items-center gap-2.5 w-full min-h-10 mt-1.5 px-3 rounded-champ border-0 bg-voile/6 text-left cursor-pointer `
      + "hover:bg-voile/10 [&>svg]:size-4 [&>svg]:shrink-0 [&>svg]:text-v-mute " + HUB_V4_FOCUS,
    "aria-label": left > 0 ? `Courses et tâches : ${left} à faire` : "Courses et tâches : rien à faire",
  }, svgIcon(HUB_V4_HOME_ICONS.cart),
  el("span", { class: "flex-1 min-w-0 text-petit font-medium text-v-ink2 truncate", text: "Courses et tâches" }),
  el("span", { class: `text-petit ${HUB_V4_MUTE}`, text: hubV4CoursesText(left) }));
  row.addEventListener("click", () => $("#hub-courses").click());
  return row;
}

/** L'en-tête v4 de l'agenda : icône, titre, quand, et « + » pour ajouter. */
function hubV4AgendaHead() {
  const parts = [svgIcon(ICONS.calendar), el("h2", { class: "m-0! flex-1 min-w-0 justify-start! gap-2 whitespace-nowrap", text: "Agenda" },
    el("small", { class: "min-w-0 truncate", text: new Date().getHours() >= 18 ? "ce soir et demain" : "aujourd'hui et demain" }))];
  if (typeof hubV4NewEvent === "function" && entities.get("calendar.maison") && canControl(entities.get("calendar.maison"))) {
    const add = el("button", {
      type: "button",
      class: `${HUB_HIT_6} grid place-items-center size-8 shrink-0 p-0 rounded-full border-0 bg-transparent text-v-ink cursor-pointer `
        + "hover:bg-voile/10 [&>svg]:size-[18px] " + HUB_V4_FOCUS,
      "aria-label": "Ajouter un rendez-vous",
      title: "Ajouter un rendez-vous",
    }, svgIcon(HUB_V4_HOME_ICONS.plus));
    add.addEventListener("click", () => hubV4NewEvent());
    parts.push(add);
  }
  return el("div", { class: "hub-v4-agenda-head flex items-center gap-2 mx-1 -mt-1 mb-0.5 [&>svg]:size-[18px] [&>svg]:shrink-0 [&>svg]:text-v-ink2" }, ...parts);
}

/** Ouvrir une pièce depuis l'accueil v4 : sa page, sinon sa fenêtre, sinon la pièce. */
function hubV4OpenRoom(name) {
  if (typeof hubV4Page === "function" && hubV4Page(name)) return;
  if (hubV4Room(name)) return;
  openRoom(name);
}
