// What the tool files of a folder are, as the user approves them. A tool file may import the
// other files of its folder (a lib/ folder, a JSON file), so those are part of every tool file's
// content: a change to any of them is a change to all. Besides those it may import only the
// packages of the folder's node_modules, which package.json and the lockfile (files of the folder)
// stand for. The worker refuses anything else, and a file whose bytes are not the ones hashed
// here, so what runs is what was approved (see worker.ts).
// The daemon decides with these hashes and the service holds to them, so both use this file.
import { createHash } from "node:crypto";
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative } from "node:path";

export const TOOL_FILE = /\.(?:ts|js|mjs)$/;

/** The names of the tool files of a folder: its top-level code files, sorted. */
export function toolFileNames(folder: string): string[] {
  try {
    return readdirSync(folder, { withFileTypes: true })
      .filter((entry) => entry.isFile() && TOOL_FILE.test(entry.name))
      .map((entry) => entry.name)
      .sort();
  } catch {
    return [];
  }
}

/** Every file under `dir` but node_modules, as paths relative to `root`, sorted. */
function filesUnder(root: string, dir: string): string[] {
  const out: string[] = [];
  let entries;
  try {
    entries = readdirSync(dir, { withFileTypes: true });
  } catch {
    return out;
  }
  for (const entry of entries) {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) {
      if (entry.name !== "node_modules") out.push(...filesUnder(root, path));
    } else if (entry.isFile()) {
      out.push(relative(root, path));
    }
  }
  return out.sort();
}

export function sha256(bytes: Uint8Array | string): string {
  return createHash("sha256").update(bytes).digest("hex");
}

/** The hash of every file of the folder, by its path in it: what a worker may load. */
export function manifest(folder: string): Record<string, string> {
  const out: Record<string, string> = {};
  for (const path of filesUnder(folder, folder)) {
    try {
      out[path] = sha256(readFileSync(join(folder, path)));
    } catch {
      // Gone since it was listed: not there.
    }
  }
  return out;
}

/** The hash of each tool file: its content and that of the other files of the folder. */
export function fingerprint(
  folder: string,
  files: Record<string, string> = manifest(folder),
): Record<string, string> {
  const tools = toolFileNames(folder).filter((name) => files[name] !== undefined);
  const own = new Set(tools);
  const support = createHash("sha256");
  for (const [path, hash] of Object.entries(files).sort(([a], [b]) => (a < b ? -1 : 1))) {
    if (!own.has(path)) support.update(path).update("\0").update(hash).update("\0");
  }
  const shared = support.digest("hex");
  const prints: Record<string, string> = {};
  for (const name of tools) {
    prints[name] = sha256(`${shared}\0${name}\0${files[name]}`);
  }
  return prints;
}

/** A cheap stand-in for the folder's content: the sizes and times of its files. */
export function signature(folder: string): string {
  return filesUnder(folder, folder)
    .map((path) => {
      try {
        const stat = statSync(join(folder, path));
        return `${path}:${stat.size}:${stat.mtimeMs}`;
      } catch {
        return path;
      }
    })
    .join("\n");
}
