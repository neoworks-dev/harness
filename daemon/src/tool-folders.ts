// The folders of tool files the tools service runs (see services/tools), and which of their files
// may run. There are three kinds:
//   user     ~/.config/neo/tools (NEO_TOOLS_DIR)        every session sees it, app "tools"
//   project  <project>/.neo/tools                        sessions inside the project
//   session  <dataDir>/session-tools/<session>          that session and its subagents; deleted
//                                                        with the session
// A tool file is code that runs with the user's rights, written by an agent (maybe without a
// prompt: bash, /approvals auto) or brought by a repository, so each runs only as the user allowed
// it: by the content it has (a hash, with the files of its folder it may import:
// services/tools/src/approval.ts). A new or changed file asks, right when it appears (the
// session's next turn, or the end of any tool call that may have written it), and one that
// changed since it was allowed stops until it is allowed again. The answers are kept, the hashes
// of a folder's allowed files go to a file its instance of the service reads, and that instance's
// connector is visible only to the sessions the folder is for.
import { createHash } from "node:crypto";
import { mkdirSync, readdirSync, renameSync, rmSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { basename, dirname, join, sep } from "node:path";
import { neoConfigDir } from "@neoworks-dev/neo/mcp-config";
import { fingerprint, signature, toolFileNames } from "../../services/tools/src/approval.ts";
import type { Asker } from "./asker.ts";
import type { Logger } from "./log.ts";
import type { SessionStore } from "./store.ts";

/** What the user said of a folder's files: each file's name and the hash it was said of. */
interface Verdicts {
  allow: Record<string, string>;
  deny: Record<string, string>;
}

type Kind = "user" | "project" | "session";

/** One folder of tool files, and whom it is for. */
interface Folder {
  folder: string;
  app: string;
  kind: Kind;
  /** The project's directory, or the session's id; empty for the user's folder. */
  owner: string;
}

const META_KEY = "tool_folders";
const MAIN = join(import.meta.dir, "..", "..", "services", "tools", "main.ts");

export interface ToolFoldersOptions {
  store: Pick<SessionStore, "getMeta" | "setMeta"> & {
    getSession(id: string): { cwd: string; parent?: string } | undefined;
  };
  asker: Pick<Asker, "ask">;
  /** The agent the question is asked as. */
  agent: string;
  log: Logger;
  /** Where the hashes of the allowed files are written, and the sessions' folders are. */
  dataDir: string;
  /** The user's own folder. */
  userFolder: string;
  /** Whether projects' tools may run at all (the setting): off, they don't run and don't ask. */
  enabled: () => boolean;
}

/** The user's own tools folder: NEO_TOOLS_DIR, else ~/.config/neo/tools (as the service has it). */
export function userToolsDir(): string {
  if (process.env.NEO_TOOLS_DIR) return process.env.NEO_TOOLS_DIR;
  return join(neoConfigDir(), "tools");
}

/** The nearest directory from `cwd` up whose .neo/tools has tool files; never the home directory. */
export function projectOf(cwd: string): string | undefined {
  const home = homedir();
  let dir = cwd;
  for (;;) {
    if (dir === home) return undefined;
    if (toolFileNames(projectFolder(dir)).length > 0) return dir;
    const parent = dirname(dir);
    if (parent === dir) return undefined;
    dir = parent;
  }
}

function projectFolder(dir: string): string {
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

/** The app name of a session's tools. */
export function sessionAppOf(session: string): string {
  return `tools-session-${session
    .toLowerCase()
    .replace(/[^a-z0-9]/g, "")
    .slice(0, 8)}`;
}

function isStrings(value: unknown): value is Record<string, string> {
  return (
    typeof value === "object" &&
    value !== null &&
    Object.values(value).every((v) => typeof v === "string")
  );
}

export class ToolFolders {
  private readonly decisions: Record<string, Verdicts> = {};
  private readonly asking = new Set<string>();
  private readonly listeners: (() => void)[] = [];
  /** The hashes of a folder, kept while its files' sizes and times stay the same. */
  private readonly scans = new Map<string, { signature: string; prints: Record<string, string> }>();

  constructor(private readonly options: ToolFoldersOptions) {
    try {
      const stored: unknown = JSON.parse(options.store.getMeta(META_KEY) ?? "{}");
      if (typeof stored === "object" && stored !== null) {
        for (const [folder, value] of Object.entries(stored)) {
          const verdicts = value as { allow?: unknown; deny?: unknown } | null;
          if (isStrings(verdicts?.allow) && isStrings(verdicts?.deny)) {
            this.decisions[folder] = { allow: verdicts.allow, deny: verdicts.deny };
          }
        }
      }
    } catch {
      // Unreadable: every file asks again.
    }
    // The folders of sessions deleted while the daemon was not running.
    for (const session of this.sessionIds()) {
      if (!options.store.getSession(session)) this.forget(session, false);
    }
    // What the services read has to be there before they start.
    this.writeAllowed(this.userFolder());
    for (const folder of Object.keys(this.decisions)) this.writeAllowed(this.describe(folder));
  }

  /** Called when what may run changes. */
  onChange(listener: () => void): void {
    this.listeners.push(listener);
  }

  /** Where the agent puts the tools of a session. */
  sessionFolder(session: string): string {
    return join(this.sessionsRoot(), session);
  }

  /**
   * What the tools service has to run now: the user's folder always (it carries the skill), and
   * each project's and session's that has a file allowed.
   */
  commands(): string[][] {
    const user = this.userFolder();
    const commands = [[...this.command(user), "--sessions", this.sessionsRoot()]];
    for (const [path, verdicts] of Object.entries(this.decisions)) {
      const folder = this.describe(path);
      if (folder.kind === "user" || Object.keys(verdicts.allow).length === 0) continue;
      if (folder.kind === "project" && !this.options.enabled()) continue;
      if (folder.kind === "session" && !this.options.store.getSession(folder.owner)) continue;
      if (toolFileNames(path).length === 0) continue;
      commands.push(this.command(folder));
    }
    return commands;
  }

  private command(folder: Folder): string[] {
    return [
      process.execPath,
      "run",
      MAIN,
      "--dir",
      folder.folder,
      "--app",
      folder.app,
      "--scope",
      folder.kind,
      "--approved",
      this.allowedPath(folder.app),
    ];
  }

  /**
   * Whether the session may see the connector: a project's tools only from inside that project,
   * a session's from that session and the subagents under it, everything else from anywhere; a
   * call with no session sees all.
   */
  visible(app: string, session: string | undefined): boolean {
    if (session === undefined) return true;
    const owner = this.ownerOf(app);
    if (owner === undefined) return true;
    if (owner.kind === "session") return this.lineage(session).includes(owner.owner);
    const cwd = this.options.store.getSession(session)?.cwd;
    return cwd !== undefined && (cwd === owner.owner || cwd.startsWith(owner.owner + sep));
  }

  /** The session and the ones it was delegated from, nearest first. */
  private lineage(session: string): string[] {
    const out: string[] = [];
    let id: string | undefined = session;
    while (id !== undefined && !out.includes(id)) {
      out.push(id);
      id = this.options.store.getSession(id)?.parent;
    }
    return out;
  }

  /** The project or session folder an app runs (it runs only once a file of it was answered). */
  private ownerOf(app: string): Folder | undefined {
    if (app === "tools") return undefined;
    for (const path of Object.keys(this.decisions)) {
      const folder = this.describe(path);
      if (folder.app === app) return folder;
    }
    return undefined;
  }

  /**
   * Asks the user, in this session, about the files of the folders it sees that nobody said
   * anything of as they are now, unless a question is already open for that folder. Doesn't wait
   * for the answer. Cheap when nothing changed: it runs after every tool call.
   */
  check(session: string): void {
    for (const folder of this.foldersOf(session)) {
      if (this.asking.has(folder.folder)) continue;
      const unanswered = this.unanswered(folder.folder);
      if (unanswered.length === 0) continue;
      this.asking.add(folder.folder);
      this.ask(session, folder, unanswered)
        .then((answered) => {
          this.asking.delete(folder.folder);
          // Files that changed while the question was open. A question that went unanswered
          // (dropped with its turn, the daemon stopping) waits for the next turn or tool call:
          // asked again now, it would land in a session nobody may be looking at.
          if (answered) this.check(session);
        })
        .catch((err: unknown) => {
          this.asking.delete(folder.folder);
          this.options.log.warn(
            `tool folders: ${err instanceof Error ? err.message : String(err)}`,
          );
        });
    }
  }

  /** The folders a session may have tools in, that exist. */
  private foldersOf(session: string): Folder[] {
    const row = this.options.store.getSession(session);
    if (!row) return [];
    const out = [this.userFolder()];
    const dir = this.options.enabled() ? projectOf(row.cwd) : undefined;
    if (dir !== undefined) out.push(this.describe(projectFolder(dir)));
    out.push(this.describe(this.sessionFolder(session)));
    return out;
  }

  /** The hashes of the folder's tool files now. */
  private prints(folder: string): Record<string, string> {
    const now = signature(folder);
    const kept = this.scans.get(folder);
    if (kept?.signature === now) return kept.prints;
    const prints = fingerprint(folder);
    this.scans.set(folder, { signature: now, prints });
    return prints;
  }

  /** The files nobody said anything of as they are now, with the hash they have. */
  private unanswered(folder: string): [string, string][] {
    const verdicts = this.decisions[folder];
    return Object.entries(this.prints(folder)).filter(
      ([name, hash]) => verdicts?.allow[name] !== hash && verdicts?.deny[name] !== hash,
    );
  }

  /** Asks about the files; whether the user answered. */
  private async ask(session: string, folder: Folder, files: [string, string][]): Promise<boolean> {
    const verdicts = this.decisions[folder.folder];
    // A file that was answered before, with other content, is a changed one.
    const names = files.map(([name]) =>
      verdicts?.allow[name] !== undefined || verdicts?.deny[name] !== undefined
        ? `${name} (changed)`
        : name,
    );
    const question = {
      user: { title: "Run your tools?", key: "Folder", who: "every session" },
      project: {
        title: "Run this project's tools?",
        key: "Project",
        who: "sessions in this project",
      },
      session: { title: "Run this session's tools?", key: "Folder", who: "this session" },
    }[folder.kind];
    const answer = await this.options.asker.ask(session, {
      type: "permission_request",
      agent: this.options.agent,
      title: question.title,
      details: [
        { key: question.key, value: folder.kind === "project" ? folder.owner : folder.folder },
        { key: "Files", value: names.join(", ") },
      ],
      content: [
        {
          type: "text",
          text: `These files run as tools with your rights, for ${question.who}. Allow them only if you trust what they do. A change to them asks again; what they run or read outside their folder is not covered.`,
        },
      ],
      options: [
        { id: "allow", name: "Allow", kind: "allow_always" },
        { id: "deny", name: "Don't allow", kind: "reject_always" },
      ],
    });
    if (answer !== "allow" && answer !== "deny") return false;
    this.decide(folder.folder, Object.fromEntries(files), answer);
    return true;
  }

  /** Keeps what the user said of these files as they were, and lets the daemon act on it. */
  decide(folder: string, files: Record<string, string>, decision: "allow" | "deny"): void {
    const verdicts = (this.decisions[folder] ??= { allow: {}, deny: {} });
    for (const [name, hash] of Object.entries(files)) {
      delete verdicts.allow[name];
      delete verdicts.deny[name];
      verdicts[decision][name] = hash;
    }
    this.save();
    this.writeAllowed(this.describe(folder));
    this.changed();
  }

  /** A session was deleted: its folder, its answers and its service go with it. */
  forget(session: string, notify = true): void {
    const folder = this.sessionFolder(session);
    rmSync(folder, { recursive: true, force: true });
    rmSync(this.allowedPath(sessionAppOf(session)), { force: true });
    this.scans.delete(folder);
    if (this.decisions[folder] === undefined) return;
    delete this.decisions[folder];
    this.save();
    if (notify) this.changed();
  }

  /** Where the hashes of an app's allowed files are written. */
  allowedPath(app: string): string {
    return join(this.options.dataDir, "tool-approvals", `${app}.json`);
  }

  private save(): void {
    this.options.store.setMeta(META_KEY, JSON.stringify(this.decisions));
  }

  private changed(): void {
    for (const listener of this.listeners) listener();
  }

  private userFolder(): Folder {
    return { folder: this.options.userFolder, app: "tools", kind: "user", owner: "" };
  }

  private sessionsRoot(): string {
    return join(this.options.dataDir, "session-tools");
  }

  /** The ids of the sessions that have a folder. */
  private sessionIds(): string[] {
    try {
      return readdirSync(this.sessionsRoot(), { withFileTypes: true })
        .filter((entry) => entry.isDirectory())
        .map((entry) => entry.name);
    } catch {
      return [];
    }
  }

  /** What a folder is, from its path. */
  private describe(folder: string): Folder {
    if (folder === this.options.userFolder) return this.userFolder();
    if (dirname(folder) === this.sessionsRoot()) {
      const session = basename(folder);
      return { folder, app: sessionAppOf(session), kind: "session", owner: session };
    }
    const dir = dirname(dirname(folder));
    return { folder, app: appOf(dir), kind: "project", owner: dir };
  }

  /** Writes the hashes of the allowed files for the folder's service: whole or not at all. */
  private writeAllowed(folder: Folder): void {
    const path = this.allowedPath(folder.app);
    try {
      mkdirSync(dirname(path), { recursive: true });
      const part = `${path}.part`;
      writeFileSync(part, JSON.stringify(this.decisions[folder.folder]?.allow ?? {}));
      renameSync(part, path);
    } catch (err) {
      this.options.log.warn(`tool folders: can't write ${path}: ${String(err)}`);
    }
  }
}
