// Tableau de bord — les tuiles du temps : la météo et ses prévisions, la
// vigilance météo et les jours Tempo.
//
// Un des fichiers de tiles.js, chargé après tiles-appareils.js par
// index.html. Les scripts partagent leurs déclarations. Pas d'innerHTML ici
// non plus.
"use strict";

// La météo : condition, température, et les prévisions des prochains jours.
const WEATHER_LABELS = {
  "sunny": ["Ensoleillé", "sun"], "clear-night": ["Nuit claire", "moon"],
  "partlycloudy": ["Éclaircies", "partly"], "cloudy": ["Nuageux", "cloud"],
  "fog": ["Brouillard", "fog"], "rainy": ["Pluie", "rain"], "pouring": ["Fortes pluies", "rain"],
  "snowy": ["Neige", "snow"], "snowy-rainy": ["Pluie et neige", "snow"],
  "lightning-rainy": ["Orages", "storm"], "exceptional": ["Exceptionnel", "cloud"],
};
const dayName = new Intl.DateTimeFormat("fr", { weekday: "short" });
const finite = (value) => typeof value === "number" && Number.isFinite(value);
const degrees = (value) => (finite(value) ? `${number.format(value)} °C` : "—");

function weatherIcon(condition) {
  const [, icon] = lookup(WEATHER_LABELS, condition) ?? ["", "cloud"];
  return icon === "sun" ? ICONS.illuminance : lookup(ICONS, icon) ?? ICONS.cloud;
}

function weatherTile(state) {
  const a = state.attributes;
  const [label] = lookup(WEATHER_LABELS, state.state) ?? [statusText(state)];
  const feels = finite(a.apparent_temperature) ? ` · ressenti ${degrees(a.apparent_temperature)}` : "";
  const details = [
    finite(a.humidity) && `Humidité ${number.format(a.humidity)}\u202f%`,
    finite(a.wind_speed) && `Vent ${number.format(a.wind_speed)} km/h`,
  ].filter(Boolean).join(" · ");
  const days = (Array.isArray(a.forecast) ? a.forecast : [])
    .filter((day) => typeof day?.datetime === "string" && !Number.isNaN(Date.parse(day.datetime)))
    .slice(0, 5)
    .map((day) => {
      const [dayLabel] = lookup(WEATHER_LABELS, day.condition) ?? [""];
      const rain = finite(day.precipitation_probability) && day.precipitation_probability >= 30
        ? `${number.format(day.precipitation_probability)}\u202f%` : "";
      return el(
        "li",
        { title: dayLabel },
        el("span", { class: "day", text: dayName.format(new Date(`${day.datetime}T12:00`)) }),
        svgIcon(weatherIcon(day.condition)),
        el("span", { class: "max", text: finite(day.temperature) ? `${Math.round(day.temperature)}°` : "—" }),
        el("span", { class: "min", text: finite(day.templow) ? `${Math.round(day.templow)}°` : "" }),
        el("small", { class: "rain", text: rain }),
      );
    });
  const value = el("div", { class: "value", text: finite(a.temperature) ? number.format(a.temperature) : "—" });
  if (finite(a.temperature)) value.append(el("small", { text: "°C" }));
  return el(
    "div",
    { id: tileId(state.entity_id), class: `${tileClass(state, "weather")} wide` },
    tileHead(state, weatherIcon(state.state), `${label}${feels}`),
    value,
    el("div", { class: "sub", text: details }),
    el("ol", { class: "forecast", "aria-label": "Prévisions" }, ...days),
    // Licence CC BY 4.0 d'Open-Meteo : la source doit être citée.
    el("small", { class: "attribution", text: typeof a.attribution === "string" ? a.attribution : "" }),
  );
}

