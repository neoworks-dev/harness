// The tools a project brings: tool files in <project>/.neo/tools (see services/tools). A repository's
// files are code that would run with the user's rights, so each runs only as the user allowed it:
// by the content it has (a hash, with the helpers it may import: services/tools/src/approval.ts).
// A new or changed file asks, right when it appears (the session's next turn, or the end of any
// tool call that may have written it), and a file that changed since it was allowed stops until
// it is allowed again. The answers are kept, the hashes of the allowed files go to a file the
// project's instance of the service reads, and its connector is visible only to sessions inside
// the project.
import { createHash } from "node:crypto";
import { mkdirSync, renameSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { basename, dirname, join, sep } from "node:path";
import { fingerprint, signature, toolFileNames } from "../../services/tools/src/approval.ts";
import type { Asker } from "./asker.ts";
import type { Logger } from "./log.ts";
import type { SessionStore } from "./store.ts";

/** What the user said of a project's files: each file's name and the hash it was said of. */
interface Verdicts {
  allow: Record<string, string>;
  deny: Record<string, string>;
}

const META_KEY = "project_tools";
const MAIN = join(import.meta.dir, "..", "..", "services", "tools", "main.ts");

export interface ProjectToolsOptions {
  store: Pick<SessionStore, "getMeta" | "setMeta"> & {
    getSession(id: string): { cwd: string } | undefined;
  };
  asker: Pick<Asker, "ask">;
  /** The agent the question is asked as. */
  agent: string;
  log: Logger;
  /** Where the hashes of the allowed files are written, for the services to read. */
  dataDir: string;
  /** Whether projects' tools may run at all (the setting): off, nothing runs and nothing asks. */
  enabled: () => boolean;
}

/** The nearest directory from `cwd` up whose .neo/tools has tool files; never the home directory. */
export function projectOf(cwd: string): string | undefined {
  const home = homedir();
  let dir = cwd;
  for (;;) {
    if (dir === home) return undefined;
    if (toolFileNames(folderOf(dir)).length > 0) return dir;
    const parent = dirname(dir);
    if (parent === dir) return undefined;
    dir = parent;
  }
}

function folderOf(dir: string): string {
  return join(dir, ".neo", "tools");
}

/** The app name of a project's tools: unique to the directory, readable in a list. */
export function appOf(dir: string): string {
  const name = basename(dir)
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-|-$/g, "")
    .slice(0, 20);
  const hash = createHash("sha256").update(dir).digest("hex").slice(0, 4);
  return `tools-${name === "" ? "project" : name}-${hash}`;
}

function isStrings(value: unknown): value is Record<string, string> {
  return (
    typeof value === "object" &&
    value !== null &&
    Object.values(value).every((v) => typeof v === "string")
  );
}

export class ProjectTools {
  private readonly decisions: Record<string, Verdicts> = {};
  private readonly asking = new Set<string>();
  private readonly listeners: (() => void)[] = [];
  /** The hashes of a folder, kept while its files' sizes and times stay the same. */
  private readonly scans = new Map<string, { signature: string; prints: Record<string, string> }>();

  constructor(private readonly options: ProjectToolsOptions) {
    try {
      const stored: unknown = JSON.parse(options.store.getMeta(META_KEY) ?? "{}");
      if (typeof stored === "object" && stored !== null) {
        for (const [dir, value] of Object.entries(stored)) {
          const verdicts = value as { allow?: unknown; deny?: unknown } | null;
          if (isStrings(verdicts?.allow) && isStrings(verdicts?.deny)) {
            this.decisions[dir] = { allow: verdicts.allow, deny: verdicts.deny };
          }
        }
      }
    } catch {
      // Unreadable: every file asks again.
    }
    // What the services read has to be there before they start.
    for (const dir of Object.keys(this.decisions)) this.writeAllowed(dir);
  }

  /** Called when what may run changes. */
  onChange(listener: () => void): void {
    this.listeners.push(listener);
  }

  /** What the tools service has to run now: one instance for each project with a file allowed. */
  commands(): string[][] {
    if (!this.options.enabled()) return [];
    const commands: string[][] = [];
    for (const [dir, verdicts] of Object.entries(this.decisions)) {
      if (Object.keys(verdicts.allow).length === 0) continue;
      if (toolFileNames(folderOf(dir)).length === 0) continue;
      commands.push([
        process.execPath,
        "run",
        MAIN,
        "--dir",
        folderOf(dir),
        "--app",
        appOf(dir),
        "--approved",
        this.allowedPath(dir),
      ]);
    }
    return commands;
  }

