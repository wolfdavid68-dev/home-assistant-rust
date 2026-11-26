// hub-v4.js : l'accueil v4 (incrustations).
//
// Usage : node --test crates/ha-api/web/tests/*.test.mjs

import assert from "node:assert/strict";
import { describe, test } from "node:test";
import { load, plain } from "./charger.mjs";

const run = load("utils.js", "hub-v3.js", "hub-v4.js");
const hubV3Style = run("hubV3Style");
const hubV4LightsText = run("hubV4LightsText");
const hubV4ShortName = run("hubV4ShortName");
const hubV4RoomSub = run("hubV4RoomSub");
const hubV4CoverWord = run("hubV4CoverWord");
const hubV4Place = run("hubV4Place");
const hubV4Percent = run("hubV4Percent");
const hubV4Brightness = run("hubV4Brightness");
const hubV4Dimmable = run("hubV4Dimmable");
const hubV4WhiteRange = run("hubV4WhiteRange");
const hubV4ClosedPart = run("hubV4ClosedPart");
const hubV4CoverLabel = run("hubV4CoverLabel");
const hubV4MomentRows = run("hubV4MomentRows");
const hubV4MomentHeat = run("hubV4MomentHeat");
const hubV4Expiry = run("hubV4Expiry");
const hubV4AskedBy = run("hubV4AskedBy");
const hubV4Suggested = run("hubV4Suggested");
const hubV4TempoText = run("hubV4TempoText");
const hubV4CoursesText = run("hubV4CoursesText");

const cover = (state, attributes = {}) => ({ entity_id: "cover.salon", state, attributes });

describe("hubV3Style", () => {
  test("v4 si choisie, sinon v3 (la maquette, par défaut)", () => {
    assert.equal(hubV3Style("v3"), "v3");
    assert.equal(hubV3Style("v4"), "v4");
    assert.equal(hubV3Style(null), "v3");
    assert.equal(hubV3Style(""), "v3");
    assert.equal(hubV3Style("v9"), "v3");
  });
});

describe("hubV4LightsText", () => {
  test("une seule lampe", () => {
    assert.equal(hubV4LightsText(1, 1), "Allumée");
    assert.equal(hubV4LightsText(0, 1), "Éteinte");
  });
  test("plusieurs lampes", () => {
    assert.equal(hubV4LightsText(0, 5), "Toutes éteintes");
    assert.equal(hubV4LightsText(5, 5), "Toutes allumées");
    assert.equal(hubV4LightsText(1, 5), "1 sur 5 allumée");
    assert.equal(hubV4LightsText(3, 5), "3 sur 5 allumées");
  });
});

describe("hubV4ShortName", () => {
  test("le nom de la pièce s'efface devant celui de la lampe", () => {
    assert.equal(hubV4ShortName("Salon plafond", "Salon"), "Plafond");
    assert.equal(hubV4ShortName("salon lampe de lecture", "Salon"), "Lampe de lecture");
  });
  test("sinon, le nom reste", () => {
    assert.equal(hubV4ShortName("Lampadaire", "Salon"), "Lampadaire");
    assert.equal(hubV4ShortName("Salon", "Salon"), "Salon");
    assert.equal(hubV4ShortName("Salonnière", "Salon"), "Salonnière");
  });
});

describe("hubV4RoomSub", () => {
  test("l'étage et la température", () => {
    assert.equal(hubV4RoomSub("Rez-de-chaussée", 21.3), "Rez-de-chaussée · 21,3°");
    assert.equal(hubV4RoomSub("", 19), "19°");
    assert.equal(hubV4RoomSub("Étage", undefined), "Étage");
    assert.equal(hubV4RoomSub("", NaN), "");
  });
});

