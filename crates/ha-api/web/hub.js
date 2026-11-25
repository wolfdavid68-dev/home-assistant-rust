// Tableau de bord — la vue « hub » de la tablette murale (étape 80).
//
// Sur la tablette accrochée au mur, à l'horizontale : le dessin de la maison
// au centre, et autour ce qu'on regarde en passant. En haut, qui est là et
// l'alerte du moment ; à gauche, l'heure, la météo et l'agenda ; à droite,
// les scènes et les pièces ; en bas, le tableau de bord complet, l'agenda,
// l'énergie et une annonce sur les enceintes.
//
// Le dessin n'est pas recopié : comme pour l'écran de veille, c'est le même
// élément (#house-stage), avec ses calques et ses zones, qui est déplacé
// ici, puis remis à sa place.
//
// Chargé après house.js et agenda.js : il utilise leurs fonctions (openRoom,
// selectPage, agendaEvents…). Pas d'innerHTML ici non plus.
"use strict";

const HUB_KEY = "ha-hub";
// Plus étroit que ça (téléphone, tablette à la verticale) : le tableau de
// bord habituel.
const hubWide = matchMedia("(min-width: 900px) and (orientation: landscape)");
const hub = {
  on: false,
  // « Toute la maison » : le tableau de bord complet, jusqu'au retour à
  // l'accueil (2 minutes sans contact).
  away: false,
};
const HUB_ICONS = {
  grid: ["M3 3h7v7H3z", "M14 3h7v7h-7z", "M3 14h7v7H3z", "M14 14h7v7h-7z"],
  bolt: ["M13 2L4 14h7l-1 8 9-12h-7z"],
  warn: ["M12 3l10 18H2z", "M12 10v4", "M12 17.5v.5"],
};
// Une couleur par personne, toujours la même pour un même nom.
// Assez foncées pour une initiale blanche lisible (contraste AA, 4,5:1 au moins).
const HUB_PEOPLE_COLORS = ["#0277bd", "#c2185b", "#2e7d32", "#b45309", "#5e35b1", "#00838f"];

function hubWanted() {
  try {
    return localStorage.getItem(HUB_KEY) !== "0";
  } catch {
    return true;
  }
}

function hubShouldShow() {
  if ($("#dashboard-view").hidden || hub.away) return false;
  // L'accueil v4 s'affiche aussi sur le téléphone, en colonne (hub-v4-telephone.js).
  if (typeof hubV4Phone === "function" && hubV4Phone()) return true;
  return document.body.classList.contains("tablet")
    && $("#hub-mode").checked
    && hubWide.matches;
}

/** Affiche ou cache la vue hub selon le moment, et la redessine. Appelé à
 *  chaque changement d'état (renderHome), et quand un réglage change. */
function updateHub() {
  const want = hubShouldShow();
  if (want && !hub.on) showHub();
  else if (!want && hub.on) hideHub();
  if (hub.on) renderHub();
}

function showHub() {
  hub.on = true;
  $("#hub").hidden = false;
  document.documentElement.dataset.hub = "on";
  // Le dessin vient au centre (sauf s'il est dans l'écran de veille : il
  // reviendra ici au réveil).
  const stage = $("#house-stage");
  if (!saver.on) $("#hub-house").append(stage);
  if (house.loading === null && token.get()) loadHouse();
}

function hideHub() {
  hub.on = false;
  $("#hub").hidden = true;
  // Les fenêtres de l'accueil v4 ne survivent pas à l'accueil.
  if (typeof hubV4Shut === "function") hubV4Shut();
  delete document.documentElement.dataset.hub;
  const stage = $("#hub-house #house-stage");
  if (stage) {
    $("#house .house-frame").append(stage);
    if (house.view) fitLabels(house.view);
  }
  // Le tableau de bord n'était plus redessiné sous la vue hub : il se remet à jour.
  scheduleHome();
}

/** Retour à l'accueil (account.js, 2 minutes sans contact) : la vue hub
 *  revient, agenda et musique refermés. */
function hubBack() {
  hub.away = false;
  if (typeof agenda !== "undefined" && agenda.open) closeAgenda();
  if (typeof music !== "undefined" && music.open) closeMusic();
  updateHub();
}

/** Quitte la vue hub pour le tableau de bord complet. */
function leaveHub() {
  hub.away = true;
  updateHub();
}

function renderHub() {
  const now = new Date();
  setText($("#hub-clock"), hourMinute.format(now));
  setText($("#hub-date"), fullDate.format(now));
  const connection = $("#connection");
  swap($("#hub-live"), el("i", { class: connection.classList.contains("live") ? "on" : "" }), connection.textContent);
  renderHubPeople();
  renderHubAlert();
  renderHubGreeting(now);
  renderHubWeather();
  renderHubAgenda(now);
  renderHubViews();
  renderHubScenes();
  renderHubRooms();
  renderHubDock();
  if (house.view) updateHouse(house.view);
  // L'accueil v3 (hub-v3.js, chargé après) : ne fait rien tant qu'il n'est pas choisi.
  if (typeof renderHubV3 === "function") renderHubV3();
}

