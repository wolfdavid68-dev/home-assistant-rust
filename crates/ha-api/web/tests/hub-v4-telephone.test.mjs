// hub-v4-telephone.js : l'accueil v4 en colonne, sur le téléphone.
//
// Usage : node --test crates/ha-api/web/tests/*.test.mjs

import assert from "node:assert/strict";
import { describe, test } from "node:test";
import { load, plain } from "./charger.mjs";

const run = load("utils.js", "hub-v3.js", "hub-v4.js", "hub-v4-pieces.js", "hub-v4-energie.js", "hub-v4-incrustations.js",
  "hub-v4-sonnette.js", "hub-v4-telephone.js");
const hubV4TelApproval = run("hubV4TelApproval");
const hubV4TelZoneName = run("hubV4TelZoneName");
const hubV4TelHeat = run("hubV4TelHeat");
const hubV4TelAgenda = run("hubV4TelAgenda");
const hubV4TelTempo = run("hubV4TelTempo");
const hubV4TelRoomsPage = run("hubV4TelRoomsPage");
const hubV4CanPosition = run("hubV4CanPosition");
const hubV4TelOrder = run("hubV4TelOrder");

describe("hubV4TelApproval", () => {
  test("un, plusieurs, aucun ordre", () => {
    assert.equal(hubV4TelApproval(1), "1 ordre attend ta validation");
    assert.equal(hubV4TelApproval(3), "3 ordres attendent ta validation");
    assert.equal(hubV4TelApproval(0), "");
    assert.equal(hubV4TelApproval(undefined), "");
  });
});

describe("hubV4TelZoneName", () => {
  test("sans le mot chauffage", () => {
    assert.equal(hubV4TelZoneName("Chauffage étage"), "étage");
    assert.equal(hubV4TelZoneName("Radiateurs salon"), "salon");
    assert.equal(hubV4TelZoneName("Chauffage"), "Chauffage");
    assert.equal(hubV4TelZoneName("Bureau"), "Bureau");
  });
});

describe("hubV4TelHeat", () => {
  test("deux zones au plus, arrondies", () => {
    const zones = [
      { name: "Chauffage rez-de-chaussée", temperature: 19.4 },
      { name: "Chauffage étage", temperature: 17.6 },
      { name: "Chauffage combles", temperature: 15 },
    ];
    assert.deepEqual(plain(hubV4TelHeat(zones)), { value: "19° · 18°", sub: "rez-de-chaussée · étage" });
  });
  test("sans température lisible : pas de carte", () => {
    assert.equal(hubV4TelHeat([{ name: "Chauffage", temperature: NaN }]), null);
    assert.equal(hubV4TelHeat([]), null);
  });
});

describe("hubV4TelAgenda", () => {
  const now = new Date(2026, 10, 19, 14, 0);
  const at = (day, hour, minute = 0) => new Date(2026, 10, day, hour, minute);
  test("rien aujourd'hui, rien ensuite", () => {
    assert.deepEqual(plain(hubV4TelAgenda([], now)), { today: "Rien aujourd'hui", later: "" });
  });
  test("le rendez-vous en cours, puis celui de demain", () => {
    const events = [
      { summary: "Piscine", startDate: at(20, 9, 5), endDate: at(20, 10), allDay: false },
      { summary: "Réunion", startDate: at(19, 13, 30), endDate: at(19, 15), allDay: false },
    ];
    assert.deepEqual(plain(hubV4TelAgenda(events, now)), { today: "En cours · Réunion", later: "Piscine demain 09:05" });
  });
  test("le prochain d'aujourd'hui ; un fini ne compte plus", () => {
    const events = [
      { summary: "Courses", startDate: at(19, 9), endDate: at(19, 10), allDay: false },
      { summary: "Dentiste", startDate: at(19, 17, 30), endDate: at(19, 18), allDay: false },
    ];
    assert.deepEqual(plain(hubV4TelAgenda(events, now)), { today: "17:30 · Dentiste", later: "" });
  });
  test("journées entières : aujourd'hui, puis un jour plus loin (comme la maquette)", () => {
    const birthday = { summary: "Anniversaire", startDate: at(19, 0), endDate: at(20, 0), allDay: true };
    const holiday = { summary: "Toussaint", startDate: new Date(2026, 10, 22), endDate: new Date(2026, 10, 23), allDay: true };
    assert.deepEqual(plain(hubV4TelAgenda([holiday, birthday], now)), { today: "Anniversaire aujourd'hui", later: "Toussaint dim. 22 nov." });
    assert.deepEqual(plain(hubV4TelAgenda([holiday], now)), { today: "Rien aujourd'hui", later: "Toussaint dim. 22 nov." });
  });
});

