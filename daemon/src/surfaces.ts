// Surfaces (docs/neoui.md): neoui programs of a session that stay, drawn in a fixed place of the
// client instead of in the chat. They open and close in the session's log. They are written by the
// agent (the `ui_surface` tool, which checks a program before it shows) or come from files: the
// user's `~/.config/neo/surfaces/*.neoui` and the enabled plugins' `surfaces/*.neoui`, opened with
// `/surface <name>`, by the agent by name, or in every new session (`open: always`).
import { readdirSync, readFileSync } from "node:fs";
import { basename, join } from "node:path";
import type { CallToolResult } from "@modelcontextprotocol/sdk/types.js";
import type { AppComponents } from "@neoworks/neoui";
import { neoConfigDir } from "@neoworks-dev/neo/mcp-config";
import { enabledPlugins } from "@neoworks-dev/neo/plugins";
import type { SurfaceOpened, SurfacePlace } from "@neoworks-dev/neo-protocol/types";
import type { SessionStore } from "./store.ts";
import { mistakesIn, mistakesNote } from "./ui-lint.ts";

/** The user's own surface files: NEO_SURFACES_DIR, else ~/.config/neo/surfaces. */
export function userSurfacesDir(): string {
  if (process.env.NEO_SURFACES_DIR) return process.env.NEO_SURFACES_DIR;
  return join(neoConfigDir(), "surfaces");
}

const PLACES: readonly SurfacePlace[] = ["bottom", "side", "page"];
/** A name is a short slug: it is in the ref and in /surface. */
const NAME = /^[a-z0-9][a-z0-9-]{0,39}$/;
const EXTENSION = ".neoui";

/** A surface file: a neoui program with what it is called and where it goes. */
export interface SurfaceFile {
  name: string;
  place: SurfacePlace;
  title?: string;
  description?: string;
  /** Opens in every new session. */
  always: boolean;
  source: string;
  /** Where it came from: the user's folder, or a plugin by name. */
  from: string;
}

function text(value: string, isError = false): CallToolResult {
  const result: CallToolResult = { content: [{ type: "text", text: value }] };
  if (isError) result.isError = true;
  return result;
}

/**
 * A surface file's text: an optional frontmatter (`---` lines around `key: value` lines: name,
 * place, title, description, open), then the program. Undefined when it names no valid name or
 * place, or has no program.
 */
