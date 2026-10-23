// Tableau de bord — le lecteur de musique (étape 92).
//
// Une page « Musique » pour les enceintes (Sonos) : la pochette en grand, le
// titre, une barre de progression qu'on touche pour avancer, les boutons
// (précédent, lecture, suivant, aléatoire, répétition), le volume, et les
// favoris Sonos avec leur pochette (des playlists YouTube Music…).
//
// Les pochettes viennent de notre serveur (`/api/media_cover/<clé>`) : une
// balise <img> ne sait pas envoyer le jeton, elles sont donc lues avec fetch
// puis montrées par une adresse blob:, comme les photos de la sonnette.
//
// La position du morceau n'est envoyée par le serveur que de temps en temps
// (`media_position` au moment `media_position_updated_at`) : la page la fait
// avancer seule, chaque seconde, tant que la musique joue.
"use strict";

const MUSIC_KEY = "ha-music-player";
/** Pochettes gardées en mémoire (les plus anciennes s'en vont). */
const MAX_MUSIC_COVERS = 80;
/** Une pochette qui n'a pas pu être lue est redemandée après ce délai. */
const MUSIC_COVER_RETRY = 30_000;
const REPEAT_NEXT = { off: "all", all: "one", one: "off" };
const REPEAT_LABELS = { off: "Répétition désactivée", all: "Répéter toute la file", one: "Répéter ce morceau" };

const music = {
  open: false,
  /** L'enceinte choisie (son identifiant), ou `null` : la première qui joue. */
  player: null,
  timer: null,
  /** Le doigt est sur la barre de progression : on ne la bouge pas. */
  seeking: false,
  /** Après un toucher sur la barre : la position voulue, en attendant que
   *  l'enceinte la confirme. */
  seek: null,
  /** Adresse de la pochette → adresse blob: (déjà lue). */
  covers: new Map(),
  loading: new Map(),
  failed: new Map(),
  /** Ce qu'affichent la grille des favoris et la liste des enceintes. */
  favorites: "",
  speakers: "",
};

Object.assign(ICONS, {
  music: ["M9 18V5l12-2v13", "M6 15a3 3 0 1 0 0 6 3 3 0 1 0 0-6z", "M18 13a3 3 0 1 0 0 6 3 3 0 1 0 0-6z"],
  shuffle: ["M16 3h5v5", "M4 20L21 3", "M21 16v5h-5", "M15 15l6 6", "M4 4l5 5"],
  repeat: ["M17 2l4 4-4 4", "M3 11V10a4 4 0 0 1 4-4h14", "M7 22l-4-4 4-4", "M21 13v1a4 4 0 0 1-4 4H3"],
  repeatOne: ["M17 2l4 4-4 4", "M3 11V10a4 4 0 0 1 4-4h14", "M7 22l-4-4 4-4", "M21 13v1a4 4 0 0 1-4 4H3", "M11 10h1.5v5"],
});

// ---------------------------------------------------------------- Pochettes

const isCoverPath = (path) => typeof path === "string" && /^\/api\/media_cover\/[0-9a-f]{32}$/.test(path);

/** L'adresse blob: d'une pochette (lue une seule fois), ou `null`. */
function loadCover(path) {
  if (!isCoverPath(path)) return Promise.resolve(null);
  if (music.covers.has(path)) return Promise.resolve(music.covers.get(path));
  if (music.loading.has(path)) return music.loading.get(path);
  if (Date.now() - (music.failed.get(path) ?? -Infinity) < MUSIC_COVER_RETRY) return Promise.resolve(null);
  const promise = fetch(path, { headers: { Authorization: `Bearer ${token.get()}` } })
    .then(async (response) => {
      if (!response.ok) throw new Error(String(response.status));
      const url = URL.createObjectURL(await response.blob());
      music.covers.set(path, url);
      music.failed.delete(path);
      // Une Map garde l'ordre d'arrivée : la première est la plus ancienne.
      while (music.covers.size > MAX_MUSIC_COVERS) {
        const [old, oldUrl] = music.covers.entries().next().value;
        music.covers.delete(old);
        URL.revokeObjectURL(oldUrl);
      }
      return url;
    })
    .catch(() => {
      music.failed.set(path, Date.now());
      return null;
    })
    .finally(() => music.loading.delete(path));
  music.loading.set(path, promise);
  return promise;
}