describe("hubV4CoverWord", () => {
  test("un volet", () => {
    assert.equal(hubV4CoverWord([cover("open")]), "Ouvert");
    assert.equal(hubV4CoverWord([cover("open", { current_position: 40 })]), "Ouvert à 40 %");
    assert.equal(hubV4CoverWord([cover("closed")]), "Fermé");
    assert.equal(hubV4CoverWord([cover("closing")]), "En mouvement");
    assert.equal(hubV4CoverWord([cover("unavailable")]), "Indisponible");
  });
  test("plusieurs volets", () => {
    assert.equal(hubV4CoverWord([cover("open"), cover("open")]), "Ouverts");
    assert.equal(hubV4CoverWord([cover("closed"), cover("closed")]), "Fermés");
    assert.equal(hubV4CoverWord([cover("open"), cover("closed")]), "Ouverts en partie");
  });
});

describe("hubV4Place", () => {
  const screen = { w: 1280, h: 800 };
  const pop = { w: 320, h: 300 };

  test("sans pièce : au centre, sans flèche", () => {
    assert.deepEqual(plain(hubV4Place(null, pop, screen)), { x: 480, y: 250, side: "", ay: 0 });
  });
  test("une pièce à gauche : la fenêtre à sa droite, la flèche en face", () => {
    const at = plain(hubV4Place({ left: 300, top: 300, width: 200, height: 100 }, pop, screen));
    assert.deepEqual(at, { x: 516, y: 200, side: "right", ay: 150 });
  });
  test("une pièce à droite : la fenêtre à sa gauche", () => {
    const at = hubV4Place({ left: 900, top: 300, width: 200, height: 100 }, pop, screen);
    assert.equal(at.side, "left");
    assert.equal(at.x, 900 - 16 - 320);
  });
  test("pas la place à droite : à gauche", () => {
    const at = hubV4Place({ left: 400, top: 300, width: 600, height: 100 }, pop, screen);
    assert.equal(at.side, "left");
  });
  test("la fenêtre reste dans l'écran, la flèche suit la pièce", () => {
    const at = hubV4Place({ left: 100, top: 20, width: 100, height: 60 }, pop, screen);
    assert.equal(at.y, 8);
    assert.equal(at.ay, 42);
  });
  test("pas la place sur les côtés : centrée sur la pièce, sans flèche", () => {
    const at = hubV4Place({ left: 10, top: 300, width: 340, height: 100 }, pop, { w: 360, h: 800 });
    assert.equal(at.side, "");
    assert.equal(at.x, 20);
  });
});

const lamp = (state, attributes = {}) => ({ entity_id: "light.salon", state, attributes });

describe("hubV4Percent et hubV4Brightness", () => {
  test("éteinte : 0 ; allumée sans luminosité : 100", () => {
    assert.equal(hubV4Percent(lamp("off", { brightness: 200 })), 0);
    assert.equal(hubV4Percent(lamp("on")), 100);
  });
  test("la luminosité en pour cent, jamais 0 pour une lampe allumée", () => {
    assert.equal(hubV4Percent(lamp("on", { brightness: 255 })), 100);
    assert.equal(hubV4Percent(lamp("on", { brightness: 128 })), 50);
    assert.equal(hubV4Percent(lamp("on", { brightness: 1 })), 1);
  });
  test("pour cent → 1 à 255", () => {
    assert.equal(hubV4Brightness(100), 255);
    assert.equal(hubV4Brightness(50), 127);
    assert.equal(hubV4Brightness(5), 13);
    assert.equal(hubV4Brightness(0), 1);
  });
});

describe("hubV4Dimmable et hubV4WhiteRange", () => {
  test("une lampe seulement allumée ou éteinte ne varie pas", () => {
    assert.equal(hubV4Dimmable(lamp("on", { supported_color_modes: ["onoff"] })), false);
    assert.equal(hubV4Dimmable(lamp("on", { supported_color_modes: ["brightness"] })), true);
    assert.equal(hubV4Dimmable(lamp("on", { brightness: 80 })), true);
    assert.equal(hubV4Dimmable(lamp("on")), false);
  });
  test("le blanc, seulement si la lampe le règle", () => {
    assert.equal(hubV4WhiteRange(lamp("on", { supported_color_modes: ["brightness"] })), null);
    assert.deepEqual(plain(hubV4WhiteRange(lamp("on", { supported_color_modes: ["color_temp"] }))), { min: 2000, max: 6500 });
    assert.deepEqual(plain(hubV4WhiteRange(lamp("on", {
      supported_color_modes: ["color_temp"], min_color_temp_kelvin: 2700, max_color_temp_kelvin: 5000,
    }))), { min: 2700, max: 5000 });
  });
});

