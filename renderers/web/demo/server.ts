// The demo's HTTP side: serves the page with the daemon's address filled in (read from
// daemon.json here, since a browser can't read files), and the bundled browser code.
import type { DaemonAddress } from "@neoworks-dev/neo-sdk";
import { daemonJsonAddress } from "@neoworks-dev/neo-sdk";

export interface DemoServerOptions {
  /** Port on 127.0.0.1; 0 picks a free one. */
  port?: number;
  /** Where to look for the daemon (NEO_SOCKET etc.); defaults to process.env. */
  env?: Record<string, string | undefined>;
  /** The bundled page script, if already built with buildDemoScript (default: build it now). */
  script?: string;
}

export interface DemoServer {
  url: string;
  port: number;
  stop(): Promise<void>;
}

const PAGE_ENTRY = new URL("./page.ts", import.meta.url).pathname;

/** JSON that is safe inside a <script> element. */
function scriptJson(value: unknown): string {
  return JSON.stringify(value).replace(/</g, "\\u003c");
}

function html(address: DaemonAddress | undefined, problem: string): string {
  const config = address ? scriptJson({ url: address.url, token: address.token }) : "null";
  const note = address ? "connecting…" : problem;
  return `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>neo chat</title>
<style>
  html, body { height: 100%; margin: 0; background: #16181d; }
  body { display: flex; flex-direction: column; font: 14px system-ui, sans-serif; }
  #chat { flex: 1; min-height: 0; }
  #form { display: flex; gap: 8px; padding: 8px; background: #1f2229; }
  #input { flex: 1; padding: 8px; border: 1px solid #343844; border-radius: 6px; color: #e6e6e6; background: #16181d; font: inherit; }
  #send { padding: 8px 16px; border: 0; border-radius: 6px; color: #16181d; background: #7aa2f7; font: inherit; cursor: pointer; }
  #status { padding: 4px 8px; color: #8a8f9c; background: #1f2229; font-size: 12px; }
</style>
</head>
<body>
<main id="chat" aria-label="chat"></main>
<form id="form"><input id="input" autocomplete="off" placeholder="Say something" aria-label="message" autofocus><button id="send" type="submit">Send</button></form>
<div id="status" role="status">${note}</div>
<script type="application/json" id="neo-config">${config}</script>
<script type="module" src="/app.js"></script>
</body>
</html>
`;
}

/** Hosts a browser may use for this server; anything else is a DNS-rebinding attempt. */
function allowedHost(host: string | null, port: number): boolean {
  return host === `127.0.0.1:${port}` || host === `localhost:${port}`;
}

/** Bundles the page's browser code (page.ts, the browser SDK and the renderer) into one script. */
export async function buildDemoScript(): Promise<string> {
  const build = await Bun.build({ entrypoints: [PAGE_ENTRY], target: "browser", minify: false });
  const bundle = build.outputs[0];
  if (!build.success || !bundle) {
    throw new Error(`could not build the demo page: ${build.logs.map(String).join("\n")}`);
  }
  return bundle.text();
}

export async function startDemoServer(options: DemoServerOptions = {}): Promise<DemoServer> {
  const script = options.script ?? (await buildDemoScript());

  const server = Bun.serve({
    hostname: "127.0.0.1",
    port: options.port ?? 0,
    async fetch(req) {
      // The page carries the daemon's token: only answer requests meant for this address.
      if (!allowedHost(req.headers.get("host"), server.port ?? 0)) {
        return new Response("forbidden", { status: 403 });
      }
      const path = new URL(req.url).pathname;
      if (path === "/app.js") {
        return new Response(script, {
          headers: { "content-type": "text/javascript; charset=utf-8" },
        });
      }
      if (path !== "/" && path !== "/daemon") return new Response("not found", { status: 404 });
      let address: DaemonAddress;
      try {
        address = await daemonJsonAddress(options.env);
      } catch (err) {
        const why = err instanceof Error ? err.message : String(err);
        if (path === "/daemon") return new Response(why, { status: 503 });
        return new Response(html(undefined, why), {
          status: 503,
          headers: { "content-type": "text/html; charset=utf-8" },
        });
      }
      if (path === "/daemon")
        return Response.json(address, { headers: { "cache-control": "no-store" } });
      return new Response(html(address, ""), {
        headers: { "content-type": "text/html; charset=utf-8", "cache-control": "no-store" },
      });
    },
  });
  return {
    url: `http://127.0.0.1:${server.port}`,
    port: server.port ?? 0,
    stop: () => server.stop(true),
  };
}
