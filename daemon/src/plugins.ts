// /plugin: marketplaces and the plugins installed from them (#25). The work is the
// harness's (harness/src/plugin-install.ts); this reads the command line and tells the user, in
// the session's log, what happened and what needs a new session.
//
//   /plugin                                   what is installed, and the marketplaces
//   /plugin marketplace add <owner/repo|url|path>, marketplace remove|update [name], marketplace list
//   /plugin install <name>@<marketplace>      (the marketplace may be left out when one has it)
//   /plugin uninstall|enable|disable <name>[@<marketplace>]
//   /plugin search [text]                     what the added marketplaces have
import {
  addMarketplace,
  installPlugin,
  marketplaceEntries,
  type PluginParts,
  PluginError,
  readMarketplaces,
  removeMarketplace,
  resolveInstalled,
  searchPlugins,
  setPluginEnabled,
  uninstallPlugin,
  updateMarketplaces,
} from "@suite/harness/plugin-install";
import { enabledPlugins, readInstalled } from "@suite/harness/plugins";
import { BadRequestError } from "./focus.ts";

const USAGE =
  "usage: /plugin [list] | marketplace add|remove|update|list … | install|uninstall|enable|disable <name>@<marketplace> | search [text]";

/** Most plugins `search` lists. */
const SEARCH_LIMIT = 25;

export interface PluginCommandParts {
  /** Where plugins are installed; the command is off without. */
  dir: string | undefined;
  /** Tells the user something in the session's log. */
  notice: (session: string, text: string, tone?: "muted" | "danger") => void;
  /** Called after the installed plugins changed: starts their MCP servers, updates the menu. */
  changed: (session: string) => Promise<void>;
  /** The directory sessions work in, for a relative path given to `marketplace add`. */
  cwd: (session: string) => string;
}

function plural(count: number, one: string, many = `${one}s`): string {
  return `${count} ${count === 1 ? one : many}`;
}

/** "2 skills, 1 command": what a plugin brings, in words. */
export function partsText(parts: PluginParts): string {
  const out: string[] = [];
  if (parts.skills > 0) out.push(plural(parts.skills, "skill"));
  if (parts.commands > 0) out.push(plural(parts.commands, "command"));
  if (parts.mcpServers > 0) out.push(plural(parts.mcpServers, "MCP server"));
  if (parts.lspServers > 0) out.push(plural(parts.lspServers, "language server"));
  if (parts.agents > 0) out.push(plural(parts.agents, "agent"));
  if (parts.hooks) out.push("hooks");
  return out.length > 0 ? out.join(", ") : "nothing neo uses";
}

export class PluginCommands {
  constructor(private readonly parts: PluginCommandParts) {}

  /** One command at a time: they all write installed.json. */
  private queue: Promise<unknown> = Promise.resolve();

  /** Runs `/plugin <args>`. Failures the user can act on are rejected as bad requests. */
  run(session: string, args: string[]): Promise<void> {
    const { dir } = this.parts;
    if (dir === undefined) {
      return Promise.reject(new BadRequestError("this daemon has no plugins directory"));
    }
    const work = async (): Promise<void> => {
      try {
        await this.dispatch(session, dir, args);
      } catch (err) {
        if (err instanceof PluginError) throw new BadRequestError(err.message);
        throw err;
      }
    };
    const result = this.queue.then(work, work);
    this.queue = result.catch(() => undefined);
    return result;
  }

  private async dispatch(session: string, dir: string, args: string[]): Promise<void> {
    const [sub, ...rest] = args;
    switch (sub) {
      case undefined:
      case "list":
        return this.list(session, dir);
      case "marketplace":
      case "market":
        return this.marketplace(session, dir, rest);
      case "install": {
        const [wanted, ...extra] = rest;
        if (wanted === undefined || extra.length > 0) {
          throw new BadRequestError("usage: /plugin install <name>@<marketplace>");
        }
        return this.install(session, dir, wanted);
      }
      case "uninstall":
      case "remove": {
        const id = this.installed(dir, rest, "uninstall");
        await uninstallPlugin(dir, id);
        this.parts.notice(session, `Uninstalled ${id}.`);
        return this.parts.changed(session);
      }
      case "enable":
      case "disable": {
        const id = this.installed(dir, rest, sub);
        await setPluginEnabled(dir, id, sub === "enable");
        const note =
          sub === "enable"
            ? "Its skills and commands load in new sessions."
            : "Sessions that already started keep its skills.";
        this.parts.notice(session, `${sub === "enable" ? "Enabled" : "Disabled"} ${id}. ${note}`);
        return this.parts.changed(session);
      }
      case "search":
        return this.search(session, dir, rest.join(" "));
      default:
        throw new BadRequestError(USAGE);
    }
  }

  private installed(dir: string, args: string[], verb: string): string {
    const [wanted, ...extra] = args;
    if (wanted === undefined || extra.length > 0) {
      throw new BadRequestError(`usage: /plugin ${verb} <name>@<marketplace>`);
    }
    return resolveInstalled(dir, wanted);
  }