function personColor(name) {
  let sum = 0;
  for (const char of name) sum = (sum * 31 + char.codePointAt(0)) % 997;
  return HUB_PEOPLE_COLORS[sum % HUB_PEOPLE_COLORS.length];
}

function renderHubPeople() {
  const people = [...entities.values()]
    .filter((s) => domainOf(s) === "person")
    .sort((a, b) => nameOf(a).localeCompare(nameOf(b), "fr"));
  const box = $("#hub-people");
  box.hidden = people.length === 0;
  swap(box, ...people.map((state) => {
    const name = nameOf(state);
    const home = state.state === "home";
    const avatar = el("span", {
      class: home ? "hub-avatar w-[34px] h-[34px] [border-radius:50%] grid [place-items:center] [:not(:where(.o134005))&]:font-bold [:not(:where(.o134006))&]:text-corps [color:#fff] [background:var(--person)] [box-shadow:0_0_0_2px_var(--bg)] [&.away]:[background:transparent] [&.away]:[color:var(--muted)] [&.away]:[box-shadow:inset_0_0_0_1.5px_var(--muted)]" : "hub-avatar away w-[34px] h-[34px] [border-radius:50%] grid [place-items:center] [:not(:where(.o134005))&]:font-bold [:not(:where(.o134006))&]:text-corps [color:#fff] [background:var(--person)] [box-shadow:0_0_0_2px_var(--bg)] [&.away]:[background:transparent] [&.away]:[color:var(--muted)] [&.away]:[box-shadow:inset_0_0_0_1.5px_var(--muted)]",
      title: `${name} : ${home ? "à la maison" : "absent"}`,
      text: name.charAt(0).toUpperCase(),
    });
    avatar.style.setProperty("--person", personColor(name));
    return avatar;
  }));
  const pill = $("#hub-home");
  pill.hidden = people.length === 0;
  const here = people.filter((s) => s.state === "home");
  const away = people.filter((s) => s.state !== "home").map(nameOf);
  setText(pill, [
    here.length === 0 ? "Personne à la maison" : `${here.length} à la maison`,
    away.length > 0 && here.length > 0 ? `${joinNames(away)} ${away.length > 1 ? "ne sont pas là" : "n'est pas là"}` : "",
  ].filter(Boolean).join(" · "));
}

/** L'alerte du moment, et de quoi y répondre (fermer le garage). Les ordres
 *  à valider (étape 48) passent en premier. */
function renderHubAlert() {
  const found = alerts();
  const pill = $("#hub-alert");
  const waiting = noticeStates().filter((s) => s.state === "notifying" && s.attributes.kind === "approval").length;
  pill.hidden = found.length === 0 && waiting === 0;
  if (waiting > 0) {
    const open = el("button", { type: "button", class: "hub-alert-action [border:0] [border-radius:999px] [padding:5px_14px] [background:rgb(255_255_255_/_0.16)] font-semibold cursor-pointer", text: "Voir" });
    open.addEventListener("click", () => {
      if (typeof hubV4Approval === "function" && hubV4Approval()) return;
      $("#open-notices").click();
    });
    swap(pill, svgIcon(HUB_ICONS.warn),
      el("b", { text: waiting === 1 ? "1 ordre attend ta validation" : `${waiting} ordres attendent ta validation` }), open);
    return;
  }
  if (found.length === 0) return;
  const { state, text } = found[0];
  const parts = [svgIcon(HUB_ICONS.warn), el("b", { text: `${nameOf(state)} : ${text}` })];
  if (found.length > 1) parts.push(el("span", { text: ` et ${found.length - 1} de plus` }));
  if (domainOf(state) === "cover" && canControl(state)) {
    const close = el("button", { type: "button", class: "hub-alert-action [border:0] [border-radius:999px] [padding:5px_14px] [background:rgb(255_255_255_/_0.16)] font-semibold cursor-pointer", text: "Fermer" });
    close.addEventListener("click", () => callService("cover", "close_cover", { entity_id: state.entity_id }));
    parts.push(close);
  }
  swap(pill, ...parts);
}

