// Language servers a plugin brings (grove#495): `lspServers` in its marketplace entry or manifest,
// or a `.lsp.json` in its folder:
//   { "clangd": { "command": "clangd", "args": ["--background-index"],
//                 "extensionToLanguage": { ".c": "c", ".cpp": "cpp" } } }
// services/lsp adds them to its fixed set.
import { join } from "node:path";
import { readJsonObject, readManifest } from "./plugin-parts.ts";
import { enabledPlugins, expandPluginRoot, type Plugin } from "./plugins.ts";
import { isRecord } from "./util.ts";

export interface PluginLspServer {
  /** The plugin's id, `name@marketplace`. */
  plugin: string;
  /** The server's name in the plugin, e.g. clangd. */
  name: string;
  /** argv; the first entry is the program. */
  command: string[];
  /** LSP languageId per file extension (lower-case, with the dot). */
  extensions: Record<string, string>;
}

function parseServers(table: unknown, plugin: Plugin): PluginLspServer[] {
  if (!isRecord(table)) return [];
  const out: PluginLspServer[] = [];
  for (const [name, value] of Object.entries(table)) {
    if (!isRecord(value) || typeof value.command !== "string" || value.command === "") continue;
    if (!isRecord(value.extensionToLanguage)) continue;
    const args: string[] = [];
    if (Array.isArray(value.args)) {
      for (const arg of value.args) if (typeof arg === "string") args.push(arg);
    }
    const extensions: Record<string, string> = {};
    for (const [ext, language] of Object.entries(value.extensionToLanguage)) {
      if (typeof language === "string") extensions[ext.toLowerCase()] = language;
    }
    if (Object.keys(extensions).length === 0) continue;
    out.push({
      plugin: plugin.id,
      name,
      command: [value.command, ...args].map((part) => expandPluginRoot(part, plugin)),
      extensions,
    });
  }
  return out;
}

/** The language servers of one plugin. */
export function pluginLspServers(plugin: Plugin): PluginLspServer[] {
  const tables: unknown[] = [plugin.entry?.lspServers, readManifest(plugin.root).lspServers];
  const file = readJsonObject(join(plugin.root, ".lsp.json"));
  if (file) tables.push(isRecord(file.lspServers) ? file.lspServers : file);
  return tables.flatMap((table) => parseServers(table, plugin));
}

/** The language servers of every enabled plugin; read each time, so a new plugin counts at once. */
export function enabledLspServers(pluginsDir: string | undefined): PluginLspServer[] {
  return enabledPlugins(pluginsDir).flatMap(pluginLspServers);
}
