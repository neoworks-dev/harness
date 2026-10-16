// Bus hub: protocol handling for one daemon, independent of the transport (server.ts adapts it to
// WebSocket-over-Unix-socket; tests drive it directly).
import type {
  Envelope,
  ErrorCode,
  FileChanged,
  SessionEventRecord,
  Theme,
  UiAction,
} from "@suite/protocol/types";
import { validateEnvelope } from "@suite/protocol/validate";
import type { AssetStore } from "./assets.ts";
import type { Logger } from "./log.ts";
import { NotFoundError, type SessionManager } from "./sessions.ts";
import type { SessionStore } from "./store.ts";
import type { ConnectorRegistry } from "./gateway/registry.ts";
import type { ViewManager } from "./view/views.ts";
import { BadRequestError, type FocusManager } from "./focus.ts";

export const PROTOCOL_VERSION = 1;

export interface Transport {
  send(frame: string): void;
}

export interface ClientHandle {
  receive(frame: string): void;
  closed(): void;
}

export interface HubParts {
  connectors?: ConnectorRegistry;
  focus?: FocusManager;
  views?: ViewManager;
  theme?: Theme;
  assets?: AssetStore;
  /** Runs a ui.action; rejects with NotFoundError/BadRequestError for impossible ones. */
  action?: (action: UiAction, from: string) => Promise<void>;
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
  views: Set<string>;
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
    private readonly parts: HubParts = {},
  ) {
    const { connectors, focus, views } = parts;
    views?.onPatch((payload) => {
      for (const client of this.clients) {
        if (client.views.has(payload.session)) this.send(client, { topic: "view.patch", payload });
      }
    });
    focus?.onChange((payload) => this.broadcastAll({ topic: "agent.focus", payload }));
    // Coalesce bursts (an app registering, then listing its tools) into one broadcast.
    let scheduled = false;
    connectors?.onChange(() => {
      if (scheduled) return;
      scheduled = true;
      queueMicrotask(() => {
        scheduled = false;
        this.broadcastAll({
          topic: "connectors.changed",
          payload: { connectors: connectors.list() },
        });
      });
    });
    // Appends are synchronous and so is subscribe's replay, so a subscriber either got an event
    // in its replay or gets it here — never both, never neither.
    store.onAppend((record) => this.broadcast(record));
  }

  connect(transport: Transport): ClientHandle {
    const client: Client = { transport, subscriptions: new Set(), views: new Set() };
    this.clients.add(client);
    return {
      receive: (frame) => this.receive(client, frame),
      closed: () => {
        this.clients.delete(client);
        void this.parts.connectors?.disconnect(client);
      },
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

  /** To every client that has said hello. */
  /** Tells every client (except the one it came from) that a file changed. */
  fileChanged(payload: FileChanged, except?: Client): void {
    for (const client of this.clients) {
      if (client !== except && client.name !== undefined) {
        this.send(client, { topic: "file.changed", payload });
      }
    }
  }

  private broadcastAll(envelope: Envelope): void {
    for (const client of this.clients) if (client.name !== undefined) this.send(client, envelope);
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
      else if (err instanceof BadRequestError) this.fail(client, id, "bad_request", err.message);
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
        if (this.parts.focus) {
          this.send(client, { topic: "agent.focus", payload: this.parts.focus.current() });
        }
        if (this.parts.theme) this.send(client, { topic: "theme", payload: this.parts.theme });
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
        const { session, app, text, images } = msg.payload;
        const { focus } = this.parts;
        if (focus && text.trim() === "/clear") {
          if (!this.sessions.info(session)) throw new NotFoundError(`no session ${session}`);
          focus.clear(session);
        } else {
          // Before queueing, so the turn already sees the new focus.
          focus?.followInput(session, app);
          this.sessions.input(session, app, text, images);
        }
        if (msg.id !== undefined) this.send(client, { topic: "bus.ack", id: msg.id });
        return;
      }
      case "app.presence": {
        const { connectors } = this.parts;
        if (!connectors) {
          this.fail(client, msg.id, "bad_request", "this daemon has no connector registry");
          return;
        }
        const { id } = msg;
        connectors.presence(client, msg.payload).then(
          () => {
            if (id !== undefined) this.send(client, { topic: "bus.ack", id });
          },
          (err: unknown) => this.fail(client, id, "internal", String(err)),
        );
        return;
      }
      case "agent.move":
        if (!this.parts.focus) {
          this.fail(client, msg.id, "bad_request", "this daemon doesn't track the agent's focus");
          return;
        }
        this.parts.focus.move(msg.payload, "user");
        this.send(client, { topic: "bus.ack", id: msg.id });
        return;
      case "view.subscribe": {
        const { session } = msg.payload;
        if (!this.parts.views) {
          this.fail(client, msg.id, "bad_request", "this daemon has no view resolver");
          return;
        }
        if (!this.sessions.info(session)) throw new NotFoundError(`no session ${session}`);
        // Synchronous, like session.subscribe: no patch can slip between snapshot and live.
        const snapshot = this.parts.views.get(session).snapshot();
        client.views.add(session);
        this.send(client, { topic: "view.snapshot", id: msg.id, payload: snapshot });
        return;
      }
      case "view.unsubscribe":
        client.views.delete(msg.payload.session);
        this.send(client, { topic: "bus.ack", id: msg.id });
        return;
      case "ui.action": {
        const { action } = this.parts;
        if (!action) {
          this.fail(client, msg.id, "bad_request", "this daemon doesn't handle actions");
          return;
        }
        if (!this.sessions.info(msg.payload.session)) {
          throw new NotFoundError(`no session ${msg.payload.session}`);
        }
        const { id } = msg;
        action(msg.payload, client.name ?? "ui").then(
          () => this.send(client, { topic: "bus.ack", id }),
          (err: unknown) => {
            if (err instanceof NotFoundError) this.fail(client, id, "not_found", err.message);
            else if (err instanceof BadRequestError)
              this.fail(client, id, "bad_request", err.message);
            else this.fail(client, id, "internal", String(err));
          },
        );
        return;
      }
      case "connectors.list":
        this.send(client, {
          topic: "connectors.list_result",
          id: msg.id,
          payload: { connectors: this.parts.connectors?.list() ?? [] },
        });
        return;
      case "asset.put": {
        const assets = this.requireAssets();
        const { path, data, name, mime } = msg.payload;
        const params: Parameters<AssetStore["put"]>[0] = { app: client.name };
        if (path !== undefined) params.path = path;
        if (data !== undefined) params.bytes = Buffer.from(data, "base64");
        if (name !== undefined) params.name = name;
        if (mime !== undefined) params.mime = mime;
        this.send(client, { topic: "asset.info", id: msg.id, payload: assets.put(params) });
        return;
      }
      case "asset.get":
        this.send(client, {
          topic: "asset.info",
          id: msg.id,
          payload: this.requireAssets().require(msg.payload.handle),
        });
        return;
      case "file.changed":
        this.fileChanged({ ...msg.payload, app: msg.payload.app ?? client.name }, client);
        return;
      case "asset.info":
      case "bus.welcome":
      case "bus.ack":
      case "bus.error":
      case "session.info":
      case "session.list_result":
      case "session.subscribed":
      case "session.events":
      case "connectors.list_result":
      case "connectors.changed":
      case "agent.focus":
      case "view.snapshot":
      case "view.patch":
      case "theme":
        this.fail(
          client,
          "id" in msg ? msg.id : undefined,
          "bad_request",
          `${msg.topic} is daemon → client only`,
        );
        return;
    }
  }

  private requireAssets(): AssetStore {
    if (!this.parts.assets) throw new BadRequestError("this daemon has no asset store");
    return this.parts.assets;
  }
}
