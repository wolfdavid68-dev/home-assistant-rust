// house.js : les onglets reçus du serveur, les volets, les calques allumés.
//
// Usage : node --test crates/ha-api/web/tests/*.test.mjs
import assert from "node:assert/strict";
import { beforeEach, describe, test } from "node:test";
import { load, plain } from "./charger.mjs";

// house-effets.js donne EFFECTS, que cleanDrawing consulte.
const run = load("utils.js", "house.js", "house-effets.js");
const cleanDrawing = (config) => plain(run("cleanDrawing")(config));
const cleanPages = (config) => plain(run("cleanPages")(config));
const coverOpenPercent = run("coverOpenPercent");
const layerShown = run("layerShown");
const roomLit = run("roomLit");
const houseImageAddress = run("houseImageAddress");

const EMPTY = { id: "maison", name: "Maison", areas: [], layers: [], labels: [], actions: [], charts: [], zones: [], ambiances: {} };

describe("cleanDrawing", () => {
  test("rien de valable : un onglet vide « Maison »", () => {
    assert.deepEqual(cleanDrawing(undefined), EMPTY);
    assert.deepEqual(cleanDrawing(null), EMPTY);
    assert.deepEqual(cleanDrawing("n'importe quoi"), EMPTY);
    assert.deepEqual(cleanDrawing({ id: "", name: 3, layers: "non", labels: {}, zones: null }), EMPTY);
  });

  test("garde les fonds, les ambiances connues et les choix valables", () => {
    const out = cleanDrawing({
      id: "garage",
      name: "Garage",
      areas: ["Cuisine", "", 4],
      background: "jour.png",
      night: "nuit.png",
      ambiances: { "nuit-pluie": "pluie.png", "inconnue": "x.png", "jour-neige": "" },
      forced: "nuit",
      effects: "neige",
      effects_level: "fort",
      secret: "jeté",
    });
    assert.equal(out.id, "garage");
    assert.equal(out.name, "Garage");
    assert.deepEqual(out.areas, ["Cuisine"]);
    assert.equal(out.background, "jour.png");
    assert.equal(out.night, "nuit.png");
    assert.deepEqual(out.ambiances, { "nuit-pluie": "pluie.png" });
    assert.equal(out.forced, "nuit");
    assert.equal(out.effects, "neige");
    assert.equal(out.effects_level, "fort");
    assert.equal(out.secret, undefined);
  });

  test("écarte les choix inconnus", () => {
    const out = cleanDrawing({ forced: "midi", effects: "tornade", effects_level: "énorme" });
    assert.equal(out.forced, undefined);
    assert.equal(out.effects, undefined);
    assert.equal(out.effects_level, undefined);
    assert.equal(cleanDrawing({ effects: "off" }).effects, "off");
  });

  test("les calques : appareil ou pièce, valeurs par défaut", () => {
    const { layers } = cleanDrawing({
      layers: [
        { image: "garage.png", entity_id: "cover.garage", states: ["open", 3], not: "oui", when: "midi" },
        { image: "volet.png", entity_id: "cover.salon", position: true, when: "night" },
        { image: "cuisine.png", area: "Cuisine", entity_id: "light.plafonnier", states: ["on"], when: "day" },
      ],
    });
    assert.deepEqual(layers, [
      { image: "garage.png", entity_id: "cover.garage", states: ["open"], not: false, position: false, when: "always" },
      { image: "volet.png", entity_id: "cover.salon", states: [], not: false, position: true, when: "night" },
      { image: "cuisine.png", area: "Cuisine", states: [], not: false, position: false, when: "day" },
    ]);
  });

  test("les valeurs, les boutons, les courbes et les zones", () => {
    const out = cleanDrawing({
      labels: [{ entity_id: "sensor.temperature", x: "12.5", y: "abc", name: "Dehors" }],
      actions: ["light.plafonnier", "", null],
      charts: [
        { entity_id: "sensor.temperature", color: "rouge", hours: 6 },
        { entity_id: "sensor.humidite", color: "fuchsia", hours: 5, name: "Humidité" },
        { color: "vert" },
      ],
      zones: [
        { area: "Cuisine", x: 10, y: 20, w: 0, h: "30" },
        { entity_id: "light.plafonnier", name: "Lampe" },
        { x: 1, y: 1 },
      ],
    });
    assert.deepEqual(out.labels, [{ entity_id: "sensor.temperature", x: 12.5, y: 0, name: "Dehors" }]);
    assert.deepEqual(out.actions, ["light.plafonnier"]);
    assert.deepEqual(out.charts, [
      { entity_id: "sensor.temperature", color: "rouge", hours: 6 },
      { entity_id: "sensor.humidite", color: "bleu", hours: 24, name: "Humidité" },
    ]);
    assert.deepEqual(out.zones, [
      { area: "Cuisine", x: 10, y: 20, w: 1, h: 30 },
      { entity_id: "light.plafonnier", x: 0, y: 0, w: 1, h: 1, name: "Lampe" },
    ]);
  });
});

