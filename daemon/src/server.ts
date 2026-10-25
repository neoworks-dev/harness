// Serves the hub as WebSocket at /bus on two listeners:
// - a Unix socket (mode 0600) for local native clients like the Go CLI;
// - 127.0.0.1 TCP with a per-start bearer token, for runtimes without Unix-socket WebSockets
//   (Bun, browsers) and for MCP clients (/mcp, the gateway). Clients find the URL and token in
//   daemon.json next to the socket (mode 0600).
import { timingSafeEqual } from "node:crypto";
import { chmodSync, existsSync, mkdirSync, unlinkSync, writeFileSync } from "node:fs";
import { dirname } from "node:path";
import type { Server, ServerWebSocket, WebSocketHandler } from "bun";
import type { AssetStore } from "./assets.ts";
import type { ClientHandle, Hub } from "./hub.ts";
import type { DaemonPaths } from "./paths.ts";

interface SocketData {
  handle?: ClientHandle;
}

export interface DaemonInfoFile {
  pid: number;
  protocol: number;
  socket: string;
  /** Base URL of the TCP listener: `${url}/bus?token=…` (WebSocket) and `${url}/mcp`. */
  url: string;
  token: string;
}

export interface ServerOptions {
  paths: Pick<DaemonPaths, "socket" | "info">;
  /** TCP port on 127.0.0.1; 0 picks a free one. */
  port: number;
  protocol: number;
  /** Handles /mcp on the TCP listener (already authenticated). */
  mcp?: (req: Request) => Promise<Response>;
  /** Serves /assets on the TCP listener: GET /assets/<id>, POST /assets (raw bytes). */
  assets?: AssetStore;
}

export interface BusServer {
  info: DaemonInfoFile;
  stop(): Promise<void>;
}

/** True if something answers on the socket (another daemon is running). */
async function isLive(socket: string): Promise<boolean> {
  try {
    await fetch("http://localhost/", { unix: socket, signal: AbortSignal.timeout(1000) });
    return true;
  } catch {
    return false;
  }
}

function sameToken(given: string | null, token: string): boolean {
  if (given === null || given.length !== token.length) return false;
  return timingSafeEqual(Buffer.from(given), Buffer.from(token));
}

function bearer(req: Request): string | null {
  const header = req.headers.get("authorization");
  if (header?.startsWith("Bearer ")) return header.slice("Bearer ".length);
  return null;
}

function websocketHandler(hub: Hub): WebSocketHandler<SocketData> {
  return {
    data: {} as SocketData,
    maxPayloadLength: 16 * 1024 * 1024,
    open(ws: ServerWebSocket<SocketData>) {
      ws.data.handle = hub.connect({ send: (frame) => ws.send(frame) });
    },
    message(ws, message) {
      ws.data.handle?.receive(typeof message === "string" ? message : message.toString());
    },
    close(ws) {
      ws.data.handle?.closed();
    },
  };
}

function upgrade(req: Request, srv: Server<SocketData>): Response | undefined {
  if (srv.upgrade(req, { data: {} })) return undefined;
  return new Response("expected a WebSocket upgrade", { status: 426 });
}

async function serveAssets(req: Request, url: URL, assets: AssetStore): Promise<Response> {
  if (req.method === "POST" && url.pathname === "/assets") {
    const bytes = new Uint8Array(await req.arrayBuffer());
    const params: Parameters<AssetStore["put"]>[0] = { bytes };
    for (const key of ["name", "mime", "app"] as const) {
      const value = url.searchParams.get(key);
      if (value !== null) params[key] = value;
    }
    try {
      return Response.json(assets.put(params));
    } catch (err) {
      return new Response(String(err), { status: 400 });
    }
  }
  const id = url.pathname.slice("/assets/".length);
  const asset = req.method === "GET" ? assets.get(id) : undefined;
  if (!asset) return new Response("not found", { status: 404 });
  return new Response(Bun.file(asset.path), {
    headers: {
      "content-type": asset.mime,
      "cache-control": "private, max-age=31536000, immutable",
      "x-content-type-options": "nosniff",
      // An SVG or HTML asset must not run script on the daemon's origin.
      "content-security-policy": "sandbox; default-src 'none'; style-src 'unsafe-inline'",
    },
  });
}

export async function startServer(hub: Hub, options: ServerOptions): Promise<BusServer> {
  const { socket, info: infoPath } = options.paths;
  mkdirSync(dirname(socket), { recursive: true, mode: 0o700 });
  if (existsSync(socket)) {
    if (await isLive(socket)) throw new Error(`another daemon is already listening on ${socket}`);
    unlinkSync(socket); // stale, left by a crash
  }

  const unix = Bun.serve<SocketData>({
    unix: socket,
    fetch(req, srv) {
      if (new URL(req.url).pathname === "/bus") return upgrade(req, srv);
      return new Response("neo daemon\n");
    },
    websocket: websocketHandler(hub),
  });
  chmodSync(socket, 0o600);

  const token = Buffer.from(crypto.getRandomValues(new Uint8Array(32))).toString("hex");
  const tcp = Bun.serve<SocketData>({
    hostname: "127.0.0.1",
    port: options.port,
    idleTimeout: 0, // MCP streams and WebSockets stay open
    async fetch(req, srv) {
      const url = new URL(req.url);
      if (url.pathname === "/bus") {
        // Browsers can't set headers on WebSockets, so the token may come as a query parameter.
        if (!sameToken(bearer(req) ?? url.searchParams.get("token"), token)) {
          return new Response("unauthorized", { status: 401 });
        }
        return upgrade(req, srv);
      }
      if (url.pathname.startsWith("/assets") && options.assets) {
        // GET may carry the token as a query parameter so <img src> works in browsers.
        const given = bearer(req) ?? (req.method === "GET" ? url.searchParams.get("token") : null);
        if (!sameToken(given, token)) return new Response("unauthorized", { status: 401 });
        return serveAssets(req, url, options.assets);
      }
      if (url.pathname === "/mcp" && options.mcp) {
        if (!sameToken(bearer(req), token)) return new Response("unauthorized", { status: 401 });
        return options.mcp(req);
      }
      return new Response("not found", { status: 404 });
    },
    websocket: websocketHandler(hub),
  });

  const info: DaemonInfoFile = {
    pid: process.pid,
    protocol: options.protocol,
    socket,
    url: `http://127.0.0.1:${tcp.port}`,
    token,
  };
  writeFileSync(infoPath, `${JSON.stringify(info, null, 2)}\n`, { mode: 0o600 });

  return {
    info,
    async stop() {
      await Promise.all([unix.stop(true), tcp.stop(true)]);
      for (const path of [socket, infoPath]) if (existsSync(path)) unlinkSync(path);
    },
  };
}