export function parseSurfaceFile(
  content: string,
  fallbackName: string,
  from: string,
): SurfaceFile | undefined {
  const fields: Record<string, string> = {};
  let source = content;
  const front = /^---\r?\n([\s\S]*?)\r?\n---\r?\n?/.exec(content);
  if (front) {
    source = content.slice(front[0].length);
    for (const line of (front[1] ?? "").split(/\r?\n/)) {
      const at = line.indexOf(":");
      if (at <= 0) continue;
      fields[line.slice(0, at).trim()] = line
        .slice(at + 1)
        .trim()
        .replace(/^["']|["']$/g, "");
    }
  }
  const name = fields.name ?? fallbackName;
  const place = fields.place ?? "bottom";
  if (!NAME.test(name) || !PLACES.includes(place as SurfacePlace)) return undefined;
  if (source.trim() === "") return undefined;
  const file: SurfaceFile = {
    name,
    place: place as SurfacePlace,
    always: fields.open === "always",
    source: source.trim(),
    from,
  };
  if (fields.title) file.title = fields.title;
  if (fields.description) file.description = fields.description;
  return file;
}

function readFolder(dir: string, from: string): SurfaceFile[] {
  let names: string[];
  try {
    names = readdirSync(dir).filter((name) => name.endsWith(EXTENSION));
  } catch {
    return [];
  }
  const out: SurfaceFile[] = [];
  for (const name of names.sort()) {
    try {
      const file = parseSurfaceFile(
        readFileSync(join(dir, name), "utf8"),
        basename(name, EXTENSION),
        from,
      );
      if (file) out.push(file);
    } catch {
      // A file that can't be read is left out.
    }
  }
  return out;
}

/** The surfaces open in the session, by name, in the order they (last) opened. */
export function openSurfaces(
  store: Pick<SessionStore, "readTypes">,
  session: string,
): SurfaceOpened[] {
  const open = new Map<string, SurfaceOpened>();
  for (const { event } of store.readTypes(session, ["surface_opened", "surface_closed"])) {
    if (event.type === "surface_opened") {
      open.delete(event.name);
      open.set(event.name, event);
    } else if (event.type === "surface_closed") {
      open.delete(event.name);
    }
  }
  return [...open.values()];
}

export interface SurfacesOptions {
  store: Pick<SessionStore, "readTypes" | "append">;
  /** The user's folder of surface files; none when absent. */
  dir?: string | undefined;
  /** Where plugins are installed: the enabled ones' `surfaces/` are read. None when absent. */
  plugins?: string | undefined;
  /** The apps' components, which a program may use. */
  apps?: () => AppComponents;
  /** Tells the user something in the session's log. */
  notice: (session: string, text: string, tone?: "muted" | "danger") => void;
}

type Opened = { ok: true; event: SurfaceOpened; updated: boolean } | { ok: false; error: string };

export class Surfaces {
  constructor(private readonly options: SurfacesOptions) {}

  /** The surface files there are, read now: the user's win a name a plugin has too. */
  files(): SurfaceFile[] {
    const byName = new Map<string, SurfaceFile>();
    for (const plugin of enabledPlugins(this.options.plugins)) {
      for (const file of readFolder(join(plugin.root, "surfaces"), plugin.name)) {
        if (!byName.has(file.name)) byName.set(file.name, file);
      }
    }
    if (this.options.dir !== undefined) {
      for (const file of readFolder(this.options.dir, "user")) byName.set(file.name, file);
    }
    return [...byName.values()].sort((a, b) => a.name.localeCompare(b.name));
  }

  /** Opens (or updates) a surface after checking its program. */
  open(
    session: string,
    name: string,
    source: string,
    place?: SurfacePlace,
    title?: string,
  ): Opened {
    const apps = this.options.apps?.() ?? {};
    const wrong = mistakesIn(`\`\`\`neoui\n${source}\n\`\`\``, apps);
    if (wrong.length > 0) {
      const said = mistakesNote(wrong, apps).split("\n").slice(1).join("\n");
      return { ok: false, error: `The program has mistakes, so it was not opened:\n${said}` };
    }
    const was = openSurfaces(this.options.store, session).find((s) => s.name === name);
    const event: SurfaceOpened = {
      type: "surface_opened",
      name,
      place: place ?? was?.place ?? "bottom",
      source,
    };
    const named = title ?? was?.title;
    if (named !== undefined) event.title = named;
    this.options.store.append(session, event);
    return { ok: true, event, updated: was !== undefined };
  }

  /** Closes a surface; false when none of that name is open. */
  close(session: string, name: string): boolean {
    if (!openSurfaces(this.options.store, session).some((s) => s.name === name)) return false;
    this.options.store.append(session, { type: "surface_closed", name });
    return true;
  }

  /** A new session: the files that ask to open with every session open in it. */
  openAlways(session: string): void {
    for (const file of this.files()) {
      if (!file.always) continue;
      const opened = this.open(session, file.name, file.source, file.place, file.title);
      if (!opened.ok)
        this.options.notice(session, `surface ${file.name}: ${opened.error}`, "danger");
    }
  }

  /**
   * The `ui_surface` tool: `open` a surface with a program (or replace the open one of that name,
   * to update it), or a file by its name; `close` one; `list` the open ones and the files.
   */
  tool(session: string, args: Record<string, unknown>): CallToolResult {
    const action = args.action ?? "list";
    if (action === "list") return text(this.listText(session));
    const name = args.name;
    if (typeof name !== "string" || !NAME.test(name)) {
      return text(
        "ui_surface: name is required: lower case letters, digits and -, like training-status",
        true,
      );
    }
    if (action === "close") {
      if (!this.close(session, name)) return text(`ui_surface: no surface ${name} is open`, true);
      return text(`Closed ${name}.`);
    }
    if (action !== "open") return text("ui_surface: action is open, close or list", true);
    const { place, title } = args;
    if (place !== undefined && !PLACES.includes(place as SurfacePlace)) {
      return text("ui_surface: place is bottom, side or page", true);
    }
    if (title !== undefined && typeof title !== "string") {
      return text("ui_surface: title must be a string", true);
    }
    let source = args.source;
    let file: SurfaceFile | undefined;
    if (source === undefined) {
      file = this.files().find((f) => f.name === name);
      if (file === undefined) {
        return text(`ui_surface: source is required (no surface file is called ${name})`, true);
      }
      source = file.source;
    }
    if (typeof source !== "string" || source.trim() === "") {
      return text("ui_surface: source is required: the neoui program", true);
    }
    const opened = this.open(
      session,
      name,
      source,
      (place as SurfacePlace | undefined) ?? file?.place,
      title ?? file?.title,
    );
    if (!opened.ok) return text(`ui_surface: ${opened.error}\nFix them and open it again.`, true);
    return text(`${opened.updated ? "Updated" : "Opened"} ${name} (${opened.event.place}).`);
  }

  /**
   * /surface: no name lists the files and the open surfaces, `/surface <name>` opens a file,
   * `/surface close <name>` closes one.
   */
  command(session: string, args: string[]): Promise<void> {
    const [first, second] = args;
    const { notice } = this.options;
    if (first === undefined) {
      notice(session, this.listText(session));
    } else if (first === "close") {
      if (second === undefined) notice(session, "/surface close <name>", "danger");
      else if (!this.close(session, second))
        notice(session, `No surface ${second} is open.`, "danger");
    } else {
      const file = this.files().find((f) => f.name === first);
      if (file === undefined) {
        notice(session, `No surface file is called ${first}. /surface lists them.`, "danger");
      } else {
        const opened = this.open(session, file.name, file.source, file.place, file.title);
        if (!opened.ok) notice(session, `surface ${file.name}: ${opened.error}`, "danger");
      }
    }
    return Promise.resolve();
  }

  private listText(session: string): string {
    const open = openSurfaces(this.options.store, session);
    const files = this.files();
    const lines: string[] = [];
    lines.push(open.length === 0 ? "No surfaces are open." : "Open:");
    for (const s of open) lines.push(`- ${s.name} (${s.place})${s.title ? `: ${s.title}` : ""}`);
    if (files.length > 0) {
      lines.push("Surface files (open one by name):");
      for (const f of files) {
        const about = f.description ?? f.title;
        lines.push(`- ${f.name} (${f.place}, ${f.from})${about ? `: ${about}` : ""}`);
      }
    }
    return lines.join("\n");
  }
}