function renderHubGreeting(now) {
  const hour = now.getHours();
  const hello = hour >= 5 && hour < 18 ? "Bonjour." : "Bonsoir.";
  const found = alerts();
  const soon = typeof agendaSoon === "function" ? agendaSoon() : null;
  const parts = found.length > 1 ? [`${hello} ${found.length} choses demandent ton attention.`]
    : soon ? [`${hello} `, soon, "."]
    : [`${hello} ${found.length === 0 ? "Tout est calme." : ""}`.trim()];
  // Une machine en marche (lave-linge SmartThings, étape 86) : quand elle finit.
  const machines = [...entities.values()]
    .filter((s) => typeof s.attributes.appareil === "string")
    // « a fini » pendant une heure, comme la carte sur le dessin (house.js).
    .filter((s) => s.state === "En marche" || (s.state === "Terminé" && Date.now() - Date.parse(s.last_changed) < MACHINE_DONE_SHOWN))
    .map((s) => (s.state === "Terminé" ? `${nameOf(s)} a fini` : typeof s.attributes.fin_heure === "string" ? `${nameOf(s)} : fin vers ${s.attributes.fin_heure}` : `${nameOf(s)} en marche`));
  // Les colis Amazon (colis.js, étape 87).
  const parcels = typeof parcelGreeting === "function" ? parcelGreeting() : null;
  if (parcels) machines.push(parcels);
  if (machines.length) parts.push(` ${machines.join(" · ")}.`);
  swap($("#hub-greeting"), ...parts);
}

function renderHubWeather() {
  const weather = [...entities.values()].find((s) => domainOf(s) === "weather");
  const card = $("#hub-weather");
  card.hidden = !weather;
  if (!weather) return;
  const a = weather.attributes;
  const label = lookup(WEATHER_LABELS, weather.state)?.[0] ?? "";
  const days = (Array.isArray(a.forecast) ? a.forecast : [])
    .filter((day) => typeof day?.datetime === "string" && !Number.isNaN(Date.parse(day.datetime)));
  const today = days[0];
  const range = today && finite(today.templow) && finite(today.temperature)
    ? `min ${Math.round(today.templow)}° · max ${Math.round(today.temperature)}°` : "";
  swap(card,
    el("div", { class: "hub-weather-now flex items-center gap-[12px] [:not(:where(.o135800))&_svg]:w-[42px] [:not(:where(.o135801))&_svg]:h-[42px] [:not(:where(.o135802))&_svg]:[color:#ffb74d] [:not(:where(.o135803))&_svg]:[stroke-width:1.5] [:not(:where(.o135804))&_svg]:flex-none [:not(:where(.o135900))&_strong]:[font-family:var(--font-display)] [:not(:where(.o135901))&_strong]:[font-weight:300] [:not(:where(.o135902))&_strong]:text-affiche-l [:not(:where(.o136000))&_span]:grid [:not(:where(.o136001))&_span]:text-courant [:not(:where(.o136100))&_small]:text-muted" },
      svgIcon(weatherIcon(weather.state)),
      el("strong", { text: finite(a.temperature) ? `${number.format(a.temperature)}°` : "—" }),
      el("span", {}, el("b", { text: label }), el("small", { text: range }))),
    el("ol", { class: "hub-forecast list-none [margin:12px_0_0] p-0 grid [grid-template-columns:repeat(4,_1fr)] gap-[6px] [&:empty]:hidden [:not(:where(.o136400))&_li]:grid [:not(:where(.o136401))&_li]:[justify-items:center] [:not(:where(.o136402))&_li]:gap-[3px] [:not(:where(.o136403))&_li]:[padding:7px_0] [:not(:where(.o136404))&_li]:[border-radius:12px] [:not(:where(.o136405))&_li]:bg-card-2 [:not(:where(.o136406))&_li]:text-legende [:not(:where(.o136407))&_li]:text-muted [&_li::first-letter]:uppercase [:not(:where(.o136600))&_svg]:w-[18px] [:not(:where(.o136601))&_svg]:h-[18px] [:not(:where(.o136700))&_b]:text-text [:not(:where(.o136701))&_b]:text-courant", "aria-label": "Prévisions" }, ...days.slice(1, 5).map((day) =>
      el("li", {},
        el("span", { text: dayName.format(new Date(`${day.datetime}T12:00`)) }),
        svgIcon(weatherIcon(day.condition)),
        el("b", { text: finite(day.temperature) ? `${Math.round(day.temperature)}°` : "—" })))),
  );
}

