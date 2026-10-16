// The page's side of its own server: JSON calls for the address bar and downloads, and the
// WebSocket that pushes page changes. Uses only fetch and WebSocket, so tests run it under Bun.
import type { DaemonAddress } from "@suite/sdk/browser";
import type { DownloadView, PageView, ServerEvent } from "../src/types.ts";

const TOKEN_HEADER = "x-browser-token";
const INITIAL_BACKOFF_MS = 250;
const MAX_BACKOFF_MS = 5000;

export interface PageApi {
  navigate(url: string): Promise<PageView>;
  download(url: string): Promise<DownloadView>;
  /** The daemon's current address (the server re-reads daemon.json). */
  daemon(): Promise<DaemonAddress>;
  /** Page changes and downloads, reconnecting if the socket drops; returns the unsubscribe. */
  onEvent(cb: (event: ServerEvent) => void): () => void;
}

function parseEvent(frame: string): ServerEvent | undefined {
  const value: unknown = JSON.parse(frame);
  if (typeof value !== "object" || value === null) return undefined;
  const type = (value as { type?: unknown }).type;
  if (type === "page" || type === "download") return value as ServerEvent;
  return undefined;
}

/** `base` is the page's origin (http://127.0.0.1:PORT). */
export function createApi(base: string, token: string): PageApi {
  async function post<T>(path: string, url: string): Promise<T> {
    const response = await fetch(`${base}${path}`, {
      method: "POST",
      headers: { "content-type": "application/json", [TOKEN_HEADER]: token },
      body: JSON.stringify({ url }),
    });
    const body: unknown = await response.json().catch(() => ({}));
    if (!response.ok) {
      const error = (body as { error?: unknown }).error;
      throw new Error(typeof error === "string" ? error : `request failed (${response.status})`);
    }
    return body as T;
  }

  return {
    navigate: (url) => post<PageView>("/api/navigate", url),
    download: (url) => post<DownloadView>("/api/download", url),
    async daemon() {
      const response = await fetch(`${base}/daemon`, { headers: { [TOKEN_HEADER]: token } });
      if (!response.ok) throw new Error(`the daemon is not running (${response.status})`);
      const address: DaemonAddress = await response.json();
      return address;
    },
    onEvent(cb) {
      let closed = false;
      let ws: WebSocket | undefined;
      let backoff = INITIAL_BACKOFF_MS;
      let timer: ReturnType<typeof setTimeout> | undefined;

      const connect = (): void => {
        const socket = new WebSocket(`${base.replace(/^http/, "ws")}/ws?token=${token}`);
        ws = socket;
        socket.onopen = () => {
          backoff = INITIAL_BACKOFF_MS;
        };
        socket.onmessage = (message) => {
          const event = parseEvent(String(message.data));
          if (event) cb(event);
        };
        socket.onclose = () => {
          if (closed) return;
          timer = setTimeout(connect, backoff);
          backoff = Math.min(backoff * 2, MAX_BACKOFF_MS);
        };
      };
      connect();
      return () => {
        closed = true;
        clearTimeout(timer);
        ws?.close();
      };
    },
  };
}
