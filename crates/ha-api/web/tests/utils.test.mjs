// utils.js : la famille d'un capteur (sa teinte) et les « il y a … ».
//
// Usage : node --test crates/ha-api/web/tests/*.test.mjs
import assert from "node:assert/strict";
import { test } from "node:test";
import { load } from "./charger.mjs";

const run = load("utils.js");
const familyOf = run("familyOf");
const ago = run("ago");

test("familyOf : le type de capteur décide d'abord", () => {
  assert.equal(familyOf({ deviceClass: "smoke" }), "alertes");
  assert.equal(familyOf({ deviceClass: "power" }), "energie");
  assert.equal(familyOf({ deviceClass: "humidity" }), "meteo");
});

test("familyOf : puis l'unité", () => {
  assert.equal(familyOf({ unit: "kWh" }), "energie");
  assert.equal(familyOf({ unit: "W" }), "energie");
  assert.equal(familyOf({ unit: "°C" }), "meteo");
  assert.equal(familyOf({ unit: "km/h" }), "meteo");
});

test("familyOf : puis le nom ou la phrase", () => {
  assert.equal(familyOf({ text: "Lave-linge" }), "machines");
  assert.equal(familyOf({ text: "Fini vers 14:30" }), "machines");
  assert.equal(familyOf({ text: "Température salon" }), "meteo");
  assert.equal(familyOf({ text: "Conso du jour" }), "energie");
  assert.equal(familyOf({ machine: true }), "machines");
});

test("familyOf : une alerte passe avant le reste", () => {
  // Une fuite sous le lave-linge reste une alerte, pas une machine.
  assert.equal(familyOf({ text: "Fuite sous le lave-linge" }), "alertes");
  assert.equal(familyOf({ text: "Lave-linge injoignable", machine: true }), "alertes");
});

test("familyOf : rien de reconnu, pas de famille", () => {
  assert.equal(familyOf({ text: "Porte d'entrée" }), "");
  assert.equal(familyOf({}), "");
  assert.equal(familyOf(), "");
  // Une unité qui ressemble seulement à une autre.
  assert.equal(familyOf({ unit: "Wb" }), "");
});

const before = (seconds) => new Date(Date.now() - seconds * 1000).toISOString();

test("ago : moins d'une minute", () => {
  assert.equal(ago(before(5)), "quelques secondes");
  assert.equal(ago(before(59)), "quelques secondes");
});

test("ago : minutes, heures, jours", () => {
  assert.equal(ago(before(5 * 60)), "5 minutes");
  assert.equal(ago(before(2 * 3600)), "2 heures");
  assert.equal(ago(before(3 * 86_400)), "3 jours");
  // « il y a 1 jour » se dit « hier ».
  assert.equal(ago(before(86_400)), "hier");
});

test("ago : une date dans le futur (horloges décalées) reste « quelques secondes »", () => {
  assert.equal(ago(new Date(Date.now() + 120_000).toISOString()), "quelques secondes");
});
