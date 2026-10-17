// The scanners of markup and styles: HTML and Svelte (with the script, the style and the braces
// of a template read as the languages they are), CSS and Markdown.
import { codeScanners } from "./syntax-code.ts";
import { Runs, type Kind, type Scanned, type Scanner, type Token } from "./syntax-runs.ts";

const ts = codeScanners.ts as Scanner;

// ---- CSS ----

const CSS_PARTS = [
  "(\\s+)",
  "(/\\*)",
  "(\\/\\/.*)",
  `("(?:[^"\\\\]|\\\\.)*"?|'(?:[^'\\\\]|\\\\.)*'?)`,
  "(@[\\w-]+)",
  "(#[0-9a-fA-F]{3,8}\\b)",
  "(-?(?:\\d*\\.)?\\d+(?:%|[a-zA-Z]+)?)",
  "(!\\s*important)",
  "(--[\\w-]+)",
  "([.#][A-Za-z_][\\w-]*)",
  "(::?[A-Za-z-]+)",
  "(-?[A-Za-z_][\\w-]*)",
  "([\\s\\S])",
];

function cssScanner(lineComments: boolean): Scanner {
  const master = new RegExp(CSS_PARTS.join("|"), "y");
  return (line, start): Scanned => {
    const out = new Runs();
    let at = 0;
    if (start === "block") {
      const end = line.indexOf("*/");
      out.add(line.slice(0, end < 0 ? line.length : end + 2), "comment");
      if (end < 0) return { tokens: out.tokens, state: "block" };
      at = end + 2;
    }
    // A line of declarations has a property at its start; one of selectors a tag, class or id.
    const declaration =
      /^\s*[-\w]+\s*:(?!:)/.test(line) && !/[{,]\s*$/.test(line.replace(/\/\*.*\*\/\s*$/, ""));
    let selector = !declaration && /\{|,\s*$/.test(line);
    /** A property can come next: at the start of a line of declarations, and after a `{` or `;`. */
    let property = declaration;
    while (at < line.length) {
      master.lastIndex = at;
      const match = master.exec(line);
      if (match === null) break;
      const text = match[0];
      let group = 1;
      while (match[group] === undefined) group += 1;
      at += text.length;
      let kind: Kind | undefined;
      switch (group) {
        case 1:
          out.add(text);
          continue;
        case 2: {
          const end = line.indexOf("*/", at);
          out.add(text + line.slice(at, end < 0 ? line.length : end + 2), "comment");
          if (end < 0) return { tokens: out.tokens, state: "block" };
          at = end + 2;
          continue;
        }
        case 3:
          if (lineComments) {
            out.add(text, "comment");
            continue;
          }
          out.add(text);
          continue;
        case 4:
          kind = "string";
          break;
        case 5:
          kind = "keyword";
          break;
        case 6:
          kind = "number";
          break;
        case 7:
          kind = "number";
          break;
        case 8:
          kind = "keyword";
          break;
        case 9:
          kind = "property";
          break;
        case 10:
          kind = selector ? "type" : undefined;
          break;
        case 11:
          kind = selector ? "keyword" : undefined;
          break;
        case 12:
          if (property && /^\s*:(?!:)/.test(line.slice(at))) kind = "property";
          else if (line[at] === "(") kind = "function";
          else if (selector) kind = "tag";
          break;
        default:
          if (text === "{") selector = false;
      }
      property = text === "{" || text === ";";
      out.add(text, kind);
    }
    return { tokens: out.tokens, state: "" };
  };
}

const css = cssScanner(false);
const scss = cssScanner(true);

// ---- HTML and Svelte ----

/** Where the `}` that closes the brace at `from` is, or -1 when the line ends first. */
function closeBrace(line: string, from: number): number {
  let depth = 0;
  let quote = "";
  for (let i = from; i < line.length; i += 1) {
    const char = line[i];
    if (quote !== "") {
      if (char === "\\") i += 1;
      else if (char === quote) quote = "";
    } else if (char === '"' || char === "'" || char === "`") {
      quote = char;
    } else if (char === "{") {
      depth += 1;
    } else if (char === "}") {
      depth -= 1;
      if (depth === 0) return i;
    }
  }
  return -1;
}

/** A `{…}` of a template at `at`, as keyword and script; returns where it ends. */
function brace(line: string, at: number, out: Runs): number {
  const close = closeBrace(line, at);
  const stop = close < 0 ? line.length : close;
  let inner = at + 1;
  const block = /^[#:/@]\w+/.exec(line.slice(inner, stop));
  out.add("{");
  if (block !== null) {
    out.add(block[0], "keyword");
    inner += block[0].length;
  }
  out.addAll(ts(line.slice(inner, stop), "").tokens);
  if (close >= 0) out.add("}");
  return close < 0 ? line.length : close + 1;
}

const TEXT = /<!--|<\/?[A-Za-z][\w:.-]*|\{|&\w+;|[^<{&]+|[\s\S]/y;
const ATTRIBUTE = /\s+|\/?>|[A-Za-z_:@.$-][\w:.$@-]*|"[^"]*"?|'[^']*'?|\{|[\s\S]/y;

/**
 * State: "" is text; "comment" inside `<!-- -->`; "tag" and "tag:script" inside a tag that goes on
 * over lines; "script:<state>" and "style:<state>" inside those elements, with the state of their
 * own language.
 */
const html: Scanner = (line, start): Scanned => {
  const out = new Runs();
  let state = start;
  let at = 0;
  while (at < line.length) {
    if (state === "comment") {
      const end = line.indexOf("-->", at);
      out.add(line.slice(at, end < 0 ? line.length : end + 3), "comment");
      if (end < 0) return { tokens: out.tokens, state };
      at = end + 3;
      state = "";
      continue;
    }
    if (state.startsWith("script:") || state.startsWith("style:")) {
      const name = state.startsWith("script:") ? "script" : "style";
      const inner = state.slice(name.length + 1);
      const end = line.toLowerCase().indexOf(`</${name}`, at);
      const code = line.slice(at, end < 0 ? line.length : end);
      const scanned = (name === "script" ? ts : css)(code, inner);
      out.addAll(scanned.tokens);
      if (end < 0) return { tokens: out.tokens, state: `${name}:${scanned.state}` };
      at = end;
      state = "";
      continue;
    }
    if (state.startsWith("tag")) {
      const element = state.slice(4);
      ATTRIBUTE.lastIndex = at;
      const match = ATTRIBUTE.exec(line);
      const text = match?.[0] ?? line.slice(at);
      at += text.length;
      if (text === "{") {
        at = brace(line, at - 1, out);
      } else if (text === ">" || text === "/>") {
        out.add(text);
        state = "";
        if (text === ">" && (element === "script" || element === "style")) state = `${element}:`;
      } else if (/^[A-Za-z_:@.$-]/.test(text)) {
        out.add(text, "attr");
      } else if (text.startsWith('"') || text.startsWith("'")) {
        out.add(text, "string");
      } else {
        out.add(text);
      }
      continue;
    }
    TEXT.lastIndex = at;
    const match = TEXT.exec(line);
    const text = match?.[0] ?? line.slice(at);
    at += text.length;
    if (text === "<!--") {
      state = "comment";
      out.add(text, "comment");
    } else if (text.startsWith("<")) {
      out.add(text, "tag");
      const element = text.replace(/^<\/?/, "").toLowerCase();
      state = text.startsWith("</") ? "tag:" : `tag:${element}`;
    } else if (text === "{") {
      at = brace(line, at - 1, out);
    } else if (text.startsWith("&")) {
      out.add(text, "constant");
    } else {
      out.add(text);
    }
  }
  return { tokens: out.tokens, state };
};

// ---- Markdown ----

const INLINE = /`[^`]*`?|\[[^\]]*\]\([^)]*\)|<https?:\/\/[^>]*>|[^`[<]+|[\s\S]/y;

function inline(text: string, out: Runs): void {
  let at = 0;
  while (at < text.length) {
    INLINE.lastIndex = at;
    const match = INLINE.exec(text);
    const part = match?.[0] ?? text.slice(at);
    at += part.length;
    if (part.startsWith("`")) {
      out.add(part, "string");
    } else if (part.startsWith("[") && part.includes("](")) {
      const split = part.indexOf("](");
      out.add(part.slice(0, split + 1), "function");
      out.add(part.slice(split + 1), "comment");
    } else if (part.startsWith("<http")) {
      out.add(part, "comment");
    } else {
      out.add(part);
    }
  }
}

/** State: "" or "fence", inside a fenced block of code, which is shown as it is. */
const markdown: Scanner = (line, state): Scanned => {
  const out = new Runs();
  const fence = /^\s*(```|~~~)/.test(line);
  if (fence) {
    const tokens: Token[] = [{ text: line, kind: "comment" }];
    return state === "fence" ? { tokens, state: "" } : { tokens, state: "fence" };
  }
  if (state === "fence") return { tokens: [{ text: line }], state };
  const heading = /^(#{1,6}\s.*)$/.exec(line);
  if (heading !== null) return { tokens: [{ text: line, kind: "function" }], state };
  if (/^\s*(?:-{3,}|\*{3,}|_{3,})\s*$/.test(line))
    return { tokens: [{ text: line, kind: "comment" }], state };
  const quote = /^(\s*>+\s?)(.*)$/.exec(line);
  const list = /^(\s*(?:[-*+]|\d+[.)])\s+)(.*)$/.exec(line);
  const lead = quote ?? list;
  if (lead !== null) {
    out.add(lead[1] ?? "", quote === null ? "keyword" : "comment");
    inline(lead[2] ?? "", out);
  } else {
    inline(line, out);
  }
  return { tokens: out.tokens, state };
};

export const markupScanners: Readonly<Record<string, Scanner>> = {
  html,
  css,
  scss,
  markdown,
};