/** Montre la pochette `path` dans `img` (cachée tant qu'elle n'est pas là :
 *  la note de musique dessinée derrière se voit). */
function showCover(img, path) {
  const wanted = isCoverPath(path) ? path : "";
  img.dataset.cover = wanted;
  const ready = music.covers.get(wanted);
  if (ready) {
    if (img.getAttribute("src") !== ready) img.src = ready;
    img.hidden = false;
    return;
  }
  img.hidden = true;
  img.removeAttribute("src");
  if (wanted === "") return;
  loadCover(wanted).then((url) => {
    if (url && img.dataset.cover === wanted) {
      img.src = url;
      img.hidden = false;
    }
  });
}

/** Une pochette carrée : la note de musique, et l'image par-dessus. */
function coverBox(className, path) {
  const img = el("img", { alt: "", decoding: "async", draggable: "false" });
  showCover(img, path);
  return el("span", { class: className }, svgIcon(ICONS.music), img);
}

// ---------------------------------------------------------------- Enceintes

function musicPlayers() {
  return [...entities.values()]
    .filter((state) => domainOf(state) === "media_player")
    .sort((a, b) => nameOf(a).localeCompare(nameOf(b), "fr"));
}

/** L'enceinte affichée : celle choisie, sinon une qui joue, sinon la première. */
function musicPlayer() {
  const players = musicPlayers();
  let wanted = music.player;
  if (wanted === null) {
    try {
      wanted = localStorage.getItem(MUSIC_KEY);
    } catch {
      wanted = null;
    }
  }
  return players.find((p) => p.entity_id === wanted)
    ?? players.find((p) => p.state === "playing")
    ?? players[0]
    ?? null;
}

function chooseMusicPlayer(entityId) {
  music.player = entityId;
  music.seek = null;
  try {
    localStorage.setItem(MUSIC_KEY, entityId);
  } catch {
    // Stockage bloqué : le choix ne sera pas retenu.
  }
  renderMusic();
}

/** Une commande pour l'enceinte affichée. */
function musicCommand(state, service, data = {}) {
  markPending(state.entity_id);
  callService("media_player", service, { entity_id: state.entity_id, ...data });
}

// ---------------------------------------------------------------- Position

/** « 3:07 », ou « 1:02:45 » au-delà d'une heure. */
function musicTime(seconds) {
  const total = Math.max(0, Math.floor(seconds));
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = String(total % 60).padStart(2, "0");
  return h > 0 ? `${h}:${String(m).padStart(2, "0")}:${s}` : `${m}:${s}`;
}

/** Où en est le morceau, maintenant : `{ position, duration }` (en secondes,
 *  `null` si l'enceinte ne le dit pas — une radio, par exemple). */
function musicPosition(state) {
  const a = state.attributes;
  const duration = finite(a.media_duration) && a.media_duration > 0 ? a.media_duration : null;
  if (!finite(a.media_position)) return { position: null, duration };
  const playing = state.state === "playing";
  const seek = music.seek;
  let position;
  if (seek && seek.entity === state.entity_id && seek.stamp === a.media_position_updated_at && Date.now() - seek.at < 10_000) {
    // L'enceinte n'a pas encore confirmé : on montre la position demandée.
    position = seek.position + (playing ? (Date.now() - seek.at) / 1000 : 0);
  } else {
    const since = Date.parse(a.media_position_updated_at);
    position = a.media_position + (playing && Number.isFinite(since) ? Math.max(0, (Date.now() - since) / 1000) : 0);
  }
  return { position: duration === null ? position : Math.min(position, duration), duration };
}

// ---------------------------------------------------------------- La page

