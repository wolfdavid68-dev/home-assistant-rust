// Accueil v4 : les autres incrustations de la maquette (« b », « c », « d »).
//
// Musique, météo, Tempo, agenda (et un nouveau rendez-vous), annonce, la
// cloche, Assist, qui est là, le lave-linge (en touchant sa carte sur le
// dessin), l'eau chaude et la sécurité. Toutes passent par le
// cadre commun (hubV4Overlay, hub-v4.js) ; chacune a un lien pour aller plus
// loin, vers l'écran complet d'avant.
//
// Les boutons de l'accueil v3 sont pris en v4 par un seul écouteur, en
// capture sur le document : le geste d'avant ne part pas. Hors v4, rien ne
// change. Styles en classes Tailwind ; pas d'innerHTML. Chargé après hub-v4.js.
"use strict";

const HUB_V4_ROW = "flex items-center gap-3 min-h-12 px-3 py-2 rounded-[16px] bg-voile/6";
const HUB_V4_ICON_BTN = "grid place-items-center size-11 shrink-0 p-0 rounded-full border-0 bg-voile/8 text-inherit "
  + "cursor-pointer transition-colors hover:bg-voile/14 disabled:opacity-35 disabled:cursor-default [&_svg]:size-5 "
  + HUB_V4_FOCUS;
const HUB_V4_INPUT = "w-full min-h-12 px-4 rounded-[16px] border-0 bg-voile/8 text-courant text-inherit "
  + "placeholder:text-v-mute/80 [color-scheme:var(--v4-scheme)] " + HUB_V4_FOCUS;
const HUB_V4_CHIP = "min-h-10 px-3 rounded-full border-0 bg-voile/8 text-petit text-inherit cursor-pointer "
  + "transition-colors hover:bg-voile/14 " + HUB_V4_FOCUS;
// Un « + » et une coche, en traits (comme ICONS).
const HUB_V4_MORE_ICONS = { plus: ["M12 5v14", "M5 12h14"] };

/** Les deux chiffres d'une heure : « 07:05 ». */
function hubV4Clock(date) {
  if (typeof date?.getTime !== "function" || Number.isNaN(date.getTime())) return "";
  return `${String(date.getHours()).padStart(2, "0")}:${String(date.getMinutes()).padStart(2, "0")}`;
}

/** « 42 min », « 1 h 05 ». */
function hubV4Minutes(minutes) {
  const m = Math.max(0, Math.round(minutes));
  return m < 60 ? `${m} min` : `${Math.floor(m / 60)} h ${String(m % 60).padStart(2, "0")}`;
}

/** Un nombre lisible, sans dépendre des formateurs du tableau. */
function hubV4Num(value) {
  return String(Math.round(value * 10) / 10).replace(".", ",");
}

/** La v4 peut-elle ouvrir une incrustation maintenant ? */
function hubV4Can() {
  return hubV4On() && hub.on && !hub.away;
}

// ---------------------------------------------------------------- Musique

/** Le titre et l'artiste d'une enceinte qui joue (ou en pause). */
function hubV4Track(state) {
  const a = state?.attributes ?? {};
  const active = state?.state === "playing" || state?.state === "paused";
  const text = (value) => (typeof value === "string" ? value.trim() : "");
  if (!active) return { title: "Pas de musique", artist: "" };
  return {
    title: text(a.media_title) || text(a.media_channel) || text(a.source) || "Musique",
    artist: text(a.media_artist) || text(a.media_album_name),
  };
}

/** Le volume (0 à 1) en pour cent, ou null. */
function hubV4Volume(state) {
  const level = Number(state?.attributes?.volume_level);
  if (state?.attributes?.volume_level == null || !Number.isFinite(level)) return null;
  return Math.max(0, Math.min(100, Math.round(level * 100)));
}

