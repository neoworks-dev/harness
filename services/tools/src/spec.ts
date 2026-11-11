// What a tool file may give, and what is wrong when it gives something else. Shared by the worker
// that loads the file and the tests.
import type { ToolSpec } from "@neoworks-dev/neo-sdk";

const NAME = /^[a-z][a-z0-9_]*$/i;

export function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** A tool as the gateway lists it: everything but the function. */
export interface ToolMeta {
  name: string;
  description: string;
  inputSchema: Record<string, unknown>;
  outputSchema?: Record<string, unknown>;
}

/** The tool a value describes, or what is wrong with it. */
export function checkTool(value: unknown): ToolSpec | string {
  if (!isRecord(value)) return "a tool is an object: { name, description, inputSchema, handler }";
  const { name, description, inputSchema, handler } = value;
  if (typeof name !== "string" || !NAME.test(name)) {
    return "name must be letters, digits and _, starting with a letter";
  }
  if (typeof description !== "string" || description.trim() === "") {
    return `${name}: description says when to use the tool`;
  }
  if (!isRecord(inputSchema)) return `${name}: inputSchema is a JSON Schema object`;
  if (typeof handler !== "function") return `${name}: handler is a function`;
  const input = objectSchema(inputSchema);
  if (input === undefined) return `${name}: inputSchema describes an object: { type: "object" }`;
  const spec: ToolSpec = {
    name,
    description,
    inputSchema: input,
    handler: handler as ToolSpec["handler"],
  };
  if (value.outputSchema !== undefined) {
    const output = isRecord(value.outputSchema) ? objectSchema(value.outputSchema) : undefined;
    if (output === undefined)
      return `${name}: outputSchema describes an object: { type: "object" }`;
    spec.outputSchema = output;
  }
  return spec;
}

/**
 * The schema as an object schema: one with no `type` is given `type: "object"`, one with another
 * type is refused. MCP rejects a whole tool list for one schema that is not an object's, so a
 * tool that gets this wrong would take every tool of the service with it.
 */
function objectSchema(schema: Record<string, unknown>): Record<string, unknown> | undefined {
  if (schema.type === undefined) return { type: "object", ...schema };
  return schema.type === "object" ? schema : undefined;
}

export function metaOf(spec: ToolSpec): ToolMeta {
  const meta: ToolMeta = {
    name: spec.name,
    description: spec.description,
    inputSchema: spec.inputSchema,
  };
  if (spec.outputSchema) meta.outputSchema = spec.outputSchema;
  return meta;
}

/** What goes between a worker and the service. */
export type ToWorker =
  /** `files` is the hash of each file of `root` (approval.ts): the only code it may load. */
  | { type: "load"; path: string; root: string; files: Record<string, string> }
  | { type: "call"; id: number; name: string; args: Record<string, unknown> }
  /** Answered at once by a worker whose thread is free: one that does not is stuck. */
  | { type: "ping" };

export type FromWorker =
  | { type: "pong" }
  | { type: "ready"; tools: ToolMeta[]; problems: string[] }
  | { type: "failed-load"; message: string }
  | { type: "result"; id: number; result: unknown }
  | { type: "failed"; id: number; message: string };