// La vigilance Météo-France : le niveau du jour (sensor.vigilance_meteo),
// les phénomènes et leurs heures, et demain (sensor.vigilance_meteo_demain).
const VIGILANCE_TODAY = "sensor.vigilance_meteo";
const VIGILANCE_TOMORROW = "sensor.vigilance_meteo_demain";
// Le niveau, sa classe CSS et l'adjectif (« vigilance orange »).
const VIGILANCE_LEVELS = { Vert: ["green", "verte"], Jaune: ["yellow", "jaune"], Orange: ["orange", "orange"], Rouge: ["red", "rouge"] };
const vigilanceRank = (level) => ["Vert", "Jaune", "Orange", "Rouge"].indexOf(level);

function vigilanceBadge(level) {
  const [color, adjective] = lookup(VIGILANCE_LEVELS, level) ?? ["unknown", ""];
  // La couleur ne suffit pas : le niveau est toujours écrit.
  return el("span", { class: `vigilance-badge level-${color}`, text: `Vigilance ${adjective}` });
}

function vigilancePhenomena(state) {
  const list = Array.isArray(state.attributes.phenomenes) ? state.attributes.phenomenes : [];
  return list.filter((p) => typeof p?.phenomene === "string" && vigilanceRank(p.niveau) >= 0);
}

/** « orange : orages, vent violent » : pour le résumé de l'accueil. */
function vigilanceSummary(state) {
  const [, adjective] = lookup(VIGILANCE_LEVELS, state.state) ?? ["", state.state];
  const names = vigilancePhenomena(state)
    .filter((p) => p.niveau === state.state)
    .map((p) => p.phenomene.toLowerCase());
  return names.length > 0 ? `${adjective} (${joinNames(names)})` : adjective;
}

const hourText = (date) => (date.getMinutes() === 0 ? `${date.getHours()} h` : `${date.getHours()} h ${String(date.getMinutes()).padStart(2, "0")}`);

/** « de 14 h à 23 h », « à partir de 16 h », « toute la journée ». */
function vigilanceRangeText(range, periodStart, periodEnd) {
  const begin = new Date(range.debut);
  const end = new Date(range.fin);
  if (Number.isNaN(begin.getTime()) || Number.isNaN(end.getTime())) return "";
  const fromStart = periodStart !== null && begin <= periodStart;
  const toEnd = periodEnd !== null && end >= periodEnd;
  if (fromStart && toEnd) return "toute la journée";
  if (fromStart) return `jusqu'à ${hourText(end)}`;
  if (toEnd) return `à partir de ${hourText(begin)}`;
  return `de ${hourText(begin)} à ${hourText(end)}`;
}

function vigilanceDetails(state) {
  const a = state.attributes;
  const start = typeof a.debut === "string" ? new Date(a.debut) : null;
  const end = typeof a.fin === "string" ? new Date(a.fin) : null;
  return vigilancePhenomena(state).map((p) => {
    const ranges = (Array.isArray(p.plages) ? p.plages : []).filter((range) => vigilanceRank(range.niveau) > 0);
    // Les heures, couleur par couleur, la plus grave d'abord :
    // « orange de 16 h à 23 h · jaune de 9 h à 16 h et à partir de 23 h ».
    const levels = [...new Set([p.niveau, ...ranges.map((range) => range.niveau)])]
      .sort((x, y) => vigilanceRank(y) - vigilanceRank(x));
    const text = levels.map((level) => {
      const hours = ranges
        .filter((range) => range.niveau === level)
        .map((range) => vigilanceRangeText(range, start, end))
        .filter(Boolean);
      const adjective = lookup(VIGILANCE_LEVELS, level)[1];
      return hours.length > 0 ? `${adjective} ${joinNames(hours)}` : adjective;
    }).join(" · ");
    const where = p.littoral === true ? " (littoral)" : "";
    return el(
      "li",
      { class: `level-${lookup(VIGILANCE_LEVELS, p.niveau)[0]}` },
      el("strong", { text: `${p.phenomene}${where}` }),
      ` : ${text}`,
    );
  });
}