/** Le lecteur : pochette, morceau, avancée, précédent / lecture / suivant, volume, enceintes. */
function hubV4Music() {
  if (!hubV4Can() || typeof musicPlayers !== "function") return false;
  const first = musicPlayer();
  if (!first) return false;
  let id = first.entity_id;
  const get = () => entities.get(id);
  const sub = el("span", { class: `text-petit ${HUB_V4_MUTE} truncate` });
  const img = el("img", { alt: "", decoding: "async", draggable: "false", class: "absolute inset-0 size-full object-cover" });
  const art = el("div", {
    class: "relative grid place-items-center h-40 rounded-[20px] overflow-hidden "
      + "bg-[radial-gradient(circle_at_30%_20%,rgb(185_163_255/0.45),rgb(16_18_42/0.6))] text-v-violet [&>svg]:size-14 [&>svg]:opacity-60",
    "aria-hidden": "true",
  }, svgIcon(ICONS.music), img);
  const title = el("b", { class: "text-moyen font-semibold truncate" });
  const artist = el("span", { class: `text-petit ${HUB_V4_MUTE} truncate` });
  const progress = el("div", { class: "relative h-1.5 rounded-full bg-voile/10 overflow-hidden", role: "img" },
    el("span", { class: "absolute inset-y-0 left-0 w-(--p) rounded-full bg-v-violet" }));
  const at = el("span");
  const length = el("span");
  const times = el("p", { class: `m-0 flex justify-between text-legende ${HUB_V4_MUTE}` }, at, length);
  const command = (service, data) => {
    const now = get();
    if (!now || !hubV4Ready(now)) return;
    musicCommand(now, service, data);
  };
  const prev = el("button", { type: "button", class: HUB_V4_ICON_BTN, "aria-label": "Précédent" }, svgIcon(ICONS.previous));
  const next = el("button", { type: "button", class: HUB_V4_ICON_BTN, "aria-label": "Suivant" }, svgIcon(ICONS.next));
  const play = el("button", {
    type: "button", class: "grid place-items-center size-16 p-0 rounded-full border-0 bg-[#b9a3ff] text-[#170f33] cursor-pointer "
      + `transition-colors hover:bg-[#cbbaff] disabled:opacity-40 disabled:cursor-default [&_svg]:size-7 ${HUB_V4_FOCUS}`,
  });
  prev.addEventListener("click", () => command("media_previous_track"));
  next.addEventListener("click", () => command("media_next_track"));
  play.addEventListener("click", () => command(get()?.state === "playing" ? "media_pause" : "media_play"));
  const controls = el("div", { class: "flex items-center justify-center gap-5" }, prev, play, next);

  let hold = 0;
  const volumeValue = el("b", { class: "relative text-corps font-semibold" });
  const range = el("input", { type: "range", min: "0", max: "100", step: "1", "aria-label": "Volume", class: HUB_V4_RANGE });
  const volume = el("label", {
    class: `relative flex items-center justify-between h-12 px-4 rounded-[16px] bg-voile/8 overflow-hidden ${HUB_V4_TRACK}`,
  },
  el("span", {
    class: "absolute inset-y-0 left-0 w-(--v) bg-linear-to-r from-v-violet/25 to-v-violet/70 "
      + "transition-[width] motion-reduce:transition-none",
    "aria-hidden": "true",
  }),
  el("span", { class: "relative flex items-center gap-2 text-corps font-medium [&_svg]:size-4" }, svgIcon(ICONS.speaker), "Volume"),
  volumeValue, range);
  const showVolume = (percent) => {
    volume.style.setProperty("--v", `${percent}%`);
    setText(volumeValue, `${percent} %`);
  };
  range.addEventListener("input", () => {
    hold = Date.now() + HUB_V4_HOLD;
    showVolume(Number(range.value));
  });
  range.addEventListener("change", () => {
    hold = Date.now() + HUB_V4_HOLD;
    command("volume_set", { volume_level: Number(range.value) / 100 });
  });

  const speakers = el("div", { class: "flex flex-col gap-1.5", role: "group", "aria-label": "Enceintes" });
  // Redessinées seulement si quelque chose change : refresh tourne chaque
  // seconde, et un bouton refait sous le doigt perd le toucher.
  let speakersMark = "";
  const paintSpeakers = () => {
    const list = musicPlayers();
    speakers.hidden = list.length < 2;
    const mark = JSON.stringify([id, ...list.map((player) => [player.entity_id, player.state, nameOf(player)])]);
    if (mark === speakersMark) return;
    speakersMark = mark;
    swap(speakers, ...list.map((player) => {
      const playing = player.state === "playing";
      const node = el("button", {
        type: "button", "aria-pressed": String(player.entity_id === id),
        class: "flex items-center gap-2.5 min-h-11 px-3 rounded-[14px] border-0 bg-voile/6 text-left text-corps text-inherit "
          + "cursor-pointer transition-colors hover:bg-voile/10 aria-pressed:bg-v-violet/20 " + HUB_V4_FOCUS,
      },
      el("span", { class: "size-2 shrink-0 rounded-full bg-voile/25 data-on:bg-v-violet", ...(playing ? { "data-on": "" } : {}) }),
      el("span", { class: "flex-1 truncate", text: nameOf(player) }),
      el("span", { class: `text-legende ${HUB_V4_MUTE}`, text: playing ? "en cours" : player.state === "paused" ? "en pause" : "" }));
      node.addEventListener("click", () => {
        id = player.entity_id;
        chooseMusicPlayer(id);
        refresh();
      });
      return node;
    }));
  };

  const refresh = () => {
    const now = get();
    if (!now) {
      hubV4Close();
      return;
    }
    const track = hubV4Track(now);
    const ready = hubV4Ready(now);
    setText(sub, nameOf(now));
    setText(title, track.title);
    setText(artist, track.artist);
    artist.hidden = track.artist === "";
    const active = now.state === "playing" || now.state === "paused";
    showCover(img, active ? now.attributes.entity_picture : "");
    const { position, duration } = musicPosition(now);
    const known = position !== null && duration !== null;
    progress.hidden = !known;
    times.hidden = !known;
    if (known) {
      progress.style.setProperty("--p", `${Math.min(100, (position / duration) * 100)}%`);
      setAttr(progress, "aria-label", `${musicTime(position)} sur ${musicTime(duration)}`);
      setText(at, musicTime(position));
      setText(length, musicTime(duration));
    }
    const playing = now.state === "playing";
    setAttr(play, "aria-label", playing ? "Pause" : "Lecture");
    if (play.dataset.icon !== String(playing)) {
      play.dataset.icon = String(playing);
      play.replaceChildren(svgIcon(playing ? ICONS.pause : ICONS.play));
    }
    for (const node of [prev, play, next]) node.disabled = !ready;
    prev.disabled = !ready || !active;
    next.disabled = !ready || !active;
    const percent = hubV4Volume(now);
    volume.hidden = percent === null;
    range.disabled = !ready;
    if (percent !== null && Date.now() >= hold) {
      range.value = String(percent);
      showVolume(percent);
    }
    paintSpeakers();
  };
  // L'avancée du morceau bouge toute seule : une fois par seconde.
  const timer = setInterval(() => {
    if (hubV4.refresh === refresh) refresh();
  }, 1000);
  hubV4Overlay({
    icon: ICONS.music,
    tone: "violet",
    title: "Musique",
    sub,
    body: [art, el("p", { class: "m-0 flex flex-col px-1" }, title, artist), progress, times, controls, volume, speakers],
    more: { label: "Tout le lecteur", run: () => { leaveHub(); openMusic(id); } },
    refresh,
    onClose: () => clearInterval(timer),
  });
  return true;
}

// ---------------------------------------------------------------- Météo

/** « vent faible » (km/h), ou "". */
function hubV4Wind(speed) {
  if (typeof speed !== "number" || !Number.isFinite(speed)) return "";
  if (speed < 12) return "vent faible";
  if (speed < 30) return "vent modéré";
  if (speed < 60) return "vent fort";
  return "vent violent";
}

/**
 * Les jours qui viennent (pas aujourd'hui), au plus quatre : min, max, et
 * où poser leur barre sur l'échelle commune (`from`, `to` de 0 à 1).
 */
function hubV4WeatherDays(forecast) {
  const ok = (v) => typeof v === "number" && Number.isFinite(v);
  const days = (Array.isArray(forecast) ? forecast : [])
    .filter((day) => typeof day?.datetime === "string" && !Number.isNaN(Date.parse(day.datetime)))
    .slice(1, 5)
    .filter((day) => ok(day.temperature));
  if (days.length === 0) return [];
  const lows = days.map((day) => (ok(day.templow) ? day.templow : day.temperature));
  const bottom = Math.min(...lows);
  const top = Math.max(...days.map((day) => day.temperature));
  const span = top - bottom || 1;
  return days.map((day, i) => ({
    datetime: day.datetime,
    condition: day.condition,
    low: Math.round(lows[i]),
    high: Math.round(day.temperature),
    from: (lows[i] - bottom) / span,
    to: (day.temperature - bottom) / span,
    rain: ok(day.precipitation_probability) && day.precipitation_probability >= 30 ? Math.round(day.precipitation_probability) : null,
  }));
}