/** Ce soir et demain : les prochains rendez-vous, le prochain mis en avant. */
function renderHubAgenda(now) {
  const card = $("#hub-agenda");
  const calendars = typeof agendaCalendars === "function" ? agendaCalendars() : [];
  card.hidden = calendars.length === 0;
  if (calendars.length === 0) return;
  const tomorrow = addDays(dayStart(now), 1);
  // Aujourd'hui d'abord, puis demain ; dans chaque jour, les heures avant les
  // journées entières (on les voit déjà dans l'agenda).
  const day = (e) => Number(e.startDate >= tomorrow);
  const events = hubMergeSame(agendaEvents(now, addDays(tomorrow, 1), calendars))
    .sort((a, b) => day(a) - day(b) || Number(a.allDay) - Number(b.allDay) || a.startDate - b.startDate)
    .slice(0, 5);
  const next = events.find((e) => !e.allDay && e.startDate > now);
  const rows = events.map((e) => {
    const later = e.startDate >= tomorrow;
    // Pas le nom de l'agenda : sa couleur, sur la barre, suffit.
    const sub = [later ? "demain" : "", e.location || ""].filter(Boolean).join(" · ");
    const row = el("button", { type: "button", class: e === next ? "hub-event next" : "hub-event" },
      el("span", { class: "hub-event-bar w-[4px] [align-self:stretch] [border-radius:4px] [background:var(--ev)] flex-none" }),
      el("span", { class: "hub-event-time w-[44px] flex-none [:not(:where(.o137202))&]:[font-family:var(--font-mono,_monospace)] [:not(:where(.o137203))&]:text-corps text-muted", text: e.allDay ? "jour" : hourMinute.format(e.startDate) }),
      el("span", { class: "hub-event-text grid min-w-0 [:not(:where(.o137400))&_b]:font-semibold [:not(:where(.o137401))&_b]:overflow-hidden [:not(:where(.o137402))&_b]:text-ellipsis [:not(:where(.o137403))&_b]:whitespace-nowrap [:not(:where(.o137500))&_small]:text-muted [:not(:where(.o137501))&_small]:text-legende [:not(:where(.o137502))&_small]:overflow-hidden [:not(:where(.o137503))&_small]:text-ellipsis [:not(:where(.o137504))&_small]:whitespace-nowrap [:not(:where(.o137600))&_small:empty]:hidden [.hub-event:has(.hub-event-when)_&]:flex-1 [:not(:where(.o137800)).hub-event:has(.hub-event-when)_&_b]:[white-space:normal] [.hub-event:has(.hub-event-when)_&_b]:[display:-webkit-box] [.hub-event:has(.hub-event-when)_&_b]:[-webkit-box-orient:vertical] [.hub-event:has(.hub-event-when)_&_b]:[-webkit-line-clamp:2] [.hub-event:has(.hub-event-when)_&_b]:[line-clamp:2]" }, el("b", { text: e.summary }), el("small", { text: sub })),
      hubEventWhen(e, next, later, now));
    row.style.setProperty("--ev", e.color);
    row.addEventListener("click", () => openEventDetails(e));
    return row;
  });
  const empty = [el("p", { class: "hub-empty m-[4px] text-muted text-corps", text: "Rien de prévu d'ici demain soir." })];
  // En v4 (hub-v4.js) : une icône et « + » dans l'en-tête, les courses en bas.
  const v4 = typeof hubV4On === "function" && hubV4On();
  const courses = v4 ? hubV4CoursesRow() : null;
  swap(card,
    v4 ? hubV4AgendaHead() : el("h2", {}, "Agenda", el("small", { text: "aujourd'hui et demain" })),
    ...(rows.length > 0 ? rows : empty),
    ...(courses ? [courses] : []),
  );
}

/** « dans 23 min » pour le prochain, « en cours » pour celui qui a commencé. */
function hubEventWhen(e, next, later, now) {
  if (!e.allDay && e.startDate <= now && e.endDate > now) return el("span", { class: "hub-event-when now", text: "en cours" });
  if (e === next && !later) return el("span", { class: "hub-event-when", text: inMinutes(e.startDate) });
  return "";
}

/** Le même rendez-vous dans deux agendas (les vacances dans l'agenda de la
 *  famille et dans celui des vacances scolaires) : une seule ligne. */
function hubMergeSame(events) {
  const seen = new Map();
  const out = [];
  for (const e of events) {
    // Sur deux jours, deux journées entières du même nom sont la même chose,
    // même si l'une commence un jour plus tôt.
    const key = `${e.allDay ? "jour" : e.startDate.getTime()}|${String(e.summary ?? "").trim().toLowerCase()}`;
    if (seen.has(key)) continue;
    seen.set(key, e);
    out.push(e);
  }
  return out;
}

/** « Devant », « Jardin »… : les onglets du dessin. */
function renderHubViews() {
  const box = $("#hub-views");
  box.hidden = house.pages.length < 2;
  // En v4 : le bouton des réglages du dessin à côté des onglets, pour qui peut le modifier.
  const tune = typeof hubV4On === "function" && hubV4On() && house.editable ? [hubViewsTune()] : [];
  swap(box, ...house.pages.map((page) => {
    const current = page.id === house.config?.id;
    const button = el("button", { type: "button", class: current ? "on" : "", "aria-pressed": String(current), text: page.name });
    button.addEventListener("click", () => {
      if (page.id !== house.config?.id) selectPage(page.id);
    });
    return button;
  }), ...tune);
}

