// app.js : « Annuler » après une scène — ce qu'on retient avant, et les ordres
// qui remettent comme avant (seulement ce qui a changé, jamais le chauffage
// ni les serrures).
//
// app.js touche la page dès son chargement : on n'en prend que les deux
// fonctions testées, avec ce qu'elles lisent.
//
// Usage : node --test crates/ha-api/web/tests/*.test.mjs
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { beforeEach, describe, test } from "node:test";
import { fileURLToPath } from "node:url";
import vm from "node:vm";

const source = readFileSync(join(dirname(fileURLToPath(import.meta.url)), "..", "app.js"), "utf8");
const take = (name) => {
  const start = source.indexOf(`function ${name}(`);
  assert.ok(start >= 0, `${name} introuvable dans app.js`);
  const end = source.indexOf("\n}\n", start);
  return source.slice(start, end + 2);
};
const context = vm.createContext({});
vm.runInContext(`
  const entities = new Map();
  const SHEDDING = "switch.delestage";
  const SHADE = "switch.protection_solaire";
  const domainOf = (state) => state.entity_id.split(".")[0];
  let allowed = () => true;
  const canControl = (state) => allowed(state);
  ${take("undoSnapshot")}
  ${take("undoOrders")}
`, context);
const run = (code) => vm.runInContext(code, context);
const entities = run("entities");
const undoSnapshot = run("undoSnapshot");
const undoOrders = run("undoOrders");
const plain = (value) => JSON.parse(JSON.stringify(value));

const put = (entity_id, state, attributes = {}) => entities.set(entity_id, { entity_id, state, attributes });

beforeEach(() => {
  entities.clear();
  run("allowed = () => true");
});

describe("undoSnapshot", () => {
  test("lumières, prises et volets ; ni chauffage, ni serrure, ni délestage", () => {
    put("light.salon", "on", { brightness: 200 });
    put("switch.prise", "off");
    put("cover.volet", "open", { current_position: 60 });
    put("climate.salon", "heat");
    put("lock.porte", "locked");
    put("switch.delestage", "on");
    put("media_player.salon", "playing");
    put("light.cassee", "unavailable");
    assert.deepEqual([...undoSnapshot().keys()].sort(), ["cover.volet", "light.salon", "switch.prise"]);
  });

  test("seulement les appareils visés, et que ce compte commande", () => {
    put("light.salon", "on");
    put("light.cuisine", "on");
    put("light.chambre", "off");
    run("allowed = (state) => state.entity_id !== 'light.chambre'");
    assert.deepEqual([...undoSnapshot(new Set(["light.salon", "light.chambre"])).keys()], ["light.salon"]);
  });
});

describe("undoOrders", () => {
  test("rien n'a bougé : rien à envoyer", () => {
    put("light.salon", "on", { brightness: 200 });
    put("cover.volet", "open", { current_position: 60 });
    assert.deepEqual(plain(undoOrders(undoSnapshot())), []);
  });

  test("la scène a éteint, allumé, baissé : chacun revient comme avant", () => {
    put("light.salon", "on", { brightness: 200 });
    put("light.couloir", "off");
    put("switch.prise", "on");
    put("cover.volet", "open", { current_position: 60 });
    put("cover.store", "closed");
    const saved = undoSnapshot();
    put("light.salon", "off");
    put("light.couloir", "on", { brightness: 255 });
    put("switch.prise", "off");
    put("cover.volet", "closed", { current_position: 0 });
    put("cover.store", "open");
    assert.deepEqual(plain(undoOrders(saved)), [
      ["light", "turn_on", { entity_id: "light.salon", brightness: 200 }],
      ["light", "turn_off", { entity_id: "light.couloir" }],
      ["switch", "turn_on", { entity_id: "switch.prise" }],
      ["cover", "set_cover_position", { entity_id: "cover.volet", position: 60 }],
      ["cover", "close_cover", { entity_id: "cover.store" }],
    ]);
  });

  test("une lumière restée allumée mais tamisée retrouve sa luminosité", () => {
    put("light.salon", "on", { brightness: 200 });
    const saved = undoSnapshot();
    put("light.salon", "on", { brightness: 60 });
    assert.deepEqual(plain(undoOrders(saved)), [["light", "turn_on", { entity_id: "light.salon", brightness: 200 }]]);
    // Un écart d'arrondi (± 2) ne compte pas.
    put("light.salon", "on", { brightness: 199 });
    assert.deepEqual(plain(undoOrders(saved)), []);
  });

  test("un appareil disparu entre-temps est laissé de côté", () => {
    put("light.salon", "on");
    const saved = undoSnapshot();
    entities.delete("light.salon");
    assert.deepEqual(plain(undoOrders(saved)), []);
  });
});
