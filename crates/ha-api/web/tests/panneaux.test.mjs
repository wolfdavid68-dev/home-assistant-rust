// utils.js : les panneaux à clé du tableau, refaits seulement s'ils changent.
//
// Usage : node --test crates/ha-api/web/tests/*.test.mjs
import assert from "node:assert/strict";
import { beforeEach, describe, test } from "node:test";
import { load } from "./charger.mjs";

const run = load("utils.js");
const objectMark = run("objectMark");
const stateMark = run("stateMark");
const panelChanged = run("panelChanged");
const forgetPanels = run("forgetPanels");
const entities = run("entities");

/** Un état comme le serveur l'envoie : un objet neuf à chaque changement. */
const state = (entity_id, value, attributes = {}) => ({ entity_id, state: value, attributes, last_updated: "2026-10-27T08:00:00Z" });

beforeEach(() => {
  entities.clear();
  forgetPanels();
});

describe("objectMark", () => {
  test("le même objet garde son numéro, un objet neuf en a un autre", () => {
    const a = state("light.salon", "on");
    assert.equal(objectMark(a), objectMark(a));
    // Même contenu, mais un nouvel objet : un nouvel état reçu.
    assert.notEqual(objectMark(a), objectMark(state("light.salon", "on")));
  });

  test("rien, un texte, un nombre", () => {
    assert.equal(objectMark(undefined), "-");
    assert.equal(objectMark(null), "-");
    assert.equal(objectMark("Salon"), "Salon");
    assert.equal(objectMark(3), "3");
  });
});

describe("stateMark", () => {
  test("un appareil absent", () => {
    assert.equal(stateMark(undefined), "-");
  });

  test("un groupe suit aussi ses membres", () => {
    const group = state("cover.tous", "open", { entity_id: ["cover.salon", "cover.cuisine"] });
    entities.set("cover.salon", state("cover.salon", "open"));
    const before = stateMark(group);
    assert.equal(stateMark(group), before);
    entities.set("cover.cuisine", state("cover.cuisine", "closed"));
    assert.notEqual(stateMark(group), before);
  });
});

describe("panelChanged", () => {
  test("refait au premier passage, puis seulement si la clé change", () => {
    assert.equal(panelChanged("volets", "a"), true);
    assert.equal(panelChanged("volets", "a"), false);
    assert.equal(panelChanged("volets", "b"), true);
    assert.equal(panelChanged("volets", "b"), false);
  });

  test("chaque panneau a sa clé", () => {
    assert.equal(panelChanged("volets", "a"), true);
    assert.equal(panelChanged("chauffage", "a"), true);
    assert.equal(panelChanged("volets", "a"), false);
  });

  test("forgetPanels : tout est refait au passage suivant", () => {
    panelChanged("volets", "a");
    panelChanged("courbes", "c");
    forgetPanels();
    assert.equal(panelChanged("volets", "a"), true);
    assert.equal(panelChanged("courbes", "c"), true);
  });

  test("un capteur qui n'est pas montré ne refait pas le panneau", () => {
    // Le panneau des volets montre deux volets ; le compteur Linky bouge.
    const key = (ids) => ids.map((id) => stateMark(entities.get(id))).join(",");
    entities.set("cover.salon", state("cover.salon", "open"));
    entities.set("cover.cuisine", state("cover.cuisine", "closed"));
    entities.set("sensor.linky_puissance", state("sensor.linky_puissance", "1520"));
    const covers = ["cover.salon", "cover.cuisine"];
    assert.equal(panelChanged("volets", key(covers)), true);
    entities.set("sensor.linky_puissance", state("sensor.linky_puissance", "1530"));
    assert.equal(panelChanged("volets", key(covers)), false);
    // Un volet bouge (un attribut seulement) : refait.
    entities.set("cover.salon", state("cover.salon", "open", { current_position: 40 }));
    assert.equal(panelChanged("volets", key(covers)), true);
    // La reconnexion : tous les états sont des objets neufs, même identiques.
    entities.set("cover.salon", state("cover.salon", "open", { current_position: 40 }));
    entities.set("cover.cuisine", state("cover.cuisine", "closed"));
    assert.equal(panelChanged("volets", key(covers)), true);
  });
});