/** Les éléments de la page, créés une fois puis mis à jour. */
const musicUi = (() => {
  const img = el("img", { alt: "", decoding: "async", draggable: "false" });
  img.hidden = true;
  const ui = {
    root: $("#music-view"),
    speakers: el("div", { class: "music-speakers", role: "group", "aria-label": "Enceinte" }),
    art: el("div", { class: "music-art" }, svgIcon(ICONS.music), img),
    img,
    where: el("p", { class: "music-where" }),
    title: el("h2", { class: "music-title" }),
    artist: el("p", { class: "music-artist" }),
    seek: el("input", { type: "range", class: "music-seek", min: "0", max: "1", step: "1", value: "0", "aria-label": "Position dans le morceau" }),
    elapsed: el("span", { class: "music-elapsed" }),
    total: el("span", { class: "music-total" }),
    progress: el("div", { class: "music-progress" }),
    shuffle: el("button", { type: "button", class: "music-mode" }, svgIcon(ICONS.shuffle)),
    previous: el("button", { type: "button", class: "music-skip", "aria-label": "Morceau précédent" }, svgIcon(ICONS.previous)),
    play: el("button", { type: "button", class: "music-play" }),
    next: el("button", { type: "button", class: "music-skip", "aria-label": "Morceau suivant" }, svgIcon(ICONS.next)),
    repeat: el("button", { type: "button", class: "music-mode" }),
    mute: el("button", { type: "button", class: "music-mute" }),
    volume: el("input", { type: "range", class: "music-volume-slider", min: "0", max: "100", step: "1", value: "0", "aria-label": "Volume" }),
    volumeValue: el("span", { class: "music-volume-value" }),
    grid: el("div", { class: "music-grid" }),
    // Pas encore de favori : on explique comment en ajouter.
    favoritesHint: el("p", {
      class: "music-hint",
      text: "Pour lancer une playlist d'ici, ajoute-la aux favoris dans l'appli Sonos : ouvre-la dans YouTube Music, touche « … » puis « Ajouter aux favoris Sonos ». Elle apparaîtra ici avec sa pochette.",
    }),
    favoritesCard: el("section", { class: "music-card music-favorites", "aria-labelledby": "music-favorites-title" }),
    empty: el("p", { class: "music-empty", text: "Aucune enceinte pour l'instant." }),
  };
  ui.progress.append(ui.seek, el("div", { class: "music-times" }, ui.elapsed, ui.total));
  ui.favoritesCard.append(el("h3", { id: "music-favorites-title", text: "Favoris" }), ui.grid, ui.favoritesHint);
  ui.now = el(
    "section",
    { class: "music-card music-now", "aria-label": "En cours de lecture" },
    ui.art,
    el("div", { class: "music-info" }, ui.where, ui.title, ui.artist),
    ui.progress,
    el("div", { class: "music-transport" }, ui.shuffle, ui.previous, ui.play, ui.next, ui.repeat),
    el("div", { class: "music-volume" }, ui.mute, ui.volume, ui.volumeValue),
  );
  ui.root.append(el("div", { class: "music-backdrop", "aria-hidden": "true" }), ui.speakers, ui.now, ui.favoritesCard, ui.empty);
  fillSlider(ui.seek);
  fillSlider(ui.volume);

  // Les gestes relisent l'enceinte au moment du toucher.
  const act = (handler) => () => {
    const state = musicPlayer();
    if (state && canControl(state)) handler(state, state.attributes);
  };
  ui.previous.addEventListener("click", act((state) => musicCommand(state, "media_previous_track")));
  ui.next.addEventListener("click", act((state) => musicCommand(state, "media_next_track")));
  ui.play.addEventListener("click", act((state) => musicCommand(state, state.state === "playing" ? "media_pause" : "media_play")));
  ui.shuffle.addEventListener("click", act((state, a) => {
    if (typeof a.shuffle === "boolean") musicCommand(state, "shuffle_set", { shuffle: !a.shuffle });
  }));
  ui.repeat.addEventListener("click", act((state, a) => {
    const next = lookup(REPEAT_NEXT, a.repeat);
    if (next) musicCommand(state, "repeat_set", { repeat: next });
  }));
  ui.mute.addEventListener("click", act((state, a) => musicCommand(state, "volume_mute", { is_volume_muted: a.is_volume_muted !== true })));
  ui.volume.addEventListener("input", () => setText(ui.volumeValue, `${ui.volume.value} %`));
  ui.volume.addEventListener("change", act((state) => musicCommand(state, "volume_set", { volume_level: Number(ui.volume.value) / 100 })));
  // La barre de progression : on la suit du doigt, puis on y saute.
  ui.seek.addEventListener("input", () => {
    music.seeking = true;
    setText(ui.elapsed, musicTime(Number(ui.seek.value)));
  });
  ui.seek.addEventListener("change", act((state, a) => {
    music.seeking = false;
    const position = Number(ui.seek.value);
    music.seek = { entity: state.entity_id, stamp: a.media_position_updated_at, position, at: Date.now() };
    musicCommand(state, "media_seek", { seek_position: position });
    tickMusic();
  }));
  ui.seek.addEventListener("blur", () => { music.seeking = false; });
  return ui;
})();

