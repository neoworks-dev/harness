// Bus hub: protocol handling for one daemon, independent of the transport (server.ts adapts it to
// WebSocket-over-Unix-socket; tests drive it directly).
import type {
  Envelope,
  ErrorCode,
  FileChanged,
  HarnessInfo,
  HarnessSettings,
  SessionEventRecord,
  SpeechAudio,
  Theme,
  UiAction,
  UserSettings,
  ViewPage,
} from "@neoworks-dev/neo-protocol/types";
import { validateEnvelope } from "@neoworks-dev/neo-protocol/validate";
import type { AssetStore } from "./assets.ts";
import type { Logger } from "./log.ts";
import { NotFoundError, type SessionManager } from "./sessions.ts";
import type { SessionStore } from "./store.ts";
import type { ConnectorRegistry } from "./gateway/registry.ts";
import { parseViewKey, type ViewManager, viewKey } from "./view/views.ts";
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
  /** The /settings toggles: sent after welcome and to every client when one flips. */
  userSettings?: {
    get: () => UserSettings;
    onChange: (listener: (settings: UserSettings) => void) => void;
  };
  assets?: AssetStore;
  /** Writes out what the user said (speech.transcribe); rejects with BadRequestError. */
  transcribe?: (audio: SpeechAudio) => Promise<string>;
  /** Runs a ui.action; rejects with NotFoundError/BadRequestError for impossible ones. */
  action?: (action: UiAction, from: string) => Promise<void>;
  /**
   * A command typed as input (/model…): its work, or undefined when the text isn't one. The
   * context is bound to the client that typed it.
   */
  command?: (session: string, text: string, context: CommandContext) => Promise<void> | undefined;
  /**
   * Input that is not a command, before it is queued: the text the agent gets in its place (a
   * plugin's /command expanded) and what the chat shows (what was typed), or undefined to leave
   * it. Also the moment to bring the session up to date on what it can do.
   */
  expand?: (session: string, text: string) => { text: string; shown: string } | undefined;
  /** The harnesses a session can switch to with their options (session.settings). */
  settings?: (session: string, known?: boolean) => Promise<HarnessSettings[]>;
  /** The agents a session can run on, for the user to pick from (harnesses.list). */
  harnesses?: () => HarnessInfo[];
}

/** What a command can do with the client that ran it. */
export interface CommandContext {
  /**
   * Asks that client for a secret (an API key, a login code). The question and the answer travel
   * over the bus to it alone and are never stored or logged. undefined: cancelled, the client
   * went away, or the signal aborted.
   */
  askSecret(title: string, placeholder?: string, signal?: AbortSignal): Promise<string | undefined>;
}

export interface DaemonInfo {
  version: string;
  pid: number;
  startedAt: number;
}

/** One connected client. Opaque outside the hub: handed to askSecret by whoever got it from a command. */
export interface Client {
  transport: Transport;
  name?: string;
  subscriptions: Set<string>;
  views: Set<string>;
}

interface PendingSecret {
  client: Client;
  /** undefined: cancelled. */
  resolve: (value: string | undefined) => void;
}

/** Best-effort id of an invalid frame, so the error can still be correlated. */
function frameId(parsed: unknown): string | undefined {
  if (typeof parsed !== "object" || parsed === null || !("id" in parsed)) return undefined;
  return typeof parsed.id === "string" ? parsed.id : undefined;
}

/** The sessions a client follows: those it subscribed to, and those it has a view of. */
function followedBy(client: Client): Set<string> {
  const sessions = new Set(client.subscriptions);
  for (const key of client.views) sessions.add(parseViewKey(key).session);
  return sessions;
}

/** How often a session's overview may update while it streams. */
const SESSION_CHANGE_MS = 250;

export class Hub {
  private readonly clients = new Set<Client>();
  private readonly pendingChanges = new Map<string, ReturnType<typeof setTimeout>>();
  /** Open secret questions by id. Kept only here: never in the store, never in a log. */
  private readonly secrets = new Map<string, PendingSecret>();
  private secretCount = 0;

