// hub-v4-incrustations.js : musique, météo, Tempo, agenda, annonce, avis,
// Assist, présence, lave-linge, eau chaude et sécurité de l'accueil v4.
//
// Usage : node --test crates/ha-api/web/tests/*.test.mjs

import assert from "node:assert/strict";
import { describe, test } from "node:test";
import { load, plain } from "./charger.mjs";

const run = load("utils.js", "hub-v3.js", "hub-v4.js", "hub-v4-pieces.js", "hub-v4-energie.js", "hub-v4-incrustations.js");
const hubV4Clock = run("hubV4Clock");
const hubV4Minutes = run("hubV4Minutes");
const hubV4Track = run("hubV4Track");
const hubV4Volume = run("hubV4Volume");
const hubV4Wind = run("hubV4Wind");
const hubV4WeatherDays = run("hubV4WeatherDays");
const hubV4TempoDay = run("hubV4TempoDay");
const hubV4AgendaDays = run("hubV4AgendaDays");
const hubV4Holiday = run("hubV4Holiday");
const hubV4WeatherSub = run("hubV4WeatherSub");
const hubV4SayData = run("hubV4SayData");
const hubV4EventData = run("hubV4EventData");
const hubV4Notices = run("hubV4Notices");
const hubV4Presence = run("hubV4Presence");
const hubV4MachineView = run("hubV4MachineView");
const hubV4TankLevel = run("hubV4TankLevel");
const hubV4SecurityLog = run("hubV4SecurityLog");

describe("hubV4Clock et hubV4Minutes", () => {
  test("deux chiffres partout", () => {
    assert.equal(hubV4Clock(new Date(2026, 10, 5, 7, 5)), "07:05");
    assert.equal(hubV4Clock(new Date("pas une date")), "");
  });
  test("minutes, puis heures", () => {
    assert.equal(hubV4Minutes(42), "42 min");
    assert.equal(hubV4Minutes(65), "1 h 05");
    assert.equal(hubV4Minutes(-3), "0 min");
  });
});

describe("hubV4Track", () => {
  test("titre et artiste quand ça joue", () => {
    const state = { state: "playing", attributes: { media_title: " Clair de lune ", media_artist: "Debussy" } };
    assert.deepEqual(plain(hubV4Track(state)), { title: "Clair de lune", artist: "Debussy" });
  });
  test("une radio : sa chaîne", () => {
    assert.equal(hubV4Track({ state: "paused", attributes: { media_channel: "France Inter" } }).title, "France Inter");
  });
  test("à l'arrêt : pas de musique", () => {
    assert.deepEqual(plain(hubV4Track({ state: "idle", attributes: { media_title: "Vieux titre" } })), { title: "Pas de musique", artist: "" });
  });
});

describe("hubV4Volume", () => {
  test("en pour cent, borné", () => {
    assert.equal(hubV4Volume({ attributes: { volume_level: 0.42 } }), 42);
    assert.equal(hubV4Volume({ attributes: { volume_level: 1.3 } }), 100);
    assert.equal(hubV4Volume({ attributes: {} }), null);
  });
});

describe("hubV4Wind", () => {
  test("des mots plutôt que des km/h", () => {
    assert.equal(hubV4Wind(5), "vent faible");
    assert.equal(hubV4Wind(20), "vent modéré");
    assert.equal(hubV4Wind(45), "vent fort");
    assert.equal(hubV4Wind(80), "vent violent");
    assert.equal(hubV4Wind(undefined), "");
  });
});

describe("hubV4WeatherDays", () => {
  const forecast = [
    { datetime: "2026-11-05", condition: "sunny", temperature: 12, templow: 4 },
    { datetime: "2026-11-06", condition: "rainy", temperature: 10, templow: 6, precipitation_probability: 38 },
    { datetime: "2026-11-07", condition: "cloudy", temperature: 14, templow: 2, precipitation_probability: 10 },
    { datetime: "2026-11-08", condition: "sunny", temperature: 16, templow: 8 },
    { datetime: "2026-11-09", condition: "sunny", temperature: 9, templow: 3 },
    { datetime: "2026-11-10", condition: "sunny", temperature: 9, templow: 3 },
  ];
  test("quatre jours, sans aujourd'hui", () => {
    const days = hubV4WeatherDays(forecast);
    assert.deepEqual(days.map((d) => d.datetime), ["2026-11-06", "2026-11-07", "2026-11-08", "2026-11-09"]);
  });
  test("les barres sur une échelle commune", () => {
    const days = plain(hubV4WeatherDays(forecast));
    // De 2° (le plus froid) à 16° (le plus chaud).
    assert.equal(days[1].from, 0);
    assert.equal(days[2].to, 1);
    assert.equal(days[0].from, 4 / 14);
  });
  test("la pluie seulement si elle est probable", () => {
    assert.deepEqual(hubV4WeatherDays(forecast).map((d) => d.rain), [38, null, null, null]);
  });
  test("rien de lisible : rien", () => {
    assert.deepEqual(plain(hubV4WeatherDays(undefined)), []);
    assert.deepEqual(plain(hubV4WeatherDays([{ datetime: "x" }])), []);
  });
});