/** Le bouton « Réglages du dessin » (v4). Neuf à chaque rendu, comme les onglets :
 *  swap() le compare à l'ancien et garde l'ancien s'il est pareil. */
function hubViewsTune() {
  const button = el("button", { type: "button", class: "hub-v4-tune", "aria-label": "Réglages du dessin de la maison",
    title: "Réglages du dessin" }, svgIcon(ICONS.sliders));
  button.addEventListener("click", () => {
    $("#open-account").click();
    // Les réglages s'ouvrent sur la rubrique en cours : on passe au dessin une fois ouverts.
    setTimeout(() => $('.set-link[data-pane="dessin"]')?.click(), 0);
  });
  return button;
}

const HUB_SCENE_TONES = ["go", "home", "night", "cine"];
const HUB_SCENES_KEY = "ha-hub-scenes";
const HUB_SCENES_MAX = 6;
// En v4, sans choix fait dans Mon compte : la scène suggérée et quatre autres.
const HUB_V4_MOMENTS = 5;
// L'icône et la couleur d'une scène, devinées d'après son nom.
const HUB_SCENE_LOOKS = [
  [/nuit|coucher|dodo/i, "moon", "night"],
  [/pars|d[ée]part|absen|quitt|vacances/i, "door", "go"],
  [/cin[ée]|film|t[ée]l[ée]|\btv\b/i, "sofa", "cine"],
  [/matin|r[ée]veil|lever|bonjour/i, "illuminance", "morning"],
  [/retour|arriv|rentr|bienvenue/i, "home", "home"],
  [/repas|d[îi]ner|d[ée]jeuner|cuisine/i, "pot", "morning"],
  [/lecture|calme|d[ée]tente|soir/i, "bulb", "cine"],
];

function hubSceneLook(state, index) {
  const found = HUB_SCENE_LOOKS.find(([pattern]) => pattern.test(nameOf(state)));
  if (found) return { icon: ICONS[found[1]], tone: found[2] };
  return {
    icon: domainOf(state) === "script" ? ICONS.play : ICONS.sparkle,
    tone: HUB_SCENE_TONES[index % HUB_SCENE_TONES.length],
  };
}

/** Les scènes et scripts qu'on peut lancer depuis ce compte. */
function hubSceneChoices() {
  return [...entities.values()]
    .filter((s) => ["scene", "script"].includes(domainOf(s)) && canControl(s))
    .sort((a, b) => nameOf(a).localeCompare(nameOf(b), "fr"));
}

/** Le choix de cet appareil (Mon compte), ou null s'il n'y en a pas. */
function hubChosenScenes() {
  try {
    const saved = JSON.parse(localStorage.getItem(HUB_SCENES_KEY) ?? "null");
    return Array.isArray(saved) ? saved.filter((id) => typeof id === "string").slice(0, HUB_SCENES_MAX) : null;
  } catch {
    return null;
  }
}

function hubScenes() {
  const choices = hubSceneChoices();
  const chosen = hubChosenScenes();
  const picked = chosen ? choices.filter((s) => chosen.includes(s.entity_id)) : [];
  // Un choix dont plus rien n'existe (scène supprimée) : le choix par défaut.
  return picked.length > 0 ? picked : choices.slice(0, HUB_SCENES_MAX);
}

