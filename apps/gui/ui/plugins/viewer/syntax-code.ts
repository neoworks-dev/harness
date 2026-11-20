// The scanners of programming languages: one engine over a language's spec (its keywords, how its
// comments and strings look), and a few more for the formats that look like code (JSON, shell, TOML).
import { Runs, type Kind, type Scanned, type Scanner } from "./syntax-runs.ts";

interface Spec {
  /** Space-separated words. */
  keywords: string;
  constants?: string;
  /** Built-in types; a word that starts in a capital is one anyway. */
  types?: string;
  /** Keywords after which the next word names a thing: `fn` a function, `struct` a type. */
  declares?: Readonly<Record<string, Kind>>;
  /** What starts a comment to the end of the line. */
  line?: string;
  /** Has `/* … *\/` comments. */
  block?: boolean;
  /** The characters that open a string of one line. */
  quotes: string;
  /** A backtick opens a string that goes on over lines. */
  backtick?: boolean;
  /** Python's and TOML's `"""` and `'''` strings. */
  triple?: boolean;
  /** `name!(` calls a macro. */
  macros?: boolean;
  /** `'a` is a lifetime, not the start of a character. */
  lifetimes?: boolean;
  /** `#[derive(Debug)]` and `#![allow(…)]`. */
  attributes?: boolean;
  /** `$name` and `${name}`: a variable of the shell. */
  variables?: boolean;
  /** `@name` in front of a definition. */
  decorators?: boolean;
  /** `#include` and the other lines of the C preprocessor. */
  directives?: boolean;
  /** A word that starts in a capital is a type, and one in capitals only a constant. */
  caps?: boolean;
}

const words = (list: string | undefined): Set<string> => new Set((list ?? "").split(/\s+/));

const escape = (text: string): string => text.replace(/[.*+?^${}()|[\]\\/]/g, "\\$&");

const DECLARES: Readonly<Record<string, Kind>> = {
  function: "function",
  fn: "function",
  func: "function",
  def: "function",
  class: "type",
  struct: "type",
  enum: "type",
  trait: "type",
  interface: "type",
  type: "type",
  impl: "type",
  mod: "type",
  namespace: "type",
  union: "type",
  record: "type",
};

