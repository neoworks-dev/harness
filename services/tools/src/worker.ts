// A worker thread that holds one tool file: it imports the file, says what tools it gives, and
// runs their handlers. A tool that hangs or crashes takes only this thread with it.
// The file may load only the files of its folder, each with the bytes the service hashed (the
// ones the user approved): every module goes through the hook below, which reads it once, checks
// it and hands Bun exactly those bytes. Built-ins (node:*, bun) are not files and are not asked;
// the packages of the folder's node_modules are covered by its package.json and lockfile.
import { relative, sep } from "node:path";
import { sha256 } from "./approval.ts";
import { checkTool, isRecord, metaOf, type FromWorker, type ToWorker } from "./spec.ts";
import type { ToolSpec } from "@neoworks-dev/neo-sdk";

declare const self: {
  postMessage(message: FromWorker): void;
  onmessage: ((event: { data: ToWorker }) => void) | null;
};

const LOADERS: Record<string, "ts" | "tsx" | "js" | "jsx" | "json" | "toml"> = {
  ts: "ts",
  mts: "ts",
  tsx: "tsx",
  js: "js",
  mjs: "js",
  cjs: "js",
  jsx: "jsx",
  json: "json",
  toml: "toml",
};

/**
 * Lets the file load its folder's files as they were hashed, and the packages of its own
 * node_modules, and nothing else. The packages are not hashed one by one: package.json and the
 * lockfile (which pins each package's integrity) are files of the folder, so `bun add` asks again.
 */
function guard(root: string, files: Record<string, string>): void {
  const packages = `${root}${sep}node_modules${sep}`.replace(/[.*+?^${}()|[\]\\/]/g, "\\$&");
  Bun.plugin({
    name: "tool-folder",
    setup(build) {
      build.onLoad({ filter: new RegExp(`^(?!${packages})`) }, async ({ path }) => {
        const rel = relative(root, path);
        if (rel.startsWith("..") || rel.startsWith(sep) || files[rel] === undefined) {
          throw new Error(
            `${path} is outside the tools folder: a tool may import only built-ins, the files of its folder and the packages installed in it`,
          );
        }
        const bytes = new Uint8Array(await Bun.file(path).arrayBuffer());
        if (sha256(bytes) !== files[rel]) {
          throw new Error(`${rel} changed since it was approved`);
        }
        const loader = LOADERS[path.slice(path.lastIndexOf(".") + 1)];
        if (loader === undefined) throw new Error(`${rel}: not code or data a tool can import`);
        return { contents: new TextDecoder().decode(bytes), loader };
      });
    },
  });
}

const specs = new Map<string, ToolSpec>();

async function load(path: string, root: string, files: Record<string, string>): Promise<void> {
  let mod: unknown;
  try {
    guard(root, files);
    mod = await import(path);
  } catch (err) {
    self.postMessage({
      type: "failed-load",
      message: `can't load: ${err instanceof Error ? err.message : String(err)}`,
    });
    return;
  }
  const exported = isRecord(mod) ? mod.default : undefined;
  const list: unknown[] = Array.isArray(exported) ? exported : [exported];
  const tools = [];
  const problems: string[] = [];
  for (const value of list) {
    const spec = checkTool(value);
    if (typeof spec === "string") {
      problems.push(exported === undefined ? "has no default export" : spec);
    } else if (specs.has(spec.name)) {
      problems.push(`${spec.name}: given twice in the file`);
    } else {
      specs.set(spec.name, spec);
      tools.push(metaOf(spec));
    }
  }
  self.postMessage({ type: "ready", tools, problems });
}

async function call(id: number, name: string, args: Record<string, unknown>): Promise<void> {
  const spec = specs.get(name);
  try {
    if (!spec) throw new Error(`no tool ${name}`);
    self.postMessage({ type: "result", id, result: await spec.handler(args) });
  } catch (err) {
    self.postMessage({
      type: "failed",
      id,
      message: err instanceof Error ? err.message : String(err),
    });
  }
}

self.onmessage = ({ data }) => {
  if (data.type === "ping") self.postMessage({ type: "pong" });
  else if (data.type === "load") void load(data.path, data.root, data.files);
  else void call(data.id, data.name, data.args);
};
