// Un faux Claude Code, pour les essais du cerveau (serveur.test.mjs) : il
// répond selon la phrase reçue, sans rien demander à personne.
import { readFileSync } from "node:fs";

const args = process.argv.slice(2);
const phrase = readFileSync(0, "utf8");
const value = (flag) => args[args.indexOf(flag) + 1];

if (phrase.includes("arguments")) {
  // Renvoie ce qu'il a reçu (en base64, que la mise en texte ne touche pas),
  // pour vérifier les options et l'environnement.
  const mcp = JSON.parse(readFileSync(value("--mcp-config"), "utf8"));
  const seen = {
    tools: value("--tools"),
    allowed: value("--allowedTools"),
    mode: value("--permission-mode"),
    strict: args.includes("--strict-mcp-config"),
    hub: mcp.mcpServers.maison.url,
    bearer: mcp.mcpServers.maison.headers.Authorization,
    oauth: process.env.CLAUDE_CODE_OAUTH_TOKEN,
    phrase,
  };
  console.log(JSON.stringify({ type: "result", subtype: "success", is_error: false, result: Buffer.from(JSON.stringify(seen)).toString("base64") }));
} else if (phrase.includes("lent")) {
  setTimeout(() => {}, 60_000);
} else if (phrase.includes("erreur")) {
  console.log(JSON.stringify({ type: "result", subtype: "success", is_error: true, result: "Failed", terminal_reason: "api_error" }));
} else if (phrase.includes("charabia")) {
  console.log("pas du JSON");
} else {
  console.log(JSON.stringify({ type: "result", subtype: "success", is_error: false, result: "**Il fait** 21 degrés au salon." }));
}