function renderHubScenes() {
  const list = hubScenes();
  const box = $("#hub-scenes");
  box.hidden = list.length === 0;
  // En v4 (hub-v4.js) : la scène de l'heure en tête, sur toute la largeur, puis deux colonnes.
  const v4 = typeof hubV4On === "function" && hubV4On();
  const suggested = v4 ? hubV4Suggested(list.map(nameOf), new Date().getHours()) : -1;
  let order = list.map((state, index) => ({ state, index }));
  if (suggested > 0) order.unshift(...order.splice(suggested, 1));
  // Le choix par défaut (rien de coché dans Mon compte) : cinq moments en v4, comme la maquette.
  if (v4 && hubChosenScenes() === null) order = order.slice(0, HUB_V4_MOMENTS);
  // Pas de case vide : 3 colonnes pour 3, 5 ou 6 scènes, 2 pour 2 ou 4.
  const columns = order.length === 1 ? 1 : v4 ? 2 : list.length % 3 === 0 || list.length === 5 ? 3 : 2;
  box.style.setProperty("--cols", String(columns));
  box.classList.toggle("narrow", columns === 3);
  swap(box, ...order.map(({ state, index }) => {
    const isScript = domainOf(state) === "script";
    const running = isScript && state.state === "on";
    const look = hubSceneLook(state, index);
    const suggest = index === suggested;
    const button = el("button", { type: "button", class: `${suggest ? "hub-v4-suggest col-span-full " : ""}hub-scene [.hub-scenes.narrow_&]:min-h-[74px] [.hub-scenes.narrow_&]:p-[11px] [:not(:where(.o140300)).hub-scenes.narrow_&_b]:text-courant [:not(:where(.o140301)).hub-scenes.narrow_&_b]:[line-height:1.2] grid gap-[8px] justify-items-start min-h-[82px] p-[13px] [border:0] [border-radius:18px] bg-card text-left cursor-pointer [:not(:where(.o140500))&_b]:text-moyen [:not(:where(.o140501))&_b]:font-semibold [&.pending]:[opacity:0.6] [&.on]:[box-shadow:inset_0_0_0_2px_#03a9f4] t-${look.tone}${running ? " on" : ""}` },
      el("span", { class: "hub-scene-icon w-[38px] h-[38px] [border-radius:12px] grid [place-items:center] [:not(:where(.o140700))&_svg]:w-[20px] [:not(:where(.o140701))&_svg]:h-[20px] [:not(:where(.o140800)).hub-scene.t-go_&]:[background:rgb(3_169_244_/_0.2)] [:not(:where(.o140801)).hub-scene.t-go_&]:[color:#03a9f4] [:not(:where(.o140900)).hub-scene.t-home_&]:[background:rgb(102_187_106_/_0.2)] [:not(:where(.o140901)).hub-scene.t-home_&]:[color:#66bb6a] [:not(:where(.o141000)).hub-scene.t-night_&]:[background:rgb(149_117_205_/_0.25)] [:not(:where(.o141001)).hub-scene.t-night_&]:[color:#b39ddb] [:not(:where(.o141100)).hub-scene.t-cine_&]:[background:rgb(255_183_77_/_0.2)] [:not(:where(.o141101)).hub-scene.t-cine_&]:[color:#ffb74d] [:not(:where(.o141200)).hub-scene.t-morning_&]:[background:rgb(255_213_79_/_0.2)] [:not(:where(.o141201)).hub-scene.t-morning_&]:[color:#f9a825]" }, svgIcon(running ? ICONS.stop : look.icon)),
      suggest ? el("span", { class: "grid min-w-0 gap-0.5" }, el("b", { text: nameOf(state) }),
        el("small", { class: "text-legende text-v-mute", text: "Suggéré à cette heure" })) : el("b", { text: nameOf(state) }));
    button.addEventListener("click", () => {
      if (!running && typeof hubV4Moment === "function" && hubV4Moment(state)) return;
      button.classList.add("pending");
      setTimeout(() => button.classList.remove("pending"), 5000);
      callService(domainOf(state), running ? "turn_off" : "turn_on", { entity_id: state.entity_id });
    });
    return button;
  }));
}

function renderHubRooms() {
  const areas = shownAreas();
  const box = $("#hub-rooms");
  box.hidden = areas.length === 0;
  // Plus de 6 pièces : deux colonnes plus serrées, pour tout voir sans faire
  // défiler. La température passe alors sous le nom.
  const compact = areas.length > 6;
  box.classList.toggle("compact", compact);
  swap(box, ...areas.map((area) => {
    const info = roomInfo(area);
    const detail = info.alert ? info.alert
      : info.lights > 0 ? `${info.lights} lumière${info.lights > 1 ? "s" : ""}`
      : info.open > 0 ? `${info.open} ouverture${info.open > 1 ? "s" : ""}`
      : info.heating ? "chauffe" : "";
    const temperature = Number.isFinite(info.temperature) ? `${number.format(info.temperature)}°` : "";
    const classes = ["hub-room"];
    if (info.alert) classes.push("alert");
    else if (info.lights > 0) classes.push("lit");
    const room = el("button", { type: "button", class: classes.join(" "), title: [area.name, detail].filter(Boolean).join(" : ") },
      el("span", { class: "hub-room-icon w-[38px] h-[38px] [border-radius:50%] grid [place-items:center] flex-none bg-card-2 text-muted [:not(:where(.o141800))&_svg]:w-[20px] [:not(:where(.o141801))&_svg]:h-[20px] [:not(:where(.o141900)).hub-room.lit_&]:[background:#ffb74d] [:not(:where(.o141901)).hub-room.lit_&]:[color:#1a1205] [.hub-room.lit_&]:[box-shadow:0_0_18px_rgb(255_183_77_/_0.45)] [:not(:where(.o142000)).hub-room.alert_&]:[background:rgb(255_152_0_/_0.25)] [:not(:where(.o142001)).hub-room.alert_&]:[color:#ffb74d] [.hub-rooms.compact_&]:w-[32px] [.hub-rooms.compact_&]:h-[32px] [:not(:where(.o143000)).hub-rooms.compact_&_svg]:w-[17px] [:not(:where(.o143001)).hub-rooms.compact_&_svg]:h-[17px]" }, svgIcon(roomIcon(area.name))),
      el("span", { class: "hub-room-text grid min-w-0 [:not(:where(.o142300))&_b]:font-semibold [:not(:where(.o142400))&_small]:text-muted [:not(:where(.o142401))&_small]:text-petit [:not(:where(.o142500))&_small:empty]:hidden [:not(:where(.o143100)).hub-rooms.compact_&_b]:text-courant [:not(:where(.o143101)).hub-rooms.compact_&_b]:overflow-hidden [:not(:where(.o143102)).hub-rooms.compact_&_b]:text-ellipsis [:not(:where(.o143103)).hub-rooms.compact_&_b]:whitespace-nowrap [:not(:where(.o143200)).hub-rooms.compact_&_small]:text-legende [:not(:where(.o143201)).hub-rooms.compact_&_small]:overflow-hidden [:not(:where(.o143202)).hub-rooms.compact_&_small]:text-ellipsis [:not(:where(.o143203)).hub-rooms.compact_&_small]:whitespace-nowrap" }, el("b", { text: area.name }),
        el("small", { text: compact ? [temperature, detail].filter(Boolean).join(" · ") : detail })),
      compact ? "" : el("span", { class: "hub-room-temp ml-[auto] [:not(:where(.o142601))&]:[font-family:var(--font-mono,_monospace)] [:not(:where(.o142602))&]:text-courant text-muted", text: temperature }));
    room.addEventListener("click", () => openRoom(area.name));
    return room;
  }));
}

