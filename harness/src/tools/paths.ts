import { homedir } from "node:os";
import { resolve } from "node:path";

/** Resolves a model-supplied path against the session cwd, expanding a leading `~`. */
export function resolvePath(cwd: string, path: string): string {
  if (path === "~") return homedir();
  if (path.startsWith("~/")) return resolve(homedir(), path.slice(2));
  return resolve(cwd, path);
}