function vigilanceTile(state) {
  const a = state.attributes;
  const known = vigilanceRank(state.state) >= 0;
  const department = typeof a.departement === "string" ? `Département ${a.departement}` : "Météo-France";
  const items = known ? vigilanceDetails(state) : [];
  const parts = [
    tileHead(state, ICONS.shield, department),
    el("div", { class: "value small-value" }, known ? vigilanceBadge(state.state) : statusText(state)),
  ];
  if (known && items.length === 0) parts.push(el("p", { class: "sub", text: "Aucun phénomène dangereux prévu aujourd'hui." }));
  if (items.length > 0) parts.push(el("ul", { class: "vigilance-list", "aria-label": "Aujourd'hui" }, ...items));
  if (known && typeof a.niveau_actuel === "string" && a.niveau_actuel !== state.state) {
    parts.push(el("p", { class: "sub", text: `En ce moment : ${lookup(VIGILANCE_LEVELS, a.niveau_actuel)?.[1] ?? a.niveau_actuel}` }));
  }
  const tomorrow = entities.get(VIGILANCE_TOMORROW);
  if (tomorrow && known) {
    let text;
    if (vigilanceRank(tomorrow.state) >= 0) {
      const names = vigilancePhenomena(tomorrow).map((p) => p.phenomene.toLowerCase());
      text = `Demain : ${lookup(VIGILANCE_LEVELS, tomorrow.state)[1]}${names.length > 0 ? ` (${joinNames(names)})` : ""}`;
    } else {
      text = "Demain : publiée vers 6 h";
    }
    parts.push(el("p", { class: "vigilance-tomorrow", text }));
  }
  parts.push(el("small", { class: "attribution", text: typeof a.attribution === "string" ? a.attribution : "" }));
  const level = known ? lookup(VIGILANCE_LEVELS, state.state)[0] : "unknown";
  return el("div", { id: tileId(state.entity_id), class: `${tileClass(state, "vigilance")} wide level-${level}` }, ...parts);
}

// Tempo (étape 60) : la couleur du jour et du lendemain, lue sur le Linky.
const TEMPO_TODAY = "sensor.linky_tempo_aujourdhui";
const TEMPO_TOMORROW = "sensor.linky_tempo_demain";
// La couleur, sa classe CSS et l'adjectif (« jour rouge »).
const TEMPO_COLORS = { Bleu: ["blue", "bleu"], Blanc: ["white", "blanc"], Rouge: ["red", "rouge"] };

function tempoBadge(color) {
  const [css, adjective] = lookup(TEMPO_COLORS, color) ?? ["unknown", ""];
  // Comme pour la vigilance, la couleur est toujours écrite.
  return el("span", { class: `tempo-badge tempo-${css}`, text: `Jour ${adjective}` });
}

/** « Demain : jour rouge », ou pas encore annoncé. */
function tempoTomorrowText(tomorrow) {
  if (lookup(TEMPO_COLORS, tomorrow.state)) return `Demain : jour ${TEMPO_COLORS[tomorrow.state][1]}`;
  if (tomorrow.state === "unknown") return "Demain : pas encore annoncé";
  return `Demain : ${statusText(tomorrow)}`;
}

function tempoTile(state) {
  const color = lookup(TEMPO_COLORS, state.state);
  const parts = [
    tileHead(state, ICONS.bolt, "D'après le compteur"),
    el("div", { class: "value small-value" }, color ? tempoBadge(state.state) : statusText(state)),
  ];
  const tomorrow = entities.get(TEMPO_TOMORROW);
  if (tomorrow) {
    // Demain annoncé : sa couleur aussi, pour la voir d'un coup d'œil.
    const line = lookup(TEMPO_COLORS, tomorrow.state) ? ["Demain : ", tempoBadge(tomorrow.state)] : [tempoTomorrowText(tomorrow)];
    parts.push(el("p", { class: "vigilance-tomorrow" }, ...line));
  }
  // Le jour Tempo commence à 6 h : à 3 h du matin, c'est encore celui de la veille.
  parts.push(el("p", { class: "sub", text: "Un jour Tempo va de 6 h à 6 h." }));
  return el("div", { id: tileId(state.entity_id), class: `${tileClass(state, "tempo")} tempo-${color?.[0] ?? "unknown"}` }, ...parts);
}
