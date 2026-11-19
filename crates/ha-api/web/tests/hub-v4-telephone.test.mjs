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
const hubV4TelNext = run("hubV4TelNext");
const hubV4TelRoomsPage = run("hubV4TelRoomsPage");
const hubV4CanPosition = run("hubV4CanPosition");

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

describe("hubV4TelNext", () => {
  const now = new Date(2026, 10, 19, 14, 0);
  const at = (day, hour, minute = 0) => new Date(2026, 10, day, hour, minute);
  test("le rendez-vous en cours", () => {
    const events = [{ summary: "Réunion", startDate: at(19, 13, 30), endDate: at(19, 15), allDay: false }];
    assert.deepEqual(plain(hubV4TelNext(events, now)), { value: "En cours", sub: "Réunion" });
  });
  test("le prochain, aujourd'hui puis demain", () => {
    const later = { summary: "Dentiste", startDate: at(19, 17, 30), endDate: at(19, 18), allDay: false };
    const tomorrow = { summary: "Piscine", startDate: at(20, 9, 5), endDate: at(20, 10), allDay: false };
    assert.deepEqual(plain(hubV4TelNext([tomorrow, later], now)), { value: "17:30", sub: "Dentiste" });
    assert.deepEqual(plain(hubV4TelNext([tomorrow], now)), { value: "Demain 09:05", sub: "Piscine" });
  });
  test("un rendez-vous fini ne compte plus ; la journée entière ensuite", () => {
    const done = { summary: "Courses", startDate: at(19, 9), endDate: at(19, 10), allDay: false };
    const day = { summary: "Anniversaire", startDate: at(19, 0), endDate: at(20, 0), allDay: true };
    assert.deepEqual(plain(hubV4TelNext([done, day], now)), { value: "Aujourd'hui", sub: "Anniversaire" });
    assert.equal(hubV4TelNext([done], now), null);
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
