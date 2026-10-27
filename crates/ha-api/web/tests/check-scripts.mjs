// Les scripts du tableau de bord se compilent-ils ensemble ?
//
// index.html charge des scripts classiques qui partagent une seule portée :
// un `const wake` déclaré dans deux fichiers casse le second au chargement
// (SyntaxError), et tout ce qu'il contient disparaît de la page. Ici, on
// colle les fichiers dans l'ordre de index.html et on compile le tout, sans
// rien exécuter : la double déclaration est une erreur de compilation.
//
// Usage : node crates/ha-api/web/tests/check-scripts.mjs
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import vm from "node:vm";

const web = join(dirname(fileURLToPath(import.meta.url)), "..");
// Ceux-là tournent à part (fil audio, service worker) : compilés seuls.
const ALONE = ["ecoute.js", "sw.js"];

const html = readFileSync(join(web, "index.html"), "utf8");
const scripts = [...html.matchAll(/<script\s+src="\/([^"?]+)(?:\?[^"]*)?"/g)].map((m) => m[1]);
if (scripts.length === 0) {
  console.error("Aucun <script src> trouvé dans index.html.");
  process.exit(1);
}

const sources = new Map(scripts.map((name) => [name, readFileSync(join(web, name), "utf8")]));
let failed = false;

function compile(source, filename) {
  try {
    new vm.Script(source, { filename });
    return null;
  } catch (error) {
    return error;
  }
}

// Chaque fichier seul : une faute de frappe est signalée à sa vraie ligne.
for (const name of [...scripts, ...ALONE]) {
  const source = sources.get(name) ?? readFileSync(join(web, name), "utf8");
  const error = compile(source, name);
  if (error) {
    failed = true;
    console.error(`${name} : ${error.message}`);
  }
}

// Puis tous ensemble, comme dans la page.
const together = (names) => names.map((name) => sources.get(name)).join("\n;\n");
if (!failed) {
  const error = compile(together(scripts), "index.html");
  if (error) {
    failed = true;
    // Le premier fichier qui ne passe plus avec ceux d'avant est le coupable.
    const culprit = scripts.find((_, i) => compile(together(scripts.slice(0, i + 1)), "index.html"));
    console.error(`${culprit ?? "?"} (avec les fichiers chargés avant lui) : ${error.message}`);
  }
}

if (failed) process.exit(1);
console.log(`${scripts.length} scripts de index.html et ${ALONE.length} à part : ils compilent ensemble.`);
