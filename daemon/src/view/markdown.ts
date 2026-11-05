// Markdown → the view format's blocks and spans, so renderers never parse markdown themselves.
import { Lexer, type Token, Tokenizer, type Tokens } from "marked";
import type {
  Block,
  ColumnAlign,
  ListItem,
  Span,
  TableBlock,
  TableCell,
  UiDoc,
} from "@neoworks-dev/neo-protocol/types";

type Style = Omit<Span, "text">;

function sameStyle(a: Span, b: Span): boolean {
  return (
    a.bold === b.bold &&
    a.italic === b.italic &&
    a.code === b.code &&
    a.strike === b.strike &&
    a.href === b.href
  );
}

function decode(text: string): string {
  return text
    .replaceAll("&lt;", "<")
    .replaceAll("&gt;", ">")
    .replaceAll("&quot;", '"')
    .replaceAll("&#39;", "'")
    .replaceAll("&amp;", "&");
}

function push(spans: Span[], text: string, style: Style): void {
  if (text === "") return;
  const span: Span = { text, ...style };
  const last = spans.at(-1);
  if (last && sameStyle(last, span)) last.text += text;
  else spans.push(span);
}

function inline(tokens: Token[] | undefined, style: Style, spans: Span[]): Span[] {
  for (const token of tokens ?? []) {
    switch (token.type) {
      case "strong":
        inline(token.tokens, { ...style, bold: true }, spans);
        break;
      case "em":
        inline(token.tokens, { ...style, italic: true }, spans);
        break;
      case "del":
        inline(token.tokens, { ...style, strike: true }, spans);
        break;
      case "codespan":
        push(spans, decode(token.text), { ...style, code: true });
        break;
      case "link":
        inline(token.tokens, { ...style, href: token.href }, spans);
        break;
      case "image":
        push(spans, `[image: ${token.text || token.href}]`, style);
        break;
      case "br":
        push(spans, "\n", style);
        break;
      case "text":
        if (token.tokens) inline(token.tokens, style, spans);
        else push(spans, decode(token.text), style);
        break;
      default:
        push(spans, decode("text" in token ? String(token.text) : token.raw), style);
    }
  }
  return spans;
}

function table(token: Tokens.Table): TableBlock {
  const cell = (c: Tokens.TableCell): TableCell => ({ spans: inline(c.tokens, {}, []) });
  const block: TableBlock = {
    type: "table_block",
    header: token.header.map(cell),
    rows: token.rows.map((row) => row.map(cell)),
  };
  // marked says null for a column the separator row doesn't align.
  if (token.align.some((a) => a !== null)) {
    block.align = token.align.map((a): ColumnAlign => a ?? "left");
  }
  return block;
}

/**
 * What turns the program of a ```neoui fence into a doc: its position among the message's
 * neoui fences, the program, and whether the fence has been closed (the program is complete).
 */
export type NeoUiHost = (index: number, program: string, closed: boolean) => UiDoc;

/** A neoui fence's language, with or without the info string after it. */
const NEOUI_LANG = /^neoui(\s|$)/;

/** A closing fence at the end of a code token's source: the token was not cut off. */
function fenceClosed(raw: string): boolean {
  const lines = raw.trimEnd().split("\n");
  return lines.length > 1 && /^ {0,3}(`{3,}|~{3,})\s*$/.test(lines.at(-1) ?? "");
}

/** The program of a fence cut off in the middle of its closing fence loses that stub. */
function programOf(token: Tokens.Code, closed: boolean): string {
  if (closed) return token.text;
  return token.text.replace(/(^|\n)[`~]{1,2}$/, "");
}

interface Fences {
  host: NeoUiHost;
  count: number;
}

