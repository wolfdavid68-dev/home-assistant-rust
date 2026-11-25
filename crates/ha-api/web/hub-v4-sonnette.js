// Accueil v4 : la sonnette (« On sonne à la porte ») et la caméra en grand.
//
// Quand on sonne, la fenêtre se pose à côté de la porte sur le dessin : la
// photo de la sonnette, « Ignorer » et « Voir la caméra ». Elle se ferme seule
// au bout de 3 minutes, comme la fenêtre d'avant (sonnette.js).
//
// « Voir la caméra » (ou toucher la porte quand on ne sonne pas) ouvre la
// dernière photo en grand, avec les derniers passages devant la porte (journal
// de 24 h), l'annonce et « Garder l'image ». Ce sont des photos, pas une image
// en direct : la sonnette n'en envoie pas. Parler à la porte n'existe pas non
// plus : rien ne fait semblant.
//
// Hors v4 (ou ailleurs que sur l'accueil), la fenêtre d'avant. Styles en
// classes Tailwind ; pas d'innerHTML. Chargé après hub-v4-incrustations.js.
"use strict";

const HUB_V4_BELL_ICONS = {
  camera: ["M3 8.5A2.5 2.5 0 0 1 5.5 6h2l1.6-2h5.8l1.6 2h2A2.5 2.5 0 0 1 21 8.5v9a2.5 2.5 0 0 1-2.5 2.5h-13A2.5 2.5 0 0 1 3 17.5z",
    "M12 9.5a3.5 3.5 0 1 0 0 7a3.5 3.5 0 1 0 0-7"],
  keep: ["M12 4v11", "M7 10l5 5 5-5", "M5 20h14"],
};
// Moins d'une minute : « à l'instant ».
const HUB_V4_JUST_NOW = 60_000;
// Une photo prise un peu avant le coup de sonnette compte comme la nouvelle.
const HUB_V4_FRESH = 15_000;
// La surface des cartes (celle des incrustations).
const HUB_V4_GLASS = "rounded-[28px] bg-scrim/80 backdrop-blur-[20px] backdrop-saturate-150 "
  + "shadow-[inset_0_1px_0_color-mix(in_srgb,var(--v3-voile)_9%,transparent),inset_0_0_0_1px_color-mix(in_srgb,var(--v3-voile)_6%,transparent),0_18px_40px_-18px_rgb(0_0_0/0.65)]";
const HUB_V4_PHOTO_BG = "bg-[radial-gradient(120%_90%_at_30%_20%,#2a2f55,#121530_60%,#0b0d22)] "
  + "shadow-[inset_0_0_0_1px_color-mix(in_srgb,var(--v3-voile)_10%,transparent)]";
const HUB_V4_TAG = "inline-flex items-center gap-2 min-h-9 px-3 rounded-[12px] bg-scrim/60 text-petit font-medium";

/** « à l'instant », « à 21:56 » ; « la dernière photo » sans heure. */
function hubV4DoorbellWhen(at, now = Date.now()) {
  if (!Number.isFinite(at) || at <= 0) return "la dernière photo";
  return now - at < HUB_V4_JUST_NOW ? "à l'instant" : `à ${hubV4Clock(new Date(at))}`;
}

/** « aujourd'hui · 21:12 », « hier · 11:02 », « 27/10 · 08:15 ». */
function hubV4DayTime(at, now = new Date()) {
  const date = new Date(at);
  if (typeof date.getTime !== "function" || Number.isNaN(date.getTime())) return "";
  const day = (d) => new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
  const days = Math.round((day(now) - day(date)) / 86_400_000);
  const label = days === 0 ? "aujourd'hui" : days === 1 ? "hier"
    : `${String(date.getDate()).padStart(2, "0")}/${String(date.getMonth() + 1).padStart(2, "0")}`;
  return `${label} · ${hubV4Clock(date)}`;
}

