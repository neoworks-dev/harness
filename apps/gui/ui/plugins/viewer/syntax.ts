// Syntax highlighting for the viewer, small enough to run in the window's JS engine (no DOM, no
// library): a line of code becomes runs of text, each with a kind the palette colours. Scanners
// go a line at a time and hand the state a construct that goes on (a block comment, a template
// string, a fenced block) to the next line, so a file is highlighted from its first line to the
// one asked for, once, and a line is only scanned when something asks for it.
import { codeScanners } from "./syntax-code.ts";
import { markupScanners } from "./syntax-markup.ts";
import type { Scanned, Scanner, Token } from "./syntax-runs.ts";

export type { Kind, Scanned, Scanner, Token } from "./syntax-runs.ts";

/** Lines longer than this are not scanned: they are minified or generated, and slow. */
export const MAX_LINE = 600;

const scanners: Readonly<Record<string, Scanner>> = { ...codeScanners, ...markupScanners };

/** The extensions of each language, and the files that have no extension to tell by. */
const EXTENSIONS: Readonly<Record<string, string>> = {
  ts: "ts",
  tsx: "ts",
  mts: "ts",
  cts: "ts",
  js: "js",
  jsx: "js",
  mjs: "js",
  cjs: "js",
  svelte: "html",
  vue: "html",
  html: "html",
  htm: "html",
  xml: "html",
  svg: "html",
  rs: "rust",
  go: "go",
  py: "python",
  pyi: "python",
  sh: "shell",
  bash: "shell",
  zsh: "shell",
  json: "json",
  jsonc: "json",
  json5: "json",
  css: "css",
  scss: "scss",
  less: "scss",
  md: "markdown",
  mdx: "markdown",
  markdown: "markdown",
  yml: "yaml",
  yaml: "yaml",
  toml: "toml",
  c: "c",
  h: "c",
  cc: "c",
  cpp: "c",
  hpp: "c",
  java: "java",
  kt: "java",
  cs: "java",
  swift: "java",
};
const FILENAMES: Readonly<Record<string, string>> = {
  ".bashrc": "shell",
  ".zshrc": "shell",
  ".profile": "shell",
  "cargo.lock": "toml",
  "go.mod": "go",
};

/** The language of a file by its name, or undefined when it is not one that is highlighted. */
export function languageOf(path: string): string | undefined {
  const name = (path.slice(path.lastIndexOf("/") + 1) || path).toLowerCase();
  const byName = FILENAMES[name];
  if (byName !== undefined) return byName;
  const dot = name.lastIndexOf(".");
  return dot < 0 ? undefined : EXTENSIONS[name.slice(dot + 1)];
}

/** One line, from `state`; a language that is not known comes back as it is. */
export function scan(language: string, line: string, state = ""): Scanned {
  const scanner = scanners[language];
  if (scanner === undefined || line.length > MAX_LINE) return { tokens: [{ text: line }], state };
  return scanner(line, state);
}

/** How far `Highlighter.warm` goes at once. */
const WARM_LINES = 400;

/**
 * The runs of the lines of one text, scanned on demand and kept. Asking for line `n` scans the
 * lines before it that were not (their states are all that is kept of them, which is cheap), so
 * the first ask near the end of a long file costs most, unless `warm` has been scanning ahead.
 */
export class Highlighter {
  private readonly states: string[] = [""];
  private readonly cache = new Map<number, Token[]>();

  constructor(
    readonly language: string,
    private readonly lines: readonly string[],
  ) {}

  /** The runs of line `index` (0-based). */
  tokens(index: number): Token[] {
    const known = this.cache.get(index);
    if (known !== undefined) return known;
    const line = this.lines[index];
    if (line === undefined) return [];
    while (this.states.length <= index) this.step();
    const scanned = scan(this.language, line, this.states[index]);
    if (this.states.length === index + 1) this.states.push(scanned.state);
    this.cache.set(index, scanned.tokens);
    return scanned.tokens;
  }

  /** Whether lines are left without their state. */
  get pending(): boolean {
    return this.states.length <= this.lines.length;
  }

  /** Scans ahead for `lines` lines, so a later jump into the file finds its state ready. */
  warm(lines = WARM_LINES): void {
    for (let i = 0; i < lines && this.pending; i += 1) this.step();
  }

  /** The state after the first line that has none. */
  private step(): void {
    const index = this.states.length - 1;
    this.states.push(scan(this.language, this.lines[index] ?? "", this.states[index]).state);
  }
}
