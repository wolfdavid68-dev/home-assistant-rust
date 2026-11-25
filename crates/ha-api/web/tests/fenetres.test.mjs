// fenetres.js : les fenêtres ont toutes le même en-tête, le même bouton
// Fermer et le même pied, celles de index.html comme celles du JS.
//
// Usage : node --test crates/ha-api/web/tests/*.test.mjs
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import { load, plain } from "./charger.mjs";

const run = load("utils.js", "fenetres.js");
const FENETRE = plain(run("FENETRE"));
const fenetreDehors = run("fenetreDehors");
const html = readFileSync(join(dirname(fileURLToPath(import.meta.url)), "..", "index.html"), "utf8");

test("fenetreDehors : un toucher hors du cadre est sur le fond", () => {
  const rect = { left: 100, right: 500, top: 50, bottom: 400 };
  assert.equal(fenetreDehors(rect, 300, 200), false);
  assert.equal(fenetreDehors(rect, 100, 50), false);
  assert.equal(fenetreDehors(rect, 99, 200), true);
  assert.equal(fenetreDehors(rect, 300, 401), true);
});

test("le bouton Fermer : 44 px, rond, avec un nom", () => {
  assert.match(FENETRE.fermer, /\bw-11\b/);
  assert.match(FENETRE.fermer, /\bh-11\b/);
  assert.match(FENETRE.fermer, /\brounded-full\b/);
  for (const name of ["principal", "secondaire", "danger", "dangerDiscret"]) assert.match(FENETRE[name], /\bmin-h-11\b/, name);
});

test("index.html : chaque bouton Fermer a les classes de fenetres.js et un aria-label", () => {
  const buttons = [...html.matchAll(/<button id="([a-z-]+-close)" class="([^"]*)"([^>]*)>/g)].filter(([, , cls]) => cls.includes("fenetre-fermer"));
  assert.ok(buttons.length >= 11, `${buttons.length} boutons Fermer`);
  for (const [, id, cls, rest] of buttons) {
    assert.equal(cls, FENETRE.fermer, id);
    assert.match(rest, /aria-label="Fermer"/, id);
  }
});

test("index.html : les en-têtes et les pieds reprennent les classes de fenetres.js", () => {
  const base = FENETRE.entete.replace(/ mb-4$/, "");
  const heads = [...html.matchAll(/class="(?:doorbell-head )?(fenetre-entete [^"]*)"/g)].map((m) => m[1]);
  assert.ok(heads.length >= 10);
  for (const cls of heads) assert.ok(cls.startsWith(base), cls);
  const titles = [...html.matchAll(/<h2 id="[a-z-]+-title" class="([^"]*)"/g)].map((m) => m[1]).filter((cls) => cls.startsWith("m-0 font-titre"));
  assert.ok(titles.length >= 12);
  for (const cls of titles) assert.equal(cls, FENETRE.titre);
  const feet = [...html.matchAll(/class="(fenetre-pied [^"]*)"/g)].map((m) => m[1]);
  for (const cls of feet) assert.ok(cls.startsWith(FENETRE.pied.replace(/ mt-5$/, "")), cls);
  // Aucun dialogue ne revient au window.confirm du navigateur.
  for (const name of ["account.js", "agenda.js", "editor.js", "house-reglage.js", "tiles-appareils.js"]) {
    const source = readFileSync(join(dirname(fileURLToPath(import.meta.url)), "..", name), "utf8");
    assert.ok(!source.includes("window.confirm("), name);
  }
});

test("formulaires : l'heure proposée est le prochain quart d'heure, la fin une heure après", () => {
  const prochain = run("formProchainQuart");
  const plus = run("formHeurePlus");
  const minutes = run("formMinutes");
  const at = (h, m) => new Date(2026, 10, 25, h, m);
  assert.equal(prochain(at(14, 7)), "14:15");
  assert.equal(prochain(at(14, 0)), "14:15");
  assert.equal(prochain(at(14, 15)), "14:30");
  assert.equal(prochain(at(9, 59)), "10:00");
  assert.equal(prochain(at(23, 50)), "23:45");
  assert.equal(plus("14:15", 60), "15:15");
  assert.equal(plus("23:30", 60), "23:59");
  assert.equal(plus("08:00", -30), "07:30");
  assert.equal(minutes("10:30") - minutes("09:45"), 45);
});

test("rien de perdu par erreur : les fenêtres de saisie demandent avant de fermer", () => {
  const web = join(dirname(fileURLToPath(import.meta.url)), "..");
  const source = (name) => readFileSync(join(web, name), "utf8");
  // Chaque éditeur garde sa fenêtre, et ses boutons Fermer passent par fenetreFermer.
  for (const [name, id] of [["agenda.js", "agenda-new"], ["editor.js", "automations"], ["house-reglage.js", "drawing"], ["account.js", "account-dialog"]]) {
    assert.match(source(name), new RegExp(`fenetreGarde\\(\\$\\("#${id}"\\)`), name);
    assert.match(source(name), new RegExp(`fenetreFermer\\(\\$\\("#${id}"\\)\\)`), name);
  }
  // Le formulaire du rendez-vous : nos erreurs sous les champs, pas de message flash global.
  const agenda = source("agenda.js");
  const nouveau = agenda.slice(agenda.indexOf("agendaNewSetup"), agenda.indexOf("// ---------------------------------------------------------------- Page Maison"));
  assert.ok(nouveau.includes("champErreur("));
  assert.ok(nouveau.includes("boutonEnCours("));
  assert.ok(!nouveau.includes("flash("));
  assert.ok(!nouveau.includes("callService("));
});
