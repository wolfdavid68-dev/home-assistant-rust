// Charge des scripts du tableau de bord dans un bac à sable (vm), comme la
// page le fait : des scripts classiques qui partagent une seule portée.
// Le strict minimum de navigateur est simulé, juste de quoi passer le
// chargement ; les fonctions testées, elles, n'y touchent pas.
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import vm from "node:vm";

const web = join(dirname(fileURLToPath(import.meta.url)), "..");
const quiet = { addEventListener() {}, removeEventListener() {} };

/** Les scripts nommés, dans l'ordre. Renvoie `run("code")`, qui exécute du
 *  code dans le bac à sable : `run("ago")` y lit une déclaration. */
export function load(...names) {
  const context = vm.createContext({
    console,
    document: { ...quiet },
    matchMedia: () => ({ matches: false, ...quiet }),
  });
  // Ce que app.js déclare et que les fonctions testées lisent.
  vm.runInContext("const entities = new Map(); let areas = []; const customNames = new Map();", context);
  for (const name of names) {
    vm.runInContext(readFileSync(join(web, name), "utf8"), context, { filename: name });
  }
  return (code) => vm.runInContext(code, context);
}

/** Un objet du bac à sable en objet ordinaire, pour `deepStrictEqual`. */
export const plain = (value) => JSON.parse(JSON.stringify(value));
