// Resolved view → DOM, one function per component and block. Pure: everything is built with the
// document it is given (so it runs under happy-dom as well as in a browser) and nothing is
// attached anywhere. Every component renders to exactly one element, and the children of a
// container component are that element's child elements (a card's title aside); render.ts relies
// on both to find the element behind a patch path.
import type {
  Block,
  ButtonComponent,
  CodeBlock,
  Component,
  DiffComponent,
  FileComponent,
  ImageComponent,
  ItemState,
  LinkComponent,
  ListBlock,
  Span,
  TableComponent,
  ViewItem,
} from "@suite/protocol/types";

export const STATE_ICONS: Record<ItemState, string> = {
  pending: "○",
  running: "◐",
  done: "✓",
  failed: "✗",
};

/** The buttons of the item being rendered, in document order: the index is part of their id. */
export interface RenderContext {
  doc: Document;
  buttons: ButtonComponent[];
}

type Attrs = Record<string, string | undefined>;

function el(
  doc: Document,
  tag: string,
  className: string,
  attrs: Attrs = {},
  ...children: (Node | string)[]
): HTMLElement {
  const node = doc.createElement(tag);
  if (className !== "") node.className = className;
  for (const [name, value] of Object.entries(attrs)) {
    if (value !== undefined) node.setAttribute(name, value);
  }
  node.append(...children);
  return node;
}

/** Only links that can't run script: the daemon passes hrefs through as the agent wrote them. */
function safeUrl(url: string): boolean {
  return /^(https?:|mailto:)/i.test(url);
}

function baseName(path: string): string {
  const trimmed = path.replace(/[\\/]+$/, "");
  const parts = trimmed.split(/[\\/]/);
  return parts.at(-1) ?? trimmed;
}

// --- text ---

function renderSpan(doc: Document, span: Span): Node {
  let node: Node = doc.createTextNode(span.text);
  if (span.code) node = el(doc, "code", "suite-inline-code", {}, node);
  if (span.bold) node = el(doc, "strong", "", {}, node);
  if (span.italic) node = el(doc, "em", "", {}, node);
  if (span.strike) node = el(doc, "del", "", {}, node);
  if (span.href !== undefined && safeUrl(span.href)) {
    node = el(
      doc,
      "a",
      "suite-link",
      { href: span.href, target: "_blank", rel: "noopener noreferrer" },
      node,
    );
  }
  return node;
}

function renderSpans(doc: Document, spans: Span[]): Node[] {
  return spans.map((span) => renderSpan(doc, span));
}

function renderCodeBlock(
  doc: Document,
  block: Pick<CodeBlock, "code" | "language">,
  className: string,
): HTMLElement {
  const code = el(doc, "code", "", {}, block.code);
  return el(doc, "pre", className, { "data-language": block.language }, code);
}

function renderList(ctx: RenderContext, block: ListBlock): HTMLElement {
  const tag = block.ordered ? "ol" : "ul";
  const start = block.ordered && block.start !== undefined ? String(block.start) : undefined;
  const list = el(ctx.doc, tag, "suite-list", { start });
  for (const item of block.items) {
    const li = el(ctx.doc, "li", "");
    if (item.checked !== undefined) {
      li.className = "suite-task";
      const box = el(ctx.doc, "input", "suite-checkbox", {
        type: "checkbox",
        disabled: "",
        "aria-label": item.checked ? "done" : "not done",
      });
      if (item.checked) box.setAttribute("checked", "");
      li.append(box);
    }
    li.append(...renderBlocks(ctx, item.blocks));
    list.append(li);
  }
  return list;
}

export function renderBlock(ctx: RenderContext, block: Block): HTMLElement {
  const { doc } = ctx;
  switch (block.type) {
    case "paragraph":
      return el(doc, "p", "", {}, ...renderSpans(doc, block.spans));
    case "heading": {
      const level = Math.min(Math.max(Math.trunc(block.level), 1), 6);
      return el(doc, `h${level}`, "suite-heading", {}, ...renderSpans(doc, block.spans));
    }
    case "list":
      return renderList(ctx, block);
    case "code_block":
      return renderCodeBlock(doc, block, "suite-code");
    case "quote":
      return el(doc, "blockquote", "suite-quote", {}, ...renderBlocks(ctx, block.blocks));
    case "rule":
      return el(doc, "hr", "suite-rule");
  }
}

export function renderBlocks(ctx: RenderContext, blocks: Block[]): HTMLElement[] {
  return blocks.map((block) => renderBlock(ctx, block));
}