function codeScanner(spec: Spec): Scanner {
  const keywords = words(spec.keywords);
  const constants = words(spec.constants);
  const types = words(spec.types);
  const declares = { ...DECLARES, ...spec.declares };
  const caps = spec.caps ?? true;

  const parts: [string, string][] = [["ws", "\\s+"]];
  if (spec.line !== undefined) parts.push(["line", `${escape(spec.line)}.*`]);
  if (spec.block) parts.push(["block", "/\\*"]);
  if (spec.triple) parts.push(["triple", `[rRbBfFuU]{0,2}(?:"""|''')`]);
  if (spec.lifetimes) parts.push(["life", "'[A-Za-z_]\\w*(?![\\w'])"]);
  for (const quote of spec.quotes) {
    const q = escape(quote);
    parts.push(["string", `${q}(?:[^${q}\\\\]|\\\\.)*${q}?`]);
  }
  if (spec.backtick) parts.push(["backtick", "`"]);
  if (spec.attributes) parts.push(["attr", "#!?\\[[^\\]]*\\]?"]);
  if (spec.variables)
    parts.push(["variable", "\\$(?:\\{[^}]*\\}?|\\(|[A-Za-z_]\\w*|[0-9@*#?$!-])"]);
  if (spec.decorators) parts.push(["decorator", "@[A-Za-z_][\\w.]*"]);
  if (spec.directives) parts.push(["directive", "#\\s*[a-z]+"]);
  parts.push([
    "number",
    "0[xX][\\da-fA-F_]+\\w*|0[bBoO][\\d_]+\\w*|(?:\\d[\\d_]*\\.?[\\d_]*|\\.\\d[\\d_]*)(?:[eE][+-]?\\d+)?\\w*",
  ]);
  parts.push(["word", spec.variables ? "[A-Za-z_][\\w-]*" : "[A-Za-z_$][\\w$]*"]);
  parts.push(["other", "[\\s\\S]"]);
  const names = parts.map(([name]) => name);
  const master = new RegExp(parts.map(([, source]) => `(${source})`).join("|"), "y");

  /** The end of what a block, template or triple-quoted string that goes on is, or -1. */
  const closing = (line: string, from: number, state: string): number => {
    if (state === "block") {
      const end = line.indexOf("*/", from);
      return end < 0 ? -1 : end + 2;
    }
    const quote = state === "backtick" ? "`" : state.slice(1);
    for (let i = from; i < line.length; i += 1) {
      if (line[i] === "\\" && state === "backtick") i += 1;
      else if (line.startsWith(quote, i)) return i + quote.length;
    }
    return -1;
  };
  const kindOfState = (state: string): Kind => (state === "block" ? "comment" : "string");

  return (line, start): Scanned => {
    const out = new Runs();
    let state = start;
    let at = 0;
    if (state !== "") {
      // A comment or string from the line before.
      const end = closing(line, 0, state);
      out.add(line.slice(0, end < 0 ? line.length : end), kindOfState(state));
      if (end < 0) return { tokens: out.tokens, state };
      state = "";
      at = end;
    }
    /** The kind the next word takes, after a keyword that declares. */
    let next: Kind | undefined;
    while (at < line.length) {
      master.lastIndex = at;
      const match = master.exec(line);
      if (match === null) break;
      const text = match[0];
      let group = 1;
      while (match[group] === undefined) group += 1;
      const name = names[group - 1];
      at += text.length;
      if (name === "ws") {
        out.add(text);
        continue;
      }
      const declared = next;
      next = undefined;
      switch (name) {
        case "line":
          out.add(text, "comment");
          break;
        case "block":
        case "triple":
        case "backtick": {
          state = name === "triple" ? `t${text.slice(-3)}` : name;
          const end = closing(line, at, state);
          const stop = end < 0 ? line.length : end;
          out.add(text + line.slice(at, stop), kindOfState(state));
          at = stop;
          if (end >= 0) state = "";
          break;
        }
        case "string":
        case "number":
        case "attr":
          out.add(text, name);
          break;
        case "variable":
          out.add(text, "property");
          break;
        case "decorator":
          out.add(text, "function");
          break;
        case "directive":
          out.add(text, "keyword");
          break;
        case "life":
          out.add(text, "type");
          break;
        case "word": {
          let kind: Kind | undefined;
          // `row.type` and `.default` name a member, whatever they are called elsewhere.
          const member = line[at - text.length - 1] === ".";
          if (member) {
            if (line[at] === "(") kind = "function";
          } else if (keywords.has(text)) {
            kind = "keyword";
            next = declares[text];
          } else if (constants.has(text)) kind = "constant";
          else if (types.has(text)) kind = "type";
          else if (declared !== undefined) kind = declared;
          else if (line[at] === "(" || (spec.macros && line.startsWith("!(", at)))
            kind = "function";
          else if (caps && /^[A-Z]/.test(text))
            kind = /^[A-Z][A-Z0-9_]+$/.test(text) ? "constant" : "type";
          out.add(text, kind);
          break;
        }
        default:
          out.add(text);
      }
    }
    return { tokens: out.tokens, state };
  };
}

const JS_KEYWORDS =
  "break case catch class const continue debugger default delete do else enum export extends finally for from function if implements import in instanceof interface let new of package private protected public return static switch throw try typeof var while with yield async await as readonly declare abstract namespace module keyof satisfies is infer get set type override accessor using";
const JS_TYPES =
  "string number boolean any unknown never void object bigint symbol Promise Array Record Partial Readonly Map Set Date Error";