function renderMusic() {
  const ui = musicUi;
  const state = musicPlayer();
  const players = musicPlayers();
  ui.empty.hidden = state !== null;
  ui.now.hidden = state === null;
  ui.favoritesCard.hidden = state === null;
  renderSpeakers(players, state);
  if (state === null) return;

  const a = state.attributes;
  const allowed = canControl(state);
  const available = !["unavailable", "unknown"].includes(state.state);
  const playing = state.state === "playing";
  const stopped = state.state === "idle" || state.state === "off";
  ui.now.classList.toggle("playing", playing);

  // La pochette, et son reflet flou derrière toute la page.
  const picture = playing || state.state === "paused" ? a.entity_picture : null;
  showCover(ui.img, picture);
  loadCover(picture).then((url) => {
    if (url) ui.root.style.setProperty("--art", `url("${url}")`);
    else if (musicPlayer()?.attributes.entity_picture !== picture || !picture) ui.root.style.removeProperty("--art");
  });
  if (!isCoverPath(picture)) ui.root.style.removeProperty("--art");

  // Où, et quoi.
  const group = (Array.isArray(a.group_members) ? a.group_members : [])
    .filter((id) => id !== state.entity_id)
    .map((id) => entities.get(id))
    .filter(Boolean)
    .map(nameOf);
  const status = available ? lookup(MEDIA_LABELS, state.state) ?? statusText(state) : "Injoignable";
  setText(ui.where, [nameOf(state), group.length > 0 ? `avec ${joinNames(group)}` : "", status].filter(Boolean).join(" · "));
  const text = (value) => (typeof value === "string" && value.trim() !== "" ? value.trim() : "");
  const channel = text(a.media_channel);
  const title = stopped ? "" : text(a.media_title) || channel || text(a.source);
  setText(ui.title, title || (stopped ? "Rien en cours" : "Sans titre"));
  setText(ui.artist, stopped
    ? "Choisis un favori pour lancer la musique."
    : [text(a.media_artist), text(a.media_album_name)].filter(Boolean).join(" — ") || (title === channel ? "" : channel));

  // Les boutons.
  ui.play.replaceChildren(svgIcon(playing ? ICONS.pause : ICONS.play));
  ui.play.setAttribute("aria-label", playing ? "Mettre en pause" : "Lancer la lecture");
  const shuffleKnown = typeof a.shuffle === "boolean";
  ui.shuffle.setAttribute("aria-pressed", String(a.shuffle === true));
  ui.shuffle.setAttribute("aria-label", shuffleKnown ? (a.shuffle ? "Lecture aléatoire activée" : "Lecture aléatoire") : "Lecture aléatoire (seulement avec une file d'attente)");
  const repeat = lookup(REPEAT_LABELS, a.repeat) ? a.repeat : null;
  ui.repeat.replaceChildren(svgIcon(repeat === "one" ? ICONS.repeatOne : ICONS.repeat));
  ui.repeat.setAttribute("aria-pressed", String(repeat === "all" || repeat === "one"));
  ui.repeat.setAttribute("aria-label", repeat ? REPEAT_LABELS[repeat] : "Répétition (seulement avec une file d'attente)");
  for (const button of [ui.previous, ui.play, ui.next]) button.disabled = !allowed || !available;
  ui.shuffle.disabled = !allowed || !available || !shuffleKnown;
  ui.repeat.disabled = !allowed || !available || repeat === null;

  // Le volume.
  const volume = finite(a.volume_level) ? Math.round(a.volume_level * 100) : 0;
  const muted = a.is_volume_muted === true;
  ui.mute.replaceChildren(svgIcon(muted ? ICONS.muted : ICONS.speaker));
  ui.mute.setAttribute("aria-label", muted ? "Remettre le son" : "Couper le son");
  ui.mute.setAttribute("aria-pressed", String(muted));
  ui.mute.disabled = !allowed || !available;
  ui.volume.disabled = !allowed || !available;
  if (document.activeElement !== ui.volume) {
    ui.volume.value = String(volume);
    ui.volume.dispatchEvent(new Event("input"));
  }
  ui.volume.setAttribute("aria-valuetext", muted ? `${volume} %, son coupé` : `${volume} %`);

  tickMusic();
  renderMusicFavorites(state, allowed && available);
}