describe("cleanPages", () => {
  test("rien reçu : un seul onglet « Maison »", () => {
    assert.deepEqual(cleanPages(null), [EMPTY]);
    // Une liste vide : l'onglet de départ (homePage), sans ambiances.
    const { ambiances, ...home } = EMPTY;
    assert.deepEqual(cleanPages({ pages: [] }), [home]);
  });

  test("un ancien dessin, sans onglets, devient l'onglet « Maison »", () => {
    const pages = cleanPages({ background: "jour.png", actions: ["light.plafonnier"] });
    assert.equal(pages.length, 1);
    assert.equal(pages[0].id, "maison");
    assert.equal(pages[0].background, "jour.png");
    assert.deepEqual(pages[0].actions, ["light.plafonnier"]);
  });

  test("chaque onglet est nettoyé", () => {
    const pages = cleanPages({ pages: [{ id: "maison", name: "Maison" }, { id: "garage", name: "Garage", effects: "tornade" }] });
    assert.deepEqual(pages.map((page) => page.id), ["maison", "garage"]);
    assert.equal(pages[1].effects, undefined);
  });
});

describe("coverOpenPercent", () => {
  const cover = (state, attributes = {}) => ({ entity_id: "cover.salon", state, attributes });

  test("la position, bornée entre 0 et 100", () => {
    assert.equal(coverOpenPercent(cover("open", { current_position: 40 })), 40);
    assert.equal(coverOpenPercent(cover("open", { current_position: "75" })), 75);
    assert.equal(coverOpenPercent(cover("open", { current_position: 120 })), 100);
    assert.equal(coverOpenPercent(cover("closed", { current_position: -5 })), 0);
  });

  test("sans position : l'état", () => {
    assert.equal(coverOpenPercent(cover("closed")), 0);
    assert.equal(coverOpenPercent(cover("open")), 100);
    assert.equal(coverOpenPercent(cover("open", { current_position: "?" })), 100);
  });

  test("on ne sait pas : null", () => {
    assert.equal(coverOpenPercent(cover("unavailable")), null);
    assert.equal(coverOpenPercent(cover("opening")), null);
  });
});

describe("layerShown et roomLit", () => {
  const set = (entity_id, state, attributes = {}) => run("entities").set(entity_id, { entity_id, state, attributes });

  beforeEach(() => {
    run("entities").clear();
    run("(list) => { areas = list; }")([
      { name: "Cuisine", entities: [{ entity_id: "light.plafonnier" }, { entity_id: "light.prise" }] },
      { name: "Garage", entities: [] },
    ]);
  });

  test("roomLit : une lampe allumée dans la pièce", () => {
    set("light.plafonnier", "off");
    assert.equal(roomLit("Cuisine"), false);
    set("light.plafonnier", "on");
    assert.equal(roomLit("Cuisine"), true);
    assert.equal(roomLit("Garage"), false);
    assert.equal(roomLit("Inconnue"), false);
  });

  test("roomLit : une prise allumée n'éclaire pas la pièce", () => {
    set("light.plafonnier", "off");
    set("light.prise", "on", { device_class: "outlet" });
    assert.equal(roomLit("Cuisine"), false);
  });

  test("un calque de pièce suit ses lampes, et le moment", () => {
    const layer = { area: "Cuisine", when: "night" };
    set("light.plafonnier", "off");
    assert.equal(layerShown(layer, true), false);
    set("light.plafonnier", "on");
    assert.equal(layerShown(layer, true), true);
    assert.equal(layerShown(layer, false), false);
    assert.equal(layerShown({ ...layer, when: "always" }, false), true);
  });

  test("un calque d'appareil suit ses états, ou leur contraire", () => {
    const layer = { entity_id: "cover.garage", states: ["open"], not: false, position: false, when: "always" };
    assert.equal(layerShown(layer, false), false, "appareil inconnu");
    set("cover.garage", "open");
    assert.equal(layerShown(layer, false), true);
    assert.equal(layerShown({ ...layer, not: true }, false), false);
    set("cover.garage", "closed");
    assert.equal(layerShown(layer, false), false);
    assert.equal(layerShown({ ...layer, not: true }, false), true);
    assert.equal(layerShown({ ...layer, not: true, when: "day" }, true), false);
  });

  test("un volet : visible dès qu'il n'est plus grand ouvert", () => {
    const layer = { entity_id: "cover.salon", states: [], not: false, position: true, when: "always" };
    set("cover.salon", "open", { current_position: 100 });
    assert.equal(layerShown(layer, false), false);
    set("cover.salon", "open", { current_position: 98 });
    assert.equal(layerShown(layer, false), false);
    set("cover.salon", "open", { current_position: 50 });
    assert.equal(layerShown(layer, false), true);
    set("cover.salon", "unavailable");
    assert.equal(layerShown(layer, false), false);
  });
});

describe("houseImageAddress", () => {
  test("avec la version : une adresse que le navigateur garde", () => {
    assert.equal(houseImageAddress({ name: "jour", version: "6123abc" }), "/api/home/images/jour?v=6123abc");
  });

  test("une nouvelle version donne une nouvelle adresse", () => {
    const before = houseImageAddress({ name: "garage_ouvert", version: "1a" });
    const after = houseImageAddress({ name: "garage_ouvert", version: "1b" });
    assert.notEqual(before, after);
  });

  test("sans version (ancien serveur) : l'adresse nue", () => {
    assert.equal(houseImageAddress({ name: "jour" }), "/api/home/images/jour");
    assert.equal(houseImageAddress({ name: "jour", version: "" }), "/api/home/images/jour");
    assert.equal(houseImageAddress({ name: "jour", version: 12 }), "/api/home/images/jour");
  });

  test("le nom et la version sont encodés", () => {
    assert.equal(houseImageAddress({ name: "a b", version: "x&y" }), "/api/home/images/a%20b?v=x%26y");
  });
});
