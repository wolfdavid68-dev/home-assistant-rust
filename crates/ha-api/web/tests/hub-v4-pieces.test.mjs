// hub-v4-pieces.js : les pages Pièce et « Toute la maison » de l'accueil v4.
//
// Usage : node --test crates/ha-api/web/tests/*.test.mjs

import assert from "node:assert/strict";
import { describe, test } from "node:test";
import { load, plain } from "./charger.mjs";

const run = load("utils.js", "hub-v3.js", "hub-v4.js", "hub-v4-pieces.js");
run(`const shortTime = new Intl.DateTimeFormat("fr", { hour: "2-digit", minute: "2-digit" });`);
const hubV4FloorRooms = run("hubV4FloorRooms");
const hubV4PageOf = run("hubV4PageOf");
const hubV4AllOf = run("hubV4AllOf");
const hubV4HeatFor = run("hubV4HeatFor");
const hubV4Zoom = run("hubV4Zoom");
const hubV4RoomLine = run("hubV4RoomLine");
const hubV4RoomCount = run("hubV4RoomCount");
const hubV4HouseLine = run("hubV4HouseLine");
const hubV4SilentSince = run("hubV4SilentSince");

const rez = {
  name: "Rez-de-chaussée",
  zones: [
    { area: "Cuisine", x: 50, y: 56, w: 30, h: 22 },
    { area: "Salon", x: 18, y: 56, w: 30, h: 22 },
    { entity_id: "light.salon", x: 20, y: 60, w: 2, h: 2 },
    { area: "Entrée", x: 56, y: 61, w: 7, h: 17 },
    { area: "Salon", x: 10, y: 40, w: 4, h: 4 },
  ],
};
const etage = { name: "Étage", zones: [{ area: "Chambre", x: 20, y: 20, w: 30, h: 30 }] };

describe("hubV4FloorRooms", () => {
  test("de gauche à droite, une fois chacune, d'après la plus grande zone", () => {
    assert.deepEqual(plain(hubV4FloorRooms(rez)), ["Salon", "Cuisine", "Entrée"]);
  });
  test("sans onglet, aucune pièce", () => {
    assert.deepEqual(plain(hubV4FloorRooms(null)), []);
  });
});

describe("hubV4PageOf", () => {
  test("l'onglet qui dessine la pièce", () => {
    assert.equal(hubV4PageOf([rez, etage], "Chambre"), etage);
    assert.equal(hubV4PageOf([rez, etage], "Garage"), null);
  });
});

describe("hubV4AllOf", () => {
  test("l'article suit l'étage", () => {
    assert.equal(hubV4AllOf("Rez-de-chaussée"), "tout le rez-de-chaussée");
    assert.equal(hubV4AllOf("Étage"), "tout l'étage");
    assert.equal(hubV4AllOf(""), "toute la maison");
  });
});

describe("hubV4HeatFor", () => {
  const zones = [
    { id: "climate.rez", name: "Chauffage rez", area: null },
    { id: "climate.etage", name: "Chauffage étage", area: null },
    { id: "climate.sdb", name: "Sèche-serviettes", area: "Salle de bain" },
  ];
  test("le chauffage de la pièce d'abord", () => {
    assert.deepEqual(plain(hubV4HeatFor(zones, "Salle de bain", "Étage", [])), { zone: zones[2], shared: false });
  });
  test("sinon celui dont le nom dit l'étage", () => {
    assert.equal(hubV4HeatFor(zones, "Salon", "Rez-de-chaussée", ["Salon"]).zone.id, "climate.rez");
    assert.equal(hubV4HeatFor(zones, "Chambre", "Étage", ["Chambre"]).zone.id, "climate.etage");
    assert.equal(hubV4HeatFor(zones, "Chambre", "Étage", ["Chambre"]).shared, true);
  });
  test("sinon celui rangé dans une pièce du même étage", () => {
    const placed = [{ id: "climate.pac", name: "PAC", area: "Cuisine" }];
    assert.equal(hubV4HeatFor(placed, "Salon", "Rez-de-chaussée", ["Salon", "Cuisine"]).zone.id, "climate.pac");
  });
  test("sinon le seul de la maison", () => {
    const one = [{ id: "climate.maison", name: "Thermostat", area: null }];
    assert.equal(hubV4HeatFor(one, "Salon", "", []).zone.id, "climate.maison");
    assert.equal(hubV4HeatFor(zones.slice(0, 2), "Garage", "", []), null);
  });
});

describe("hubV4Zoom", () => {
  test("la pièce tient au centre de la scène", () => {
    const at = hubV4Zoom({ x: 20, y: 40, w: 20, h: 20 }, { w: 1000, h: 800 }, 1);
    // Largeur limitée par la hauteur : 0,42 × 800 / 0,2 = 1680 px.
    assert.equal(at.width, 1680);
    assert.equal(at.ring.width, 336);
    assert.equal(at.ring.left + at.ring.width / 2, 500);
    assert.equal(at.ring.top + at.ring.height / 2, 400);
  });
});

describe("hubV4RoomLine", () => {
  test("lumières puis volets", () => {
    assert.equal(hubV4RoomLine(2, 3, "Fermé", 1), "2 sur 3 allumées · volet fermé");
    assert.equal(hubV4RoomLine(0, 3, "Ouverts", 2), "Toutes éteintes · volets ouverts");
    assert.equal(hubV4RoomLine(1, 1, "", 0), "Lumière allumée");
    assert.equal(hubV4RoomLine(0, 0, "", 0), "");
  });
});

describe("hubV4RoomCount", () => {
  test("n/m, ou un tiret sans lampe", () => {
    assert.equal(hubV4RoomCount(2, 3), "2/3");
    assert.equal(hubV4RoomCount(0, 0), "–");
  });
});

describe("hubV4HouseLine", () => {
  test("le résumé de la maison", () => {
    assert.equal(hubV4HouseLine({ lights: 29, lit: 23, covers: 9, open: 2 }, 0), "23 sur 29 allumées · 7 volets fermés · tout est calme");
    assert.equal(hubV4HouseLine({ lights: 4, lit: 0, covers: 2, open: 2 }, 1), "Toutes les lumières éteintes · volets ouverts · 1 chose à voir");
    assert.equal(hubV4HouseLine({ lights: 0, lit: 0, covers: 0, open: 0 }, 0), "Tout est calme");
  });
});

describe("hubV4SilentSince", () => {
  const now = new Date(2026, 10, 19, 18, 0);
  test("l'heure du jour, hier, ou plus", () => {
    assert.equal(hubV4SilentSince(new Date(2026, 10, 19, 14, 5).toISOString(), now), "depuis 14:05");
    assert.equal(hubV4SilentSince(new Date(2026, 10, 18, 22, 0).toISOString(), now), "depuis hier");
    assert.equal(hubV4SilentSince(new Date(2026, 10, 10).toISOString(), now), "depuis plusieurs jours");
    assert.equal(hubV4SilentSince("n'importe quoi", now), "");
  });
});