const ts = codeScanner({
  keywords: JS_KEYWORDS,
  constants: "true false null undefined NaN Infinity this super",
  types: JS_TYPES,
  line: "//",
  block: true,
  quotes: "\"'",
  backtick: true,
});

const rust = codeScanner({
  keywords:
    "as async await break const continue crate dyn else enum extern fn for if impl in let loop match mod move mut pub ref return self static struct super trait type unsafe use where while union",
  constants: "true false None Some Ok Err self",
  types:
    "i8 i16 i32 i64 i128 isize u8 u16 u32 u64 u128 usize f32 f64 bool char str String Vec Option Result Box Self",
  line: "//",
  block: true,
  quotes: "\"'",
  macros: true,
  lifetimes: true,
  attributes: true,
});

const go = codeScanner({
  keywords:
    "break case chan const continue default defer else fallthrough for func go goto if import interface map package range return select struct switch type var",
  constants: "true false nil iota",
  types:
    "int int8 int16 int32 int64 uint uint8 uint16 uint32 uint64 uintptr float32 float64 complex64 complex128 string bool byte rune error any",
  line: "//",
  block: true,
  quotes: "\"'",
  backtick: true,
});

const python = codeScanner({
  keywords:
    "and as assert async await break class continue def del elif else except finally for from global if import in is lambda nonlocal not or pass raise return try while with yield match case",
  constants: "True False None self cls",
  types: "int str float bool list dict set tuple bytes object type",
  line: "#",
  quotes: "\"'",
  triple: true,
  decorators: true,
});

const shellWords = codeScanner({
  keywords:
    "if then else elif fi for while until do done case esac in function select return exit break continue export local readonly declare unset set shift source alias eval exec echo cd test trap read printf",
  line: "#",
  quotes: "\"'",
  variables: true,
  caps: false,
});

/** Keywords after which a command starts. */
const SHELL_LEADS = new Set(["if", "then", "else", "elif", "do", "while", "until", "time", "!", "{"]);
/** In the uncoloured text of a line: blanks or a redirect, an operator, a flag, a word. */
const SHELL_PLAIN = /(\s+|[<>]+&?|&>>?)|(&&|\|\||;;|\|&|[|;&(){}])|(--?[A-Za-z0-9][\w-]*)|([^\s|;&(){}<>]+)/y;

/**
 * Shell as the generic scanner reads it, and what it leaves plain picked out the way a command is
 * read: the command each list, pipeline or `$(…)` starts with, its flags and the operators between.
 * A line after one that ends in `\` goes on with the command's arguments; any other starts anew.
 */
const shell: Scanner = (line, state) => {
  const out = new Runs();
  let command = state !== "\\";
  for (const token of shellWords(line, "").tokens) {
    if (token.kind !== undefined) {
      out.add(token.text, token.kind);
      if (token.kind === "keyword") command = SHELL_LEADS.has(token.text);
      else if (token.kind !== "comment") command = token.text === "$(";
      continue;
    }
    let at = 0;
    while (at < token.text.length) {
      SHELL_PLAIN.lastIndex = at;
      const match = SHELL_PLAIN.exec(token.text);
      if (match === null) break;
      const [text, blank, operator, flag] = match;
      at += text.length;
      if (blank !== undefined) out.add(text);
      else if (operator !== undefined) {
        out.add(text, "keyword");
        command = operator !== ")" && operator !== "}";
      } else if (flag !== undefined && !command) out.add(text, "attr");
      else if (command && /^[\w.-]+=/.test(text)) out.add(text, "property");
      else {
        out.add(text, command ? "function" : undefined);
        command = false;
      }
    }
  }
  return { tokens: out.tokens, state: line.endsWith("\\") ? "\\" : "" };
};

