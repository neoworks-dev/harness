// A stand-in for the daemon that tests steer by hand: welcome + theme on hello, the session list,
// a snapshot per view.subscribe (the session's own, or an agent's), an ack for the rest. Tests
// push patches and broadcasts, drop connections, and make it stay silent about chosen topics.
import type { Envelope, SessionInfo, Theme, ViewItem, ViewSnapshot } from "@suite/protocol/types";
import type { Server, ServerWebSocket } from "bun";

export const THEME: Theme = {
  name: "test",
  colors: {
    fg: "#fff",
    bg: "#000",
    surface: "#111",
    border: "#222",
    accent: "#333",
    muted: "#444",
    success: "#555",
    danger: "#666",
  },
  spacing: { s: 1, m: 2, l: 3 },
  text: {
    title: { size: 15, weight: 600, mono: false },
    body: { size: 14, weight: 400, mono: false },
    code: { size: 13, weight: 400, mono: true },
  },
};

export class FakeDaemon {
  readonly token = "secret";
  readonly received: Envelope[] = [];
  /** The session's own view. */
  items: ViewItem[] = [];
  /** The views of agents' work, by agent name. */
  agentItems: Record<string, ViewItem[]> = {};
  seq = 1;
  sessions: SessionInfo[] = [];
  /** Topics that are received but never answered. */
  readonly silentOn = new Set<string>();
  /** Topics that are answered with a bad_request error. */
  readonly refusing = new Set<string>();
  private readonly sockets = new Set<ServerWebSocket<undefined>>();
  private readonly server: Server<undefined>;

  constructor(port = 0) {
    this.server = Bun.serve<undefined>({
      hostname: "127.0.0.1",
      port,
      fetch: (req, srv) => {
        const url = new URL(req.url);
        if (url.pathname !== "/bus" || url.searchParams.get("token") !== this.token) {
          return new Response("unauthorized", { status: 401 });
        }
        if (srv.upgrade(req, { data: undefined })) return undefined;
        return new Response("upgrade required", { status: 426 });
      },
      websocket: {
        open: (ws) => {
          this.sockets.add(ws);
        },
        close: (ws) => {
          this.sockets.delete(ws);
        },
        message: (ws, message) => this.handle(ws, JSON.parse(String(message))),
      },
    });
  }

  get url(): string {
    return `http://127.0.0.1:${this.server.port}`;
  }

  get port(): number {
    return this.server.port ?? 0;
  }

  topics(): string[] {
    return this.received.map((e) => e.topic);
  }

  broadcast(envelope: Envelope): void {
    for (const ws of this.sockets) ws.send(JSON.stringify(envelope));
  }

  /** Cuts every connection, like a daemon crash. */
  dropConnections(): void {
    for (const ws of this.sockets) ws.terminate();
  }

  /** Not awaited: Bun's stop() promise can hang once a socket was closed from the server side. */
  stop(): void {
    void this.server.stop(true);
  }

  private snapshot(session: string, agent?: string): ViewSnapshot {
    if (agent === undefined) return { session, seq: this.seq, items: this.items };
    return { session, agent, seq: this.seq, items: this.agentItems[agent] ?? [] };
  }

  private handle(ws: ServerWebSocket<undefined>, msg: Envelope): void {
    this.received.push(msg);
    if (this.silentOn.has(msg.topic)) return;
    if (this.refusing.has(msg.topic) && "id" in msg) {
      ws.send(
        JSON.stringify({
          topic: "bus.error",
          id: msg.id,
          payload: { code: "bad_request", message: `${msg.topic} refused` },
        }),
      );
      return;
    }
    const send = (envelope: Envelope): void => {
      ws.send(JSON.stringify(envelope));
    };
    switch (msg.topic) {
      case "bus.hello":
        send({
          topic: "bus.welcome",
          payload: { protocol: 1, daemonVersion: "0", pid: 1, startedAt: 0 },
        });
        send({ topic: "theme", payload: THEME });
        return;
      case "session.list":
        send({ topic: "session.list_result", id: msg.id, payload: { sessions: this.sessions } });
        return;
      case "connectors.list":
        send({ topic: "connectors.list_result", id: msg.id, payload: { connectors: [] } });
        return;
      case "view.subscribe":
        if (msg.payload.session === "missing") {
          send({
            topic: "bus.error",
            id: msg.id,
            payload: { code: "not_found", message: "no such session" },
          });
          return;
        }
        send({
          topic: "view.snapshot",
          id: msg.id,
          payload: this.snapshot(msg.payload.session, msg.payload.agent),
        });
        return;
      case "view.unsubscribe":
      case "ui.action":
      case "session.unsubscribe":
        send({ topic: "bus.ack", id: msg.id });
        return;
      case "app.presence":
      case "app.context":
        if (msg.id !== undefined) send({ topic: "bus.ack", id: msg.id });
        return;
      default:
    }
  }
}