  constructor(
    private readonly store: SessionStore,
    private readonly sessions: SessionManager,
    private readonly info: DaemonInfo,
    private readonly log: Logger,
    private readonly parts: HubParts = {},
  ) {
    const { connectors, focus, views } = parts;
    views?.onPatch((payload) => {
      const key = viewKey(payload.session, payload.agent);
      for (const client of this.clients) {
        if (client.views.has(key)) this.send(client, { topic: "view.patch", payload });
      }
    });
    focus?.onChange((payload) => this.broadcastAll({ topic: "agent.focus", payload }));
    parts.userSettings?.onChange((payload) => {
      this.broadcastAll({ topic: "user.settings", payload });
    });
    focus?.onLeave((session) => this.releaseIfUnused(session));
    sessions.onRemove((id) => this.sessionRemoved(id));
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
    store.onAppend((record) => {
      this.broadcast(record);
      // A turn that ends while somebody has the session open is seen.
      if (record.event.type === "turn_ended" && this.viewed(record.session)) {
        store.markSeen(record.session);
      }
      this.sessionChanged(record.session);
    });
    sessions.onStateChange((id) => this.sessionChanged(id));
  }

  connect(transport: Transport): ClientHandle {
    const client: Client = { transport, subscriptions: new Set(), views: new Set() };
    this.clients.add(client);
    return {
      receive: (frame) => this.receive(client, frame),
      closed: () => {
        this.clients.delete(client);
        for (const [id, pending] of this.secrets) {
          if (pending.client !== client) continue;
          this.secrets.delete(id);
          pending.resolve(undefined);
        }
        void this.parts.connectors?.disconnect(client);
        for (const session of followedBy(client)) this.releaseIfUnused(session);
      },
    };
  }

  get clientCount(): number {
    return this.clients.size;
  }

  /**
   * Asks one client for a secret and waits. Sent as secret.request to that client only; nothing
   * about it reaches the session log or any other client. Resolves with the value, or undefined
   * when the client answers without one, disconnects, or the signal aborts first (the client is
   * not told: its field closes with the session or the next question).
   */
  askSecret(
    client: Client,
    question: { session: string; title: string; placeholder?: string },
    signal?: AbortSignal,
  ): Promise<string | undefined> {
    if (!this.clients.has(client) || signal?.aborted === true) return Promise.resolve(undefined);
    this.secretCount += 1;
    const id = `s${this.secretCount}`;
    return new Promise((resolve) => {
      const done = (value: string | undefined): void => {
        signal?.removeEventListener("abort", onAbort);
        resolve(value);
      };
      const onAbort = (): void => {
        this.secrets.delete(id);
        done(undefined);
      };
      this.secrets.set(id, { client, resolve: done });
      signal?.addEventListener("abort", onAbort, { once: true });
      const payload: Extract<Envelope, { topic: "secret.request" }>["payload"] = {
        id,
        session: question.session,
        title: question.title,
      };
      if (question.placeholder !== undefined) payload.placeholder = question.placeholder;
      this.send(client, { topic: "secret.request", payload });
    });
  }

  /**
   * Deletes the session if it is empty (see SessionManager.removable) and nothing follows it
   * any more: no client's subscription or view, and not the agent's current session.
   */
  private releaseIfUnused(session: string): void {
    if (this.isFollowed(session)) {
      return;
    }
    if (this.parts.focus?.current().session === session) {
      return;
    }
    if (!this.sessions.removable(session)) {
      return;
    }
    this.sessions.remove(session);
  }

  /** Whether some client has the session's own view open: somebody is looking at it. */
  private viewed(session: string): boolean {
    const key = viewKey(session);
    for (const client of this.clients) {
      if (client.views.has(key)) return true;
    }
    return false;
  }

  /** Whether some client has the session subscribed, or one of its views. */
  private isFollowed(session: string): boolean {
    for (const client of this.clients) {
      if (followedBy(client).has(session)) {
        return true;
      }
    }
    return false;
  }

  /** Tells every client a session is gone; a session.changed still pending for it is dropped. */
  private sessionRemoved(id: string): void {
    const pending = this.pendingChanges.get(id);
    if (pending !== undefined) {
      clearTimeout(pending);
      this.pendingChanges.delete(id);
    }
    this.broadcastAll({ topic: "session.removed", payload: { id } });
  }

  private broadcast(record: SessionEventRecord): void {
    for (const client of this.clients) {
      if (client.subscriptions.has(record.session)) {
        this.send(client, { topic: "session.events", payload: record });
      }
    }
  }

