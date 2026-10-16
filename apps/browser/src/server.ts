// The browser's own HTTP side on 127.0.0.1: the page (with the daemon's address and a per-start
// token in it), its bundled script, small JSON endpoints for the address bar and downloads, and a
// WebSocket that pushes page changes (including the ones the agent makes through the tools).
//
// The page carries secrets (the daemon token), so every request must be addressed to this
// server's own 127.0.0.1:PORT / localhost:PORT (DNS rebinding) and the endpoints need the token.
import { timingSafeEqual } from "node:crypto";
import { readDaemonEndpoint } from "@suite/sdk";
import { viewOf, type Browser } from "./browser.ts";
import type { ServerEvent } from "./types.ts";

export const TOKEN_HEADER = "x-browser-token";
const TOPIC = "events";
const MAX_BODY = 64 * 1024;
const PAGE_ENTRY = new URL("../page/page.ts", import.meta.url).pathname;

export interface BrowserServerOptions {
  browser: Browser;
  /** Port on 127.0.0.1; 0 picks a free one. */
  port?: number;
  /** Where to look for the daemon (SUITE_SOCKET etc.); defaults to process.env. */
  env?: Record<string, string | undefined>;
  /** The bundled page script, if already built with buildPageScript (default: build it now). */
  script?: string;
}

export interface BrowserServer {
  url: string;
  port: number;
  /** The token the page's requests carry. */
  token: string;
  stop(): Promise<void>;
}

/** Bundles the page's browser code (the page, the browser SDK and the chat renderer). */
export async function buildPageScript(): Promise<string> {
  const build = await Bun.build({ entrypoints: [PAGE_ENTRY], target: "browser", minify: false });
  const bundle = build.outputs[0];
  if (!build.success || !bundle) {
    throw new Error(`could not build the browser page: ${build.logs.map(String).join("\n")}`);
  }
  return bundle.text();
}

/** JSON that is safe inside a <script> element. */
function scriptJson(value: unknown): string {
  return JSON.stringify(value).replace(/</g, "\\u003c");
}

const CSP = [
  "default-src 'none'",
  "script-src 'self'",
  "style-src 'unsafe-inline'",
  // Page images come from anywhere on the web; chat images from the daemon.
  "img-src http: https: data:",
  "connect-src 'self' ws://127.0.0.1:* ws://localhost:*",
  "base-uri 'none'",
  "form-action 'none'",
].join("; ");

const STYLE = `
  * { box-sizing: border-box; }
  html, body { height: 100%; margin: 0; background: #16181d; color: #e6e6e6; }
  body { display: flex; font: 14px system-ui, sans-serif; }
  #browse { flex: 1; min-width: 0; display: flex; flex-direction: column; overflow: auto; }
  #address { display: flex; gap: 8px; padding: 8px; background: #1f2229; position: sticky; top: 0; }
  #url { flex: 1; padding: 8px; border: 1px solid #343844; border-radius: 6px; color: inherit; background: #16181d; font: inherit; }
  button { padding: 6px 12px; border: 0; border-radius: 6px; color: #16181d; background: #7aa2f7; font: inherit; cursor: pointer; }
  button:disabled { opacity: 0.5; cursor: default; }
  #title { margin: 16px 16px 4px; font-size: 20px; }
  #note { margin: 0 16px 8px; color: #8a8f9c; font-size: 12px; }
  #images { display: grid; grid-template-columns: repeat(auto-fill, minmax(180px, 1fr)); gap: 12px; padding: 8px 16px; }
  .tile { display: flex; flex-direction: column; gap: 6px; padding: 8px; border: 1px solid #343844; border-radius: 8px; background: #1f2229; min-width: 0; }
  .tile img { width: 100%; height: 120px; object-fit: contain; background: #16181d; border-radius: 4px; }
  .tile .alt { color: #8a8f9c; font-size: 12px; overflow-wrap: anywhere; }
  .tile .handle { color: #9ece6a; font: 12px ui-monospace, monospace; overflow-wrap: anywhere; }
  #text { margin: 0; padding: 8px 16px 24px; white-space: pre-wrap; overflow-wrap: anywhere; font: 13px/1.5 ui-monospace, monospace; color: #c8ccd4; }
  #pane { width: 380px; min-width: 280px; display: flex; flex-direction: column; border-left: 3px solid #343844; background: #16181d; }
  #pane.focused { border-left-color: #7aa2f7; box-shadow: inset 6px 0 12px -8px #7aa2f7; }
  #pane-title { padding: 8px; background: #1f2229; color: #8a8f9c; font-size: 12px; }
  #pane.focused #pane-title { color: #7aa2f7; }
  #chat { flex: 1; min-height: 0; }
  #chat-form { display: flex; gap: 8px; padding: 8px; background: #1f2229; }
  #chat-input { flex: 1; padding: 8px; border: 1px solid #343844; border-radius: 6px; color: inherit; background: #16181d; font: inherit; }
  #status { padding: 4px 8px; color: #8a8f9c; background: #1f2229; font-size: 12px; }
`;

