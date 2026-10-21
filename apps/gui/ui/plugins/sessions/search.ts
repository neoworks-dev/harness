// Searching the sessions in the sidebar: which ones match what was typed, in what order, and
// where in the text the match is, for the row to show. Plain functions. Only what the daemon lists
// is searched (title, last message, project); searching the whole conversation needs the daemon.
import type { SessionInfo } from "@neoworks-dev/neo-protocol/types";
import { previewOf, sessionTitle } from "./sessions.ts";

/** Where a result matched, which is what its row shows. */
export type Field = "title" | "preview" | "project";

export interface Match {
  info: SessionInfo;
  field: Field;
}

/** A stretch of text, and whether it is what was searched for. */
export interface Run {
  text: string;
  hit: boolean;
}

/** The words of a query, lower case, each once. */
export function termsOf(query: string): string[] {
  return [...new Set(query.toLowerCase().split(/\s+/).filter(Boolean))];
}

/** How well `term` sits in `text` (lower case): 0 at its start, 1 at a word's start, 2 inside; none if absent. */
function placing(text: string, term: string): number | undefined {
  const at = text.indexOf(term);
  if (at < 0) return undefined;
  if (at === 0) return 0;
  return /[\s\-_./:]/.test(text[at - 1] ?? "") ? 1 : 2;
}

/** Every term's placing in `text`, summed; none unless all of them are in it. */
function wholeScore(text: string, terms: readonly string[]): number | undefined {
  let sum = 0;
  for (const term of terms) {
    const score = placing(text, term);
    if (score === undefined) return undefined;
    sum += score;
  }
  return sum;
}

interface Ranked extends Match {
  tier: number;
  score: number;
}

/** A session's place in the results (the lower the tier and score the better), or none if it does not match. */
function rank(info: SessionInfo, terms: readonly string[], project: string): Ranked | undefined {
  const title = sessionTitle(info).toLowerCase();
  const preview = previewOf(info).toLowerCase();
  const where = project.toLowerCase();
  const inTitle = wholeScore(title, terms);
  if (inTitle !== undefined) return { info, field: "title", tier: 0, score: inTitle };
  const inPreview = wholeScore(preview, terms);
  if (inPreview !== undefined) return { info, field: "preview", tier: 1, score: inPreview };
  const inProject = wholeScore(where, terms);
  if (inProject !== undefined) return { info, field: "project", tier: 2, score: inProject };
  // Each word somewhere: "login site" finds "Fix the login" in the project "site".
  let score = 0;
  let field: Field = "title";
  for (const term of terms) {
    const placed = [placing(title, term), placing(preview, term), placing(where, term)];
    const at = placed.findIndex((one) => one !== undefined);
    if (at < 0) return undefined;
    score += Math.min(...placed.filter((one): one is number => one !== undefined));
    if (at > 0 && field === "title") field = at === 1 ? "preview" : "project";
  }
  return { info, field, tier: 3, score };
}

/**
 * The sessions every word of `query` is found in (title, last message or project name), best
 * first: a match in the title, then in the message, then in the project, then spread over them;
 * within each the earlier in the text, then the newer session. Nothing for an empty query.
 */
export function searchSessions(
  sessions: readonly SessionInfo[],
  query: string,
  projectOf: (info: SessionInfo) => string,
): Match[] {
  const terms = termsOf(query);
  if (terms.length === 0) return [];
  const found: Ranked[] = [];
  for (const info of sessions) {
    const ranked = rank(info, terms, projectOf(info));
    if (ranked) found.push(ranked);
  }
  found.sort(
    (a, b) =>
      a.tier - b.tier ||
      a.score - b.score ||
      b.info.lastActivity - a.info.lastActivity ||
      (a.info.id < b.info.id ? -1 : 1),
  );
  return found.map(({ info, field }) => ({ info, field }));
}

/** `text` cut into runs, those holding a term marked. Case is ignored. */
export function highlight(text: string, terms: readonly string[]): Run[] {
  const hay = text.toLowerCase();
  // A letter whose lower case is longer would shift the positions; leave such text unmarked.
  if (hay.length !== text.length || terms.length === 0 || text === "") {
    return [{ text, hit: false }];
  }
  const marked = new Array<boolean>(text.length).fill(false);
  for (const term of terms) {
    for (let at = hay.indexOf(term); at >= 0; at = hay.indexOf(term, at + 1)) {
      marked.fill(true, at, at + term.length);
    }
  }
  const runs: Run[] = [];
  let start = 0;
  for (let i = 1; i <= text.length; i++) {
    if (i < text.length && marked[i] === marked[start]) continue;
    runs.push({ text: text.slice(start, i), hit: marked[start] === true });
    start = i;
  }
  return runs;
}

/** The part of `text` from just before its first match, with … where it is cut, so the match shows on one line. */
export function around(text: string, terms: readonly string[], lead = 12): string {
  const hay = text.toLowerCase();
  const at = Math.min(...terms.map((term) => hay.indexOf(term)).filter((found) => found >= 0));
  if (!Number.isFinite(at) || at <= lead) return text;
  return `…${text.slice(at - lead)}`;
}
