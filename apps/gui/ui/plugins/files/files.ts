// The files panel: what git says changed, plus what the agent and its guests touched this
// session (from the diffs of their tool calls). Ported from apps/term/src/widgets/files.rs and
// git.rs; the host only runs git (`git_status`), the parsing is in lib/git-status.ts.
import type { SessionEventRecord } from "@suite/protocol/types";
import { relFromPrefix, relativize, type Change, type GitStatus } from "../../lib/git-status.ts";

/** What a file shows: git's change, or `clean` for one an agent touched that git sees no difference in. */
export type Mark = Change | "clean";

/** A file an agent changed this session. */
export interface Touched {
  rel: string;
  /** Who changed it: "agent" (the session's own), "claude"… */
  tags: readonly string[];
  /** The first change created the file. */
  added: boolean;
}

/** What the session's events have said about files so far. */
export interface Touches {
  /** toolCallId -> the guest that made the call (absent: the session's own agent). */
  toolAgents: Readonly<Record<string, string | undefined>>;
  touched: readonly Touched[];
}

export const NO_TOUCHES: Touches = { toolAgents: {}, touched: [] };

function touch(touched: readonly Touched[], rel: string, tag: string, added: boolean): Touched[] {
  const at = touched.findIndex((t) => t.rel === rel);
  if (at < 0) return [...touched, { rel, tags: [tag], added }];
  return touched.map((t, index) =>
    index !== at || t.tags.includes(tag) ? t : { ...t, tags: [...t.tags, tag] },
  );
}

/**
 * The touches after one more event of the session in `cwd`: diffs in tool call updates name the
 * files. Returns the same object when the event touches nothing.
 */
export function applyTouch(state: Touches, record: SessionEventRecord, cwd: string): Touches {
  const { event } = record;
  if (event.type === "tool_call") {
    if (state.toolAgents[event.toolCallId] === event.agent) return state;
    return { ...state, toolAgents: { ...state.toolAgents, [event.toolCallId]: event.agent } };
  }
  if (event.type !== "tool_call_update") return state;
  let touched = state.touched;
  for (const content of event.content ?? []) {
    if (content.type !== "diff" || content.path === "") continue;
    const tag = state.toolAgents[event.toolCallId] ?? "agent";
    touched = touch(touched, relativize(cwd, content.path), tag, content.oldText === undefined);
  }
  return touched === state.touched ? state : { ...state, touched };
}

export interface FileEntry {
  /** Relative to the session directory when inside it. */
  rel: string;
  orig?: string;
  mark: Mark;
  /** Who changed it in this session. */
  tags: readonly string[];
}

/** Git sees no difference in a touched file it knows nothing about; without git it was added or modified. */
function touchedMark(inRepo: boolean, touched: Touched): Mark {
  if (inRepo) return "clean";
  return touched.added ? "added" : "modified";
}

/** Git's changes, then the files only an agent touched. Empty tags when nobody did. */
export function fileEntries(
  status: GitStatus | undefined,
  touched: readonly Touched[],
): FileEntry[] {
  const tagsFor = (rel: string): string[] => {
    const tags: string[] = [];
    for (const t of touched) {
      const hit = t.rel === rel || (rel.endsWith("/") && t.rel.startsWith(rel));
      if (hit) for (const tag of t.tags) if (!tags.includes(tag)) tags.push(tag);
    }
    return tags;
  };
  const entries: FileEntry[] = [];
  for (const e of status?.entries ?? []) {
    const rel = relFromPrefix(status?.prefix ?? "", e.path);
    const entry: FileEntry = { rel, mark: e.change, tags: tagsFor(rel) };
    if (e.orig !== undefined) entry.orig = relFromPrefix(status?.prefix ?? "", e.orig);
    entries.push(entry);
  }
  for (const t of touched) {
    const listed = entries.some(
      (e) => e.rel === t.rel || (e.rel.endsWith("/") && t.rel.startsWith(e.rel)),
    );
    if (listed) continue;
    entries.push({ rel: t.rel, mark: touchedMark(status !== undefined, t), tags: t.tags });
  }
  return entries;
}

/** One letter for the margin: M, A, D, R, ?, U, · */
export function glyphOfMark(mark: Mark): string {
  switch (mark) {
    case "modified":
      return "M";
    case "added":
      return "A";
    case "deleted":
      return "D";
    case "renamed":
      return "R";
    case "untracked":
      return "?";
    case "conflicted":
      return "U";
    case "clean":
      return "·";
  }
}