describe("hubV4ClosedPart et hubV4CoverLabel", () => {
  test("la position connue d'abord, sinon l'état", () => {
    assert.equal(hubV4ClosedPart(cover("open", { current_position: 40 })), 60);
    assert.equal(hubV4ClosedPart(cover("closed")), 100);
    assert.equal(hubV4ClosedPart(cover("open")), 0);
    assert.equal(hubV4ClosedPart(cover("unknown")), 50);
  });
  test("ce qu'affiche la fenêtre", () => {
    assert.deepEqual(plain(hubV4CoverLabel(cover("open", { current_position: 40 }))), { big: "40 %", small: "ouvert" });
    assert.deepEqual(plain(hubV4CoverLabel(cover("closed", { current_position: 0 }))), { big: "Fermé", small: "" });
    assert.deepEqual(plain(hubV4CoverLabel(cover("open"))), { big: "Ouvert", small: "" });
  });
});

describe("hubV4MomentRows", () => {
  const house = new Map([
    ["light.salon", { entity_id: "light.salon", state: "on", attributes: { brightness: 255 } }],
    ["light.coin", { entity_id: "light.coin", state: "off", attributes: {} }],
    ["cover.salon", { entity_id: "cover.salon", state: "open", attributes: { current_position: 100 } }],
    ["switch.tele", { entity_id: "switch.tele", state: "off", attributes: {} }],
  ]);
  const get = (id) => house.get(id);

  test("maintenant → après, par sorte d'appareil", () => {
    const rows = plain(hubV4MomentRows({
      "light.salon": { state: "on", brightness: 77 },
      "light.coin": { state: "on", brightness: 77 },
      "cover.salon": { state: "closed" },
    }, get));
    assert.deepEqual(rows, [
      { domain: "light", label: "Lumières", now: "1 sur 2 allumée", then: "toutes allumées à 30 %", done: false },
      { domain: "cover", label: "Volets", now: "ouvert", then: "fermé", done: false },
    ]);
  });
  test("ce qui est déjà fait le dit", () => {
    const rows = plain(hubV4MomentRows({ "switch.tele": { state: "off" }, "light.salon": { state: "on", brightness: 255 } }, get));
    assert.deepEqual(rows, [
      { domain: "light", label: "Lumières", now: "allumée", then: "déjà allumée", done: true },
      { domain: "switch", label: "Prises", now: "éteinte", then: "déjà éteinte", done: true },
    ]);
  });
  test("une scène qui ne dit rien ne donne aucune ligne", () => {
    assert.deepEqual(plain(hubV4MomentRows(undefined, get)), []);
    assert.deepEqual(plain(hubV4MomentRows({ "media_player.radio": { state: "on" } }, get)), []);
  });  test("le chauffage et la musique qui s'arrête", () => {
    const more = new Map([
      ["climate.etage", { entity_id: "climate.etage", state: "heat", attributes: { friendly_name: "Étage", preset_mode: "confort" } }],
      ["media_player.salon", { entity_id: "media_player.salon", state: "playing", attributes: { friendly_name: "Salle à manger" } }],
    ]);
    const rows = plain(hubV4MomentRows({
      "climate.etage": { state: "on", preset_mode: "eco" },
      "media_player.salon": { state: "off" },
    }, (id) => more.get(id)));
    assert.deepEqual(rows, [
      { domain: "climate", label: "Chauffage", now: "en confort", then: "Eco", done: false },
      { domain: "media_player", label: "Musique", now: "Salle à manger", then: "s'arrête", done: false },
    ]);
  });
});

