// Serves the hub as WebSocket (path /bus) over a Unix socket.
import { chmodSync, existsSync, mkdirSync, unlinkSync } from "node:fs";
import { dirname } from "node:path";
import type { ClientHandle, Hub } from "./hub.ts";

interface SocketData {
  handle?: ClientHandle;
}

export interface BusServer {
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

export async function startServer(hub: Hub, socket: string): Promise<BusServer> {
  mkdirSync(dirname(socket), { recursive: true, mode: 0o700 });
  if (existsSync(socket)) {
    if (await isLive(socket)) throw new Error(`another daemon is already listening on ${socket}`);
    unlinkSync(socket); // stale, left by a crash
  }

  const server = Bun.serve({
    unix: socket,
    fetch(req, srv) {
      if (new URL(req.url).pathname === "/bus") {
        if (srv.upgrade(req, { data: {} })) return undefined;
        return new Response("expected a WebSocket upgrade", { status: 426 });
      }
      return new Response("suite daemon\n");
    },
    websocket: {
      data: {} as SocketData,
      maxPayloadLength: 16 * 1024 * 1024,
      open(ws) {
        ws.data.handle = hub.connect({ send: (frame) => ws.send(frame) });
      },
      message(ws, message) {
        ws.data.handle?.receive(typeof message === "string" ? message : message.toString());
      },
      close(ws) {
        ws.data.handle?.closed();
      },
    },
  });
  // Only this user may talk to the daemon (see "Local security" in the design's open questions).
  chmodSync(socket, 0o600);

  return {
    async stop() {
      await server.stop(true);
      if (existsSync(socket)) unlinkSync(socket);
    },
  };
}
