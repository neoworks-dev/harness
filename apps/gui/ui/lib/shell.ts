// Shell commands laid out as the terminal app lays them out (renderers/ratatui/src/shell.rs): a
// line too long is broken after its `&&`, `||`, `;` (and `|` when a pipeline alone is too long),
// only where bash takes a newline the same way. Plain functions, tested without a window.

/**
 * `source` with every line longer than `width` broken after its `&&`, `||` and `;`, the parts
 * after the first indented two more; after `|` too when that is not enough. Not inside quotes,
 * parentheses or `$(…)`, comments or heredoc bodies. Lines that fit stay as they are.
 */
export function reflowCommand(source: string, width: number): string {
  const lists = reflowAt(source, width, false);
  if (lists.split("\n").every((line) => line.length <= width)) return lists;
  return reflowAt(source, width, true);
}

function reflowAt(source: string, width: number, pipes: boolean): string {
  const chars = [...source];
  let out = "";
  // The current line, and where it may break (char indices into it).
  let line: string[] = [];
  let breaks: number[] = [];
  let quote: string | undefined;
  let depth = 0;
  // Heredocs opened on the current line: their terminator and whether tabs are stripped.
  let heredocs: [string, boolean][] = [];
  let wordStart = true;
  let i = 0;
  while (i < chars.length) {
    const c = chars[i]!;
    if (c === "\n") {
      out += flush(line, breaks, width) + "\n";
      line = [];
      breaks = [];
      wordStart = true;
      i += 1;
      for (const [term, strip] of heredocs) {
        while (i < chars.length) {
          let end = chars.indexOf("\n", i);
          if (end < 0) end = chars.length;
          const body = chars.slice(i, end).join("");
          out += body;
          i = end;
          if (i < chars.length) {
            out += "\n";
            i += 1;
          }
          if ((strip ? body.replace(/^\t+/, "") : body) === term) break;
        }
      }
      heredocs = [];
      continue;
    }
    line.push(c);
    i += 1;
    if (quote === "'") {
      if (c === "'") quote = undefined;
      continue;
    }
    if (quote !== undefined) {
      if (c === "\\" && i < chars.length && chars[i] !== "\n") {
        line.push(chars[i]!);
        i += 1;
      } else if (c === quote) quote = undefined;
      continue;
    }
    switch (c) {
      case "\\":
        if (i < chars.length && chars[i] !== "\n") {
          line.push(chars[i]!);
          i += 1;
        }
        wordStart = false;
        break;
      case "'":
      case '"':
      case "`":
        quote = c;
        wordStart = false;
        break;
      case "#":
        if (wordStart) {
          while (i < chars.length && chars[i] !== "\n") {
            line.push(chars[i]!);
            i += 1;
          }
        } else wordStart = false;
        break;
      case "(":
        depth += 1;
        wordStart = true;
        break;
      case ")":
        depth -= 1;
        wordStart = true;
        break;
      case "<":
        if (chars[i] === "<" && chars[i + 1] === "<") {
          // A here-string, not a heredoc.
          line.push("<", "<");
          i += 2;
        } else if (chars[i] === "<") {
          line.push("<");
          i += 1;
          const strip = chars[i] === "-";
          if (strip) {
            line.push("-");
            i += 1;
          }
          while (chars[i] === " " || chars[i] === "\t") {
            line.push(chars[i]!);
            i += 1;
          }
          let term = "";
          while (i < chars.length) {
            const n = chars[i]!;
            if (/\s/.test(n) || ";|&<>()".includes(n)) break;
            line.push(n);
            if (n !== "'" && n !== '"' && n !== "\\") term += n;
            i += 1;
          }
          // `1 << 2` shifts; a terminator is a word.
          if (/^[\p{L}_]/u.test(term)) heredocs.push([term, strip]);
        }
        wordStart = false;
        break;
      case "|":
      case "&":
      case ";": {
        let op: string = c;
        while (i < chars.length && "|&;".includes(chars[i]!)) {
          op += chars[i];
          line.push(chars[i]!);
          i += 1;
        }
        // `>&`/`&>` redirect and a lone `&` backgrounds: no break after those.
        const redirect = op.startsWith("&") && chars[i] === ">";
        const list = op === "&&" || op === "||" || op === ";" || op === ";;";
        if (depth === 0 && !redirect && (list || (pipes && (op === "|" || op === "|&")))) {
          breaks.push(line.length);
        }
        wordStart = true;
        break;
      }
      default:
        wordStart = /\s/.test(c);
    }
  }
  return out + flush(line, breaks, width);
}

/** One line: as it is when it fits, else broken at `breaks`. */
function flush(chars: readonly string[], breaks: readonly number[], width: number): string {
  const line = chars.join("");
  if (breaks.length === 0 || chars.length <= width) return line;
  const indent = /^[ \t]*/.exec(line)?.[0] ?? "";
  let out = "";
  let from = 0;
  let first = true;
  for (const at of [...breaks, chars.length]) {
    const raw = chars.slice(from, at).join("");
    from = at;
    const part = first ? raw.trimEnd() : raw.trim();
    if (part === "") continue;
    if (!first) out += `\n${indent}  `;
    out += part;
    first = false;
  }
  return out;
}
