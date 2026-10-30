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
const hubV3CoversText = run("hubV3CoversText");
const hubV3CoverSummary = run("hubV3CoverSummary");
const hubV3SecurityText = run("hubV3SecurityText");
const hubV3Temp = run("hubV3Temp");
const hubV3Clean = run("hubV3Clean");
const hubV3MusicTitle = run("hubV3MusicTitle");
const hubV3SecurityShort = run("hubV3SecurityShort");
const hubV3CoverLabel = run("hubV3CoverLabel");
const hubV3IsShutter = run("hubV3IsShutter");
const hubV3IsSecurity = run("hubV3IsSecurity");

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
    assert.equal(hubV3Kind("cover", "shutter"), "volets");
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

  test("portes de garage et portails : sécurité, pas volets", () => {
    assert.equal(hubV3Kind("cover", "garage"), "securite");
    assert.equal(hubV3Kind("cover", "gate"), "securite");
    assert.equal(hubV3Kind("cover", "door"), "securite");
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

describe("hubV3CoversText", () => {
  test("aucun, tous fermés, tous ouverts, une partie", () => {
    assert.equal(hubV3CoversText(0, 0), "Aucun volet");
    assert.equal(hubV3CoversText(0, 1), "Le volet est fermé");
    assert.equal(hubV3CoversText(0, 7), "Les 7 volets sont fermés");
    assert.equal(hubV3CoversText(1, 1), "Le volet est ouvert");
    assert.equal(hubV3CoversText(3, 3), "Les 3 volets sont ouverts");
    assert.equal(hubV3CoversText(1, 3), "1 ouvert · 2 fermés");
    assert.equal(hubV3CoversText(2, 3), "2 ouverts · 1 fermé");
  });
});

describe("hubV3CoverSummary", () => {
  test("les volets d'une pièce", () => {
    assert.equal(hubV3CoverSummary(0, 2), "Fermés");
    assert.equal(hubV3CoverSummary(2, 2), "Ouverts");
    assert.equal(hubV3CoverSummary(1, 3), "1 ouvert sur 3");
  });
});

describe("hubV3SecurityText", () => {
  test("en ordre, une ou deux choses, ou le compte", () => {
    assert.equal(hubV3SecurityText([]), "Tout est en ordre");
    assert.equal(hubV3SecurityText(["Fenêtre ouverte"]), "Fenêtre ouverte");
    assert.equal(hubV3SecurityText(["A", "B"]), "A · B");
    assert.equal(hubV3SecurityText(["A", "B", "C"]), "3 choses à vérifier");
  });
});

describe("hubV3Temp", () => {
  test("une décimale au plus, à la française", () => {
    assert.equal(hubV3Temp(21.2), "21,2°");
    assert.equal(hubV3Temp(20), "20°");
  });
});

test("HUB_V3_PIN_LENSES et HUB_V3_READ_ICONS : les cinq lentilles à pastilles", () => {
  const pins = plain(run("HUB_V3_PIN_LENSES"));
  assert.deepEqual(Object.keys(pins), ["lumieres", "volets", "chauffage", "musique", "securite"]);
  for (const lens of Object.values(pins)) assert.equal(typeof lens.icon, "string");
  assert.deepEqual(Object.keys(plain(run("HUB_V3_READ_ICONS"))), Object.keys(pins));
});

describe("hubV3Clean", () => {
  test("une chaîne nettoyée, sinon rien", () => {
    assert.equal(hubV3Clean(" a "), "a");
    assert.equal(hubV3Clean(3), "");
  });
});

describe("hubV3MusicTitle", () => {
  test("le titre, la radio, rien à l'arrêt", () => {
    assert.equal(hubV3MusicTitle({ state: "playing", attributes: { media_title: " Titre " } }), "Titre");
    assert.equal(hubV3MusicTitle({ state: "paused", attributes: { media_channel: "Radio" } }), "Radio");
    assert.equal(hubV3MusicTitle({ state: "idle", attributes: { media_title: "Titre" } }), "");
  });
});

describe("hubV3SecurityShort", () => {
  test("en ordre ou le compte", () => {
    assert.equal(hubV3SecurityShort(0), "En ordre");
    assert.equal(hubV3SecurityShort(1), "1 à vérifier");
    assert.equal(hubV3SecurityShort(2), "2 à vérifier");
  });
});

describe("hubV3CoverLabel", () => {
  test("fermés, ouverts, une partie", () => {
    assert.equal(hubV3CoverLabel("Salon", 0, 1), "Salon : le volet est fermé. Toucher pour ouvrir.");
    assert.equal(hubV3CoverLabel("Salon", 0, 2), "Salon : les 2 volets sont fermés. Toucher pour ouvrir.");
    assert.equal(hubV3CoverLabel("Salon", 2, 2), "Salon : les 2 volets sont ouverts. Toucher pour fermer.");
    assert.equal(hubV3CoverLabel("Salon", 1, 3), "Salon : 1 volet ouvert sur 3. Toucher pour fermer.");
  });

  test("sans droit de commande, pas d'invitation à toucher", () => {
    assert.equal(hubV3CoverLabel("Salon", 1, 3, false), "Salon : 1 volet ouvert sur 3.");
  });
});

const device = (entityId, deviceClass) => ({ entity_id: entityId, attributes: { device_class: deviceClass } });

describe("hubV3IsShutter", () => {
  test("les volets, pas les portes de garage ni les portails", () => {
    assert.equal(hubV3IsShutter(device("cover.salon", "shutter")), true);
    assert.equal(hubV3IsShutter(device("cover.salon", undefined)), true);
    assert.equal(hubV3IsShutter(device("cover.garage", "garage")), false);
    assert.equal(hubV3IsShutter(device("cover.portail", "gate")), false);
    assert.equal(hubV3IsShutter(device("light.salon", undefined)), false);
  });
});

describe("hubV3IsSecurity", () => {
  test("ouvertures, alertes, serrures, alarme, sirène, garage", () => {
    for (const [id, dc] of [["binary_sensor.porte", "door"], ["binary_sensor.fumee", "smoke"],
      ["binary_sensor.boitier", "tamper"], ["cover.garage", "garage"], ["lock.entree", undefined],
      ["alarm_control_panel.maison", undefined], ["siren.maison", undefined]]) {
      assert.equal(hubV3IsSecurity(device(id, dc)), true, id);
    }
  });

  test("pas les mouvements, les piles, les volets ni les lumières", () => {
    for (const [id, dc] of [["binary_sensor.couloir", "motion"], ["binary_sensor.pile", "battery"],
      ["cover.salon", "shutter"], ["light.salon", undefined]]) {
      assert.equal(hubV3IsSecurity(device(id, dc)), false, id);
    }
  });
});
