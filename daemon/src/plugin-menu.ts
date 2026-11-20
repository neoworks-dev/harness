// /plugin as a menu that stays open: the installed plugins (pick one to enable, disable or
// uninstall it), what the marketplaces offer (pick one to install it), the marketplaces (update,
// remove) and a line to add one. What is picked is done by the /plugin commands
// (plugins.ts); this builds what the menu shows. Plain functions.
import type { Choice, ChoiceOption } from "@neoworks-dev/neo-protocol/types";

/** Which screen of the menu is shown. */
export type MenuView =
  | { kind: "top" }
  | { kind: "plugin"; id: string }
  | { kind: "install" }
  | { kind: "market"; name: string };

export interface MenuData {
  installed: { id: string; version?: string; enabled: boolean }[];
  markets: { name: string; plugins?: number }[];
  /** What the marketplaces offer that isn't installed. */
  available: { id: string; description: string }[];
}

const BACK: ChoiceOption = { id: "back", name: "← Back" };

function short(text: string, max = 90): string {
  const flat = text.replace(/\s+/g, " ").trim();
  return flat.length > max ? `${flat.slice(0, max - 1)}…` : flat;
}

/** The menu as it is now; `busy` names what is being done, `note` how the last thing went. */
export function menuChoice(
  view: MenuView,
  data: MenuData,
  busy?: string,
  note?: string,
): Omit<Choice, "id"> {
  let title: string;
  let options: ChoiceOption[];
  switch (view.kind) {
    case "plugin": {
      const plugin = data.installed.find((p) => p.id === view.id);
      title = plugin ? `${plugin.id}${plugin.version ? ` ${plugin.version}` : ""}` : view.id;
      options = plugin
        ? [
            {
              id: "toggle",
              name: plugin.enabled ? "Disable" : "Enable",
              description: plugin.enabled
                ? "sessions that already started keep its skills"
                : "its skills and commands load in new sessions",
            },
            { id: "uninstall", name: "Uninstall", description: "remove it and its files" },
            BACK,
          ]
        : [BACK];
      break;
    }
    case "install":
      title = data.available.length > 0 ? "Install a plugin" : "Nothing more to install";
      options = [
        ...data.available.map((p) => ({
          id: `get:${p.id}`,
          name: p.id,
          description: short(p.description) || "no description",
        })),
        BACK,
      ];
      break;
    case "market": {
      const market = data.markets.find((m) => m.name === view.name);
      title = `Marketplace ${view.name}`;
      options = market
        ? [
            { id: "update", name: "Update", description: "fetch it again for newer plugins" },
            {
              id: "remove",
              name: "Remove",
              description: "and uninstall the plugins installed from it",
            },
            BACK,
          ]
        : [BACK];
      break;
    }
    default:
      title = "Plugins";
      options = [
        ...data.installed.map((p) => ({
          id: `plugin:${p.id}`,
          name: `${p.id} · ${p.enabled ? "enabled" : "disabled"}`,
          description: "enable, disable or uninstall",
        })),
        {
          id: "install",
          name: "Install a plugin…",
          description:
            data.markets.length === 0
              ? "add a marketplace first"
              : `${data.available.length} available from the marketplaces`,
        },
        ...data.markets.map((m) => ({
          id: `market:${m.name}`,
          name: `Marketplace ${m.name}`,
          description:
            m.plugins === undefined ? "unreadable" : `${m.plugins} plugins · update, remove`,
        })),
        {
          id: "add",
          name: "Add a marketplace…",
          description: "a path, a git URL or owner/repo",
          input: { value: "" },
        },
      ];
  }
  if (busy !== undefined) title += ` · ${busy}…`;
  else if (note !== undefined) title += ` · ${note}`;
  return { type: "choice", title, options, stays: true };
}

/** What a pick does: another screen, or a /plugin command to run (and the screen after it). */
export type MenuStep =
  | { kind: "view"; view: MenuView }
  | { kind: "run"; args: string[]; busy: string; then: MenuView }
  | { kind: "none" };

/** The step a pick of `option` (with the text an input asked for) takes from `view`. */
export function menuStep(view: MenuView, data: MenuData, option: string, value?: string): MenuStep {
  const top: MenuView = { kind: "top" };
  if (option === "back") return { kind: "view", view: top };
  if (view.kind === "top") {
    if (option === "install") return { kind: "view", view: { kind: "install" } };
    if (option.startsWith("plugin:")) {
      return { kind: "view", view: { kind: "plugin", id: option.slice(7) } };
    }
    if (option.startsWith("market:")) {
      return { kind: "view", view: { kind: "market", name: option.slice(7) } };
    }
    if (option === "add" && value !== undefined && value.trim() !== "") {
      const source = value.trim();
      return {
        kind: "run",
        args: ["marketplace", "add", source],
        busy: `adding ${source}`,
        then: { kind: "install" },
      };
    }
    return { kind: "none" };
  }
  if (view.kind === "plugin") {
    const plugin = data.installed.find((p) => p.id === view.id);
    if (!plugin) return { kind: "view", view: top };
    if (option === "toggle") {
      const verb = plugin.enabled ? "disable" : "enable";
      return { kind: "run", args: [verb, plugin.id], busy: `${verb.slice(0, -1)}ing`, then: view };
    }
    if (option === "uninstall") {
      return { kind: "run", args: ["uninstall", plugin.id], busy: "uninstalling", then: top };
    }
  }
  if (view.kind === "install" && option.startsWith("get:")) {
    const id = option.slice(4);
    return { kind: "run", args: ["install", id], busy: `installing ${id}`, then: top };
  }
  if (view.kind === "market") {
    if (option === "update") {
      return {
        kind: "run",
        args: ["marketplace", "update", view.name],
        busy: "updating",
        then: view,
      };
    }
    if (option === "remove") {
      return {
        kind: "run",
        args: ["marketplace", "remove", view.name],
        busy: "removing",
        then: top,
      };
    }
  }
  return { kind: "none" };
}