describe("hubV4MomentHeat", () => {
  const heat = (state, attributes = {}) => ({ state, attributes });
  test("un préréglage, une consigne, ou coupé", () => {
    assert.equal(hubV4MomentHeat([{ state: "on", temperature: 19 }], [heat("heat", { temperature: 21 })]).then, "19°");
    assert.equal(hubV4MomentHeat([{ state: "off" }], [heat("heat")]).then, "coupé");
    assert.equal(hubV4MomentHeat([{ state: "on", hvac_mode: "off" }], [heat("off")]).then, "déjà coupé");
  });
  test("déjà fait", () => {
    const row = plain(hubV4MomentHeat([{ state: "on", preset_mode: "nuit" }], [heat("heat", { preset_mode: "nuit" })]));
    assert.deepEqual(row, { now: "en nuit", then: "déjà nuit", done: true });
  });
  test("une cible qui ne dit rien : pas de ligne", () => {
    assert.equal(hubV4MomentHeat([{ state: "on" }], [heat("heat")]), null);
    assert.equal(hubV4MomentHeat([{ state: "on", preset_mode: "eco" }, { state: "on", preset_mode: "confort" }], []), null);
  });
});

describe("hubV4Expiry et hubV4AskedBy", () => {
  const now = Date.parse("2026-11-19T20:00:00Z");
  test("le temps qu'il reste avant le refus", () => {
    assert.equal(hubV4Expiry("2026-11-19T20:10:00Z", now), "Sans réponse, il est refusé dans 10 min.");
    assert.equal(hubV4Expiry("2026-11-19T20:00:30Z", now), "Sans réponse, il est refusé dans moins d'une minute.");
    assert.equal(hubV4Expiry(undefined, now), "");
  });
  test("qui a demandé, et à quelle heure (locale)", () => {
    const at = new Date(2026, 10, 19, 21, 42).toISOString();
    assert.equal(hubV4AskedBy("à la voix (Alexa)", at), "Demandé à la voix (Alexa) · 21:42");
    assert.equal(hubV4AskedBy("", at), "Demandé à 21:42");
    assert.equal(hubV4AskedBy("", "n'importe quoi"), "Demandé");
  });
});

describe("hubV4Suggested : la scène de l'heure (Moments)", () => {
  const names = ["Bonjour", "Bonne nuit", "Cinéma", "Je pars", "Je rentre", "Soirée"];
  test("le matin, le midi, le soir, la nuit", () => {
    assert.equal(hubV4Suggested(names, 7), 0);
    assert.equal(hubV4Suggested(names, 13), 3);
    assert.equal(hubV4Suggested(names, 18), 4);
    assert.equal(hubV4Suggested(names, 22), 1);
    assert.equal(hubV4Suggested(names, 2), 1);
  });
  test("rien qui corresponde : pas de suggestion", () => {
    assert.equal(hubV4Suggested(["Cinéma", "Lecture"], 7), -1);
    assert.equal(hubV4Suggested([], 22), -1);
  });
});

describe("hubV4TempoText et hubV4CoursesText", () => {
  test("la couleur du jour, puis le jour", () => {
    assert.equal(hubV4TempoText("Bleu", "aujourd'hui"), "Bleu aujourd'hui");
    assert.equal(hubV4TempoText("Rouge", "demain"), "Rouge demain");
  });
  test("pas encore annoncé, ou un état inconnu", () => {
    assert.equal(hubV4TempoText("unknown", "demain"), "Demain : pas encore annoncé");
    assert.equal(hubV4TempoText("unavailable", "aujourd'hui"), "Aujourd'hui : indisponible");
  });
  test("la liste des courses : vide ou le nombre", () => {
    assert.equal(hubV4CoursesText(0), "vides");
    assert.equal(hubV4CoursesText(3), "3");
  });
});