  /**
   * session.changed for overviews: at most one per session per SESSION_CHANGE_MS (a streaming
   * turn appends many events), and always one after the last change.
   */
  private sessionChanged(id: string): void {
    if (this.pendingChanges.has(id)) return;
    const timer = setTimeout(() => {
      this.pendingChanges.delete(id);
      try {
        const payload = this.sessions.info(id);
        if (payload) this.broadcastAll({ topic: "session.changed", payload });
      } catch (err) {
        // The store closed in between (shutdown, tests that skip close()).
        this.log.warn(`session.changed for ${id}: ${String(err)}`);
      }
    }, SESSION_CHANGE_MS);
    this.pendingChanges.set(id, timer);
  }

  /** Drops pending broadcasts; call before closing the store. */
  close(): void {
    for (const timer of this.pendingChanges.values()) clearTimeout(timer);
    this.pendingChanges.clear();
  }

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

  /**
   * Answers a request when the work is done (with the envelope it made, else an ack), or fails
   * it with what went wrong.
   */
  private settle(client: Client, id: string | undefined, work: Promise<Envelope | void>): void {
    work.then(
      (reply) => {
        if (reply) this.send(client, reply);
        else if (id !== undefined) this.send(client, { topic: "bus.ack", id });
      },
      (err: unknown) => this.failed(client, id, err),
    );
  }

