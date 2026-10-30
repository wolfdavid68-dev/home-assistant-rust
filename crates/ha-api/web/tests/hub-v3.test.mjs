// hub-v3.js : l'accueil v3 (salut, résumé, lentilles de la maison).
//
// Usage : node --test crates/ha-api/web/tests/*.test.mjs

import assert from "node:assert/strict";
import { describe, test } from "node:test";
import { load, plain } from "./charger.mjs";

const run = load("utils.js", "hub-v3.js");
const hubV3Hello = run("hubV3Hello");
const hubV3Attention = run("hubV3Attention");
const hubV3Count = run("hubV3Count");
const hubV3Kind = run("hubV3Kind");
const hubV3ZoneCenter = run("hubV3ZoneCenter");
const hubV3PinLabel = run("hubV3PinLabel");

describe("hubV3Hello", () => {
  test("le salut suit l'heure", () => {
    assert.equal(hubV3Hello(4), "Bonsoir.");
    assert.equal(hubV3Hello(5), "Bonjour.");
    assert.equal(hubV3Hello(17), "Bonjour.");
    assert.equal(hubV3Hello(18), "Bonsoir.");
    assert.equal(hubV3Hello(0), "Bonsoir.");
    assert.equal(hubV3Hello(23), "Bonsoir.");
  });
});

describe("hubV3Attention", () => {
  test("rien, une chose, ou le compte", () => {
    assert.equal(hubV3Attention(0), "Tout est calme.");
    assert.equal(hubV3Attention(1), "Une chose demande ton attention.");
    assert.equal(hubV3Attention(3), "3 choses demandent ton attention.");
  });
});

describe("hubV3Count", () => {
  test("le singulier et le pluriel", () => {
    assert.equal(hubV3Count(1, "lumière allumée", "lumières allumées"), "1 lumière allumée");
    assert.equal(hubV3Count(2, "lumière allumée", "lumières allumées"), "2 lumières allumées");
  });
});

describe("hubV3Kind", () => {
  test("lumières, sauf les prises", () => {
    assert.equal(hubV3Kind("light", undefined), "lumieres");
    assert.equal(hubV3Kind("light", "outlet"), "");
  });

  test("volets et chauffage", () => {
    assert.equal(hubV3Kind("cover", undefined), "volets");
    assert.equal(hubV3Kind("climate", undefined), "chauffage");
    assert.equal(hubV3Kind("sensor", "temperature"), "chauffage");
    assert.equal(hubV3Kind("sensor", "power"), "");
  });

  test("musique", () => {
    assert.equal(hubV3Kind("media_player", undefined), "musique");
  });

  test("sécurité : ouvertures et serrures", () => {
    assert.equal(hubV3Kind("binary_sensor", "door"), "securite");
    assert.equal(hubV3Kind("binary_sensor", "battery"), "");
    assert.equal(hubV3Kind("lock", undefined), "securite");
    assert.equal(hubV3Kind("switch", undefined), "");
  });
});

test("HUB_V3_LENSES : les six lentilles, dans l'ordre", () => {
  const lenses = plain(run("HUB_V3_LENSES")).map((l) => l.id);
  assert.deepEqual(lenses, ["vivant", "lumieres", "volets", "chauffage", "musique", "securite"]);
});

describe("hubV3ZoneCenter", () => {
  test("le milieu de la zone, en %", () => {
    assert.deepEqual(plain(hubV3ZoneCenter({ x: 10, y: 20, w: 30, h: 10 })), { x: 25, y: 25 });
  });
});

describe("hubV3PinLabel", () => {
  test("éteint, une seule, toutes, une partie", () => {
    assert.equal(hubV3PinLabel("Salon", 0, 3), "Salon : tout est éteint. Toucher pour allumer.");
    assert.equal(hubV3PinLabel("Cuisine", 1, 1), "Cuisine : la lumière est allumée. Toucher pour éteindre.");
    assert.equal(hubV3PinLabel("Salon", 3, 3), "Salon : les 3 lumières sont allumées. Toucher pour éteindre.");
    assert.equal(hubV3PinLabel("Salon", 1, 3), "Salon : 1 lumière allumée sur 3. Toucher pour éteindre.");
  });

  test("sans droit de commande, pas d'invitation à toucher", () => {
    assert.equal(hubV3PinLabel("Salon", 2, 3, false), "Salon : 2 lumières allumées sur 3.");
  });
});
