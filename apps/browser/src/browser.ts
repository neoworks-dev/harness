// The browser's brain, without any UI or bus: navigate, read, list images, download. The tools,
// the page's HTTP endpoints and the address bar all call these.
import type { Asset, AssetPut } from "@suite/protocol/types";
import { extractPage, truncate, type PageLink } from "./extract.ts";
import {
  addressToUrl,
  decodeBody,
  fetchUrl,
  MAX_DOWNLOAD_BYTES,
  MAX_PAGE_BYTES,
  nameFromUrl,
  parseHttpUrl,
  type Fetched,
} from "./fetch.ts";
import {
  MAX_VIEW_TEXT,
  type DownloadView,
  type PageImage,
  type PageView,
  type ServerEvent,
} from "./types.ts";

/** A page that was loaded. */
export interface LoadedPage {
  url: string;
  title: string;
  text: string;
  images: PageImage[];
  links: PageLink[];
}

export interface BrowserDeps {
  /** Stores bytes in the daemon's asset store. */
  putAsset(asset: AssetPut): Promise<Asset>;
}

const GENERIC_TYPES = new Set([
  "",
  "application/octet-stream",
  "binary/octet-stream",
  "text/plain",
]);
const HTML_TYPES = new Set(["", "text/html", "application/xhtml+xml"]);

function isTextType(type: string): boolean {
  return (
    type.startsWith("text/") ||
    type === "application/json" ||
    type === "application/xml" ||
    type.endsWith("+json") ||
    type.endsWith("+xml")
  );
}

/** The page as the UI shows it. */
export function viewOf(page: LoadedPage): PageView {
  return {
    url: page.url,
    title: page.title,
    text: truncate(page.text, MAX_VIEW_TEXT),
    images: page.images,
    links: page.links.length,
  };
}

function cutNote(fetched: Fetched): string {
  if (!fetched.truncated) return "";
  return `\n\n[cut at ${MAX_PAGE_BYTES} bytes]`;
}

async function load(fetched: Fetched): Promise<LoadedPage> {
  const { url, type } = fetched;
  const name = nameFromUrl(url, type);
  if (HTML_TYPES.has(type)) {
    const page = await extractPage(decodeBody(fetched), url);
    const note = cutNote(fetched);
    return {
      url,
      title: page.title,
      text: page.text + note,
      images: page.images,
      links: page.links,
    };
  }
  if (type.startsWith("image/")) {
    return { url, title: name, text: "", images: [{ src: url, alt: name }], links: [] };
  }
  if (isTextType(type)) {
    const note = cutNote(fetched);
    return { url, title: name, text: decodeBody(fetched).trim() + note, images: [], links: [] };
  }
  throw new Error(`can't show ${type} (${url}); use download to keep the file`);
}

/** Why read and images have nothing to say. */
export const NO_PAGE = "no page is loaded; call navigate first";

export class Browser {
  private current: LoadedPage | undefined;
  private navigations = 0;
  private readonly listeners = new Set<(event: ServerEvent) => void>();

  constructor(private readonly deps: BrowserDeps) {}

  /** The page being shown; undefined before the first navigation. */
  get page(): LoadedPage | undefined {
    return this.current;
  }

  /** Called with every page change and every download, whoever asked for it. */
  onEvent(cb: (event: ServerEvent) => void): () => void {
    this.listeners.add(cb);
    return () => this.listeners.delete(cb);
  }

  private emit(event: ServerEvent): void {
    for (const cb of [...this.listeners]) cb(event);
  }

  /**
   * Loads `input`: an http(s) URL, a bare host (https assumed) or something relative to the
   * current page. Rejects with a readable message on failures. If another navigation started
   * meanwhile, this one still returns its page but the later one wins the screen.
   */
  async navigate(input: string): Promise<LoadedPage> {
    const text = input.trim();
    let url: URL;
    if (this.current && /^([/.?#])/.test(text)) url = parseHttpUrl(text, this.current.url);
    else url = addressToUrl(text);
    const mine = ++this.navigations;
    const fetched = await fetchUrl(url, {
      maxBytes: MAX_PAGE_BYTES,
      accept: "text/html,application/xhtml+xml,text/*;q=0.8,image/*;q=0.5,*/*;q=0.1",
    });
    const page = await load(fetched);
    if (mine === this.navigations) {
      this.current = page;
      this.emit({ type: "page", page: viewOf(page) });
    }
    return page;
  }

  /** Fetches `input` (relative to the current page) into the asset store. */
  async download(input: string): Promise<DownloadView & { path: string }> {
    const url = parseHttpUrl(input, this.current?.url);
    const fetched = await fetchUrl(url, { maxBytes: MAX_DOWNLOAD_BYTES, strict: true });
    const put: AssetPut = {
      data: Buffer.from(fetched.bytes).toString("base64"),
      name: nameFromUrl(fetched.url, fetched.type),
    };
    // Servers often label everything octet-stream or text/plain; the store sniffs better then.
    if (!GENERIC_TYPES.has(fetched.type)) put.mime = fetched.type;
    const asset = await this.deps.putAsset(put);
    const download = {
      handle: asset.handle,
      name: asset.name,
      mime: asset.mime,
      size: asset.size,
      url: fetched.url,
      path: asset.path,
    };
    this.emit({ type: "download", download });
    return download;
  }
}
