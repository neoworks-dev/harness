// Rough page extraction with Bun's HTMLRewriter: title, readable text, links and images. No DOM,
// no layout, no JavaScript: whatever the HTML says before scripts run.
import { MAX_VIEW_TEXT, type PageImage } from "./types.ts";

export interface PageLink {
  href: string;
  text: string;
}

export interface ExtractedPage {
  title: string;
  /** Readable text: one line per block, blank lines squeezed. */
  text: string;
  images: PageImage[];
  links: PageLink[];
}

export const MAX_IMAGES = 100;
export const MAX_LINKS = 300;

const SKIPPED = "script, style, noscript, template, svg, title";
const BLOCKS =
  "p, div, br, hr, h1, h2, h3, h4, h5, h6, li, ul, ol, tr, table, section, article, header, footer, nav, main, aside, pre, blockquote, figure, figcaption, form, dt, dd";
const ENTITIES: Record<string, string> = {
  amp: "&",
  lt: "<",
  gt: ">",
  quot: '"',
  apos: "'",
  nbsp: " ",
};

/** The handful of entities pages actually use; anything unknown stays as written. */
export function decodeEntities(text: string): string {
  return text.replace(/&(#x[0-9a-f]+|#\d+|[a-z]+);/gi, (whole, name: string) => {
    const lower = name.toLowerCase();
    if (lower.startsWith("#x") || lower.startsWith("#")) {
      const code = lower.startsWith("#x") ? parseInt(lower.slice(2), 16) : Number(lower.slice(1));
      if (code > 0 && code <= 0x10ffff) return String.fromCodePoint(code);
      return whole;
    }
    return ENTITIES[lower] ?? whole;
  });
}

/** An absolute http(s) URL for `ref` relative to `base`, else undefined. */
function resolveHttp(ref: string | null, base: string): string | undefined {
  if (ref === null || ref.trim() === "") return undefined;
  try {
    const url = new URL(ref.trim(), base);
    if (url.protocol !== "http:" && url.protocol !== "https:") return undefined;
    url.hash = "";
    return url.href;
  } catch {
    return undefined;
  }
}

/** The biggest candidate of a srcset ("a.png 1x, b.png 2x" or "a.png 480w, b.png 800w"). */
export function bestSrcset(srcset: string): string | undefined {
  let best: string | undefined;
  let bestWeight = -1;
  for (const candidate of srcset.split(/,\s+|,$/)) {
    const [url, descriptor] = candidate.trim().split(/\s+/);
    if (url === undefined || url === "") continue;
    const weight = descriptor === undefined ? 1 : parseFloat(descriptor);
    if (weight > bestWeight) {
      best = url;
      bestWeight = Number.isNaN(weight) ? 1 : weight;
    }
  }
  return best;
}

function tidy(raw: string): string {
  const lines = raw.split("\n").map((line) => line.trim());
  const out: string[] = [];
  for (const line of lines) {
    if (line === "" && (out.length === 0 || out.at(-1) === "")) continue;
    out.push(line);
  }
  return out.join("\n").trim();
}

interface RawImage {
  ref: string;
  alt: string;
  /** Lower first: page images before icons. */
  rank: number;
}

/**
 * Reads `html` (the page at `pageUrl`). Relative URLs are resolved against `<base href>` if there
 * is one, else the page.
 */
