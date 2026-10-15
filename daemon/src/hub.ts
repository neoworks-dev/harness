// Bus hub: protocol handling for one daemon, independent of the transport (server.ts adapts it to
// WebSocket-over-Unix-socket; tests drive it directly).
import type { Envelope, ErrorCode, SessionEventRecord } from "@suite/protocol/types";
import { validateEnvelope } from "@suite/protocol/validate";
import type { Logger } from "./log.ts";
import { NotFoundError, type SessionManager } from "./sessions.ts";
import type { SessionStore } from "./store.ts";

export const PROTOCOL_VERSION = 1;

export interface Transport {
  send(frame: string): void;
}

export interface ClientHandle {
  receive(frame: string): void;
  closed(): void;
}

export interface DaemonInfo {
  version: string;
  pid: number;
  startedAt: number;
}

interface Client {
  transport: Transport;
  name?: string;
  subscriptions: Set<string>;
}

/** Best-effort id of an invalid frame, so the error can still be correlated. */
function frameId(parsed: unknown): string | undefined {
  if (typeof parsed !== "object" || parsed === null || !("id" in parsed)) return undefined;
  return typeof parsed.id === "string" ? parsed.id : undefined;
}

export class Hub {
  private readonly clients = new Set<Client>();

  constructor(
    private readonly store: SessionStore,
    private readonly sessions: SessionManager,
    private readonly info: DaemonInfo,
    private readonly log: Logger,
  ) {
    // Appends are synchronous and so is subscribe's replay, so a subscriber either got an event
    // in its replay or gets it here — never both, never neither.
    store.onAppend((record) => this.broadcast(record));
  }

  connect(transport: Transport): ClientHandle {
    const client: Client = { transport, subscriptions: new Set() };
    this.clients.add(client);
    return {
      receive: (frame) => this.receive(client, frame),
      closed: () => this.clients.delete(client),
    };
  }

  get clientCount(): number {
    return this.clients.size;
  }

  private broadcast(record: SessionEventRecord): void {
    for (const client of this.clients) {
      if (client.subscriptions.has(record.session)) {
        this.send(client, { topic: "session.events", payload: record });
      }
    }
  }

  private send(client: Client, envelope: Envelope): void {
    try {
      client.transport.send(JSON.stringify(envelope));
    } catch (err) {
      this.log.warn(`send to ${client.name ?? "client"} failed: ${String(err)}`);
    }
  }

  private fail(client: Client, id: string | undefined, code: ErrorCode, message: string): void {
    const envelope: Envelope = { topic: "bus.error", payload: { code, message } };
    if (id !== undefined) envelope.id = id;
    this.send(client, envelope);
  }

  private receive(client: Client, frame: string): void {
    let parsed: unknown;
    try {
      parsed = JSON.parse(frame);
    } catch {
      this.fail(client, undefined, "bad_request", "frame is not valid JSON");
      return;
    }
    const result = validateEnvelope(parsed);
    if (!result.ok) {
      this.fail(client, frameId(parsed), "bad_request", result.error);
      return;
    }
    const msg = result.value;
    if (client.name === undefined && msg.topic !== "bus.hello") {
      this.fail(client, "id" in msg ? msg.id : undefined, "bad_request", "send bus.hello first");
      return;
    }
    try {
      this.handle(client, msg);
    } catch (err) {
      const id = "id" in msg ? msg.id : undefined;
      if (err instanceof NotFoundError) this.fail(client, id, "not_found", err.message);
      else {
        this.log.error(`handling ${msg.topic}: ${String(err)}`);
        this.fail(client, id, "internal", String(err));
      }
    }
  }

  private handle(client: Client, msg: Envelope): void {
    switch (msg.topic) {
      case "bus.hello":
        if (msg.payload.protocol !== PROTOCOL_VERSION) {
          const message = `daemon speaks protocol ${PROTOCOL_VERSION}, client sent ${msg.payload.protocol}`;
          this.fail(client, undefined, "unsupported_protocol", message);
          return;
        }
        client.name = msg.payload.client;
        this.send(client, {
          topic: "bus.welcome",
          payload: {
            protocol: PROTOCOL_VERSION,
            daemonVersion: this.info.version,
            pid: this.info.pid,
            startedAt: this.info.startedAt,
          },
        });
        return;
      case "session.create":
        this.send(client, {
          topic: "session.info",
          id: msg.id,
          payload: this.sessions.create(msg.payload),
        });
        return;
      case "session.list":
        this.send(client, {
          topic: "session.list_result",
          id: msg.id,
          payload: { sessions: this.sessions.list() },
        });
        return;
      case "session.subscribe": {
        const { session, fromSeq = 1 } = msg.payload;
        const info = this.sessions.info(session);
        if (!info) throw new NotFoundError(`no session ${session}`);
        for (const record of this.store.read(session, fromSeq)) {
          this.send(client, { topic: "session.events", payload: record });
        }
        client.subscriptions.add(session);
        this.send(client, {
          topic: "session.subscribed",
          id: msg.id,
          payload: { session: info, replayedThrough: info.lastSeq },
        });
        return;
      }
      case "session.unsubscribe":
        client.subscriptions.delete(msg.payload.session);
        this.send(client, { topic: "bus.ack", id: msg.id });
        return;
      case "session.cancel":
        this.sessions.cancel(msg.payload.session);
        this.send(client, { topic: "bus.ack", id: msg.id });
        return;
      case "session.input": {
        const { session, app, text } = msg.payload;
        this.sessions.input(session, app, text);
        if (msg.id !== undefined) this.send(client, { topic: "bus.ack", id: msg.id });
        return;
      }
      case "app.presence":
        // Registration arrives with the MCP gateway in Phase 2.
        this.fail(client, undefined, "bad_request", "app.presence is not supported yet");
        return;
      case "bus.welcome":
      case "bus.ack":
      case "bus.error":
      case "session.info":
      case "session.list_result":
      case "session.subscribed":
      case "session.events":
        this.fail(
          client,
          "id" in msg ? msg.id : undefined,
          "bad_request",
          `${msg.topic} is daemon → client only`,
        );
        return;
    }
  }
}