/** La météo : maintenant, puis quatre jours, barres min / max et pluie possible. */
function hubV4Weather() {
  if (!hubV4Can()) return false;
  const get = () => [...entities.values()].find((s) => domainOf(s) === "weather");
  if (!get()) return false;
  const icon = el("span", { class: "text-v-sky [&_svg]:size-12", "aria-hidden": "true" });
  const temp = el("b", { class: "font-chiffres text-affiche-l font-extralight leading-none tracking-[-0.03em]" });
  const label = el("b", { class: "text-courant font-semibold" });
  const feel = el("span", { class: `text-petit ${HUB_V4_MUTE}` });
  const now = el("div", { class: "flex items-center gap-3 px-1" }, icon, temp, el("p", { class: "m-0 flex flex-col" }, label, feel));
  const daysTitle = el("b", { class: HUB_V4_LABEL, text: "4 jours" });
  const list = el("ul", { class: "m-0 p-0 list-none flex flex-col gap-1" });
  const refresh = () => {
    const weather = get();
    if (!weather) {
      hubV4Close();
      return;
    }
    const a = weather.attributes;
    if (icon.dataset.condition !== weather.state) {
      icon.dataset.condition = weather.state;
      icon.replaceChildren(svgIcon(weatherIcon(weather.state)));
    }
    setText(temp, typeof a.temperature === "number" ? `${hubV4Num(a.temperature)}°` : "—");
    setText(label, lookup(WEATHER_LABELS, weather.state)?.[0] ?? "");
    const parts = [];
    if (typeof a.apparent_temperature === "number") parts.push(`ressenti ${Math.round(a.apparent_temperature)}°`);
    const wind = hubV4Wind(a.wind_speed);
    if (wind) parts.push(wind);
    if (typeof a.humidity === "number") parts.push(`humidité ${Math.round(a.humidity)} %`);
    setText(feel, parts.join(" · "));
    const days = hubV4WeatherDays(a.forecast);
    daysTitle.hidden = days.length === 0;
    swap(list, ...days.flatMap((day) => {
      const bar = el("span", {
        class: "absolute inset-y-0 left-(--from) right-(--to) rounded-full bg-linear-to-r from-[#9ad8ff] to-[#ffbf5e]",
      });
      bar.style.setProperty("--from", `${Math.round(day.from * 100)}%`);
      bar.style.setProperty("--to", `${Math.round((1 - day.to) * 100)}%`);
      const row = el("li", { class: "grid grid-cols-[40px_24px_32px_1fr_32px] items-center gap-2 min-h-10 px-2 text-corps [&_svg]:size-5" },
        el("span", { class: "first-letter:uppercase", text: dayName.format(new Date(`${day.datetime.slice(0, 10)}T12:00`)) }),
        svgIcon(weatherIcon(day.condition)),
        el("span", { class: `text-right ${HUB_V4_MUTE}`, text: `${day.low}°` }),
        el("span", { class: "relative h-1.5 rounded-full bg-voile/10" }, bar),
        el("span", { class: "font-semibold", text: `${day.high}°` }));
      if (day.rain === null) return [row];
      return [row, el("li", { class: "pl-[72px] -mt-1 text-legende text-v-sky", text: `Pluie possible · ${day.rain} %` })];
    }));
  };
  hubV4Overlay({
    icon: ICONS.partly,
    tone: "sky",
    title: "Météo",
    sub: el("span", { class: `text-petit ${HUB_V4_MUTE}`, text: "Maintenant et 4 jours" }),
    body: [now, daysTitle, list],
    spot: hubV4Rect(document.querySelector("#hub-weather")),
    refresh,
  });
  return true;
}

// ---------------------------------------------------------------- Tempo

/** La couleur d'un jour Tempo : son ton (blue, white, red, unknown) et son mot. */
function hubV4TempoDay(state) {
  if (!state) return { tone: "unknown", text: "—" };
  const known = { Bleu: "blue", Blanc: "white", Rouge: "red" };
  if (Object.hasOwn(known, state.state)) return { tone: known[state.state], text: state.state };
  if (state.state === "unavailable") return { tone: "unknown", text: "Indisponible" };
  return { tone: "unknown", text: "À venir" };
}

/** Tempo : aujourd'hui, demain, et quand arrive la couleur de demain. */
function hubV4Tempo() {
  if (!hubV4Can() || !entities.get(TEMPO_TODAY)) return false;
  const day = (label) => {
    const dot = el("span", {
      class: "size-3.5 shrink-0 rounded-full border-2 border-dashed border-voile/40 "
        + "data-[tone=blue]:border-0 data-[tone=blue]:bg-[#5b8cff] data-[tone=white]:border-0 data-[tone=white]:bg-white data-[tone=white]:ring-1 data-[tone=white]:ring-voile/30 "
        + "data-[tone=red]:border-0 data-[tone=red]:bg-[#ff6b6b]",
    });
    const word = el("b", { class: "text-grand font-semibold" });
    const node = el("div", { class: "flex flex-col gap-1 p-3 rounded-[18px] bg-voile/6" },
      el("span", { class: `text-petit ${HUB_V4_MUTE}`, text: label }), el("span", { class: "flex items-center gap-2" }, dot, word));
    return { node, dot, word };
  };
  const today = day("Aujourd'hui");
  const tomorrow = day("Demain");
  const note = el("p", { class: `m-0 px-1 text-petit ${HUB_V4_MUTE}` });
  const paint = (part, state) => {
    const { tone, text } = hubV4TempoDay(state);
    part.dot.dataset.tone = tone;
    setText(part.word, text);
  };
  hubV4Overlay({
    icon: ICONS.bolt,
    title: "Tempo",
    sub: el("span", { class: `text-petit ${HUB_V4_MUTE}`, text: "La couleur du jour EDF" }),
    body: [el("div", { class: "grid grid-cols-2 gap-2" }, today.node, tomorrow.node), note],
    spot: hubV4Rect(document.querySelector("#hub-v3-tempo")),
    refresh: () => {
      if (!entities.get(TEMPO_TODAY)) {
        hubV4Close();
        return;
      }
      paint(today, entities.get(TEMPO_TODAY));
      const next = entities.get(TEMPO_TOMORROW);
      paint(tomorrow, next);
      const waiting = hubV4TempoDay(next).tone === "unknown";
      setText(note, waiting ? "La couleur de demain arrive vers 11 h." : "Un jour Tempo va de 6 h à 6 h.");
    },
  });
  return true;
}

// ---------------------------------------------------------------- Agenda

/**
 * Les rendez-vous groupés par jour, sur `count` jours à partir d'aujourd'hui.
 * Aujourd'hui est toujours là (même vide) ; les autres jours, seulement s'il
 * y a quelque chose.
 */
function hubV4AgendaDays(events, now, count = 7) {
  const start = new Date(now.getFullYear(), now.getMonth(), now.getDate());
  const days = [];
  for (let i = 0; i < count; i++) {
    const from = new Date(start.getFullYear(), start.getMonth(), start.getDate() + i);
    const to = new Date(start.getFullYear(), start.getMonth(), start.getDate() + i + 1);
    // Un rendez-vous à l'heure qui déborde sur le lendemain n'y revient pas.
    const list = events.filter((e) => e.startDate < to && e.endDate > from && (i === 0 || e.startDate >= from || e.allDay));
    if (i === 0 || list.length > 0) days.push({ date: from, events: list });
  }
  return days;
}