describe("hubV4TelTempo", () => {
  test("l'adjectif avec sa majuscule", () => {
    assert.equal(hubV4TelTempo("bleu", "aujourd'hui"), "Bleu aujourd'hui");
    assert.equal(hubV4TelTempo("rouge", "demain"), "Rouge demain");
    assert.equal(hubV4TelTempo("", "demain"), "");
    assert.equal(hubV4TelTempo(undefined, "demain"), "");
  });
});

describe("hubV4TelRoomsPage", () => {
  const outside = { id: "dehors", zones: [] };
  const ground = { id: "rdc", zones: [{ area: "Salon" }] };
  const upstairs = { id: "etage", zones: [{ area: "Chambre" }] };
  const pages = [outside, ground, upstairs];
  test("la vue de dehors n'a pas de pièce : le premier étage qui en a", () => {
    assert.equal(hubV4TelRoomsPage(pages, "dehors"), "rdc");
  });
  test("un étage affiché reste affiché", () => {
    assert.equal(hubV4TelRoomsPage(pages, "etage"), "etage");
  });
  test("une zone d'appareil seul ne compte pas ; aucune pièce : null", () => {
    assert.equal(hubV4TelRoomsPage([outside, { id: "jardin", zones: [{ entity_id: "light.jardin" }] }], "dehors"), null);
    assert.equal(hubV4TelRoomsPage([], undefined), null);
  });
});

describe("hubV4CanPosition", () => {
  const cover = (attributes) => ({ attributes });
  test("supported_features en nombre : le bit 4", () => {
    assert.equal(hubV4CanPosition(cover({ supported_features: 15, current_position: 40 })), true);
    assert.equal(hubV4CanPosition(cover({ supported_features: 11, current_position: 40 })), false);
  });
  test("supported_features en liste", () => {
    assert.equal(hubV4CanPosition(cover({ supported_features: ["open", "close", "set_position"] })), true);
    assert.equal(hubV4CanPosition(cover({ supported_features: ["open", "close"], current_position: 40 })), false);
  });
  test("sans supported_features : la position donnée", () => {
    assert.equal(hubV4CanPosition(cover({ current_position: 40 })), true);
    assert.equal(hubV4CanPosition(cover({})), false);
    assert.equal(hubV4CanPosition(undefined), false);
  });
});

describe("hubV4TelOrder", () => {
  const rooms = ["Salon", "Cuisine", "Chambre"];
  test("pièce par pièce, dans l'ordre des pièces, puis par nom", () => {
    const items = [
      { id: "cover.chambre", name: "Volet chambre", area: "Chambre" },
      { id: "cover.baie", name: "Volet baie", area: "Salon" },
      { id: "cover.cuisine", name: "Volet cuisine", area: "Cuisine" },
      { id: "cover.angle", name: "Volet angle", area: "Salon" },
    ];
    assert.deepEqual(plain(hubV4TelOrder(items, rooms)), ["cover.angle", "cover.baie", "cover.cuisine", "cover.chambre"]);
  });
  test("sans pièce : à la fin", () => {
    const items = [
      { id: "climate.maison", name: "Chauffage", area: "" },
      { id: "climate.salon", name: "Chauffage salon", area: "Salon" },
    ];
    assert.deepEqual(plain(hubV4TelOrder(items, rooms)), ["climate.salon", "climate.maison"]);
    assert.deepEqual(plain(hubV4TelOrder([], rooms)), []);
  });
});
