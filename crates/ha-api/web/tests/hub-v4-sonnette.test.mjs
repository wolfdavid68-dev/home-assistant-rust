// hub-v4-sonnette.js : « On sonne à la porte » et la caméra en grand (accueil v4).
//
// Usage : node --test crates/ha-api/web/tests/*.test.mjs

import assert from "node:assert/strict";
import { describe, test } from "node:test";
import { load, plain } from "./charger.mjs";

const run = load("utils.js", "hub-v3.js", "hub-v4.js", "hub-v4-pieces.js", "hub-v4-energie.js", "hub-v4-incrustations.js", "hub-v4-sonnette.js");
const hubV4DoorbellWhen = run("hubV4DoorbellWhen");
const hubV4DayTime = run("hubV4DayTime");
const hubV4DoorSensors = run("hubV4DoorSensors");
const hubV4Passages = run("hubV4Passages");

describe("hubV4DoorbellWhen", () => {
  const now = new Date(2026, 10, 19, 21, 56, 30).getTime();
  test("moins d'une minute : à l'instant", () => {
    assert.equal(hubV4DoorbellWhen(now - 20_000, now), "à l'instant");
  });
  test("plus tard : l'heure", () => {
    assert.equal(hubV4DoorbellWhen(new Date(2026, 10, 19, 21, 5).getTime(), now), "à 21:05");
  });
  test("sans heure : la dernière photo", () => {
    assert.equal(hubV4DoorbellWhen(null, now), "la dernière photo");
    assert.equal(hubV4DoorbellWhen(0, now), "la dernière photo");
  });
});

describe("hubV4DayTime", () => {
  const now = new Date(2026, 10, 19, 22, 0);
  test("aujourd'hui, hier, puis la date", () => {
    assert.equal(hubV4DayTime(new Date(2026, 10, 19, 21, 12).toISOString(), now), "aujourd'hui · 21:12");
    assert.equal(hubV4DayTime(new Date(2026, 10, 18, 11, 2).toISOString(), now), "hier · 11:02");
    assert.equal(hubV4DayTime(new Date(2026, 9, 27, 8, 15).toISOString(), now), "27/10 · 08:15");
  });
  test("une date illisible : rien", () => {
    assert.equal(hubV4DayTime("pas une date", now), "");
  });
});

describe("hubV4DoorSensors", () => {
  test("la sonnette et les capteurs de la même porte", () => {
    const ids = ["binary_sensor.entree_sonnette", "binary_sensor.entree_mouvement", "binary_sensor.garage_porte",
      "camera.entree_photo", "binary_sensor.entreesud_mouvement"];
    assert.deepEqual(plain(hubV4DoorSensors("binary_sensor.entree_sonnette", ids)),
      ["binary_sensor.entree_sonnette", "binary_sensor.entree_mouvement"]);
  });
  test("les noms anglais de ring-mqtt aussi", () => {
    assert.deepEqual(plain(hubV4DoorSensors("binary_sensor.front_ding", ["binary_sensor.front_ding", "binary_sensor.front_motion"])),
      ["binary_sensor.front_ding", "binary_sensor.front_motion"]);
  });
});

describe("hubV4Passages", () => {
  const bell = "binary_sensor.entree_sonnette";
  const ids = [bell, "binary_sensor.entree_mouvement", "light.salon"];
  const entries = [
    { entity_id: "binary_sensor.entree_mouvement", at: "2026-11-19T17:40:00Z", name: "Entrée mouvement", message: "détecte un mouvement" },
    { entity_id: "binary_sensor.entree_mouvement", at: "2026-11-19T17:42:00Z", name: "Entrée mouvement", message: "ne détecte plus de mouvement" },
    { entity_id: bell, at: "2026-11-19T20:12:00Z", name: "Entrée sonnette", message: "a sonné" },
    { entity_id: "light.salon", at: "2026-11-19T20:30:00Z", name: "Salon", message: "s'est allumé" },
    { entity_id: bell, at: "pas une date", name: "Entrée sonnette", message: "a sonné" },
  ];
  test("le plus récent d'abord, sans la fin du mouvement ni les autres pièces", () => {
    const list = plain(hubV4Passages(entries, bell, ids));
    assert.deepEqual(list.map((item) => item.text), ["On a sonné", "Mouvement devant la porte"]);
    assert.deepEqual(list.map((item) => item.kind), ["bell", "motion"]);
    assert.equal(list[0].at, "2026-11-19T20:12:00Z");
  });
  test("un autre capteur de la porte : son nom de la maison", () => {
    const door = "binary_sensor.entree_ouverture";
    const list = plain(hubV4Passages([{ entity_id: door, at: "2026-11-19T08:00:00Z", name: "x", message: "s'est ouvert" }],
      bell, [bell, door], new Map([[door, "Porte d'entrée"]])));
    assert.deepEqual(list, [{ at: "2026-11-19T08:00:00Z", text: "Porte d'entrée s'est ouvert", kind: "other" }]);
  });
  test("au plus trois, et rien d'un journal illisible", () => {
    const many = Array.from({ length: 5 }, (_, i) => ({ entity_id: bell, at: `2026-11-19T1${i}:00:00Z`, message: "a sonné" }));
    assert.equal(hubV4Passages(many, bell, ids).length, 3);
    assert.deepEqual(plain(hubV4Passages(null, bell, ids)), []);
  });
});
