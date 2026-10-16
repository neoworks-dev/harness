// What the page's server and the page (browser code) exchange. Plain JSON, no node imports.

export interface PageImage {
  /** Absolute http(s) URL. */
  src: string;
  alt: string;
}

/** A loaded page as the page UI shows it. */
export interface PageView {
  url: string;
  title: string;
  /** Readable text, up to MAX_VIEW_TEXT characters. */
  text: string;
  images: PageImage[];
  /** How many links the page has (their list stays on the server). */
  links: number;
}

/** What a download produced: the asset plus where it came from. */
export interface DownloadView {
  handle: string;
  name: string;
  mime: string;
  size: number;
  /** The URL it was fetched from (after redirects). */
  url: string;
}

/** Pushed from the server to the page over its WebSocket. */
export type ServerEvent =
  { type: "page"; page: PageView | null } | { type: "download"; download: DownloadView };

export const MAX_VIEW_TEXT = 50_000;