/** La barre de progression et les temps (chaque seconde). */
function tickMusic() {
  const ui = musicUi;
  const state = musicPlayer();
  if (state === null) return;
  const stopped = state.state === "idle" || state.state === "off";
  const { position, duration } = musicPosition(state);
  ui.progress.hidden = stopped || position === null;
  if (ui.progress.hidden) return;
  ui.progress.classList.toggle("live", duration === null);
  ui.seek.disabled = duration === null || !canControl(state) || !["playing", "paused"].includes(state.state);
  if (duration === null) {
    // Une radio : pas de fin, rien où sauter.
    ui.seek.max = "1";
    ui.seek.value = "1";
    ui.seek.style.setProperty("--fill", "100%");
    ui.seek.removeAttribute("aria-valuetext");
    music.seeking = false;
    setText(ui.elapsed, musicTime(position));
    setText(ui.total, "En direct");
    return;
  }
  if (music.seeking) return;
  ui.seek.max = String(Math.max(1, Math.floor(duration)));
  ui.seek.value = String(Math.floor(position));
  ui.seek.style.setProperty("--fill", `${Math.min(100, (position / duration) * 100)}%`);
  ui.seek.setAttribute("aria-valuetext", `${musicTime(position)} sur ${musicTime(duration)}`);
  setText(ui.elapsed, musicTime(position));
  setText(ui.total, `-${musicTime(duration - position)}`);
}

/** Les enceintes, à choisir (s'il y en a plusieurs). */
function renderSpeakers(players, current) {
  const ui = musicUi;
  ui.speakers.hidden = players.length < 2;
  const signature = players.map((p) => `${p.entity_id}|${nameOf(p)}|${p.state}|${p === current ? 1 : 0}`).join("\n");
  if (ui.speakers.hidden || music.speakers === signature) return;
  music.speakers = signature;
  ui.speakers.replaceChildren(...players.map((player) => {
    const chip = el("button", { type: "button", class: "music-speaker", "aria-pressed": String(player === current) },
      svgIcon(ICONS.speaker), nameOf(player));
    if (player.state === "playing") {
      chip.classList.add("playing");
      chip.append(el("span", { class: "music-eq", "aria-hidden": "true" }, el("i"), el("i"), el("i")));
      chip.setAttribute("aria-label", `${nameOf(player)}, en lecture`);
    }
    chip.addEventListener("click", () => chooseMusicPlayer(player.entity_id));
    return chip;
  }));
}

/** Les favoris Sonos (étape 91), avec leur pochette : un toucher les lance. */
function renderMusicFavorites(state, allowed) {
  const ui = musicUi;
  const a = state.attributes;
  const names = (Array.isArray(a.source_list) ? a.source_list : []).filter((n) => typeof n === "string" && n !== "");
  const pictures = a.source_pictures && typeof a.source_pictures === "object" ? a.source_pictures : {};
  ui.grid.hidden = names.length === 0;
  ui.favoritesHint.hidden = names.length > 0;
  const signature = JSON.stringify([state.entity_id, allowed, a.source ?? null, names.map((name) => [name, pictures[name] ?? null])]);
  if (music.favorites === signature) return;
  music.favorites = signature;
  ui.grid.replaceChildren(...names.map((name) => {
    const current = a.source === name;
    const card = el("button", { type: "button", class: "music-fav", title: name, "aria-pressed": String(current), "aria-label": `Lancer ${name}` },
      coverBox("music-fav-art", pictures[name]),
      el("span", { class: "music-fav-name", text: name }));
    if (current) card.append(el("span", { class: "music-eq", "aria-hidden": "true" }, el("i"), el("i"), el("i")));
    card.disabled = !allowed;
    card.addEventListener("click", () => {
      const now = musicPlayer();
      if (now && canControl(now)) musicCommand(now, "select_source", { source: name });
    });
    return card;
  }));
}