/** L'agenda : les jours qui viennent ; « + » pour un rendez-vous dans l'agenda de la maison. */
function hubV4Agenda() {
  if (!hubV4Can() || typeof agendaCalendars !== "function" || agendaCalendars().length === 0) return false;
  const list = el("ul", { class: "m-0 p-0 list-none flex flex-col gap-1.5" });
  const add = el("button", { type: "button", class: `${HUB_V4_BTN} justify-start` }, svgIcon(HUB_V4_MORE_ICONS.plus), "Nouveau rendez-vous");
  add.addEventListener("click", () => hubV4NewEvent());
  const dateDay = new Intl.DateTimeFormat("fr", { weekday: "short" });
  const refresh = () => {
    const calendars = agendaCalendars();
    if (calendars.length === 0) {
      hubV4Close();
      return;
    }
    const now = new Date();
    const start = dayStart(now);
    const events = hubMergeSame(agendaEvents(start, addDays(start, 7), calendars));
    const days = hubV4AgendaDays(events, now);
    swap(list, ...days.map(({ date, events: dayEvents }) => {
      const rows = dayEvents.length === 0
        ? [el("p", { class: `m-0 py-2 text-corps ${HUB_V4_MUTE}`, text: "Rien aujourd'hui" })]
        : dayEvents.map((e) => {
          const row = el("button", {
            type: "button",
            class: "flex items-center gap-2.5 w-full min-h-11 px-2 rounded-[12px] border-0 bg-transparent text-left text-inherit "
              + `cursor-pointer transition-colors hover:bg-voile/8 ${HUB_V4_FOCUS}`,
          },
          el("span", { class: "w-1 self-stretch my-1.5 shrink-0 rounded-full bg-(--ev)" }),
          el("span", { class: "flex flex-col min-w-0" },
            el("b", { class: "text-corps font-semibold truncate", text: String(e.summary ?? "") }),
            el("span", {
              class: `text-legende ${HUB_V4_MUTE} truncate`,
              text: [e.allDay ? "toute la journée" : hubV4Clock(e.startDate), e.location || ""].filter(Boolean).join(" · "),
            })));
          row.style.setProperty("--ev", e.color);
          row.addEventListener("click", () => {
            hubV4Close();
            openEventDetails(e);
          });
          return row;
        });
      return el("li", { class: "grid grid-cols-[44px_1fr] gap-2 items-start p-1.5 rounded-[16px] bg-voile/6" },
        el("span", { class: `flex flex-col items-center pt-1.5 text-legende ${HUB_V4_MUTE}` },
          dateDay.format(date), el("b", { class: "text-grand font-semibold text-v-ink leading-tight", text: String(date.getDate()) })),
        el("div", { class: "flex flex-col min-w-0" }, ...rows));
    }));
    add.hidden = !calendars.some((c) => c.id === "calendar.maison" && !c.readOnly && canControl(c.state));
  };
  hubV4Overlay({
    icon: ICONS.calendar,
    tone: "green",
    title: "Agenda",
    sub: el("span", { class: `text-petit ${HUB_V4_MUTE}`, text: "Les jours qui viennent" }),
    body: [list, add],
    more: { label: "Tout l'agenda", run: () => { leaveHub(); openAgenda(); } },
    refresh,
  });
  return true;
}

/** « 2026-11-05 » pour un champ date. */
function hubV4DayValue(date) {
  return `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, "0")}-${String(date.getDate()).padStart(2, "0")}`;
}

/** Ce qu'envoie « Ajouter » (calendar.create_event), ou null s'il manque le titre ou le jour. */
function hubV4EventData(title, day, time) {
  const summary = String(title ?? "").trim();
  if (summary === "" || !/^\d{4}-\d{2}-\d{2}$/.test(day ?? "")) return null;
  const data = { entity_id: "calendar.maison", summary };
  if (/^\d{2}:\d{2}$/.test(time ?? "")) {
    data.start_date_time = `${day} ${time}`;
    // Une heure par défaut.
    const [h, m] = time.split(":").map(Number);
    if (h < 23) data.end_date_time = `${day} ${String(h + 1).padStart(2, "0")}:${String(m).padStart(2, "0")}`;
  } else {
    data.start_date = day;
  }
  return data;
}

/** Nouveau rendez-vous : un titre, un jour, une heure (vide : toute la journée). */
function hubV4NewEvent() {
  if (!hubV4Can()) return false;
  const field = (label, input) => el("label", { class: "flex flex-col gap-1" },
    el("span", { class: `px-1 text-legende ${HUB_V4_MUTE}`, text: label }), input);
  const title = el("input", { type: "text", class: HUB_V4_INPUT, placeholder: "Dentiste, anniversaire…", maxlength: "120", required: "" });
  const day = el("input", { type: "date", class: HUB_V4_INPUT, required: "" });
  day.value = hubV4DayValue(new Date());
  const time = el("input", { type: "time", class: HUB_V4_INPUT });
  const cancel = el("button", { type: "button", class: `${HUB_V4_BTN} flex-1`, text: "Annuler" });
  const save = el("button", { type: "submit", class: `${HUB_V4_MAIN} flex-1`, text: "Ajouter" });
  const note = el("p", { class: `m-0 px-1 text-legende ${HUB_V4_MUTE}`, text: "Sans heure, il prend toute la journée." });
  const form = el("form", { class: "flex flex-col gap-2.5" },
    field("Titre", title), el("div", { class: "grid grid-cols-2 gap-2" }, field("Jour", day), field("Heure", time)), note,
    el("div", { class: "flex gap-2" }, cancel, save));
  cancel.addEventListener("click", () => hubV4Agenda() || hubV4Close());
  form.addEventListener("submit", async (event) => {
    event.preventDefault();
    const data = hubV4EventData(title.value, day.value, time.value);
    if (!data) return;
    save.disabled = true;
    // Le rendez-vous n'est pas parti : la fenêtre reste ouverte, pour réessayer.
    const sent = await callService("calendar", "create_event", data);
    save.disabled = false;
    if (sent) hubV4Agenda() || hubV4Close();
  });
  hubV4Overlay({
    icon: ICONS.calendar,
    tone: "green",
    title: "Nouveau rendez-vous",
    sub: el("span", { class: `text-petit ${HUB_V4_MUTE}`, text: "Dans l'agenda de la maison" }),
    body: [form],
    refresh: () => {},
  });
  title.focus();
  return true;
}

// ---------------------------------------------------------------- Annonce

const HUB_V4_SAY_IDEAS = ["À table !", "On part dans 5 min", "Extinction des feux"];