/** Les capteurs de la même porte : `binary_sensor.porte_sonnette` → `binary_sensor.porte…`. */
function hubV4DoorSensors(doorbellId, ids) {
  const base = String(doorbellId).replace(/^binary_sensor\./, "").replace(/_(ding|sonnette)$/, "");
  return ids.filter((id) => id === doorbellId || id.startsWith(`binary_sensor.${base}_`));
}

/**
 * Les derniers passages devant la porte, d'après le journal : le coup de
 * sonnette et le mouvement, pas leur fin (« ne détecte plus… »). `names` :
 * l'identifiant → le nom de la maison. Les plus récents d'abord.
 */
function hubV4Passages(entries, doorbellId, ids, names = new Map(), keep = 3) {
  const wanted = new Set(hubV4DoorSensors(doorbellId, ids));
  return (Array.isArray(entries) ? entries : [])
    .filter((entry) => wanted.has(entry?.entity_id) && typeof entry.at === "string" && !Number.isNaN(Date.parse(entry.at)))
    .filter((entry) => typeof entry.message === "string" && !/^ne /.test(entry.message))
    .sort((a, b) => Date.parse(b.at) - Date.parse(a.at))
    .slice(0, keep)
    .map((entry) => {
      const bell = entry.entity_id === doorbellId;
      const motion = /mouvement/.test(entry.message);
      const name = names.get(entry.entity_id) ?? entry.name ?? "";
      const text = bell ? "On a sonné" : motion ? "Mouvement devant la porte" : `${name} ${entry.message}`.trim();
      return { at: entry.at, text, kind: bell ? "bell" : motion ? "motion" : "other" };
    });
}

/** Le rectangle (écran) de la porte de la sonnette sur le dessin, ou null. */
function hubV4DoorbellSpot(entityId) {
  for (const { zone, node } of house.view?.zones ?? []) {
    if (zone.entity_id === entityId && node.isConnected) return hubV4Rect(node);
  }
  return null;
}

/** La v4 prend la sonnette ? (l'accueil est affiché, en v4). */
function hubV4DoorbellReady() {
  return typeof hubV4Can === "function" && hubV4Can();
}

/** Le récit de l'accueil (« On sonne à la porte. ») suit la fenêtre. */
function hubV4DoorbellHello() {
  if (typeof hubV3RenderHello === "function" && typeof hubV3 !== "undefined" && hubV3.ui) hubV3RenderHello();
}

/** Le minuteur de fermeture : 3 minutes après le dernier toucher. */
function hubV4DoorbellTimer() {
  clearTimeout(doorbell.closeTimer);
  doorbell.closeTimer = setTimeout(() => hubV4Close(), DOORBELL_OPEN);
}

