// Tool search over names and descriptions. Plain token matching is enough at this size (design §5);
// embeddings can come later if the tool count grows.
import type { StoredConnector, StoredTool } from "../store.ts";

export interface SearchHit {
  connector: StoredConnector;
  tool: StoredTool;
  open: boolean;
}

const LIMIT = 8;

function tokens(text: string): string[] {
  return text
    .toLowerCase()
    .split(/[^a-z0-9]+/)
    .filter((t) => t.length > 1);
}

function score(query: string[], connector: StoredConnector, tool: StoredTool): number {
  const name = tokens(`${connector.app} ${tool.name}`);
  const description = tool.description.toLowerCase();
  let total = 0;
  for (const word of query) {
    if (name.includes(word)) total += 3;
    else if (name.some((n) => n.includes(word) || word.includes(n))) total += 2;
    else if (description.includes(word)) total += 1;
  }
  return total;
}

/**
 * Best matches for `query`, open connectors first among equal scores. An exact full name
 * (`app.tool`) always matches; an empty query with `app` lists all of that connector's tools.
 */
export function searchTools(
  connectors: { connector: StoredConnector; open: boolean }[],
  query: string,
  app?: string,
): SearchHit[] {
  const scoped = app === undefined ? connectors : connectors.filter((c) => c.connector.app === app);
  const words = tokens(query);
  const all = scoped.flatMap(({ connector, open }) =>
    connector.tools.map((tool) => ({ connector, tool, open })),
  );
  if (words.length === 0) {
    // Listing everything only makes sense within one connector.
    if (app === undefined) return [];
    return all;
  }

  const exact = all.find((h) => `${h.connector.app}.${h.tool.name}` === query.trim());
  if (exact) return [exact];
  return all
    .map((hit) => ({ hit, score: score(words, hit.connector, hit.tool) }))
    .filter((s) => s.score > 0)
    .sort((a, b) => b.score - a.score || Number(b.hit.open) - Number(a.hit.open))
    .slice(0, LIMIT)
    .map((s) => s.hit);
}
