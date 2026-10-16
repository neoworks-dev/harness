// The files panel: what git says changed, plus what the agent and its guests touched this
// session (from the diffs of their tool calls). Ported from apps/term/src/widgets/files.rs and
// git.rs; the host only runs git (`git_status`), the parsing is here.
import type { SessionEventRecord } from "@suite/protocol/types";

export type Change = "modified" | "added" | "deleted" | "renamed" | "untracked" | "conflicted";
/** What a file shows: git's change, or `clean` for one an agent touched that git sees no difference in. */
export type Mark = Change | "clean";

export interface GitEntry {
  /** Relative to the repository root, as git prints it. */
  path: string;
  /** The old path of a rename or copy. */
  orig?: string;
  change: Change;
}

export interface GitStatus {
  head?: string;
  detached: boolean;
  /** Commits ahead of / behind the upstream (0 without one). */
  ahead: number;
  behind: number;
  entries: GitEntry[];
  /** Where the session's directory is inside the repository (`src/a/`, empty at the root). */
  prefix: string;
}

function changeOf(xy: string): Change {
  const has = (c: string): boolean => xy.includes(c);
  if (has("R")) return "renamed";
  if (has("D")) return "deleted";
  if (has("A") || has("C")) return "added";
  return "modified";
}

/** `git status --porcelain=v2 --branch -z` output. Unknown records are skipped. */
export function parseStatus(out: string, prefix = ""): GitStatus {
  const status: GitStatus = { detached: false, ahead: 0, behind: 0, entries: [], prefix };
  const records = out.split("\0").filter((r) => r !== "");
  for (let i = 0; i < records.length; i++) {
    const record = records[i] ?? "";
    const space = record.indexOf(" ");
    if (space < 0) continue;
    const kind = record.slice(0, space);
    const rest = record.slice(space + 1);
    // Fields are separated by single spaces; the path, last, may hold spaces.
    const fields = (count: number): string[] | undefined => {
      const parts = rest.split(" ");
      if (parts.length < count) return undefined;
      return [...parts.slice(0, count - 1), parts.slice(count - 1).join(" ")];
    };
    if (kind === "#") {
      const cut = rest.indexOf(" ");
      const key = rest.slice(0, cut);
      const value = rest.slice(cut + 1);
      if (key === "branch.head") {
        if (value === "(detached)") status.detached = true;
        else status.head = value;
      } else if (key === "branch.ab") {
        for (const part of value.split(" ")) {
          if (part.startsWith("+")) status.ahead = Number.parseInt(part.slice(1), 10) || 0;
          else if (part.startsWith("-")) status.behind = Number.parseInt(part.slice(1), 10) || 0;
        }
      }
    } else if (kind === "1") {
      // 1 <XY> <sub> <mH> <mI> <mW> <hH> <hI> <path>
      const f = fields(8);
      if (f) status.entries.push({ path: f[7] ?? "", change: changeOf(f[0] ?? "") });
    } else if (kind === "2") {
      // 2 <XY> <sub> <mH> <mI> <mW> <hH> <hI> <X><score> <path> NUL <origPath>
      const f = fields(9);
      const orig = records[++i];
      if (f) {
        const change = changeOf(f[0] ?? "");
        const entry: GitEntry = {
          path: f[8] ?? "",
          change: change === "deleted" ? change : "renamed",
        };
        if (orig !== undefined) entry.orig = orig;
        status.entries.push(entry);
      }
    } else if (kind === "u") {
      // u <XY> <sub> <m1> <m2> <m3> <mW> <h1> <h2> <h3> <path>
      const f = fields(10);
      if (f) status.entries.push({ path: f[9] ?? "", change: "conflicted" });
    } else if (kind === "?") {
      status.entries.push({ path: rest, change: "untracked" });
    }
  }
  return status;
}

/** "main ↑1 ↓2", "detached". */
export function branchLabel(status: GitStatus): string {
  let label = status.detached ? "detached" : (status.head ?? "?");
  if (status.ahead > 0) label += ` ↑${status.ahead}`;
  if (status.behind > 0) label += ` ↓${status.behind}`;
  return label;
}

/** `path` from the repository root, as seen from the directory `prefix` (`a/b/`) inside it. */
export function relFromPrefix(prefix: string, path: string): string {
  const pre = prefix.split("/").filter((c) => c !== "");
  if (pre.length === 0) return path;
  const comps = path.split("/").filter((c) => c !== "");
  // Only directories can be shared: the last component of the path is the file itself.
  const dirs = Math.max(0, comps.length - 1);
  let common = 0;
  while (common < pre.length && common < dirs && pre[common] === comps[common]) common++;
  let out = "../".repeat(pre.length - common) + comps.slice(common).join("/");
  if (path.endsWith("/")) out += "/";
  return out;
}

/** A path from a tool's diff, relative to `cwd` when it is inside it. */
export function relativize(cwd: string, path: string): string {
  const dir = cwd.replace(/\/+$/, "");
  let rel: string;
  if (path === dir) rel = "";
  else if (path.startsWith("/"))
    rel = path.startsWith(`${dir}/`) ? path.slice(dir.length + 1) : path;
  else rel = path.startsWith("./") ? path.slice(2) : path;
  return rel === "" ? "." : rel;
}

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