/** L'annonce : une phrase, des idées, où la dire. */
function hubV4Announce() {
  if (!hubV4Can() || hubAnnouncer() === null) return false;
  const input = el("input", { type: "text", class: HUB_V4_INPUT, placeholder: "Le dîner est prêt", maxlength: "200", "aria-label": "Message" });
  const ideas = el("div", { class: "flex flex-wrap gap-1.5", role: "group", "aria-label": "Idées" },
    ...HUB_V4_SAY_IDEAS.map((text) => {
      const node = el("button", { type: "button", class: HUB_V4_CHIP, text });
      node.addEventListener("click", () => {
        input.value = text;
        input.focus();
      });
      return node;
    }));
  // Où : partout, ou une seule enceinte.
  let where = "";
  const whereLabel = el("b", { class: HUB_V4_LABEL, text: "Où" });
  const whereBox = el("div", { class: "flex flex-wrap gap-1 p-1 rounded-[16px] bg-voile/6", role: "group", "aria-label": "Où" });
  const send = el("button", { type: "submit", class: HUB_V4_MAIN }, svgIcon(ICONS.megaphone), "Annoncer");
  const form = el("form", { class: "flex flex-col gap-2.5" }, input, ideas, whereLabel, whereBox, send);
  form.addEventListener("submit", async (event) => {
    event.preventDefault();
    const announcer = hubAnnouncer();
    const message = input.value.trim();
    if (!announcer || message === "") return;
    const speakers = where && announcer.speakers.includes(where) ? [where] : announcer.speakers;
    send.disabled = true;
    const sent = await callService("tts", "speak", { entity_id: announcer.tts.entity_id, media_player_entity_id: speakers, message });
    send.disabled = false;
    // L'annonce n'est pas partie : la fenêtre reste ouverte, le texte aussi.
    if (sent) hubV4Close();
  });
  const refresh = () => {
    const announcer = hubAnnouncer();
    if (!announcer) {
      hubV4Close();
      return;
    }
    if (where && !announcer.speakers.includes(where)) where = "";
    const choices = [["", "Partout"], ...announcer.speakers.map((id) => [id, nameOf(entities.get(id))])];
    whereLabel.hidden = announcer.speakers.length < 2;
    whereBox.hidden = announcer.speakers.length < 2;
    swap(whereBox, ...choices.map(([id, text]) => {
      const node = el("button", {
        type: "button", "aria-pressed": String(id === where),
        class: "flex-1 min-h-10 px-3 rounded-[12px] border-0 bg-transparent text-petit font-medium text-inherit cursor-pointer "
          + `transition-colors hover:bg-voile/8 aria-pressed:bg-[#9ad8ff] aria-pressed:text-[#0b1a2b] ${HUB_V4_FOCUS}`,
        text,
      });
      node.addEventListener("click", () => {
        where = id;
        refresh();
      });
      return node;
    }));
  };
  hubV4Overlay({
    icon: ICONS.megaphone,
    tone: "sky",
    title: "Annonce",
    sub: el("span", { class: `text-petit ${HUB_V4_MUTE}`, text: "La maison le dit à voix haute" }),
    body: [form],
    refresh,
  });
  input.focus();
  return true;
}

// ---------------------------------------------------------------- Notifications

/** Les avis à montrer : à valider, puis à réparer (erreurs d'abord), puis les autres ; les plus récents en tête. */
function hubV4Notices(states) {
  const rank = (s) => {
    if (s.attributes.kind === "approval") return 0;
    if (s.attributes.kind === "repair") return s.attributes.severity === "error" ? 1 : 2;
    return 3;
  };
  return states
    .filter((s) => s.state === "notifying")
    .sort((a, b) => rank(a) - rank(b) || String(b.attributes.created_at).localeCompare(String(a.attributes.created_at)));
}

/** La cloche : les avis actifs ; un ordre à valider s'ouvre dans sa propre fenêtre. */
function hubV4Bell() {
  if (!hubV4Can() || typeof noticeStates !== "function") return false;
  const sub = el("span", { class: `text-petit ${HUB_V4_MUTE}` });
  const list = el("ul", { class: "m-0 p-0 list-none flex flex-col gap-1.5" });
  const refresh = () => {
    const active = hubV4Notices(noticeStates());
    setText(sub, active.length === 0 ? "Rien à signaler" : active.length === 1 ? "1 avis" : `${active.length} avis`);
    swap(list, ...(active.length === 0
      ? [el("li", { class: `px-1 py-3 text-corps ${HUB_V4_MUTE}`, text: "Rien à signaler." })]
      : active.map((state) => {
        const a = state.attributes;
        const approval = a.kind === "approval";
        const urgent = approval || (a.kind === "repair" && a.severity === "error");
        const created = new Date(a.created_at);
        const icon = el("span", {
          class: "grid place-items-center size-9 shrink-0 rounded-full bg-voile/8 [&_svg]:size-[18px] "
            + "data-urgent:bg-v-amber/20 data-urgent:text-v-amber",
          "aria-hidden": "true", ...(urgent ? { "data-urgent": "" } : {}),
        }, svgIcon(approval ? ICONS.shield : a.kind === "repair" ? ICONS.gauge : ICONS.bell));
        const text = el("p", { class: "m-0 flex flex-col flex-1 min-w-0" },
          el("b", { class: "text-corps font-semibold truncate", text: String(a.title ?? "Notification") }),
          el("span", { class: `text-legende ${HUB_V4_MUTE} line-clamp-2`, text: String(a.message ?? "") }));
        const time = el("time", { class: `shrink-0 self-start pt-1 text-legende ${HUB_V4_MUTE}`, text: hubV4Clock(created) });
        if (approval) {
          const row = el("button", {
            type: "button", class: `${HUB_V4_ROW} w-full border-0 text-left text-inherit cursor-pointer hover:bg-voile/10 ${HUB_V4_FOCUS}`,
          }, icon, text, time);
          row.addEventListener("click", () => hubV4Approval());
          return el("li", {}, row);
        }
        const item = el("li", { class: HUB_V4_ROW }, icon, text, time);
        if (canControl(state)) {
          const dismiss = el("button", {
            type: "button", class: `${HUB_V4_ICON_BTN} size-9 bg-transparent`, "aria-label": `Retirer : ${String(a.title ?? "l'avis")}`,
          }, svgIcon(HUB_V4_ICONS.close));
          dismiss.addEventListener("click", () => {
            dismiss.disabled = true;
            callService("persistent_notification", "dismiss", { entity_id: state.entity_id });
          });
          item.append(dismiss);
        }
        return item;
      })));
  };
  hubV4Overlay({
    icon: ICONS.bell,
    title: "Notifications",
    sub,
    body: [list],
    more: { label: "Tous les avis", run: () => document.querySelector("#open-notices")?.click() },
    refresh,
  });
  return true;
}

// ---------------------------------------------------------------- Assist

/** Les bulles de la conversation (celles de la fenêtre Assist), les dernières seulement. */
function hubV4AssistLines(items, keep = 6) {
  return items
    .map((item) => ({ from: item.classList.contains("from-me") ? "me" : "home", text: item.textContent.trim(), error: item.classList.contains("is-error") }))
    .filter((line) => line.text !== "")
    .slice(-keep);
}

