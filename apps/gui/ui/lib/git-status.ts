// What the repository and paths say about a session's directory: `status --porcelain=v2` parsed,
// and paths shown from the session's directory. The files panel and the viewer both read them.

export type Change = "modified" | "added" | "deleted" | "renamed" | "untracked" | "conflicted";

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