function blocks(tokens: Token[], fences?: Fences): Block[] {
  const out: Block[] = [];
  for (const token of tokens) {
    switch (token.type) {
      case "space":
      case "def":
        break;
      case "heading":
        out.push({ type: "heading", level: token.depth, spans: inline(token.tokens, {}, []) });
        break;
      case "paragraph":
      case "text":
        out.push({
          type: "paragraph",
          spans: token.tokens ? inline(token.tokens, {}, []) : [{ text: decode(token.text) }],
        });
        break;
      case "list": {
        const items: ListItem[] = token.items.map(
          (item: Token & { task: boolean; checked?: boolean; tokens: Token[] }) => {
            const listItem: ListItem = {
              blocks: blocks(item.tokens.filter((t) => t.type !== "checkbox")),
            };
            if (item.task) listItem.checked = item.checked === true;
            return listItem;
          },
        );
        const list: Block = { type: "list", ordered: token.ordered, items };
        if (token.ordered && typeof token.start === "number") list.start = token.start;
        out.push(list);
        break;
      }
      case "code": {
        if (fences && token.lang !== undefined && NEOUI_LANG.test(token.lang)) {
          const closed = fenceClosed(token.raw);
          const ui = fences.host(fences.count++, programOf(token as Tokens.Code, closed), closed);
          out.push({ type: "neoui", ui });
          break;
        }
        const code: Block = { type: "code_block", code: token.text };
        if (token.lang) code.language = token.lang;
        out.push(code);
        break;
      }
      case "blockquote":
        out.push({ type: "quote", blocks: blocks(token.tokens ?? []) });
        break;
      case "hr":
        out.push({ type: "rule" });
        break;
      case "table":
        out.push(table(token as Tokens.Table));
        break;
      default:
        // HTML and anything else: kept as monospace source rather than dropped.
        out.push({ type: "code_block", code: token.raw.replace(/\n+$/, "") });
    }
  }
  return out;
}

/**
 * Without setext headings (a line underlined with `===` or `---`): marked's rule for them tries
 * every paragraph to its end at every block, most of the time a long chat takes to parse. Agents
 * write `#` headings; a `---` under a line is a rule.
 */
class NoSetext extends Tokenizer {
  override lheading(): undefined {
    return undefined;
  }
}

function lex(source: string): Token[] {
  return Lexer.lex(source, { gfm: true, tokenizer: new NoSetext() });
}

/**
 * With `neoui`, the message's top-level neoui fences become `neoui` blocks (the daemon's
 * messages); without it they stay code, as everywhere else (templates, messages between agents).
 */
export function parseMarkdown(source: string, neoui?: NeoUiHost): Block[] {
  const tokens = lex(source);
  return blocks(tokens, neoui && { host: neoui, count: 0 });
}

/** A link reference definition, which can change how text above it reads. */
const LINK_DEFINITION = /^ {0,3}\[[^\]\n]+\]:/m;

/**
 * A message's markdown parsed as it grows. Text added at the end can only change the last
 * top-level block, so the blocks before it are kept, the same objects, and only the rest is parsed
 * again: a long message no longer costs more to extend with every chunk. A link definition can
 * change text above it and a carriage return changes offsets, so either means parsing it all.
 */
export class GrowingMarkdown {
  /** How much of the source the settled blocks come from. */
  private chars = 0;
  private settled: Block[] = [];
  /** neoui fences among the settled blocks. */
  private fences = 0;

  /** The blocks of `source`, which extends what the previous call was given. */
  parse(source: string, neoui?: NeoUiHost): Block[] {
    if (source.length < this.chars || source.includes("\r") || LINK_DEFINITION.test(source)) {
      this.reset();
      return parseMarkdown(source, neoui);
    }
    const tokens = lex(source.slice(this.chars));
    let last = tokens.length - 1;
    while (last > 0 && tokens[last]?.type === "space") last--;
    const done = tokens.slice(0, Math.max(last, 0));
    const fences = neoui && { host: neoui, count: this.fences };
    this.settled.push(...blocks(done, fences));
    for (const token of done) this.chars += token.raw.length;
    if (fences) this.fences = fences.count;
    return [...this.settled, ...blocks(tokens.slice(Math.max(last, 0)), fences)];
  }

  /** Forgets what was parsed: the next call parses the whole source. */
  reset(): void {
    this.chars = 0;
    this.settled = [];
    this.fences = 0;
  }
}