/** Assist : la conversation, des idées, et « Demande à la maison… ». */
function hubV4Assist() {
  const log = document.querySelector("#assist-log");
  if (!hubV4Can() || !log || typeof assistAsk !== "function") return false;
  const chat = el("div", { class: "flex flex-col gap-1.5 max-h-64 overflow-auto px-1", "aria-live": "polite" });
  const paint = () => {
    const lines = hubV4AssistLines([...log.children]);
    swap(chat, ...(lines.length === 0
      ? [el("p", { class: `m-0 py-2 text-corps ${HUB_V4_MUTE}`, text: "Demande en français : « éteins le salon », « quel temps demain ? »…" })]
      : lines.map((line) => el("p", {
        class: "m-0 max-w-[85%] px-3 py-2 rounded-[16px] text-corps leading-snug [overflow-wrap:anywhere] "
          + "data-[from=me]:self-end data-[from=me]:rounded-br-[4px] data-[from=me]:bg-[#9ad8ff] data-[from=me]:text-[#0b1a2b] "
          + "data-[from=home]:self-start data-[from=home]:rounded-bl-[4px] data-[from=home]:bg-voile/10 "
          + "data-error:shadow-[inset_3px_0_0_#ff6b6b]",
        "data-from": line.from, ...(line.error ? { "data-error": "" } : {}), text: line.text,
      }))));
    chat.lastElementChild?.scrollIntoView?.({ block: "end" });
  };
  const watch = new MutationObserver(paint);
  watch.observe(log, { childList: true });
  const examples = [...document.querySelectorAll("#assist-examples button")].map((b) => b.textContent.trim()).filter(Boolean).slice(0, 2);
  const input = el("input", { type: "text", class: `${HUB_V4_INPUT} flex-1`, placeholder: "Demande à la maison…", "aria-label": "Demande", maxlength: "300" });
  const send = el("button", { type: "submit", class: HUB_V4_ICON_BTN, "aria-label": "Envoyer" }, svgIcon(HUB_V4_ICONS.go));
  const ask = (text) => {
    const asked = text.trim();
    if (!asked) return;
    input.value = "";
    assistAsk(asked);
  };
  const form = el("form", { class: "flex items-center gap-2" }, input, send);
  form.addEventListener("submit", (event) => {
    event.preventDefault();
    ask(input.value);
  });
  const ideas = el("div", { class: "flex flex-wrap gap-1.5", role: "group", "aria-label": "Idées" },
    ...examples.map((text) => {
      const node = el("button", { type: "button", class: HUB_V4_CHIP, text });
      node.addEventListener("click", () => ask(text));
      return node;
    }));
  ideas.hidden = examples.length === 0;
  hubV4Overlay({
    icon: ICONS.mic,
    tone: "sky",
    title: "Assist",
    sub: el("span", { class: `text-petit ${HUB_V4_MUTE}`, text: "Demande à la maison" }),
    body: [chat, ideas, form],
    refresh: paint,
    onClose: () => watch.disconnect(),
  });
  input.focus();
  return true;
}

// ---------------------------------------------------------------- Qui est là

/** « à la maison depuis 18:40 », « absent depuis hier »… */
function hubV4Presence(state, now = new Date()) {
  const home = state.state === "home";
  const since = new Date(state.last_changed);
  const word = home ? "à la maison" : state.state === "not_home" ? "absent" : state.state === "unknown" ? "on ne sait pas" : state.state;
  if (Number.isNaN(since.getTime()) || state.state === "unknown") return word;
  const sameDay = since.toDateString() === now.toDateString();
  const yesterday = new Date(now.getFullYear(), now.getMonth(), now.getDate() - 1).toDateString() === since.toDateString();
  const when = sameDay ? hubV4Clock(since) : yesterday ? `hier ${hubV4Clock(since)}` : `le ${since.getDate()}/${since.getMonth() + 1}`;
  return `${word} depuis ${when}`;
}

/** Qui est là : chaque personne, à la maison ou non. Seulement s'il y en a. */
function hubV4People() {
  const persons = () => [...entities.values()].filter((s) => domainOf(s) === "person")
    .sort((a, b) => Number(b.state === "home") - Number(a.state === "home") || nameOf(a).localeCompare(nameOf(b), "fr"));
  if (!hubV4Can() || persons().length === 0) return false;
  const sub = el("span", { class: `text-petit ${HUB_V4_MUTE}` });
  const list = el("ul", { class: "m-0 p-0 list-none flex flex-col gap-1.5" });
  hubV4Overlay({
    icon: ICONS.person,
    tone: "green",
    title: "Qui est là",
    sub,
    body: [list],
    more: { label: "Mon compte", run: () => document.querySelector("#open-account")?.click() },
    refresh: () => {
      const people = persons();
      const home = people.filter((s) => s.state === "home").length;
      setText(sub, home === 0 ? "Personne à la maison" : home === people.length ? "Tout le monde est là" : `${home} sur ${people.length} à la maison`);
      swap(list, ...people.map((state) => {
        const name = nameOf(state);
        const avatar = el("span", {
          class: "relative grid place-items-center size-10 shrink-0 rounded-full bg-(--pc) text-moyen font-semibold text-white "
            + "data-away:bg-voile/12 data-away:text-v-ink2",
          "aria-hidden": "true", ...(state.state === "home" ? {} : { "data-away": "" }),
        }, name.charAt(0).toUpperCase(), el("span", {
          class: "absolute -right-0.5 -bottom-0.5 size-3 rounded-full border-2 border-scrim bg-v-ok "
            + "[[data-away]>&]:bg-v-mute/50",
        }));
        avatar.style.setProperty("--pc", personColor(name));
        return el("li", { class: HUB_V4_ROW }, avatar, el("p", { class: "m-0 flex flex-col min-w-0" },
          el("b", { class: "text-courant font-semibold truncate", text: name }),
          el("span", { class: `text-petit ${HUB_V4_MUTE}`, text: hubV4Presence(state) })));
      }));
    },
  });
  return true;
}

// ---------------------------------------------------------------- Lave-linge

/**
 * Ce que dit une machine (lave-linge…) : `phase` (running, paused, done ou
 * "" si elle n'est pas à montrer), un titre, une ligne et l'avancée (0 à 100,
 * null si inconnue).
 */
function hubV4MachineView(state, now = Date.now(), shown = 3_600_000) {
  const a = state.attributes;
  const name = nameOf(state);
  const cycle = typeof a.cycle === "string" && a.cycle ? a.cycle.toLowerCase() : "";
  const left = typeof a.reste_minutes === "number" && Number.isFinite(a.reste_minutes) ? a.reste_minutes : null;
  const progress = typeof a.progression === "number" && Number.isFinite(a.progression) ? Math.max(0, Math.min(100, a.progression)) : null;
  if (state.state === "Terminé") {
    const since = Date.parse(state.last_changed);
    if (!Number.isFinite(since) || now - since >= shown) return { phase: "", title: name, sub: "", progress: null };
    return { phase: "done", title: `${name} a fini`, sub: `à vider · depuis ${hubV4Clock(new Date(since))}`, progress: 100 };
  }
  if (state.state === "En marche" || state.state === "En pause") {
    const paused = state.state === "En pause";
    const parts = [];
    if (paused) parts.push("en pause");
    else if (typeof a.fin_heure === "string" && a.fin_heure) parts.push(`fin vers ${a.fin_heure}`);
    if (left !== null) parts.push(`encore ${hubV4Minutes(left)}`);
    return { phase: paused ? "paused" : "running", title: cycle ? `${name} · ${cycle}` : name, sub: parts.join(" · "), progress };
  }
  return { phase: "", title: name, sub: "", progress: null };
}