// ---------------------------------------------------------------- Ouvrir, fermer

function openMusic(entityId) {
  if (entityId) chooseMusicPlayer(entityId);
  if (typeof agenda !== "undefined" && agenda.open) closeAgenda(false);
  if (typeof closeRoomTab === "function") closeRoomTab(false);
  if ($("#room").open) $("#room").close();
  music.open = true;
  $(".layout").hidden = true;
  $("#empty").hidden = true;
  musicUi.root.hidden = false;
  document.documentElement.dataset.music = "open";
  renderMusic();
  musicNav();
  clearInterval(music.timer);
  music.timer = setInterval(tickMusic, 1000);
  window.scrollTo({ top: 0 });
}

/** Retour à la maison. `render` : redessiner le tableau (sauf si l'appelant
 *  s'en charge). */
function closeMusic(render = true) {
  if (!music.open) return;
  music.open = false;
  clearInterval(music.timer);
  music.timer = null;
  music.seeking = false;
  musicUi.root.hidden = true;
  $(".layout").hidden = false;
  delete document.documentElement.dataset.music;
  musicNav();
  if (render) renderAll();
}

/** Le bouton de la barre du bas et l'onglet du tableau : seulement s'il y a
 *  une enceinte. */
function musicNav() {
  const any = musicPlayers().length > 0;
  const dock = $("#dock-music");
  if (dock.hidden === any) {
    dock.hidden = !any;
    const visible = $(".dock").querySelectorAll(":scope > :not([hidden])").length;
    $(".dock").style.gridTemplateColumns = `repeat(${visible}, minmax(0, 1fr))`;
  }
  dock.classList.toggle("active", music.open);
  const elsewhere = (typeof agenda !== "undefined" && agenda.open) || (typeof roomTab !== "undefined" && roomTab.open);
  $("#dock-home").classList.toggle("active", !music.open && !elsewhere);
  if (typeof renderTabs === "function" && typeof house !== "undefined" && house.pages?.length) renderTabs();
}

/** Une entité a changé (app.js) : la page suit, si c'est une enceinte. */
function musicChanged(entityId, oldState, newState) {
  if (!entityId.startsWith("media_player.")) return;
  // Une enceinte apparue ou disparue : le bouton et l'onglet.
  if (!oldState || !newState) musicNav();
  if (music.open) renderMusic();
}

/** Sur la tuile de l'enceinte (tiles.js) : la pochette et le titre en petit ;
 *  un toucher ouvre le lecteur sur cette enceinte. */
function mediaNowPlaying(state) {
  const a = state.attributes;
  const active = state.state === "playing" || state.state === "paused";
  const text = (value) => (typeof value === "string" && value.trim() !== "" ? value.trim() : "");
  const title = active ? text(a.media_title) || text(a.media_channel) || text(a.source) : "";
  const artist = active ? text(a.media_artist) : "";
  const button = el("button", { type: "button", class: "media-now", "aria-label": `Ouvrir le lecteur de ${nameOf(state)}` },
    coverBox("media-now-art", active ? a.entity_picture : null),
    el("span", { class: "media-now-text" },
      el("b", { text: title || "Lecteur de musique" }),
      el("span", { text: artist || (title ? "" : "Pochette, favoris, aléatoire…") })),
    svgIcon(["M9 6l6 6-6 6"]));
  button.addEventListener("click", () => openMusic(state.entity_id));
  return button;
}

/** Toutes les entités viennent d'arriver (app.js, à la connexion). */
function musicLoaded() {
  musicNav();
  if (music.open) renderMusic();
}

$("#dock-music").addEventListener("click", () => openMusic());
$("#dock-home").addEventListener("click", () => closeMusic());
// L'onglet de la page revient au premier plan : la position est recalculée.
document.addEventListener("visibilitychange", () => {
  if (music.open && !document.hidden) tickMusic();
});