  /** Fails a request with the error's kind. */
  private failed(client: Client, id: string | undefined, err: unknown): void {
    if (err instanceof NotFoundError) this.fail(client, id, "not_found", err.message);
    else if (err instanceof BadRequestError) this.fail(client, id, "bad_request", err.message);
    else {
      this.log.error(`request ${id ?? "-"}: ${String(err)}`);
      this.fail(client, id, "internal", String(err));
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
        if (this.parts.userSettings) {
          this.send(client, { topic: "user.settings", payload: this.parts.userSettings.get() });
        }
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
      case "session.settings": {
        const { session, known } = msg.payload;
        const { settings } = this.parts;
        if (!settings) throw new BadRequestError("this daemon has no harness settings");
        if (!this.sessions.info(session)) throw new NotFoundError(`no session ${session}`);
        settings(session, known).then(
          (harnesses) =>
            this.send(client, {
              topic: "session.settings_result",
              id: msg.id,
              payload: { harnesses },
            }),
          (err: unknown) => this.failed(client, msg.id, err),
        );
        return;
      }
      case "session.subscribe": {
        const { session, fromSeq = 1, skipTypes = [], withoutResults = [] } = msg.payload;
        const info = this.sessions.info(session);
        if (!info) throw new NotFoundError(`no session ${session}`);
        const light = skipTypes.length > 0 || withoutResults.length > 0;
        const replay = light
          ? this.store.readLight(session, fromSeq, skipTypes, withoutResults)
          : this.store.read(session, fromSeq);
        for (const record of replay) {
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
        this.releaseIfUnused(msg.payload.session);
        return;
      case "session.cancel":
        this.sessions.cancel(msg.payload.session);
        this.send(client, { topic: "bus.ack", id: msg.id });
        return;
      case "session.unqueue": {
        const { session, app } = msg.payload;
        const inputs = this.sessions.unqueue(session, app);
        this.send(client, { topic: "session.unqueued", id: msg.id, payload: { inputs } });
        return;
      }
      case "session.input": {
        const { session, app, text, images, steer } = msg.payload;
        const { focus } = this.parts;
        const command = this.parts.command?.(session, text, {
          askSecret: (title, placeholder, signal) => {
            const question: Parameters<Hub["askSecret"]>[1] = { session, title };
            if (placeholder !== undefined) question.placeholder = placeholder;
            return this.askSecret(client, question, signal);
          },
        });
        if (command) {
          if (!this.sessions.info(session)) throw new NotFoundError(`no session ${session}`);
          this.settle(client, msg.id, command);
          return;
        }
        if (text.trim() === "/clear") {
          this.sessions.clearContext(session);
        } else if (focus && text.trim() === "/new") {
          if (!this.sessions.info(session)) throw new NotFoundError(`no session ${session}`);
          focus.fresh(session, app);
        } else {
          // Before queueing, so the turn already sees the new focus.
          focus?.followInput(session, app);
          const expanded = this.parts.expand?.(session, text);
          const sent = expanded ?? { text, shown: undefined };
          this.sessions.input(session, app, sent.text, images, sent.shown, steer === true);
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
          (err: unknown) => {
            if (err instanceof BadRequestError) this.fail(client, id, "bad_request", err.message);
            else this.fail(client, id, "internal", String(err));
          },
        );
        return;
      }
      case "app.context": {
        const { app, context } = msg.payload;
        if (!this.parts.connectors?.setContext(client, app, context)) {
          throw new NotFoundError(`${app} isn't registered on this connection`);
        }
        if (msg.id !== undefined) this.send(client, { topic: "bus.ack", id: msg.id });
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
        const { session, agent, tail } = msg.payload;
        if (!this.parts.views) {
          this.fail(client, msg.id, "bad_request", "this daemon has no view resolver");
          return;
        }
        if (!this.sessions.info(session)) throw new NotFoundError(`no session ${session}`);
        // Synchronous, like session.subscribe: no patch can slip between snapshot and live.
        const snapshot = this.parts.views.get(session, agent).snapshot(tail);
        client.views.add(viewKey(session, agent));
        this.send(client, { topic: "view.snapshot", id: msg.id, payload: snapshot });
        // Opened: whatever it finished is seen.
        if (agent === undefined && this.store.markSeen(session)) this.sessionChanged(session);
        return;
      }
      case "view.unsubscribe":
        client.views.delete(viewKey(msg.payload.session, msg.payload.agent));
        this.send(client, { topic: "bus.ack", id: msg.id });
        this.releaseIfUnused(msg.payload.session);
        return;
      case "view.page": {
        const { session, agent, before, count } = msg.payload;
        if (!this.parts.views) {
          this.fail(client, msg.id, "bad_request", "this daemon has no view resolver");
          return;
        }
        if (!this.sessions.info(session)) throw new NotFoundError(`no session ${session}`);
        const page = this.parts.views.get(session, agent).page(before, count);
        if (!page) throw new NotFoundError(`no item ${before} in the view of ${session}`);
        const payload: ViewPage = { session, items: page.items };
        if (agent !== undefined) payload.agent = agent;
        if (page.before > 0) payload.before = page.before;
        this.send(client, { topic: "view.page_result", id: msg.id, payload });
        return;
      }
      case "ui.action": {
        const { action } = this.parts;
        if (!action) {
          this.fail(client, msg.id, "bad_request", "this daemon doesn't handle actions");
          return;
        }
        if (!this.sessions.info(msg.payload.session)) {
          throw new NotFoundError(`no session ${msg.payload.session}`);
        }
        this.settle(client, msg.id, action(msg.payload, client.name ?? "ui"));
        return;
      }
      case "connectors.list":
        this.send(client, {
          topic: "connectors.list_result",
          id: msg.id,
          payload: { connectors: this.parts.connectors?.list() ?? [] },
        });
        return;
      case "harnesses.list":
        this.send(client, {
          topic: "harnesses.list_result",
          id: msg.id,
          payload: { harnesses: this.parts.harnesses?.() ?? [] },
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
      case "asset.fetch": {
        const assets = this.requireAssets();
        const { id } = msg;
        this.settle(
          client,
          id,
          assets.fetch(msg.payload.url).then((payload) => ({ topic: "asset.info", id, payload })),
        );
        return;
      }
      case "speech.transcribe": {
        const { transcribe } = this.parts;
        if (!transcribe) throw new BadRequestError("this daemon can't transcribe speech");
        const { id } = msg;
        this.settle(
          client,
          id,
          transcribe(msg.payload).then((text) => ({ topic: "speech.text", id, payload: { text } })),
        );
        return;
      }
      case "secret.answer": {
        // Never log or echo msg.payload.value: not here, not in an error.
        const { id, value } = msg.payload;
        const pending = this.secrets.get(id);
        if (!pending) throw new NotFoundError(`no open secret question ${id}`);
        if (pending.client !== client) {
          throw new BadRequestError(`secret question ${id} was not asked of this client`);
        }
        this.secrets.delete(id);
        pending.resolve(value);
        return;
      }
      case "file.changed":
        this.fileChanged({ ...msg.payload, app: msg.payload.app ?? client.name }, client);
        return;
      case "asset.info":
      case "bus.welcome":
      case "bus.ack":
      case "bus.error":
      case "session.info":
      case "session.list_result":
      case "session.settings_result":
      case "session.subscribed":
      case "session.events":
      case "session.removed":
      case "connectors.list_result":
      case "connectors.changed":
      case "agent.focus":
      case "view.snapshot":
      case "view.patch":
      case "secret.request":
      case "theme":
      case "user.settings":
      case "speech.text":
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
