import { homedir } from "node:os";
import { resolve } from "node:path";

/** Resolves asset://… handles (from tool results) to the stored file's path. */
export type AssetResolver = (handle: string) => string | undefined;

const ASSET_HANDLE = /asset:\/\/[a-z]+-[0-9a-f]{12}/g;

/**
 * Resolves a model-supplied path against the session cwd, expanding a leading `~`. An asset
 * handle resolves to its stored (read-only) file.
 */
export function resolvePath(cwd: string, path: string, assets?: AssetResolver): string {
  if (path.startsWith("asset://")) {
    const resolved = assets?.(path);
    if (resolved === undefined) throw new Error(`Unknown asset ${path}.`);
    return resolved;
  }
  if (path === "~") return homedir();
  if (path.startsWith("~/")) return resolve(homedir(), path.slice(2));
  return resolve(cwd, path);
}

/** Like resolvePath for tools that change files: assets are never written in place. */
export function resolveWritablePath(cwd: string, path: string): string {
  if (path.startsWith("asset://")) {
    throw new Error(`${path} is read-only; copy it into the project first (bash: cp).`);
  }
  return resolvePath(cwd, path);
}

/** Replaces the asset handles in a shell command with their file paths; unknown ones stay. */
export function resolveAssetsInCommand(command: string, assets?: AssetResolver): string {
  if (!assets) return command;
  return command.replace(ASSET_HANDLE, (handle) => {
    const path = assets(handle);
    if (path === undefined) return handle;
    return /^[\w./-]+$/.test(path) ? path : `'${path.replaceAll("'", `'\\''`)}'`;
  });
}