  /**
   * Whether the session may see the connector: a project's tools only from inside that project,
   * everything else from anywhere; a call with no session sees all.
   */
  visible(app: string, session: string | undefined): boolean {
    if (session === undefined) return true;
    const dir = Object.keys(this.decisions).find((d) => appOf(d) === app);
    if (dir === undefined) return true;
    const cwd = this.options.store.getSession(session)?.cwd;
    return cwd !== undefined && (cwd === dir || cwd.startsWith(dir + sep));
  }

  /**
   * Asks the user, in this session, about the files of the project it works in that nobody said
   * anything of as they are now, unless one question is already open for the project. Doesn't
   * wait for the answer. Cheap when nothing changed: it runs after every tool call.
   */
  check(session: string): void {
    if (!this.options.enabled()) return;
    const cwd = this.options.store.getSession(session)?.cwd;
    const dir = cwd === undefined ? undefined : projectOf(cwd);
    if (dir === undefined || this.asking.has(dir)) return;
    const unanswered = this.unanswered(dir);
    if (unanswered.length === 0) return;
    this.asking.add(dir);
    this.ask(session, dir, unanswered)
      .catch((err: unknown) => {
        this.options.log.warn(`project tools: ${err instanceof Error ? err.message : String(err)}`);
      })
      .finally(() => {
        this.asking.delete(dir);
        // Files that changed while the question was open.
        this.check(session);
      });
  }

  /** The hashes of the project's tool files now. */
  private prints(dir: string): Record<string, string> {
    const folder = folderOf(dir);
    const now = signature(folder);
    const kept = this.scans.get(dir);
    if (kept?.signature === now) return kept.prints;
    const prints = fingerprint(folder);
    this.scans.set(dir, { signature: now, prints });
    return prints;
  }

  /** The files nobody said anything of as they are now, with the hash they have. */
  private unanswered(dir: string): [string, string][] {
    const verdicts = this.decisions[dir];
    return Object.entries(this.prints(dir)).filter(
      ([name, hash]) => verdicts?.allow[name] !== hash && verdicts?.deny[name] !== hash,
    );
  }

  private async ask(session: string, dir: string, files: [string, string][]): Promise<void> {
    const verdicts = this.decisions[dir];
    // A file that was answered before, with other content, is a changed one.
    const names = files.map(([name]) =>
      verdicts?.allow[name] !== undefined || verdicts?.deny[name] !== undefined
        ? `${name} (changed)`
        : name,
    );
    const answer = await this.options.asker.ask(session, {
      type: "permission_request",
      agent: this.options.agent,
      title: "Run this project's tools?",
      details: [
        { key: "Project", value: dir },
        { key: "Files", value: names.join(", ") },
      ],
      content: [
        {
          type: "text",
          text: "These files run as tools with your rights, for the agents working in this project. Allow them only if you trust what they do; a file that changes later asks again.",
        },
      ],
      options: [
        { id: "allow", name: "Allow", kind: "allow_once" },
        { id: "deny", name: "Don't allow", kind: "reject_once" },
      ],
    });
    // No answer (the turn ended, the session went): ask again next time.
    if (answer !== "allow" && answer !== "deny") return;
    this.decide(dir, Object.fromEntries(files), answer);
  }

  /** Keeps what the user said of these files as they were, and lets the daemon act on it. */
  decide(dir: string, files: Record<string, string>, decision: "allow" | "deny"): void {
    const verdicts = (this.decisions[dir] ??= { allow: {}, deny: {} });
    for (const [name, hash] of Object.entries(files)) {
      delete verdicts.allow[name];
      delete verdicts.deny[name];
      verdicts[decision][name] = hash;
    }
    this.options.store.setMeta(META_KEY, JSON.stringify(this.decisions));
    this.writeAllowed(dir);
    for (const listener of this.listeners) listener();
  }

  /** Where the hashes of a project's allowed files are written. */
  allowedPath(dir: string): string {
    return join(this.options.dataDir, "project-tools", `${appOf(dir)}.json`);
  }

  /** Writes the hashes of the allowed files for the project's service: whole or not at all. */
  private writeAllowed(dir: string): void {
    const path = this.allowedPath(dir);
    try {
      mkdirSync(dirname(path), { recursive: true });
      const part = `${path}.part`;
      writeFileSync(part, JSON.stringify(this.decisions[dir]?.allow ?? {}));
      renameSync(part, path);
    } catch (err) {
      this.options.log.warn(`project tools: can't write ${path}: ${String(err)}`);
    }
  }
}