// --- leaf components ---

function renderImageFallback(doc: Document, image: ImageComponent): HTMLElement {
  let size = "";
  if (image.width !== undefined && image.height !== undefined) {
    size = ` ${image.width}×${image.height}`;
  }
  return el(
    doc,
    "div",
    "suite-image-fallback",
    { role: "img", "aria-label": image.alt ?? "image" },
    `[image${size} · ${image.src}]`,
  );
}

function renderImage(ctx: RenderContext, image: ImageComponent): HTMLElement {
  const { doc } = ctx;
  const figure = el(doc, "figure", "suite-image");
  // asset:// handles resolve in Phase 5, and a bare path means nothing to a browser: both get the
  // text box, as does an image that fails to load.
  if (/^(data:image\/|https?:)/i.test(image.src)) {
    const img = el(doc, "img", "", {
      src: image.src,
      alt: image.alt ?? "",
      width: image.width === undefined ? undefined : String(image.width),
      height: image.height === undefined ? undefined : String(image.height),
      loading: "lazy",
    });
    img.addEventListener("error", () => img.replaceWith(renderImageFallback(doc, image)));
    figure.append(img);
  } else {
    figure.append(renderImageFallback(doc, image));
  }
  if (image.caption !== undefined) figure.append(el(doc, "figcaption", "", {}, image.caption));
  return figure;
}

function renderFile(doc: Document, file: FileComponent): HTMLElement {
  return el(
    doc,
    "span",
    "suite-file",
    { title: file.path },
    el(doc, "span", "suite-file-icon", { "aria-hidden": "true" }, "📄"),
    ` ${file.name ?? baseName(file.path)}`,
  );
}

function lineNumber(n: number | undefined): string {
  if (n === undefined) return "";
  return String(n);
}

function renderDiff(doc: Document, diff: DiffComponent): HTMLElement {
  const figure = el(doc, "figure", "suite-diff");
  if (diff.path !== undefined) {
    figure.append(el(doc, "figcaption", "suite-diff-path", {}, diff.path));
  }
  const body = el(doc, "pre", "suite-diff-body");
  for (const line of diff.lines) {
    const sign = { add: "+", remove: "-", context: " ", hunk: "" }[line.kind];
    const row = el(doc, "span", "suite-diff-line", { "data-kind": line.kind });
    if (line.kind !== "hunk") {
      row.append(
        el(doc, "span", "suite-diff-num", {}, lineNumber(line.old)),
        el(doc, "span", "suite-diff-num", {}, lineNumber(line.new)),
        el(doc, "span", "suite-diff-sign", { "aria-hidden": "true" }, sign),
      );
    }
    row.append(el(doc, "span", "suite-diff-text", {}, line.text));
    body.append(row);
  }
  figure.append(body);
  return figure;
}

function renderTable(doc: Document, table: TableComponent): HTMLElement {
  const head = el(doc, "tr", "");
  for (const column of table.columns) head.append(el(doc, "th", "", { scope: "col" }, column));
  const body = el(doc, "tbody", "");
  for (const row of table.rows) {
    body.append(el(doc, "tr", "", {}, ...row.map((cell) => el(doc, "td", "", {}, cell))));
  }
  const grid = el(doc, "table", "suite-table", {}, el(doc, "thead", "", {}, head), body);
  return el(doc, "div", "suite-table-wrap", {}, grid);
}

function renderProgress(doc: Document, value: number, label: string | undefined): HTMLElement {
  const fraction = Math.min(Math.max(value, 0), 1);
  const percent = Math.round(fraction * 100);
  const bar = el(doc, "div", "suite-progress-bar");
  bar.style.setProperty("--suite-progress", `${percent}%`);
  const track = el(
    doc,
    "div",
    "suite-progress-track",
    {
      role: "progressbar",
      "aria-valuemin": "0",
      "aria-valuemax": "100",
      "aria-valuenow": String(percent),
      "aria-label": label,
    },
    bar,
  );
  const root = el(doc, "div", "suite-progress", {}, track);
  root.append(el(doc, "span", "suite-progress-percent", {}, `${percent}%`));
  if (label !== undefined) root.append(el(doc, "span", "suite-progress-label", {}, label));
  return root;
}

/** The key that triggers button `index`: its own `key`, else its 1-based position (as in the TUI). */
export function buttonKey(button: ButtonComponent, index: number): string {
  return button.key ?? String(index + 1);
}

