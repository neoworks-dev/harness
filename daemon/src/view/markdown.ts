// Markdown → the view format's blocks and spans, so renderers never parse markdown themselves.
import { Lexer, type Token } from "marked";
import type { Block, ListItem, Span } from "@suite/protocol/types";

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

function blocks(tokens: Token[]): Block[] {
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
      default:
        // Tables, HTML and anything else: kept as monospace source rather than dropped.
        out.push({ type: "code_block", code: token.raw.replace(/\n+$/, "") });
    }
  }
  return out;
}

export function parseMarkdown(source: string): Block[] {
  return blocks(Lexer.lex(source, { gfm: true }));
}
