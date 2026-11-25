// hub-v4-energie.js : la page Énergie de l'accueil v4.
//
// Usage : node --test crates/ha-api/web/tests/*.test.mjs

import assert from "node:assert/strict";
import { describe, test } from "node:test";
import { load, plain } from "./charger.mjs";

const run = load("utils.js", "hub-v3.js", "hub-v4.js", "hub-v4-pieces.js", "hub-v4-energie.js");
const hubV4EnergyValue = run("hubV4EnergyValue");
const hubV4KwhText = run("hubV4KwhText");
const hubV4EnergyShares = run("hubV4EnergyShares");
const hubV4EnergyCompare = run("hubV4EnergyCompare");
const hubV4EnergyMeters = run("hubV4EnergyMeters");
const hubV4EnergyLead = run("hubV4EnergyLead");

const kwh = (entity_id, state, attributes = {}) => ({
  entity_id, state: String(state),
  attributes: { unit_of_measurement: "kWh", device_class: "energy", ...attributes },
});

describe("hubV4EnergyValue", () => {
  test("un nombre positif, sinon rien", () => {
    assert.equal(hubV4EnergyValue({ state: "3832" }), 3832);
    assert.equal(hubV4EnergyValue({ state: "unavailable" }), null);
    assert.equal(hubV4EnergyValue({ state: "" }), null);
    assert.equal(hubV4EnergyValue({ state: "-2" }), null);
    assert.equal(hubV4EnergyValue(undefined), null);
  });
});

describe("hubV4KwhText", () => {
  test("une décimale sous 10 kWh, aucune au-delà", () => {
    assert.equal(hubV4KwhText(4.63), "4,6");
    assert.equal(hubV4KwhText(3832.4).replace(/\s/g, " "), "3 832");
  });
});

describe("hubV4EnergyShares", () => {
  test("les pourcentages font 100", () => {
    const shares = plain(hubV4EnergyShares([{ key: "a", kwh: 2266 }, { key: "b", kwh: 1566 }]));
    assert.equal(shares.total, 3832);
    assert.deepEqual(shares.parts.map((p) => p.pct), [59, 41]);
  });
  test("trois tiers : le reste va au plus fort", () => {
    const shares = plain(hubV4EnergyShares([{ kwh: 1 }, { kwh: 1 }, { kwh: 1 }]));
    assert.equal(shares.parts.reduce((s, p) => s + p.pct, 0), 100);
  });
  test("un total nul : zéro partout", () => {
    assert.deepEqual(plain(hubV4EnergyShares([{ kwh: 0 }])).parts.map((p) => p.pct), [0]);
  });
});

describe("hubV4EnergyCompare", () => {
  test("une part de la pompe à chaleur", () => {
    assert.equal(hubV4EnergyCompare(297, 3832), "À peu près 8 % de ce que consomme la pompe à chaleur.");
    assert.equal(hubV4EnergyCompare(1, 3832), "Moins de 1 % de ce que consomme la pompe à chaleur.");
    assert.equal(hubV4EnergyCompare(5000, 3832), "Plus que la pompe à chaleur.");
    assert.equal(hubV4EnergyCompare(10, 0), "");
  });
});

describe("hubV4EnergyMeters", () => {
  test("ni la pompe à chaleur, ni le Linky, ni ce qui est déjà montré", () => {
    const states = [
      kwh("sensor.pac_consommation_totale", 3832),
      kwh("sensor.linky_index", 12000),
      kwh("sensor.lave_linge_energie", 297),
      kwh("sensor.four_energie", 40),
      kwh("sensor.frigo_energie", 120),
      { entity_id: "sensor.salon_temperature", state: "19", attributes: { unit_of_measurement: "°C" } },
      kwh("sensor.cassé", "unavailable"),
    ];
    const ids = plain(hubV4EnergyMeters(states, ["sensor.lave_linge_energie"])).map((s) => s.entity_id);
    assert.deepEqual(ids, ["sensor.frigo_energie", "sensor.four_energie"]);
  });
});

describe("hubV4EnergyLead", () => {
  test("la phrase suit les compteurs", () => {
    assert.equal(hubV4EnergyLead(3832, 297), "La pompe à chaleur fait presque tout : elle chauffe la maison et l'eau.");
    assert.equal(hubV4EnergyLead(100, 500), "La pompe à chaleur chauffe la maison et l'eau.");
    assert.equal(hubV4EnergyLead(0, 5), "Ce que consomment les appareils mesurés.");
    assert.equal(hubV4EnergyLead(0, 0), "Aucun compteur d'énergie pour l'instant.");
  });
});

describe("hubV4EnergyTempo", () => {
  const hubV4EnergyTempo = run("hubV4EnergyTempo");
  test("bleu, blanc, rouge, ou pas encore annoncé", () => {
    assert.deepEqual(plain(hubV4EnergyTempo({ state: "Bleu" })), { tone: "blue", text: "Bleu" });
    assert.deepEqual(plain(hubV4EnergyTempo({ state: "Blanc" })), { tone: "white", text: "Blanc" });
    assert.deepEqual(plain(hubV4EnergyTempo({ state: "Rouge" })), { tone: "red", text: "Rouge" });
    assert.deepEqual(plain(hubV4EnergyTempo({ state: "unknown" })), { tone: "unknown", text: "À venir" });
    assert.deepEqual(plain(hubV4EnergyTempo({ state: "unavailable" })), { tone: "unknown", text: "Indisponible" });
    assert.deepEqual(plain(hubV4EnergyTempo(undefined)), { tone: "unknown", text: "—" });
  });
});