/** « On sonne à la porte » : la photo, « Ignorer », « Voir la caméra ». */
function hubV4Doorbell(state, at = doorbellRang(state)) {
  if (!hubV4DoorbellReady()) return false;
  const id = state.entity_id;
  const live = state.state === "on";
  const sub = el("span", { class: `text-petit ${HUB_V4_MUTE} truncate` });
  const img = el("img", { alt: "Photo prise par la sonnette", class: "absolute inset-0 size-full object-cover" });
  img.hidden = true;
  const empty = el("span", { class: "grid justify-items-center gap-2 text-corps text-white/75 [&_svg]:size-9 [&_svg]:opacity-60" },
    svgIcon(HUB_V4_BELL_ICONS.camera), el("span"));
  const wait = el("span", { class: `absolute left-2.5 top-2.5 ${HUB_V4_TAG}`, text: "La nouvelle photo arrive…" });
  const stamp = el("span", { class: `absolute left-2.5 bottom-2.5 ${HUB_V4_TAG} [font-variant-numeric:tabular-nums]` });
  const photo = el("div", { class: `relative grid place-items-center h-[189px] overflow-hidden rounded-[14px] ${HUB_V4_PHOTO_BG}` },
    empty, img, wait, stamp);
  const ignore = el("button", { type: "button", class: `${HUB_V4_BTN} flex-1`, text: "Ignorer" });
  ignore.addEventListener("click", () => hubV4Close());
  const look = el("button", { type: "button", class: `${HUB_V4_MAIN} flex-[1.5] whitespace-nowrap` }, svgIcon(HUB_V4_BELL_ICONS.camera), "Voir la caméra");
  look.addEventListener("click", () => hubV4Camera(id));
  // L'heure du coup de sonnette, avant le premier rendu (refresh s'en sert).
  doorbell.at = live && at !== null ? at : 0;
  const refresh = () => {
    const now = entities.get(id) ?? state;
    setText(sub, `Sonnette · ${doorbellPlace(now)} · ${hubV4DoorbellWhen(doorbell.at || at)}`);
    const camera = doorbellCamera(id);
    if (!camera) {
      img.hidden = true;
      wait.hidden = true;
      stamp.hidden = true;
      empty.hidden = false;
      setText(empty.lastChild, "Pas de caméra pour cette sonnette.");
      return;
    }
    const shot = Number(camera.attributes.image_at);
    const known = Number.isFinite(shot) && shot > 0;
    // La photo d'avant le coup de sonnette : montrée, en attendant la nouvelle.
    wait.hidden = !doorbell.at || (known && shot >= doorbell.at - HUB_V4_FRESH);
    stamp.hidden = !known;
    if (known) setText(stamp, `${doorbellPlace(now)} · ${hubV4Clock(new Date(shot))}`);
    empty.hidden = known;
    setText(empty.lastChild, "Pas encore de photo");
    showCameraImage(camera, img);
  };
  hubV4Overlay({
    icon: ICONS.bell,
    tone: "amber",
    title: live ? "On sonne à la porte" : "On a sonné",
    sub,
    body: [photo, el("div", { class: "flex gap-2" }, ignore, look)],
    spot: hubV4DoorbellSpot(id),
    refresh,
    // L'événement « close » arrive après coup : si une autre fenêtre de la
    // sonnette s'est ouverte entre-temps, son minuteur et sa caméra restent.
    onClose: () => {
      if (doorbell.owner !== mine) return;
      doorbell.owner = null;
      clearTimeout(doorbell.closeTimer);
      if (doorbell.current === id) doorbell.current = null;
      hubV4DoorbellHello();
    },
  });
  // Après hubV4Overlay : il ferme d'abord la fenêtre d'avant (qui oublie la sonnette).
  const mine = {};
  doorbell.owner = mine;
  doorbell.current = id;
  hubV4DoorbellHello();
  hubV4DoorbellTimer();
  return true;
}