function html(config: unknown): string {
  return `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="referrer" content="no-referrer">
<title>suite browser</title>
<style>${STYLE}</style>
</head>
<body>
<main id="browse">
  <form id="address"><input id="url" type="text" autocomplete="off" spellcheck="false" placeholder="Enter a URL" aria-label="address"><button type="submit">Go</button></form>
  <h1 id="title"></h1>
  <div id="note" role="status"></div>
  <section id="images" aria-label="images"></section>
  <pre id="text"></pre>
</main>
<aside id="pane" aria-label="agent chat">
  <div id="pane-title">agent</div>
  <div id="chat"></div>
  <form id="chat-form"><input id="chat-input" autocomplete="off" placeholder="Say something" aria-label="message"><button type="submit">Send</button></form>
  <div id="status" role="status">connecting…</div>
</aside>
<script type="application/json" id="browser-config">${scriptJson(config)}</script>
<script type="module" src="/app.js"></script>
</body>
</html>
`;
}

function sameToken(given: string | null, token: string): boolean {
  if (given === null || given.length !== token.length) return false;
  return timingSafeEqual(Buffer.from(given), Buffer.from(token));
}

function errorText(err: unknown): string {
  if (err instanceof Error) return err.message;
  return String(err);
}

/** The `url` of a JSON body `{url}`. */
async function urlOf(req: Request): Promise<string> {
  const length = Number(req.headers.get("content-length") ?? 0);
  if (length > MAX_BODY) throw new Error("request too large");
  const body: unknown = await req.json();
  const url = typeof body === "object" && body !== null ? (body as { url?: unknown }).url : null;
  if (typeof url !== "string" || url.trim() === "") throw new Error("url is required");
  return url;
}

export async function startBrowserServer(options: BrowserServerOptions): Promise<BrowserServer> {
  const { browser } = options;
  const script = options.script ?? (await buildPageScript());
  const token = Buffer.from(crypto.getRandomValues(new Uint8Array(24))).toString("hex");

  const hostOk = (host: string | null, port: number): boolean =>
    host === `127.0.0.1:${port}` || host === `localhost:${port}`;
  const originOk = (origin: string | null, port: number): boolean =>
    origin === null ||
    origin === `http://127.0.0.1:${port}` ||
    origin === `http://localhost:${port}`;

  const server = Bun.serve({
    hostname: "127.0.0.1",
    port: options.port ?? 0,
    idleTimeout: 0, // the page's WebSocket may sit quiet
    async fetch(req, srv) {
      const port = srv.port ?? 0;
      if (!hostOk(req.headers.get("host"), port)) return new Response("forbidden", { status: 403 });
      const url = new URL(req.url);
      const authorized = sameToken(req.headers.get(TOKEN_HEADER), token);

      if (req.method === "GET" && url.pathname === "/app.js") {
        return new Response(script, {
          headers: { "content-type": "text/javascript; charset=utf-8" },
        });
      }
      if (req.method === "GET" && url.pathname === "/ws") {
        const given = sameToken(url.searchParams.get("token"), token);
        if (!given || !originOk(req.headers.get("origin"), port)) {
          return new Response("forbidden", { status: 403 });
        }
        if (srv.upgrade(req, { data: undefined })) return undefined;
        return new Response("expected a WebSocket upgrade", { status: 426 });
      }
      if (req.method === "GET" && (url.pathname === "/" || url.pathname === "/daemon")) {
        let address: { url: string; token: string } | null = null;
        let problem = "";
        try {
          const endpoint = await readDaemonEndpoint(options.env);
          address = { url: endpoint.url, token: endpoint.token };
        } catch (err) {
          problem = errorText(err);
        }
        if (url.pathname === "/daemon") {
          if (!authorized) return new Response("forbidden", { status: 403 });
          if (!address) return new Response(problem, { status: 503 });
          return Response.json(address, { headers: { "cache-control": "no-store" } });
        }
        return new Response(html({ daemon: address, problem, token }), {
          headers: {
            "content-type": "text/html; charset=utf-8",
            "cache-control": "no-store",
            "content-security-policy": CSP,
          },
        });
      }
      if (
        req.method === "POST" &&
        (url.pathname === "/api/navigate" || url.pathname === "/api/download")
      ) {
        if (!authorized) return new Response("forbidden", { status: 403 });
        try {
          const target = await urlOf(req);
          if (url.pathname === "/api/navigate") {
            return Response.json(viewOf(await browser.navigate(target)));
          }
          return Response.json(await browser.download(target));
        } catch (err) {
          return Response.json({ error: errorText(err) }, { status: 400 });
        }
      }
      return new Response("not found", { status: 404 });
    },
    websocket: {
      open(ws) {
        ws.subscribe(TOPIC);
        const now = browser.page;
        if (now) {
          const event: ServerEvent = { type: "page", page: viewOf(now) };
          ws.send(JSON.stringify(event));
        }
      },
      message() {},
    },
  });

  const off = browser.onEvent((event) => {
    server.publish(TOPIC, JSON.stringify(event));
  });
  return {
    url: `http://127.0.0.1:${server.port}`,
    port: server.port ?? 0,
    token,
    async stop() {
      off();
      await server.stop(true);
    },
  };
}