/** Une machine qui tourne : le programme, l'heure de fin, ce qui reste. */
function hubV4Machine(entityId) {
  if (!hubV4Can() || !entities.get(entityId)) return false;
  const sub = el("span", { class: `text-petit ${HUB_V4_MUTE} truncate` });
  const end = el("b", { class: "font-chiffres text-affiche-l font-extralight leading-none tracking-[-0.03em]" });
  const left = el("span", { class: `text-petit ${HUB_V4_MUTE}` });
  const bar = el("div", { class: "relative h-2 rounded-full bg-voile/10 overflow-hidden" },
    el("span", { class: "absolute inset-y-0 left-0 w-(--mp) rounded-full bg-v-sky transition-[width] motion-reduce:transition-none" }));
  hubV4Overlay({
    icon: ICONS.washer,
    tone: "sky",
    title: nameOf(entities.get(entityId)),
    sub,
    body: [el("div", { class: "flex flex-col gap-1 px-1" }, end, left), bar],
    more: account.permissions?.history !== false ? { label: "L'historique", run: () => openHistory(entityId) } : null,
    refresh: () => {
      const state = entities.get(entityId);
      if (!state) {
        hubV4Close();
        return;
      }
      const a = state.attributes;
      const cycle = typeof a.cycle === "string" && a.cycle ? `${a.cycle.charAt(0).toUpperCase()}${a.cycle.slice(1)}` : "";
      setText(sub, [state.state, cycle].filter(Boolean).join(" · "));
      const view = hubV4MachineView(state);
      const done = state.state === "Terminé";
      setText(end, done ? "Fini" : typeof a.fin_heure === "string" && a.fin_heure ? a.fin_heure : "—");
      const minutes = typeof a.reste_minutes === "number" ? a.reste_minutes : null;
      setText(left, done ? view.sub || "à vider" : `fin prévue${minutes !== null ? ` · encore ${hubV4Minutes(minutes)}` : ""}`);
      bar.hidden = view.progress === null;
      bar.style.setProperty("--mp", `${view.progress ?? 0}%`);
    },
  });
  return true;
}

// ---------------------------------------------------------------- Eau chaude

/** Le remplissage du ballon (0 à 1) : la température entre 20° et la consigne max. */
function hubV4TankLevel(current, max = 65) {
  if (typeof current !== "number" || !Number.isFinite(current)) return null;
  const top = typeof max === "number" && Number.isFinite(max) && max > 20 ? max : 65;
  return Math.max(0, Math.min(1, (current - 20) / (top - 20)));
}

/** Ce que fait le ballon, quand il n'a pas de préréglage. */
const HUB_V4_WATER_STATES = { heat: "En chauffe", auto: "Automatique", heat_pump: "Pompe à chaleur", eco: "Éco", electric: "Électrique" };

/** L'eau chaude : le ballon, la consigne avec − et +, le mode. */
function hubV4Water() {
  const get = () => entities.get(HUB_V3_WATER);
  if (!hubV4Can() || !get()) return false;
  const fill = el("span", {
    class: "absolute inset-x-0 bottom-0 h-(--lv) bg-linear-to-t from-[#ff9a6b] to-[#ffbf5e] transition-[height] motion-reduce:transition-none",
  });
  const tank = el("div", {
    class: "relative w-16 h-28 shrink-0 rounded-[18px_18px_14px_14px] bg-voile/8 overflow-hidden shadow-[inset_0_0_0_1px_color-mix(in_srgb,var(--v3-voile)_12%,transparent)]",
    role: "img",
  }, fill);
  const temp = el("b", { class: "font-chiffres text-affiche-l font-extralight leading-none tracking-[-0.03em]" });
  const target = el("span", { class: `text-petit ${HUB_V4_MUTE}` });
  const mode = el("span", { class: `text-petit ${HUB_V4_MUTE}` });
  const nudge = (direction) => {
    const node = el("button", {
      type: "button", class: `${HUB_V4_ICON_BTN} text-titre font-light`,
      "aria-label": direction > 0 ? "Monter la consigne" : "Baisser la consigne", text: direction > 0 ? "+" : "−",
    });
    node.addEventListener("click", () => nudgeHeating(HUB_V3_WATER, direction));
    return node;
  };
  const minus = nudge(-1);
  const plus = nudge(1);
  const modeLabel = el("b", { class: HUB_V4_LABEL, text: "Mode" });
  const modes = el("div", { class: "grid grid-flow-col auto-cols-fr gap-1 p-1 rounded-[16px] bg-voile/6", role: "group", "aria-label": "Mode" });
  hubV4Overlay({
    icon: ICONS.flame,
    title: "Eau chaude",
    sub: el("span", { class: `text-petit ${HUB_V4_MUTE}`, text: "Ballon thermodynamique" }),
    body: [
      el("div", { class: "flex items-center gap-4 px-1" }, tank,
        el("div", { class: "flex flex-col gap-1 flex-1" }, temp, el("span", { class: `text-petit ${HUB_V4_MUTE}`, text: "dans le ballon" }), mode)),
      el("div", { class: "flex items-center justify-between gap-3 px-1" }, minus, target, plus),
      modeLabel, modes,
    ],
    refresh: () => {
      const state = get();
      if (!state) {
        hubV4Close();
        return;
      }
      const a = state.attributes;
      const current = typeof a.current_temperature === "number" ? a.current_temperature : null;
      setText(temp, current === null ? "—" : `${hubV4Num(current)}°`);
      const level = hubV4TankLevel(current, a.max_temp);
      fill.style.setProperty("--lv", `${Math.round((level ?? 0) * 100)}%`);
      setAttr(tank, "aria-label", current === null ? "Ballon" : `Ballon à ${hubV4Num(current)} degrés`);
      // Sans préréglage, heatMode rend l'état brut (« heat ») : on le dit en français.
      const said = heatMode(state);
      setText(mode, said === state.state ? HUB_V4_WATER_STATES[said] ?? said : said);
      const value = state.state !== "off" && state.state !== "unavailable" ? heatTarget(state) : null;
      setText(target, value === null ? "Pas de consigne" : `Consigne ${hubV4Num(value)}°`);
      const { step, min, max } = heatLimits(state);
      const ready = hubV4Ready(state);
      minus.disabled = !ready || value === null || state.state !== "heat" || value - step < min - 1e-9;
      plus.disabled = !ready || value === null || state.state !== "heat" || value + step > max + 1e-9;
      const presets = Array.isArray(a.preset_modes) ? a.preset_modes.filter((p) => typeof p === "string") : [];
      modeLabel.hidden = presets.length === 0;
      modes.hidden = presets.length === 0;
      swap(modes, ...presets.map((preset) => {
        const node = el("button", {
          type: "button", "aria-pressed": String(a.preset_mode === preset),
          class: "min-h-10 px-2 rounded-[12px] border-0 bg-transparent text-petit font-medium text-inherit cursor-pointer "
            + "transition-colors hover:bg-voile/8 aria-pressed:bg-[#ffbf5e] aria-pressed:text-[#1f1305] "
            + `disabled:opacity-40 disabled:cursor-default ${HUB_V4_FOCUS}`,
          text: lookup(PRESET_LABELS, preset) ?? `${preset.charAt(0).toUpperCase()}${preset.slice(1)}`,
        });
        node.disabled = !ready;
        node.addEventListener("click", () => {
          const now = get();
          if (!now || !hubV4Ready(now)) return;
          markPending(HUB_V3_WATER);
          callService("climate", "set_preset_mode", { entity_id: HUB_V3_WATER, preset_mode: preset });
        });
        return node;
      }));
    },
  });
  return true;
}

// ---------------------------------------------------------------- Sécurité