const c = codeScanner({
  keywords:
    "auto break case catch class const constexpr continue default delete do else enum explicit extern for friend goto if inline namespace new noexcept operator private protected public register return sizeof static struct switch template this throw try typedef typename union using virtual volatile while",
  constants: "true false NULL nullptr",
  types:
    "int long short char float double void bool unsigned signed size_t uint8_t uint16_t uint32_t uint64_t int8_t int16_t int32_t int64_t string",
  line: "//",
  block: true,
  quotes: "\"'",
  directives: true,
});

const java = codeScanner({
  keywords:
    "abstract as break case catch class const continue data default do else enum extends final finally for fun func if implements import in instanceof interface internal is let namespace new object open override package private protected public return sealed static struct super switch synchronized this throw throws try typealias val var void volatile when where while using var",
  constants: "true false null nil",
  types:
    "int long short byte char float double boolean String Int Long Double Float Boolean Unit Any Void",
  line: "//",
  block: true,
  quotes: "\"'",
  decorators: true,
});

/** JSON, with the comments that JSONC and JSON5 allow; a string before a colon is a key. */
const jsonValues = codeScanner({
  keywords: "",
  constants: "true false null",
  line: "//",
  block: true,
  quotes: "\"'",
  caps: false,
});

const json: Scanner = (line, state) => {
  const scanned = jsonValues(line, state);
  const { tokens } = scanned;
  tokens.forEach((token, i) => {
    const after = tokens[i + 1];
    if (
      token.kind === "string" &&
      after !== undefined &&
      after.kind === undefined &&
      /^\s*:/.test(after.text)
    ) {
      token.kind = "property";
    }
  });
  return scanned;
};

const tomlValues = codeScanner({
  keywords: "",
  constants: "true false inf nan",
  line: "#",
  quotes: "\"'",
  triple: true,
  caps: false,
});

const toml: Scanner = (line, state) => {
  if (state !== "") return tomlValues(line, state);
  const section = /^(\s*)(\[\[?[^\]]*\]\]?)(.*)$/.exec(line);
  if (section !== null) {
    const out = new Runs();
    out.add(section[1] ?? "");
    out.add(section[2] ?? "", "type");
    out.addAll(tomlValues(section[3] ?? "", "").tokens);
    return { tokens: out.tokens, state: "" };
  }
  const key = /^(\s*)((?:[\w.-]+|"[^"]*"|'[^']*')(?:\s*\.\s*(?:[\w-]+|"[^"]*"))*)(\s*=)(.*)$/.exec(
    line,
  );
  if (key === null) return tomlValues(line, state);
  const rest = tomlValues(key[4] ?? "", "");
  const out = new Runs();
  out.add(key[1] ?? "");
  out.add(key[2] ?? "", "property");
  out.add(key[3] ?? "");
  out.addAll(rest.tokens);
  return { tokens: out.tokens, state: rest.state };
};

/** YAML a line at a time: the key, then a value that is a string, number or word. */
const yamlValues = codeScanner({
  keywords: "",
  constants: "true false null yes no on off True False Null ~",
  line: "#",
  quotes: "\"'",
  caps: false,
});

const yaml: Scanner = (line) => {
  if (/^---|^\.\.\.\s*$/.test(line))
    return { tokens: [{ text: line, kind: "comment" }], state: "" };
  const key = /^(\s*(?:-\s+)*)((?:[^\s#:'"-][^#:]*?|"[^"]*"|'[^']*'))(\s*:)(?=\s|$)(.*)$/.exec(
    line,
  );
  if (key === null) return { tokens: yamlValues(line, "").tokens, state: "" };
  const out = new Runs();
  out.add(key[1] ?? "");
  out.add(key[2] ?? "", "property");
  out.add(key[3] ?? "");
  out.addAll(yamlValues(key[4] ?? "", "").tokens);
  return { tokens: out.tokens, state: "" };
};

export const codeScanners: Readonly<Record<string, Scanner>> = {
  ts,
  js: ts,
  rust,
  go,
  python,
  shell,
  c,
  java,
  json,
  toml,
  yaml,
};
