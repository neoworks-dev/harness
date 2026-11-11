// What a project's tool files are, as the user approves them: each top-level tool file by the
// content it has now. A tool file may import a helper, so the files under the folder that are
// not tool files themselves (a lib/ folder, a JSON file) are part of every tool file's content;
// a change to any of them is a change to all. Packages in node_modules are not covered.
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
function filesUnder(root: string, dir: string, skip: (path: string) => boolean): string[] {
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
      if (entry.name !== "node_modules") out.push(...filesUnder(root, path, skip));
    } else if (entry.isFile() && !skip(path)) {
      out.push(relative(root, path));
    }
  }
  return out.sort();
}

/** The files of the folder that are not its own tool files: what the tool files may import. */
function supportFiles(folder: string): string[] {
  const tools = new Set(toolFileNames(folder));
  return filesUnder(folder, folder, (path) => tools.has(relative(folder, path)));
}

/** The hash of each tool file: its content and that of the support files. */
export function fingerprint(folder: string): Record<string, string> {
  const support = createHash("sha256");
  for (const path of supportFiles(folder)) {
    support
      .update(path)
      .update("\0")
      .update(readFileSync(join(folder, path)))
      .update("\0");
  }
  const shared = support.digest("hex");
  const prints: Record<string, string> = {};
  for (const name of toolFileNames(folder)) {
    prints[name] = createHash("sha256")
      .update(shared)
      .update("\0")
      .update(name)
      .update("\0")
      .update(readFileSync(join(folder, name)))
      .digest("hex");
  }
  return prints;
}

/** A cheap stand-in for the folder's content: the sizes and times of its files. */
export function signature(folder: string): string {
  const files = [...toolFileNames(folder), ...supportFiles(folder)];
  return files
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