/** Les passages notés par le journal pour ces appareils, les plus récents d'abord. */
function hubV4SecurityLog(entries, ids, keep = 4) {
  const wanted = new Set(ids);
  return (Array.isArray(entries) ? entries : [])
    .filter((entry) => wanted.has(entry?.entity_id) && typeof entry.at === "string" && !Number.isNaN(Date.parse(entry.at)))
    .sort((a, b) => Date.parse(b.at) - Date.parse(a.at))
    .slice(0, keep);
}

/** Sécurité : chaque appareil et son état, puis les derniers passages (journal de 24 h). */
function hubV4Security() {
  const states = () => [...entities.values()].filter(hubV3IsSecurity).sort((a, b) => nameOf(a).localeCompare(nameOf(b), "fr"));
  if (!hubV4Can() || states().length === 0) return false;
  const sub = el("span", { class: `text-petit ${HUB_V4_MUTE}` });
  const list = el("ul", { class: "m-0 p-0 list-none flex flex-col gap-1.5" });
  const logLabel = el("b", { class: HUB_V4_LABEL, text: "Derniers passages" });
  const log = el("ul", { class: "m-0 p-0 list-none flex flex-col gap-1 px-1 text-petit" });
  logLabel.hidden = true;
  log.hidden = true;
  const allowed = account.permissions?.history !== false;
  if (allowed) {
    api("GET", "/api/logbook?hours=24").then(({ status, data }) => {
      if (status !== 200 || !log.isConnected) return;
      const entries = hubV4SecurityLog(data?.entries, states().map((s) => s.entity_id));
      logLabel.hidden = entries.length === 0;
      log.hidden = entries.length === 0;
      swap(log, ...entries.map((entry) => el("li", { class: "flex gap-3" },
        el("time", { class: `shrink-0 w-11 ${HUB_V4_MUTE}`, datetime: entry.at, text: hubV4Clock(new Date(entry.at)) }),
        // Le nom de la maison (celui de la liste au-dessus) plutôt que celui du journal.
        el("span", { class: "min-w-0 truncate", text: `${entities.has(entry.entity_id) ? nameOf(entities.get(entry.entity_id)) : entry.name ?? ""} ${entry.message ?? ""}`.trim() }))));
    }).catch(() => {});
  }
  const iconOf = (state) => {
    const domain = domainOf(state);
    if (domain === "lock") return ICONS.lock;
    if (domain === "siren") return ICONS.siren;
    if (domain === "binary_sensor" && ["door", "garage_door", "opening", "window"].includes(state.attributes.device_class)) return ICONS.door;
    return ICONS.shield;
  };
  hubV4Overlay({
    icon: ICONS.shield,
    tone: "green",
    title: "Sécurité",
    sub,
    body: [list, logLabel, log],
    more: allowed ? { label: "Le journal", run: () => openLogbook() } : null,
    refresh: () => {
      const all = states();
      if (all.length === 0) {
        hubV4Close();
        return;
      }
      const issues = all.map(hubV3SecurityIssue).filter(Boolean);
      setText(sub, issues.length === 0 ? "Tout est calme" : issues.length === 1 ? issues[0] : `${issues.length} choses à voir`);
      swap(list, ...all.map((state) => {
        const issue = hubV3SecurityIssue(state) !== null;
        return el("li", { class: HUB_V4_ROW },
          el("span", { class: `shrink-0 ${HUB_V4_MUTE} [&_svg]:size-5`, "aria-hidden": "true" }, svgIcon(iconOf(state))),
          el("b", { class: "flex-1 min-w-0 text-corps font-medium truncate", text: nameOf(state) }),
          el("span", {
            class: "shrink-0 px-2.5 py-1 rounded-full text-legende font-semibold bg-v-ok/12 "
              // Le texte tire un peu vers l'encre : lisible (AA) sur la teinte, en Clair comme en Sombre.
              + "text-[color-mix(in_srgb,var(--v3-ok)_78%,var(--v3-ink))] data-issue:bg-v-alert/12 "
              + "data-issue:text-[color-mix(in_srgb,var(--v3-alert)_78%,var(--v3-ink))]",
            ...(issue ? { "data-issue": "" } : {}),
            text: hubV3SecurityWord(state),
          }));
      }));
    },
  });
  return true;
}

// ---------------------------------------------------------------- Les boutons de l'accueil

/** Ce qui ouvre quoi, en v4 : le premier sélecteur qui contient le toucher l'emporte. */
const HUB_V4_TAPS = [
  ["#hub-v3-music .hub-v3-music-open, #hub-v3-music-btn", () => hubV4Music()],
  ["#hub #hub-weather", () => hubV4Weather()],
  ["#hub-v3-tempo", () => hubV4Tempo()],
  ["#hub-open-agenda", () => hubV4Agenda()],
  ["#hub-announce", () => hubV4Announce()],
  ["#hub-v3-bell", () => hubV4Bell()],
  ["#hub-v3-assist", () => hubV4Assist()],
  ["#hub-v3-account", () => hubV4People()],
  ["#hub .hub-v3-zone.water", () => hubV4Water()],
  // La carte d'une machine en marche (house-tableau.js) : sa fenêtre, plutôt que l'historique.
  ["#hub .ml-card", (card) => {
    if (typeof machineLive !== "object") return false;
    for (const [id, item] of machineLive.cards) if (item.node === card) return hubV4Machine(id);
    return false;
  }],
];

/** Vrai si le toucher (sur `target`) a ouvert une incrustation v4. */
function hubV4Tap(target) {
  if (!hubV4On() || typeof target?.closest !== "function") return false;
  for (const [selector, open] of HUB_V4_TAPS) {
    const node = target.closest(selector);
    if (node && open(node)) return true;
  }
  return false;
}

document.addEventListener("click", (event) => {
  if (hubV4Tap(event.target)) {
    event.stopImmediatePropagation();
    event.preventDefault();
  }
}, { capture: true });

// La météo et Tempo ne sont pas des boutons : en v4, ils le deviennent (clavier compris).
const HUB_V4_TAP_CARDS = [["#hub-weather", "Météo en détail"], ["#hub-v3-tempo", "Tempo en détail"]];
document.addEventListener("keydown", (event) => {
  if (event.key !== "Enter" && event.key !== " ") return;
  const card = HUB_V4_TAP_CARDS.find(([selector]) => event.target?.matches?.(selector));
  if (card && hubV4Tap(event.target)) event.preventDefault();
});

/** Appelé après chaque rendu de l'accueil (hub-v4.js) : les cartes à toucher. */
function renderHubV4Incr() {
  const on = hubV4On();
  for (const [selector, label] of HUB_V4_TAP_CARDS) {
    const node = document.querySelector?.(selector);
    if (!node) continue;
    if (on) {
      setAttr(node, "role", "button");
      setAttr(node, "tabindex", "0");
      setAttr(node, "aria-label", label);
      node.classList.add("cursor-pointer");
    } else if (node.getAttribute("role") === "button") {
      node.removeAttribute("role");
      node.removeAttribute("tabindex");
      node.removeAttribute("aria-label");
      node.classList.remove("cursor-pointer");
    }
  }
}
