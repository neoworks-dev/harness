// The lsp service's tools. Positions are 1-based lines; a column, or a symbol on that line
// (models are bad at counting columns), picks the character.
import { readFileSync, writeFileSync } from "node:fs";
import { isAbsolute, relative } from "node:path";
import { fileURLToPath } from "node:url";
import type { ToolSpec } from "@neoworks-dev/neo-sdk";
import type { Diagnostic, Range, ServerPool } from "./servers.ts";

export interface Location {
  path: string;
  line: number;
  column: number;
  text: string;
}

const SEVERITY = ["", "error", "warning", "info", "hint"];

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function str(args: Record<string, unknown>, key: string): string | undefined {
  const value = args[key];
  return typeof value === "string" && value !== "" ? value : undefined;
}

function num(args: Record<string, unknown>, key: string): number | undefined {
  const value = args[key];
  return typeof value === "number" && Number.isInteger(value) ? value : undefined;
}

function requirePath(args: Record<string, unknown>): string {
  const path = str(args, "path");
  if (path === undefined || !isAbsolute(path)) throw new Error("path must be an absolute path.");
  return path;
}

function escapeRegExp(text: string): string {
  return text.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

function lineOf(text: string, line: number): string {
  return text.split("\n")[line] ?? "";
}

/** 0-based LSP position from {line, column?, symbol?}. */
export function position(
  text: string,
  args: Record<string, unknown>,
): { line: number; character: number } {
  const line = num(args, "line");
  if (line === undefined || line < 1) throw new Error("line must be a 1-based line number.");
  const content = text.split("\n")[line - 1];
  if (content === undefined) throw new Error(`The file has no line ${line}.`);
  const symbol = str(args, "symbol");
  if (symbol !== undefined) {
    const at = content.search(new RegExp(`(?<![\\w$])${escapeRegExp(symbol)}(?![\\w$])`));
    if (at === -1) throw new Error(`"${symbol}" isn't on line ${line}: ${content.trim()}`);
    return { line: line - 1, character: at };
  }
  const column = num(args, "column");
  if (column !== undefined) return { line: line - 1, character: Math.max(0, column - 1) };
  return { line: line - 1, character: content.length - content.trimStart().length };
}

function location(value: unknown, read: (path: string) => string): Location | undefined {
  if (!isRecord(value)) return undefined;
  // Location or LocationLink
  const uri = typeof value.uri === "string" ? value.uri : value.targetUri;
  const range = value.range ?? value.targetSelectionRange;
  if (typeof uri !== "string" || !uri.startsWith("file:") || !isRecord(range)) return undefined;
  const { start } = range as unknown as Range;
  const path = fileURLToPath(uri);
  return {
    path,
    line: start.line + 1,
    column: start.character + 1,
    text: lineOf(read(path), start.line).trim(),
  };
}

function locations(result: unknown, read: (path: string) => string): Location[] {
  const list = Array.isArray(result) ? result : [result];
  const out: Location[] = [];
  for (const item of list) {
    const loc = location(item, read);
    if (loc) out.push(loc);
  }
  return out;
}

function hoverText(result: unknown): string {
  if (!isRecord(result)) return "";
  const parts = Array.isArray(result.contents) ? result.contents : [result.contents];
  const out: string[] = [];
  for (const part of parts) {
    if (typeof part === "string") out.push(part);
    else if (isRecord(part) && typeof part.value === "string") {
      out.push(
        typeof part.language === "string"
          ? `\`\`\`${part.language}\n${part.value}\n\`\`\``
          : part.value,
      );
    }
  }
  return out.join("\n\n").trim();
}

interface TextEdit {
  range: Range;
  newText: string;
}

/** The text edits of a WorkspaceEdit by file path (changes or documentChanges). */
export function editsOf(edit: unknown): Map<string, TextEdit[]> {
  const out = new Map<string, TextEdit[]>();
  if (!isRecord(edit)) return out;
  const add = (uri: unknown, edits: unknown): void => {
    if (typeof uri !== "string" || !Array.isArray(edits)) return;
    const path = fileURLToPath(uri);
    out.set(path, [...(out.get(path) ?? []), ...(edits as TextEdit[])]);
  };
  if (isRecord(edit.changes))
    for (const [uri, edits] of Object.entries(edit.changes)) add(uri, edits);
  if (Array.isArray(edit.documentChanges)) {
    for (const change of edit.documentChanges) {
      if (isRecord(change) && isRecord(change.textDocument))
        add(change.textDocument.uri, change.edits);
    }
  }
  return out;
}

function offset(text: string, pos: { line: number; character: number }): number {
  let at = 0;
  for (let line = 0; line < pos.line; line++) {
    const next = text.indexOf("\n", at);
    if (next === -1) return text.length;
    at = next + 1;
  }
  return Math.min(at + pos.character, text.length);
}

/** Applies LSP text edits (UTF-16 positions, which are JS string indexes). */
export function applyEdits(text: string, edits: TextEdit[]): string {
  const sorted = edits
    .map((e) => ({
      from: offset(text, e.range.start),
      to: offset(text, e.range.end),
      text: e.newText,
    }))
    .sort((a, b) => b.from - a.from);
  let out = text;
  for (const e of sorted) out = out.slice(0, e.from) + e.text + out.slice(e.to);
  return out;
}

const positionArgs = {
  path: { type: "string", description: "Absolute path of the file." },
  line: { type: "integer", description: "1-based line." },
  symbol: { type: "string", description: "The identifier on that line (preferred over column)." },
  column: { type: "integer", description: "1-based column, if no symbol is given." },
};

export function createTools(pool: ServerPool, cwdRelative?: string): ToolSpec[] {
  const read = (path: string): string => {
    try {
      return readFileSync(path, "utf8");
    } catch {
      return "";
    }
  };
  const show = (path: string): string =>
    cwdRelative !== undefined && path.startsWith(cwdRelative) ? relative(cwdRelative, path) : path;

  async function at(
    method: string,
    args: Record<string, unknown>,
    extra: Record<string, unknown> = {},
  ): Promise<unknown> {
    const path = requirePath(args);
    const { server, languageId } = pool.get(path);
    const { uri } = await server.sync(path, languageId);
    const pos = position(server.text(uri) ?? read(path), args);
    return server.request(method, { textDocument: { uri }, position: pos, ...extra });
  }

  return [
    {
      name: "diagnostics",
      description:
        "Errors and warnings for a file from its language server (Go, TypeScript/JavaScript, Rust). Reads the file from disk, so call it after editing.",
      inputSchema: {
        type: "object",
        properties: { path: positionArgs.path },
        required: ["path"],
      },
      handler: async (args) => {
        const path = requirePath(args);
        const { server, languageId } = pool.get(path);
        const since = Date.now();
        const { uri, changed } = await server.sync(path, languageId);
        const list: Diagnostic[] = await server.diagnostics(uri, since, changed ? 8000 : 500);
        const items = list.map((d) => ({
          line: d.range.start.line + 1,
          column: d.range.start.character + 1,
          severity: SEVERITY[d.severity ?? 1] ?? "error",
          message: d.message,
          source: d.source ?? "",
        }));
        const errors = items.filter((d) => d.severity === "error").length;
        const warnings = items.filter((d) => d.severity === "warning").length;
        const lines = items.map(
          (d) => `${show(path)}:${d.line}:${d.column}: ${d.severity}: ${d.message}`,
        );
        return {
          content: [{ type: "text", text: lines.join("\n") || `${show(path)}: no problems.` }],
          structuredContent: { path, errors, warnings, diagnostics: items },
        };
      },
    },
    {
      name: "definition",
      description: "Where the symbol at a position is defined.",
      inputSchema: { type: "object", properties: positionArgs, required: ["path", "line"] },
      handler: async (args) => {
        const found = locations(await at("textDocument/definition", args), read);
        const text = found
          .map((l) => `${show(l.path)}:${l.line}:${l.column}: ${l.text}`)
          .join("\n");
        return {
          content: [{ type: "text", text: text || "No definition found." }],
          structuredContent: { locations: found },
        };
      },
    },
    {
      name: "references",
      description: "Every reference to the symbol at a position, including its declaration.",
      inputSchema: { type: "object", properties: positionArgs, required: ["path", "line"] },
      handler: async (args) => {
        const found = locations(
          await at("textDocument/references", args, { context: { includeDeclaration: true } }),
          read,
        );
        const text = found
          .map((l) => `${show(l.path)}:${l.line}:${l.column}: ${l.text}`)
          .join("\n");
        return {
          content: [{ type: "text", text: text || "No references found." }],
          structuredContent: { count: found.length, locations: found },
        };
      },
    },
    {
      name: "hover",
      description: "Type and documentation of the symbol at a position.",
      inputSchema: { type: "object", properties: positionArgs, required: ["path", "line"] },
      handler: async (args) => {
        const text = hoverText(await at("textDocument/hover", args));
        return {
          content: [{ type: "text", text: text || "Nothing to show." }],
          structuredContent: { text },
        };
      },
    },
    {
      name: "rename",
      description:
        "Renames the symbol at a position across the project and writes the changed files.",
      inputSchema: {
        type: "object",
        properties: { ...positionArgs, newName: { type: "string", description: "The new name." } },
        required: ["path", "line", "newName"],
      },
      handler: async (args) => {
        const newName = str(args, "newName");
        if (newName === undefined) throw new Error("newName is required.");
        const edits = editsOf(await at("textDocument/rename", args, { newName }));
        if (edits.size === 0) throw new Error("The language server can't rename this.");
        const files: { path: string; edits: number }[] = [];
        for (const [path, list] of edits) {
          writeFileSync(path, applyEdits(read(path), list));
          files.push({ path, edits: list.length });
        }
        const text = files.map((f) => `${show(f.path)}: ${f.edits} edits`).join("\n");
        return {
          content: [{ type: "text", text: `Renamed to ${newName}.\n${text}` }],
          structuredContent: { newName, files },
        };
      },
    },
  ];
}

/** Result views for the tools above (protocol/README.md#templates). */
export const templates = {
  diagnostics: {
    type: "card",
    title: "{{data.path}}",
    children: [
      {
        type: "kv",
        items: { Errors: "{{data.errors}}", Warnings: "{{data.warnings}}" },
      },
      {
        type: "table",
        if: "data.diagnostics",
        columns: ["line", "severity", "message"],
        rows: "{{data.diagnostics}}",
      },
    ],
  },
  definition: {
    type: "stack",
    children: [
      {
        type: "stack",
        each: "data.locations",
        as: "loc",
        children: [
          { type: "file", path: "{{loc.path}}", name: "{{loc.path}}:{{loc.line}}" },
          { type: "code", code: "{{loc.text}}" },
        ],
      },
    ],
  },
  references: {
    type: "card",
    title: "{{data.count}} references",
    children: [{ type: "table", columns: ["path", "line", "text"], rows: "{{data.locations}}" }],
  },
  hover: { type: "text", markdown: "{{data.text}}" },
  rename: {
    type: "card",
    title: "Renamed to {{data.newName}}",
    children: [{ type: "table", columns: ["path", "edits"], rows: "{{data.files}}" }],
  },
};