describe("hubV4TempoDay", () => {
  test("les trois couleurs", () => {
    assert.deepEqual(plain(hubV4TempoDay({ state: "Rouge" })), { tone: "red", text: "Rouge" });
    assert.deepEqual(plain(hubV4TempoDay({ state: "Bleu" })), { tone: "blue", text: "Bleu" });
  });
  test("pas encore connue, ou absente", () => {
    assert.equal(hubV4TempoDay({ state: "unknown" }).text, "À venir");
    assert.equal(hubV4TempoDay({ state: "unavailable" }).text, "Indisponible");
    assert.equal(hubV4TempoDay(undefined).tone, "unknown");
  });
});

describe("hubV4AgendaDays", () => {
  const now = new Date(2026, 10, 5, 9, 0);
  const at = (d, h, m = 0) => new Date(2026, 10, d, h, m);
  test("un jour férié fait apparaître son jour", () => {
    const days = hubV4AgendaDays([], now, 7, [{ date: new Date(2026, 10, 11), name: "Armistice" }]);
    assert.deepEqual(plain(days.map((d) => [d.date.getDate(), d.holidays])), [[5, []], [11, ["Armistice"]]]);
  });
  test("un jour férié hors de la fenêtre : rien", () => {
    assert.equal(hubV4AgendaDays([], now, 3, [{ date: new Date(2026, 10, 11), name: "Armistice" }]).length, 1);
  });
  test("aujourd'hui est toujours là, les jours vides non", () => {
    const events = [{ summary: "Dentiste", startDate: at(7, 14), endDate: at(7, 15), allDay: false }];
    const days = hubV4AgendaDays(events, now);
    assert.deepEqual(plain(days.map((d) => d.date.getDate())), [5, 7]);
    assert.equal(days[0].events.length, 0);
    assert.equal(days[1].events[0].summary, "Dentiste");
  });
  test("une journée entière sur trois jours revient chaque jour", () => {
    const events = [{ summary: "Vacances", startDate: at(5, 0), endDate: at(8, 0), allDay: true }];
    assert.deepEqual(plain(hubV4AgendaDays(events, now).map((d) => d.date.getDate())), [5, 6, 7]);
  });
  test("une soirée qui finit après minuit reste sur son jour", () => {
    const events = [{ summary: "Fête", startDate: at(6, 21), endDate: at(7, 1), allDay: false }];
    assert.deepEqual(plain(hubV4AgendaDays(events, now).map((d) => d.date.getDate())), [5, 6]);
  });
});

describe("hubV4EventData", () => {
  test("avec une heure : une heure de rendez-vous", () => {
    assert.deepEqual(plain(hubV4EventData(" Dentiste ", "2026-11-07", "14:30")), {
      entity_id: "calendar.maison", summary: "Dentiste",
      start_date_time: "2026-11-07 14:30", end_date_time: "2026-11-07 15:30",
    });
  });
  test("sans heure : toute la journée", () => {
    assert.deepEqual(plain(hubV4EventData("Anniversaire", "2026-11-07", "")), {
      entity_id: "calendar.maison", summary: "Anniversaire", start_date: "2026-11-07",
    });
  });
  test("tard le soir : pas de fin le lendemain", () => {
    assert.equal(hubV4EventData("Film", "2026-11-07", "23:15").end_date_time, undefined);
  });
  test("sans titre ou sans jour : rien", () => {
    assert.equal(hubV4EventData("  ", "2026-11-07", ""), null);
    assert.equal(hubV4EventData("Dentiste", "", "10:00"), null);
  });
});

describe("hubV4Notices", () => {
  const notice = (id, attributes, state = "notifying") => ({ entity_id: id, state, attributes });
  test("à valider, puis erreurs, puis le reste ; les plus récents d'abord", () => {
    const list = hubV4Notices([
      notice("a", { created_at: "2026-11-05T08:00:00Z" }),
      notice("b", { kind: "repair", severity: "warning", created_at: "2026-11-05T09:00:00Z" }),
      notice("c", { kind: "approval", created_at: "2026-11-05T07:00:00Z" }),
      notice("d", { kind: "repair", severity: "error", created_at: "2026-11-05T06:00:00Z" }),
      notice("e", { created_at: "2026-11-05T10:00:00Z" }),
      notice("f", { created_at: "2026-11-05T11:00:00Z" }, "dismissed"),
    ]);
    assert.deepEqual(list.map((s) => s.entity_id), ["c", "d", "b", "e", "a"]);
  });
});

describe("hubV4Presence", () => {
  const now = new Date(2026, 10, 5, 20, 0);
  test("depuis quand", () => {
    assert.equal(hubV4Presence({ entity_id: "person.a", state: "home", last_changed: new Date(2026, 10, 5, 18, 40).toISOString() }, now),
      "à la maison depuis 18:40");
    assert.equal(hubV4Presence({ entity_id: "person.a", state: "not_home", last_changed: new Date(2026, 10, 4, 7, 5).toISOString() }, now),
      "absent depuis hier 07:05");
    assert.equal(hubV4Presence({ entity_id: "person.a", state: "not_home", last_changed: new Date(2026, 10, 1, 7, 5).toISOString() }, now),
      "absent depuis le 1/11");
  });
  test("une zone, ou rien de connu", () => {
    assert.equal(hubV4Presence({ entity_id: "person.a", state: "unknown", last_changed: "" }, now), "on ne sait pas");
    assert.match(hubV4Presence({ entity_id: "person.a", state: "Travail", last_changed: now.toISOString() }, now), /^Travail depuis/);
  });
});