  private async list(session: string, dir: string): Promise<void> {
    const installed = readInstalled(dir).plugins;
    const lines: string[] = [];
    const ids = Object.keys(installed).sort();
    if (ids.length === 0) lines.push("No plugins installed.");
    else lines.push("Plugins:");
    const on = new Set(enabledPlugins(dir).map((plugin) => plugin.id));
    for (const id of ids) {
      const plugin = installed[id];
      const state = on.has(id) ? "enabled" : "disabled";
      let version = "";
      if (plugin?.version && plugin.version !== "unknown") version = ` ${plugin.version}`;
      lines.push(`  ${id}${version} (${state})`);
    }
    const markets = Object.values(readMarketplaces(dir).marketplaces);
    if (markets.length === 0) {
      lines.push(
        "No marketplaces. Add one: /plugin marketplace add anthropics/claude-plugins-official",
      );
    } else {
      lines.push("Marketplaces:");
      for (const record of markets) lines.push(await this.marketLine(record));
    }
    lines.push(
      "/plugin search [text] · install <name>@<marketplace> · uninstall · enable · disable",
    );
    this.parts.notice(session, lines.join("\n"));
  }

  private async marketLine(
    record: Awaited<ReturnType<typeof addMarketplace>>["record"],
  ): Promise<string> {
    let count = "unreadable";
    try {
      count = plural((await marketplaceEntries(record)).length, "plugin");
    } catch {
      // listed anyway: the user may want to remove it
    }
    return `  ${record.name} (${count})`;
  }

  private async marketplace(session: string, dir: string, args: string[]): Promise<void> {
    const [verb, ...rest] = args;
    switch (verb) {
      case undefined:
      case "list":
        return this.list(session, dir);
      case "add": {
        if (rest.length !== 1 || rest[0] === undefined) {
          throw new BadRequestError("usage: /plugin marketplace add <owner/repo | git URL | path>");
        }
        this.parts.notice(session, `Adding ${rest[0]}…`, "muted");
        const { record, plugins } = await addMarketplace(dir, rest[0], this.parts.cwd(session));
        this.parts.notice(
          session,
          `Added marketplace ${record.name} with ${plural(plugins, "plugin")}. Install one with /plugin install <name>@${record.name}; /plugin search to look.`,
        );
        return;
      }
      case "remove": {
        if (rest.length !== 1 || rest[0] === undefined) {
          throw new BadRequestError("usage: /plugin marketplace remove <name>");
        }
        const removed = await removeMarketplace(dir, rest[0]);
        let also = "";
        if (removed.length > 0) also = ` and uninstalled ${removed.join(", ")}`;
        this.parts.notice(session, `Removed marketplace ${rest[0]}${also}.`);
        return this.parts.changed(session);
      }
      case "update": {
        if (rest.length > 1) throw new BadRequestError("usage: /plugin marketplace update [name]");
        this.parts.notice(session, "Updating…", "muted");
        const names = await updateMarketplaces(dir, rest[0]);
        this.parts.notice(
          session,
          `Updated ${names.join(", ")}. /plugin install <name>@<marketplace> fetches a newer plugin.`,
        );
        return;
      }
      default:
        throw new BadRequestError("usage: /plugin marketplace add|remove|update|list");
    }
  }

  private async install(session: string, dir: string, wanted: string): Promise<void> {
    this.parts.notice(session, `Installing ${wanted}…`, "muted");
    const result = await installPlugin(dir, wanted);
    const { parts } = result;
    const notes: string[] = [];
    if (parts.skills > 0 || parts.commands > 0) {
      notes.push("Its skills and commands are there in new sessions (/clear starts one).");
    }
    if (parts.mcpServers > 0) notes.push("Its MCP servers start now.");
    if (parts.lspServers > 0) notes.push("Its language servers are used from the next file.");
    if (parts.agents > 0 || parts.hooks) {
      notes.push("Its agents and hooks aren't used by neo yet.");
    }
    this.parts.notice(
      session,
      `Installed ${result.id} ${result.version}: ${partsText(parts)}. ${notes.join(" ")}`.trim(),
    );
    await this.parts.changed(session);
  }

  private async search(session: string, dir: string, text: string): Promise<void> {
    const found = await searchPlugins(dir, text);
    if (Object.keys(readMarketplaces(dir).marketplaces).length === 0) {
      throw new BadRequestError(
        "no marketplaces yet: /plugin marketplace add anthropics/claude-plugins-official",
      );
    }
    if (found.length === 0) {
      this.parts.notice(session, `No plugin matches "${text}".`);
      return;
    }
    const lines = found.slice(0, SEARCH_LIMIT).map(({ id, description }) => {
      const flat = description.replace(/\s+/g, " ");
      return `  ${id}  ${flat.length > 90 ? `${flat.slice(0, 89)}…` : flat}`;
    });
    if (found.length > SEARCH_LIMIT)
      lines.push(`  … ${found.length - SEARCH_LIMIT} more: narrow the search`);
    this.parts.notice(session, lines.join("\n"));
  }
}
