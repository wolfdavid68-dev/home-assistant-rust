// Les essais du cerveau, avec un faux Claude Code : node --test packaging/docker/cerveau/serveur.test.mjs
import assert from "node:assert/strict";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

import { MAX_REPLY, MAX_TEXT, ask, prepare, service, settings, spoken } from "./serveur.mjs";

const FAUX = fileURLToPath(new URL("./faux-claude.mjs", import.meta.url));

function secrets() {
  const dir = mkdtempSync(join(tmpdir(), "cerveau-essai-"));
  const file = (name, value) => {
    const path = join(dir, name);
    writeFileSync(path, `${value}\n`);
    return path;
  };
  return {
    CERVEAU_CLE_FILE: file("cle", "cle-du-hub"),
    CLAUDE_CODE_OAUTH_TOKEN_FILE: file("oauth", "jeton-abonnement"),
    CERVEAU_MAISON_JETON_FILE: file("maison", "jeton-maison"),
    CERVEAU_CLAUDE: FAUX,
    CERVEAU_DELAI: "5",
  };
}

function setup() {
  const config = settings(secrets());
  const paths = prepare(config);
  return { config, paths };
}

async function started(server) {
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  return `http://127.0.0.1:${server.address().port}`;
}

function post(base, body, key = "cle-du-hub") {
  return fetch(`${base}/demande`, {
    method: "POST",
    headers: { authorization: `Bearer ${key}`, "content-type": "application/json" },
    body: typeof body === "string" ? body : JSON.stringify(body),
  });
}

test("les réglages viennent des fichiers de secrets", () => {
  const { config } = setup();
  assert.equal(config.key, "cle-du-hub");
  assert.equal(config.hub, "http://ha-server:8123/api/mcp");
  assert.equal(config.model, "haiku");
  assert.equal(config.timeoutMs, 5000);
  assert.throws(() => settings({}), /CERVEAU_CLE_FILE manque/);
});

test("Claude n'a que les outils de la maison, avec le compte donné", async () => {
  const { config, paths } = setup();
  const seen = JSON.parse(Buffer.from(await ask(config, paths, "montre tes arguments"), "base64").toString());
  assert.equal(seen.tools, "");
  assert.equal(seen.allowed, "mcp__maison__get_house_state,mcp__maison__assist,mcp__maison__call_service");
  assert.equal(seen.mode, "dontAsk");
  assert.equal(seen.strict, true);
  assert.equal(seen.hub, "http://ha-server:8123/api/mcp");
  assert.equal(seen.bearer, "Bearer jeton-maison");
  assert.equal(seen.oauth, "jeton-abonnement");
  assert.equal(seen.phrase, "<phrase>montre tes arguments</phrase>");
});

test("la réponse est mise en texte simple", async () => {
  const { config, paths } = setup();
  assert.equal(await ask(config, paths, "quelle température au salon"), "Il fait 21 degrés au salon.");
  assert.equal(spoken("# Titre\n\n- un\n- deux"), "Titre - un - deux");
  const long = spoken(`${"Une phrase assez longue pour remplir. ".repeat(40)}`);
  assert.ok(long.length <= MAX_REPLY && long.endsWith("."));
});

test("un échec de Claude est une erreur, sans le détail", async () => {
  const { config, paths } = setup();
  await assert.rejects(ask(config, paths, "erreur"), /Claude a échoué \(api_error\)/);
  await assert.rejects(ask(config, paths, "charabia"), /réponse illisible/);
  await assert.rejects(ask({ ...config, timeoutMs: 500 }, paths, "lent"), /délai dépassé/);
});

test("le service demande la clé du hub et une phrase", async (t) => {
  const server = service("cle-du-hub", async (text) => `Reçu : ${text}`);
  t.after(() => server.close());
  const base = await started(server);
  assert.equal((await fetch(`${base}/sante`)).status, 200);
  assert.equal((await post(base, { texte: "bonjour" }, "mauvaise")).status, 401);
  assert.equal((await post(base, "pas du json")).status, 400);
  assert.equal((await post(base, { texte: "" })).status, 400);
  assert.equal((await post(base, { texte: "x".repeat(MAX_TEXT + 1) })).status, 400);
  const reply = await post(base, { texte: "  bonjour  " });
  assert.equal(reply.status, 200);
  assert.deepEqual(await reply.json(), { reponse: "Reçu : bonjour" });
});

test("une seule phrase à la fois", async (t) => {
  let release;
  const server = service("cle-du-hub", () => new Promise((resolve) => (release = resolve)));
  t.after(() => server.close());
  const base = await started(server);
  const first = post(base, { texte: "première" });
  while (!release) await new Promise((resolve) => setTimeout(resolve, 10));
  assert.equal((await post(base, { texte: "deuxième" })).status, 503);
  release("Fait.");
  assert.equal((await first).status, 200);
});

test("un échec répond 502", async (t) => {
  const server = service("cle-du-hub", async () => {
    throw new Error("délai dépassé");
  });
  t.after(() => server.close());
  const base = await started(server);
  assert.equal((await post(base, { texte: "bonjour" })).status, 502);
});