describe("hubV4MachineView", () => {
  const machine = (state, attributes = {}, last_changed = "2026-11-05T20:00:00Z") => ({
    entity_id: "sensor.lave_linge", state, last_changed, attributes: { friendly_name: "Lave-linge", appareil: "lave_linge", ...attributes },
  });
  test("en marche : programme, heure de fin, ce qui reste", () => {
    const view = plain(hubV4MachineView(machine("En marche", { cycle: "Coton 40°", fin_heure: "22:40", reste_minutes: 42, progression: 61 })));
    assert.deepEqual(view, { phase: "running", title: "Lave-linge · coton 40°", sub: "fin vers 22:40 · encore 42 min", progress: 61 });
  });
  test("en pause", () => {
    const view = hubV4MachineView(machine("En pause", { reste_minutes: 75 }));
    assert.equal(view.phase, "paused");
    assert.equal(view.sub, "en pause · encore 1 h 15");
    assert.equal(view.progress, null);
  });
  test("fini : montré une heure", () => {
    const end = Date.parse("2026-11-05T20:00:00Z");
    assert.equal(hubV4MachineView(machine("Terminé"), end + 10 * 60_000).phase, "done");
    assert.equal(hubV4MachineView(machine("Terminé"), end + 2 * 3_600_000).phase, "");
  });
  test("à l'arrêt : rien", () => {
    assert.equal(hubV4MachineView(machine("Arrêt")).phase, "");
  });
});

describe("hubV4TankLevel", () => {
  test("de 20° à la consigne max", () => {
    assert.equal(hubV4TankLevel(42.5, 65), 0.5);
    assert.equal(hubV4TankLevel(10, 65), 0);
    assert.equal(hubV4TankLevel(80, 65), 1);
    assert.equal(hubV4TankLevel(null), null);
  });
});

describe("hubV4SecurityLog", () => {
  test("les passages de ces appareils, les plus récents d'abord", () => {
    const entries = [
      { at: "2026-11-05T08:00:00Z", entity_id: "binary_sensor.porte_entree", message: "ouverte" },
      { at: "2026-11-05T09:00:00Z", entity_id: "light.salon", message: "allumée" },
      { at: "2026-11-05T10:00:00Z", entity_id: "binary_sensor.porte_entree", message: "fermée" },
      { at: "pas une date", entity_id: "binary_sensor.porte_entree" },
    ];
    const log = hubV4SecurityLog(entries, ["binary_sensor.porte_entree"]);
    assert.deepEqual(log.map((e) => e.message), ["fermée", "ouverte"]);
    assert.deepEqual(plain(hubV4SecurityLog(undefined, [])), []);
  });
});

describe("hubV4Holiday", () => {
  test("la date du capteur et son nom", () => {
    const day = hubV4Holiday({ state: "2026-11-11", attributes: { nom: "Armistice" } });
    assert.equal(day.name, "Armistice");
    assert.equal(day.date.getTime(), new Date(2026, 10, 11).getTime());
  });
  test("sans nom, ou sans date", () => {
    assert.equal(hubV4Holiday({ state: "2026-12-25", attributes: {} }).name, "Jour férié");
    assert.equal(hubV4Holiday({ state: "unknown", attributes: {} }), null);
    assert.equal(hubV4Holiday(undefined), null);
  });
});

describe("hubV4WeatherSub", () => {
  test("le soir, puis le jour", () => {
    assert.equal(hubV4WeatherSub(21), "Ce soir et les jours qui viennent");
    assert.equal(hubV4WeatherSub(2), "Ce soir et les jours qui viennent");
    assert.equal(hubV4WeatherSub(9), "Aujourd'hui et les jours qui viennent");
  });
});

describe("hubV4SayData", () => {
  const announcer = { tts: { entity_id: "tts.sonos" }, speakers: ["media_player.salon", "media_player.cuisine"] };
  test("partout : le message, les enceintes, le volume", () => {
    assert.deepEqual(plain(hubV4SayData(announcer, "all", "  À table !  ", 50)), {
      entity_id: "tts.sonos", media_player_entity_id: ["media_player.salon", "media_player.cuisine"], volume: 50, message: "À table !",
    });
  });
  test("le carillon seul n'a pas besoin de message", () => {
    assert.deepEqual(plain(hubV4SayData(announcer, "chime", "ignoré", 30)), {
      entity_id: "tts.sonos", media_player_entity_id: ["media_player.salon", "media_player.cuisine"], volume: 30,
    });
  });
  test("partout sans message : rien", () => {
    assert.equal(hubV4SayData(announcer, "all", "   ", 70), null);
  });
});
