// Templates: the authoring format apps register for their results. Same components as the view
// format, except that text has `markdown` (not blocks) and diff has `oldText`/`newText` (not
// lines). Any string may contain {{path}}; a string that is exactly "{{path}}" takes the raw
// value (an array for table rows, a number for progress). Any node may carry
// `"if": "path"` (dropped when falsy) and `"each": "path", "as": "name"` (repeated per element,
// default name `item`). Paths are dotted: data.size.w, item.name, data.rows.0.
import type { Component } from "@suite/protocol/types";
import { validateDef } from "@suite/protocol/validate";
import { diffLines } from "./diff.ts";
import { parseMarkdown } from "./markdown.ts";

export class TemplateError extends Error {}

type Scope = Record<string, unknown>;
type Node = Record<string, unknown>;

const WHOLE = /^\{\{\s*([\w.-]+)\s*\}\}$/;
const PART = /\{\{\s*([\w.-]+)\s*\}\}/g;
const CONTROL = new Set(["if", "each", "as"]);

function isNode(value: unknown): value is Node {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function lookup(scope: Scope, path: string): unknown {
  let value: unknown = scope;
  for (const key of path.split(".")) {
    if (Array.isArray(value)) value = value[Number(key)];
    else if (isNode(value)) value = value[key];
    else return undefined;
  }
  return value;
}

function stringify(value: unknown): string {
  switch (typeof value) {
    case "string":
      return value;
    case "number":
    case "boolean":
    case "bigint":
      return String(value);
    case "object":
      if (value === null) return "";
      return JSON.stringify(value);
    default:
      return "";
  }
}

function asArray(value: unknown): unknown[] {
  if (Array.isArray(value)) return value;
  return [];
}

function substitute(value: unknown, scope: Scope): unknown {
  if (typeof value === "string") {
    const whole = WHOLE.exec(value);
    if (whole) return lookup(scope, whole[1] ?? "");
    return value.replace(PART, (_, path: string) => stringify(lookup(scope, path)));
  }
  if (Array.isArray(value)) return value.map((v) => substitute(v, scope));
  if (isNode(value)) {
    return Object.fromEntries(Object.entries(value).map(([k, v]) => [k, substitute(v, scope)]));
  }
  return value;
}

function truthy(value: unknown): boolean {
  if (Array.isArray(value)) return value.length > 0;
  return Boolean(value);
}

/** Turns template-only fields into view fields and coerces substituted values to their types. */
function finish(node: Node): Node {
  switch (node.type) {
    case "text": {
      const { markdown, ...rest } = node;
      if (markdown === undefined) return node;
      return { ...rest, blocks: parseMarkdown(stringify(markdown)) };
    }
    case "diff": {
      const { oldText, newText, ...rest } = node;
      if (newText === undefined) return node;
      const old = oldText === undefined || oldText === null ? undefined : stringify(oldText);
      return { ...rest, lines: diffLines(old, stringify(newText)) };
    }
    case "table": {
      const columns = asArray(node.columns).map(stringify);
      const rows = asArray(node.rows);
      return {
        ...node,
        columns,
        rows: rows.map((row) => {
          // Rows may be arrays, or objects read in column order.
          if (Array.isArray(row)) return row.map(stringify);
          if (isNode(row)) return columns.map((c) => stringify(row[c]));
          return [stringify(row)];
        }),
      };
    }
    case "kv": {
      const items = node.items;
      let list = asArray(items);
      if (isNode(items)) list = Object.entries(items).map(([key, value]) => ({ key, value }));
      return {
        ...node,
        items: list.map((i: unknown) =>
          isNode(i)
            ? { key: stringify(i.key), value: stringify(i.value) }
            : { key: "", value: stringify(i) },
        ),
      };
    }
    case "progress":
      return { ...node, value: Math.min(1, Math.max(0, Number(node.value) || 0)) };
    case "image": {
      const out: Node = { ...node };
      for (const key of ["width", "height"]) {
        if (out[key] === undefined || out[key] === "") delete out[key];
        else out[key] = Math.round(Number(out[key]));
      }
      return out;
    }
    default:
      return node;
  }
}

function resolveNode(node: unknown, scope: Scope): Node[] {
  if (!isNode(node) || typeof node.type !== "string") {
    throw new TemplateError(`not a component: ${JSON.stringify(node)}`);
  }
  if (typeof node.each === "string") {
    const list = lookup(scope, node.each);
    const name = typeof node.as === "string" ? node.as : "item";
    const { each: _each, as: _as, ...single } = node;
    if (!Array.isArray(list)) return [];
    return list.flatMap((element) => resolveNode(single, { ...scope, [name]: element }));
  }
  if (typeof node.if === "string" && !truthy(lookup(scope, node.if))) return [];

  const out: Node = {};
  for (const [key, value] of Object.entries(node)) {
    if (CONTROL.has(key)) continue;
    if (key === "children") {
      out.children = asArray(value).flatMap((child) => resolveNode(child, scope));
    } else {
      out[key] = substitute(value, scope);
    }
  }
  // Optional fields that came out empty are dropped rather than sent as "".
  for (const [key, value] of Object.entries(out)) if (value === undefined) delete out[key];
  return [finish(out)];
}

/** Fills a template with a tool result's data. Throws TemplateError if the result isn't valid. */
export function resolveTemplate(template: unknown, data: unknown): Component {
  const nodes = resolveNode(template, { data });
  const root = nodes.length === 1 ? nodes[0] : { type: "stack", children: nodes };
  const problem = validateDef("view.schema.json#/$defs/Component", root);
  if (problem !== undefined) throw new TemplateError(`resolved template is invalid: ${problem}`);
  return root as unknown as Component;
}
