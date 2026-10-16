-- Jetons d'assistant IA (étape 55) : créés en ligne de commande
-- (`ha-server user token`), ils ne servent qu'à MCP (`/api/mcp`). Le tableau
-- de bord, l'API et le WebSocket les refusent : un assistant n'est jamais
-- traité comme une personne.
ALTER TABLE access_tokens ADD COLUMN program INTEGER NOT NULL DEFAULT 0 CHECK (program IN (0, 1));