export async function extractPage(html: string, pageUrl: string): Promise<ExtractedPage> {
  const pieces: string[] = [];
  const titleParts: string[] = [];
  const rawImages: RawImage[] = [];
  const rawLinks: PageLink[] = [];
  let metaTitle = "";
  let baseHref: string | null = null;
  let skip = 0;
  let link: PageLink | undefined;

  const addImage = (ref: string | null, alt: string, rank: number): void => {
    if (ref !== null) rawImages.push({ ref, alt: decodeEntities(alt), rank });
  };

  const rewriter = new HTMLRewriter()
    .on(SKIPPED, {
      element(el) {
        skip += 1;
        el.onEndTag(() => {
          skip -= 1;
        });
      },
    })
    .on("title", {
      text(chunk) {
        titleParts.push(chunk.text);
      },
    })
    .on(BLOCKS, {
      element(el) {
        pieces.push("\n");
        if (el.canHaveContent) {
          el.onEndTag(() => {
            pieces.push("\n");
          });
        }
      },
    })
    .on("base[href]", {
      element(el) {
        baseHref ??= el.getAttribute("href");
      },
    })
    .on("a[href]", {
      element(el) {
        const href = el.getAttribute("href");
        if (href === null) return;
        const entry: PageLink = { href, text: "" };
        rawLinks.push(entry);
        link = entry;
        el.onEndTag(() => {
          link = undefined;
        });
      },
    })
    .on("img", {
      element(el) {
        const width = Number(el.getAttribute("width"));
        const height = Number(el.getAttribute("height"));
        // Tracking pixels.
        if (width > 0 && width <= 2 && height > 0 && height <= 2) return;
        const alt = el.getAttribute("alt") ?? "";
        const srcset = el.getAttribute("srcset");
        const big = srcset === null ? undefined : bestSrcset(srcset);
        if (big !== undefined) addImage(big, alt, 1);
        addImage(el.getAttribute("src") ?? el.getAttribute("data-src"), alt, 1);
      },
    })
    .on("picture source[srcset]", {
      element(el) {
        const srcset = el.getAttribute("srcset");
        const big = srcset === null ? undefined : bestSrcset(srcset);
        if (big !== undefined) addImage(big, "", 1);
      },
    })
    .on("meta", {
      element(el) {
        const key = el.getAttribute("property") || el.getAttribute("name") || "";
        const content = el.getAttribute("content");
        if (key === "og:image" || key === "twitter:image") addImage(content, "og:image", 0);
        if (key === "og:title" && content !== null) metaTitle = content;
      },
    })
    .on("link[rel][href]", {
      element(el) {
        const rel = (el.getAttribute("rel") ?? "").toLowerCase().split(/\s+/);
        if (rel.includes("icon") || rel.includes("apple-touch-icon")) {
          addImage(el.getAttribute("href"), "icon", 2);
        }
      },
    })
    .onDocument({
      text(chunk) {
        if (skip > 0) return;
        const text = chunk.text.replace(/\s+/g, " ");
        pieces.push(text);
        if (link) link.text += text;
      },
    });
  await rewriter.transform(new Response(html)).text();

  const base = resolveHttp(baseHref, pageUrl) ?? pageUrl;
  const seen = new Set<string>();
  const images: PageImage[] = [];
  for (const raw of rawImages.toSorted((a, b) => a.rank - b.rank)) {
    const src = resolveHttp(decodeEntities(raw.ref), base);
    if (src === undefined || seen.has(src)) continue;
    seen.add(src);
    images.push({ src, alt: raw.alt.trim() });
    if (images.length >= MAX_IMAGES) break;
  }
  const links: PageLink[] = [];
  for (const raw of rawLinks) {
    const href = resolveHttp(decodeEntities(raw.href), base);
    if (href === undefined) continue;
    links.push({ href, text: decodeEntities(raw.text).trim() });
    if (links.length >= MAX_LINKS) break;
  }
  const title = tidy(decodeEntities(titleParts.join("").replace(/\s+/g, " ")));
  return {
    title: title === "" ? decodeEntities(metaTitle).trim() : title,
    text: tidy(decodeEntities(pieces.join(""))),
    images,
    links,
  };
}

/** `text` cut to `max` characters, with an ellipsis when something was dropped. */
export function truncate(text: string, max: number = MAX_VIEW_TEXT): string {
  if (text.length <= max) return text;
  return `${text.slice(0, max).trimEnd()}…`;
}
