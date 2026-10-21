// What a plugin's folder holds, read the way Claude Code reads it: the manifest
// (`.claude-plugin/plugin.json`, which a marketplace entry may stand in for or add to) and the
// paths it points at. Skills, commands and MCP servers each read their own part with these.
import { readFileSync } from "node:fs";
import { isAbsolute, join, relative, resolve, sep } from "node:path";
import type { Plugin } from "./plugins.ts";
import { isRecord } from "./util.ts";

/** A JSON file's object; undefined when it is missing, broken or not an object. */
export function readJsonObject(path: string): Record<string, unknown> | undefined {
  try {
    const parsed: unknown = JSON.parse(readFileSync(path, "utf8"));
    return isRecord(parsed) ? parsed : undefined;
  } catch {
    return undefined;
  }
}

/** `.claude-plugin/plugin.json`, empty when it is missing or broken. */
export function readManifest(root: string): Record<string, unknown> {
  return readJsonObject(join(root, ".claude-plugin", "plugin.json")) ?? {};
}

/** Whether `path` is `root` or inside it (a manifest must not point out of its plugin). */
export function inside(root: string, path: string): boolean {
  const rel = relative(root, path);
  return rel === "" || (!rel.startsWith(`..${sep}`) && rel !== ".." && !isAbsolute(rel));
}

/**
 * The paths a manifest or the marketplace entry names under `key` (a string or a list of them,
 * relative to the plugin), in the plugin's folder. Both are read; a path out of the plugin is left
 * out.
 */
export function declaredPaths(plugin: Plugin, key: string): string[] {
  const out: string[] = [];
  for (const source of [readManifest(plugin.root), plugin.entry ?? {}]) {
    const value = source[key];
    let list: unknown[] = [];
    if (typeof value === "string") list = [value];
    else if (Array.isArray(value)) list = value;
    for (const item of list) {
      if (typeof item !== "string") continue;
      const path = resolve(plugin.root, item);
      if (inside(plugin.root, path) && !out.includes(path)) out.push(path);
    }
  }
  return out;
}
