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

const cover = (state, attributes = {}) => ({ entity_id: "cover.salon", state, attributes });

describe("hubV3Style", () => {
  test("seuls v3 et v4 sont retenus", () => {
    assert.equal(hubV3Style("v3"), "v3");
    assert.equal(hubV3Style("v4"), "v4");
    assert.equal(hubV3Style(null), "");
    assert.equal(hubV3Style("v9"), "");
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