function renderButton(ctx: RenderContext, button: ButtonComponent): HTMLElement {
  const index = ctx.buttons.length;
  ctx.buttons.push(button);
  const key = buttonKey(button, index);
  return el(
    ctx.doc,
    "button",
    "suite-button",
    { type: "button", "data-button": String(index), "aria-keyshortcuts": key },
    el(ctx.doc, "span", "suite-button-label", {}, button.label),
    " ",
    el(ctx.doc, "kbd", "suite-key", { "aria-hidden": "true" }, key),
  );
}

function renderLink(doc: Document, link: LinkComponent): HTMLElement {
  const text = link.text ?? link.url;
  if (!safeUrl(link.url)) return el(doc, "span", "suite-link", {}, text);
  return el(
    doc,
    "a",
    "suite-link",
    { href: link.url, target: "_blank", rel: "noopener noreferrer", title: link.url },
    text,
  );
}

// --- components ---

export function renderComponent(ctx: RenderContext, component: Component): HTMLElement {
  const { doc } = ctx;
  switch (component.type) {
    case "card": {
      const card = el(doc, "section", "suite-card", { "data-tone": component.tone });
      if (component.title !== undefined) {
        card.append(el(doc, "h3", "suite-card-title", {}, component.title));
      }
      card.append(...component.children.map((child) => renderComponent(ctx, child)));
      return card;
    }
    case "row":
    case "stack": {
      const tag = component.type === "row" ? "suite-row" : "suite-stack";
      const box = el(doc, "div", tag, { "data-gap": component.gap });
      box.append(...component.children.map((child) => renderComponent(ctx, child)));
      return box;
    }
    case "text":
      return el(
        doc,
        "div",
        "suite-text",
        { "data-role": component.role, "data-tone": component.tone },
        ...renderBlocks(ctx, component.blocks),
      );
    case "image":
      return renderImage(ctx, component);
    case "file":
      return renderFile(doc, component);
    case "code":
      return renderCodeBlock(doc, component, "suite-code");
    case "diff":
      return renderDiff(doc, component);
    case "table":
      return renderTable(doc, component);
    case "kv": {
      const list = el(doc, "dl", "suite-kv");
      for (const { key, value } of component.items) {
        list.append(el(doc, "dt", "", {}, key), el(doc, "dd", "", {}, value));
      }
      return list;
    }
    case "progress":
      return renderProgress(doc, component.value, component.label);
    case "button":
      return renderButton(ctx, component);
    case "link":
      return renderLink(doc, component);
  }
}

// --- items ---

export interface RenderedItem {
  el: HTMLElement;
  /** The item's buttons in document order; `data-button` on each rendered button is its index. */
  buttons: ButtonComponent[];
  /** The element holding the item's component (the header, if any, is its sibling). */
  body: HTMLElement;
}

export function renderItem(doc: Document, item: ViewItem): RenderedItem {
  const ctx: RenderContext = { doc, buttons: [] };
  const body = el(doc, "div", "suite-item-body", {}, renderComponent(ctx, item.component));
  const attrs: Attrs = { "data-ref": item.ref, "data-kind": item.kind, "data-state": item.state };
  let root: HTMLElement;
  switch (item.kind) {
    case "user": {
      root = el(doc, "article", "suite-item", attrs);
      const header = item.label === undefined || item.label === "" ? "»" : `» ${item.label}`;
      root.append(el(doc, "header", "suite-item-header", {}, header));
      break;
    }
    case "tool": {
      root = el(doc, "article", "suite-item", attrs);
      const header = el(doc, "header", "suite-item-header");
      if (item.state !== undefined) {
        header.append(
          el(
            doc,
            "span",
            "suite-state",
            { "data-state": item.state, role: "img", "aria-label": item.state },
            STATE_ICONS[item.state],
          ),
          " ",
        );
      }
      if (item.label !== undefined) header.append(el(doc, "span", "suite-label", {}, item.label));
      root.append(header);
      break;
    }
    case "thought":
      root = el(
        doc,
        "details",
        "suite-item",
        attrs,
        el(doc, "summary", "suite-item-header", {}, "… thinking"),
      );
      break;
    case "agent":
    case "notice":
      root = el(doc, "article", "suite-item", attrs);
      break;
  }
  root.append(body);
  if (ctx.buttons.length > 0) root.setAttribute("tabindex", "0");
  return { el: root, buttons: ctx.buttons, body };
}