/** La caméra de la porte en grand : la dernière photo, les passages, l'annonce. */
function hubV4Camera(entityId) {
  if (!hubV4DoorbellReady()) return false;
  const state = entities.get(entityId);
  if (!state) return false;
  hubV4Close();
  const dialog = el("dialog", {
    class: "fixed inset-0 m-0 p-0 w-screen h-dvh max-w-none max-h-none border-0 bg-transparent overflow-hidden "
      + "text-v-ink font-sans [color-scheme:var(--v4-scheme)] [font-variant-numeric:tabular-nums] backdrop:bg-transparent",
    "aria-labelledby": "hub-v4-cam-title",
  });
  const veil = el("div", {
    class: "absolute inset-0 bg-scrim/84 backdrop-blur-[6px] animate-v4-fade motion-reduce:animate-none",
  });
  const back = el("button", {
    type: "button", class: `${HUB_V4_ICON_BTN} bg-voile/10`, "aria-label": "Retour à l'accueil",
  }, svgIcon(ICONS.back));
  back.addEventListener("click", () => hubV4Close());
  const title = el("b", { id: "hub-v4-cam-title", class: "text-petit font-medium tracking-[0.02em] text-v-ink2" });
  const head = el("header", { class: "col-span-full flex items-center gap-3" }, back,
    el("span", { class: "inline-flex items-center gap-2 [&_svg]:size-4" }, svgIcon(HUB_V4_BELL_ICONS.camera), title));

  // La photo, avec sa barre du haut : l'heure de la photo, fermer.
  const img = el("img", { alt: "", class: "absolute inset-0 size-full object-contain" });
  img.hidden = true;
  const empty = el("span", { class: "grid justify-items-center gap-2.5 text-courant text-white/75 [&_svg]:size-11 [&_svg]:opacity-60" },
    svgIcon(HUB_V4_BELL_ICONS.camera), el("span"));
  const shotTag = el("span", { class: HUB_V4_TAG },
    el("i", { class: "size-2 rounded-full bg-v-amber shadow-[0_0_8px_var(--v3-amber)]", "aria-hidden": "true" }), el("span"));
  const close = el("button", { type: "button", class: `${HUB_V4_ICON_BTN} bg-scrim/60`, "aria-label": "Fermer" },
    svgIcon(HUB_V4_ICONS.close));
  close.addEventListener("click", () => hubV4Close());
  const view = el("section", {
    class: `relative grid place-items-center min-h-[280px] overflow-hidden ${HUB_V4_GLASS} ${HUB_V4_PHOTO_BG} `
      + "animate-v4-pop motion-reduce:animate-none",
    "aria-label": "Dernière photo de la sonnette",
  }, empty, img, el("div", { class: "absolute left-4 right-4 top-4 flex items-center gap-2.5" },
    shotTag, el("span", { class: "flex-1" }), close));

  // La barre du bas : l'annonce (s'il y a de quoi la dire), garder l'image.
  const say = el("button", { type: "button", class: `${HUB_V4_BTN} flex-1 min-h-12` }, svgIcon(ICONS.megaphone), "Annonce");
  say.addEventListener("click", () => hubV4Announce());
  const keep = el("a", { class: `${HUB_V4_BTN} flex-1 min-h-12 no-underline` }, svgIcon(HUB_V4_BELL_ICONS.keep), "Garder l'image");
  keep.hidden = true;
  const bar = el("div", { class: "flex gap-2" }, say, keep);

  // À côté : les derniers passages (journal de 24 h), et ce qu'il faut savoir.
  const passages = el("ul", { class: "m-0 p-0 list-none flex flex-col gap-1" });
  const log = el("section", { class: `flex flex-col gap-2.5 p-3 ${HUB_V4_GLASS} animate-v4-pop motion-reduce:animate-none`, "aria-label": "Derniers passages" },
    el("div", { class: "flex items-center gap-3 min-h-12" },
      el("span", { class: "grid place-items-center size-11 shrink-0 rounded-2xl bg-v-sky/16 text-v-sky [&_svg]:size-[22px]", "aria-hidden": "true" },
        svgIcon(ICONS.archive)),
      el("p", { class: "m-0 flex flex-col" }, el("b", { class: "text-moyen font-semibold leading-tight", text: "Derniers passages" }),
        el("span", { class: `text-petit ${HUB_V4_MUTE}`, text: "les dernières 24 heures" }))),
    passages);
  log.hidden = true;
  const note = (icon, text) => el("p", {
    class: "m-0 flex items-center gap-2 min-h-11 px-3 rounded-[16px] bg-voile/4 text-petit text-v-ink2 [&_svg]:size-4 [&_svg]:shrink-0",
  }, svgIcon(icon), el("span", { text }));
  const know = el("section", { class: `flex flex-col gap-1.5 p-3 ${HUB_V4_GLASS} animate-v4-pop motion-reduce:animate-none`, "aria-label": "À savoir" },
    note(ICONS.home, "Les images restent à la maison, rien n'est publié."),
    note(ICONS.timer, `Se ferme seule après ${Math.round(DOORBELL_OPEN / 60_000)} min sans toucher.`));
  const side = el("aside", { class: "row-span-2 flex flex-col gap-2.5 min-h-0 overflow-auto max-md:row-span-1" }, log, know);

  const grid = el("div", {
    class: "relative h-full grid grid-cols-[minmax(0,1fr)_356px] grid-rows-[auto_minmax(0,1fr)_auto] gap-x-6 gap-y-4 "
      + "px-11 py-8 max-md:grid-cols-1 max-md:grid-rows-none max-md:overflow-auto max-md:px-4 max-md:py-4",
  }, head, view, side, bar);
  dialog.append(veil, grid);

  if (account.permissions?.history !== false) {
    api("GET", "/api/logbook?hours=24").then(({ status, data }) => {
      if (status !== 200 || !passages.isConnected) return;
      const names = new Map([...entities.values()].map((s) => [s.entity_id, nameOf(s)]));
      const items = hubV4Passages(data?.entries, entityId, [...entities.keys()], names);
      log.hidden = items.length === 0;
      swap(passages, ...items.map((item) => el("li", { class: "flex items-center gap-3 min-h-14 p-2 rounded-[16px] bg-voile/4" },
        el("span", {
          class: "grid place-items-center size-10 shrink-0 rounded-[12px] bg-voile/10 text-v-ink2 "
            + "shadow-[inset_0_0_0_1px_color-mix(in_srgb,var(--v3-voile)_8%,transparent)] [&_svg]:size-[18px]",
          "aria-hidden": "true",
        }, svgIcon(item.kind === "bell" ? ICONS.bell : item.kind === "motion" ? ICONS.motion : ICONS.door)),
        el("p", { class: "m-0 flex-1 min-w-0 flex flex-col leading-snug" },
          el("span", { class: "font-medium truncate", text: item.text }),
          el("time", { class: `text-petit ${HUB_V4_MUTE}`, datetime: item.at, text: hubV4DayTime(item.at) })))));
    }).catch(() => {});
  }

  const refresh = () => {
    const now = entities.get(entityId) ?? state;
    const place = doorbellPlace(now);
    setText(title, `Caméra · ${place}`);
    say.hidden = typeof hubAnnouncer !== "function" || hubAnnouncer() === null;
    const camera = doorbellCamera(entityId);
    const shot = Number(camera?.attributes.image_at);
    const known = Number.isFinite(shot) && shot > 0;
    shotTag.hidden = !known;
    if (known) setText(shotTag.lastChild, `Dernière photo · ${place} · ${hubV4Clock(new Date(shot))}`);
    empty.hidden = known;
    setText(empty.lastChild, camera ? "Pas encore de photo" : "Pas de caméra pour cette sonnette.");
    if (!camera || !known) {
      img.hidden = true;
      keep.hidden = true;
      return;
    }
    setAttr(img, "alt", `Photo de la sonnette, ${place}, à ${hubV4Clock(new Date(shot))}`);
    cameraUrl(camera).catch(() => null).then((url) => {
      if (!img.isConnected) return;
      img.hidden = !url;
      keep.hidden = !url;
      if (!url) return;
      if (img.getAttribute("src") !== url) img.src = url;
      if (keep.getAttribute("href") !== url) keep.href = url;
      const at = new Date(shot);
      keep.download = `sonnette-${at.getFullYear()}${String(at.getMonth() + 1).padStart(2, "0")}${String(at.getDate()).padStart(2, "0")}`
        + `-${hubV4Clock(at).replace(":", "")}.jpg`;
    });
  };

  veil.addEventListener("click", () => hubV4Close());
  // Toucher l'écran repousse la fermeture.
  dialog.addEventListener("pointerdown", hubV4DoorbellTimer);
  dialog.addEventListener("keydown", hubV4DoorbellTimer);
  const focusBack = document.activeElement;
  const mine = {};
  dialog.addEventListener("close", () => {
    // Comme pour la fenêtre « On sonne » : seulement si elle est encore la dernière.
    if (doorbell.owner === mine) {
      doorbell.owner = null;
      clearTimeout(doorbell.closeTimer);
      if (doorbell.current === entityId) doorbell.current = null;
    }
    dialog.remove();
    if (hubV4.dialog === dialog) {
      hubV4.dialog = null;
      hubV4.refresh = null;
    }
    if (focusBack?.isConnected && !document.querySelector("dialog[open]")) focusBack.focus();
  });
  hubV4.dialog = dialog;
  hubV4.refresh = refresh;
  doorbell.owner = mine;
  doorbell.current = entityId;
  refresh();
  document.body.append(dialog);
  dialog.showModal();
  hubV4DoorbellTimer();
  return true;
}
