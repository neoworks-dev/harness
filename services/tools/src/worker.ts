// A worker thread that holds one tool file: it imports the file, says what tools it gives, and
// runs their handlers. A tool that hangs or crashes takes only this thread with it.
import { checkTool, isRecord, metaOf, type FromWorker, type ToWorker } from "./spec.ts";
import type { ToolSpec } from "@neoworks-dev/neo-sdk";

declare const self: {
  postMessage(message: FromWorker): void;
  onmessage: ((event: { data: ToWorker }) => void) | null;
};

const specs = new Map<string, ToolSpec>();

async function load(path: string): Promise<void> {
  let mod: unknown;
  try {
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
  if (data.type === "load") void load(data.path);
  else void call(data.id, data.name, data.args);
};