/** La puissance du moment, pour le bouton Énergie : le compteur d'abord. */
function hubPower() {
  const sensors = [...entities.values()].filter((s) => domainOf(s) === "sensor"
    && s.attributes.device_class === "power" && Number.isFinite(Number(s.state)));
  const meter = sensors.find((s) => /linky|compteur|papp|sinsts/.test(s.entity_id)) ?? sensors[0];
  if (!meter) return "";
  const watts = Number(meter.state) * (meter.attributes.unit_of_measurement === "kW" ? 1000 : 1);
  return watts >= 1000 ? `${number.format(Math.round(watts / 100) / 10)} kW` : `${Math.round(watts)} W`;
}

/** Le service d'annonce (étape 73), s'il y a des enceintes et qu'on peut s'en servir. */
function hubAnnouncer() {
  const tts = [...entities.values()].find((s) => domainOf(s) === "tts" && canControl(s));
  if (!tts) return null;
  const speakers = (Array.isArray(tts.attributes.enceintes) ? tts.attributes.enceintes : []).filter((id) => entities.has(id));
  const voice = typeof tts.attributes.voix === "string" ? tts.attributes.voix : "";
  return speakers.length > 0 && voice !== "" && !voice.startsWith("aucune") ? { tts, speakers } : null;
}

function renderHubDock() {
  $("#hub-open-agenda").hidden = $("#hub-agenda").hidden;
  renderHubCourses();
  const energy = $("#hub-energy");
  energy.hidden = $("#open-energy").hidden;
  const power = hubPower();
  setText(energy.lastChild, power || "Énergie");
  $("#hub-announce").hidden = hubAnnouncer() === null;
}

// ---------------------------------------------------------------- Courses
// La liste de courses (étape 81 : celle de l'appli Iglou quand elle est
// reliée), en grand : on coche en rangeant les courses, on ajoute en passant.

const HUB_LIST = "todo.courses";

function renderHubCourses() {
  const state = entities.get(HUB_LIST);
  const button = $("#hub-courses");
  button.hidden = !state;
  if (!state) {
    if ($("#hub-list").open) $("#hub-list").close();
    return;
  }
  const items = todoItems(state);
  const left = items.filter((item) => item.status !== "completed").length;
  setText(button.lastChild, state.state === "unavailable" ? "Courses" : left === 0 ? "Courses" : `Courses · ${left}`);
  if ($("#hub-list").open) renderHubList(state);
}

function renderHubList(state) {
  const items = todoItems(state);
  const iglou = state.attributes.source === "iglou";
  const left = items.filter((item) => item.status !== "completed").length;
  $("#hub-list-title").textContent = left === 0 ? "Courses" : `Courses · ${left} à acheter`;
  $("#hub-list-note").textContent = state.state === "unavailable"
    ? "Iglou ne répond pas : la liste sera relue dans une minute."
    : iglou ? "La liste de l'appli Iglou. Coche ce qui est dans le panier ; on range dans l'appli." : "";
  $("#hub-list-items").replaceChildren(...(items.length > 0
    ? todoRows(state, items)
    : [el("li", { class: "hub-list-empty [padding:1.2rem_0.4rem] text-muted", text: "Rien à acheter." })]));
}

$("#hub-courses").addEventListener("click", () => {
  const state = entities.get(HUB_LIST);
  if (!state) return;
  renderHubList(state);
  $("#hub-list").showModal();
});
$("#hub-list-close").addEventListener("click", () => $("#hub-list").close());
$("#hub-list-form").addEventListener("submit", async (event) => {
  event.preventDefault();
  const input = $("#hub-list-text");
  const item = input.value.trim();
  if (item === "") return;
  // Effacé seulement si l'ordre est parti.
  if (await callService("todo", "add_item", { entity_id: HUB_LIST, item }) && input.value.trim() === item) input.value = "";
});

// ---------------------------------------------------------------- Annonce
// « Annonce à la maison » : une phrase, lue sur toutes les enceintes.

$("#hub-say-form").addEventListener("submit", async (event) => {
  event.preventDefault();
  const input = $("#hub-say-text");
  const announcer = hubAnnouncer();
  const message = input.value.trim();
  if (!announcer || message === "") return;
  const sent = await callService("tts", "speak", {
    entity_id: announcer.tts.entity_id,
    media_player_entity_id: announcer.speakers,
    message,
  });
  // L'annonce n'est pas partie : la fenêtre reste ouverte, le texte aussi.
  if (!sent) return;
  input.value = "";
  $("#hub-say").close();
});
$("#hub-say-close").addEventListener("click", () => $("#hub-say").close());

// ---------------------------------------------------------------- Boutons et réglage

function hubButton(id, icon, text) {
  $(id).replaceChildren(svgIcon(icon), el("span", { text }));
}
hubButton("#hub-all", HUB_ICONS.grid, "Toute la maison");
hubButton("#hub-open-agenda", ICONS.calendar, "Agenda");
hubButton("#hub-courses", ICONS.list, "Courses");
hubButton("#hub-energy", HUB_ICONS.bolt, "Énergie");
hubButton("#hub-announce", ICONS.megaphone, "Annonce à la maison");

$("#hub-all").addEventListener("click", leaveHub);
$("#hub-open-agenda").addEventListener("click", () => {
  leaveHub();
  openAgenda();
});
$("#hub-energy").addEventListener("click", () => openEnergy());
$("#hub-announce").addEventListener("click", () => {
  $("#hub-say").showModal();
  $("#hub-say-text").focus();
});

$("#hub-mode").checked = hubWanted();
$("#hub-mode").addEventListener("change", (event) => {
  try {
    if (event.target.checked) localStorage.removeItem(HUB_KEY);
    else localStorage.setItem(HUB_KEY, "0");
  } catch {
    // Stockage bloqué : le choix ne durera que le temps de la page.
  }
  hub.away = false;
  updateHub();
});

// Le choix des scènes, dans Mon compte : une case par scène ou script.
function renderHubPick() {
  const chosen = new Set(hubScenes().map((s) => s.entity_id));
  const full = chosen.size >= HUB_SCENES_MAX;
  const choices = hubSceneChoices();
  $("#hub-pick-list").replaceChildren(...(choices.length === 0
    ? [el("p", { class: "account m-0 text-muted [:not(:where(.o024100))&_strong]:text-text [.assist-dialog_&]:m-0 [.assist-dialog_&]:text-petit", text: "Aucune scène ni script pour l'instant." })]
    : choices.map((state) => {
      const box = el("input", { type: "checkbox", value: state.entity_id });
      box.checked = chosen.has(state.entity_id);
      box.disabled = full && !box.checked;
      box.addEventListener("change", saveHubPick);
      return el("label", { class: "check [.preset-form_&]:text-corps [label&]:flex [label&]:items-center [label&]:gap-[0.45rem] [label&]:text-text" }, box, ` ${nameOf(state)}`);
    })));
}

function saveHubPick() {
  const ids = [...document.querySelectorAll("#hub-pick-list input:checked")].map((box) => box.value);
  try {
    if (ids.length === 0) localStorage.removeItem(HUB_SCENES_KEY);
    else localStorage.setItem(HUB_SCENES_KEY, JSON.stringify(ids.slice(0, HUB_SCENES_MAX)));
  } catch {
    // Stockage bloqué : rien à garder.
  }
  renderHubPick();
  if (hub.on) renderHubScenes();
}

$("#hub-pick").addEventListener("toggle", () => {
  if ($("#hub-pick").open) renderHubPick();
});
$("#tablet-mode").addEventListener("change", () => {
  hub.away = false;
  updateHub();
});
hubWide.addEventListener("change", updateHub);
updateHub();
