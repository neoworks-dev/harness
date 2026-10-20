// The window's connection to the daemon and the session it shows, as reactive state. Plugins reach
// it through the `daemon` service (services/daemon.ts), and never talk to the client directly.
import { invoke } from "@neoworks/svelte-gpui";
import type {
  AgentFocus,
  AppPresence,
  HarnessSettings,
  SecretRequest,
  SessionEventRecord,
  SessionInfo,
  SessionSettingsPayload,
  Theme,
  ViewItem,
} from "@suite/protocol/types";
import {
  assetUrl,
  connect,
  ViewState,
  type Client,
  type ConnectionState,
  type DaemonAddress,
  type Unsubscribe,
} from "@suite/sdk";
import * as host from "./host/git.ts";
import { paletteOf } from "../lib/palette.ts";
import { chooseSession, pickAppId } from "../lib/startup.ts";
import {
  applyEvent,
  initialStatus,
  nextApprovals,
  optionLabel,
  type SessionStatus,
} from "../lib/status.ts";

/** How long the settings that arrive together (a replay) have to come before asking. */
const OFFERED_DELAY_MS = 50;

type RecordListener = (record: SessionEventRecord) => void;
type SecretListener = (request: SecretRequest) => void;

function presence(app: string, cwd: string): AppPresence {
  return { app, kind: "app", title: "Desktop", open: true, context: { cwd } };
}

function message(error: unknown): string {
  if (error instanceof Error) return error.message;
  return String(error);
}

class Suite {
  /** "starting" until the first connection, then the client's state. */
  connection = $state<ConnectionState | "starting">("starting");
  /** Why starting or the last request failed, for the window to show. */
  error = $state<string>();
  /** The name this window registered under: desktop, desktop-2… */
  appId = $state("desktop");
  /** Every session, kept current by the daemon. */
  sessions = $state.raw<readonly SessionInfo[]>([]);
  /** Where the agent is: its current session and the app showing it. */
  focus = $state.raw<AgentFocus>();
  theme = $state.raw<Theme>();
  /** What the window draws with, from `theme`. */
  palette = $derived(paletteOf(this.theme));

  /** The session this window shows. */
  sessionId = $state<string>();
  /** Its directory. */
  cwd = $state("");
  /** Its chat, as the daemon resolved it. Replaced (never mutated) on every change. */
  items = $state.raw<readonly ViewItem[]>([]);
  status = $state.raw<SessionStatus>(initialStatus());
  /** The git branch checked out in `cwd`, if any. */
  branch = $state<string | null>(null);
  /**
   * What the harnesses offered when last asked, for the names of the values the status shows
   * (`Haiku 5.5` for the model `haiku`). Asked without starting anything when the session or one
   * of its settings changes, and again when a picker asks for all of it.
   */
  offered = $state.raw<readonly HarnessSettings[]>([]);

  private client?: Client;
  private readonly view = new ViewState();
  private stopSession?: Unsubscribe;
  /** The session this window asked the agent to move to, until the daemon confirms it. */
  private movingTo?: string;
  private readonly recordListeners = new Set<RecordListener>();
  private offeredScheduled = false;
  private readonly secretListeners = new Set<SecretListener>();

  /** The session shown now, from the live list. */
  get session(): SessionInfo | undefined {
    return this.sessions.find((session) => session.id === this.sessionId);
  }

  /** Connects, registers the window and opens the session `suite gui` asked for. */
  async start(): Promise<void> {
    try {
      const startup = await invoke("startup", null);
      const client = await connect({
        client: "desktop",
        address: async () => (this.daemonAddress = await invoke("daemon_address", null)),
        waitForDaemon: true,
        onError: (error) => (this.error = error.message),
      });
      this.client = client;
      this.connection = client.state.value;
      client.state.on((state) => (this.connection = state));
      client.sessions.on((sessions) => (this.sessions = sessions));
      client.focus.on((focus) => this.followFocus(focus));
      client.theme.on((theme) => (this.theme = theme));
      client.onSecretRequest((request) => {
        for (const listener of this.secretListeners) listener(request);
      });
      this.theme = client.theme.value;

      this.appId = pickAppId(await client.listConnectors());
      await client.setPresence(presence(this.appId, startup.cwd));
      const sessions = await client.listSessions();
      this.sessions = sessions;
      const focus = await client.focus.waitFor(3000).catch(() => undefined);
      this.focus = focus;
      const args = { session: startup.session ?? undefined, new: startup.new };
      const choice = chooseSession(sessions, focus, startup.cwd, args);
      if ("error" in choice) throw new Error(choice.error);
      if ("new" in choice) await this.newSession(startup.cwd);
      else await this.open(choice.existing);
    } catch (error) {
      this.error = message(error);
    }
  }

  /** Shows `id` in this window and moves the agent here with it. */
  async open(id: string): Promise<void> {
    this.movingTo = id;
    this.show(id);
    await this.need()
      .moveAgent({ app: this.appId, session: id })
      .catch((error: unknown) => {
        this.movingTo = undefined;
        this.error = message(error);
      });
  }

  /**
   * Someone else made another session current while the agent is in this window (`/clear`,
   * `suite move`): show it too. Focus changes from before our own move are stale, so they are
   * skipped until the one for the session we moved to arrives.
   */
  private followFocus(focus: AgentFocus | undefined): void {
    this.focus = focus;
    if (!focus) return;
    if (this.movingTo !== undefined) {
      if (focus.session === this.movingTo) this.movingTo = undefined;
      return;
    }
    if (focus.app === this.appId && focus.session !== this.sessionId) this.show(focus.session);
  }

  /** Follows `id`'s view and events in place of the session shown before. */
  private show(id: string): void {
    const client = this.need();
    this.stopSession?.();
    this.sessionId = id;
    // A session just made by /clear may not be listed yet; it is in the same directory.
    this.cwd = this.sessions.find((session) => session.id === id)?.cwd ?? this.cwd;
    this.view.reset();
    this.items = [];
    this.status = initialStatus();
    this.offered = [];
    this.askOffered();
    // The history arrives one event per message, and the chat redraws after each message: a
    // session of thousands of events froze the window for seconds. So the replay is kept until
    // it is over and then applied at once. The items wait for it too: the chat's rows depend on
    // what the events say (which turns ended), and rows built before would all be built again.
    let replay: SessionEventRecord[] | undefined = [];
    const showItems = (): void => {
      if (replay === undefined) this.items = [...this.view.items];
    };
    const stopView = client.subscribeView(id, {
      onSnapshot: (snapshot) => {
        this.view.applySnapshot(snapshot);
        showItems();
      },
      onPatch: (patch) => {
        if (this.view.applyPatch(patch).length > 0) showItems();
      },
    });
    const apply = (record: SessionEventRecord): void => {
      this.status = applyEvent(this.status, record);
      for (const listener of this.recordListeners) listener(record);
    };
    const stopEvents = client.subscribe(id, {
      onEvent: (event, record) => {
        if (replay !== undefined) {
          replay.push(record);
          return;
        }
        apply(record);
        if (event.type === "setting_changed") this.askOffered();
      },
      onLive: () => {
        if (replay === undefined) return;
        const records = replay;
        replay = undefined;
        for (const record of records) apply(record);
        if (records.some((record) => record.event.type === "setting_changed")) this.askOffered();
        showItems();
      },
    });
    this.stopSession = () => {
      stopView();
      stopEvents();
    };
    void this.refreshBranch();
  }

  /** Starts a new session (in `cwd`, else the current directory) and shows it. */
  async newSession(cwd = this.cwd): Promise<void> {
    const session = await this.need().createSession({ cwd });
    await this.open(session.id);
  }

  /** Sends what the user typed, commands (`/model …`) included. */
  async send(text: string, images?: string[]): Promise<void> {
    if (this.sessionId === undefined) return;
    await this.guard(this.need().input(this.sessionId, text, { images }));
  }

  /** Sends a command (`/title`) to a session that may not be the one shown. */
  async sendTo(session: string, text: string): Promise<void> {
    await this.guard(this.need().input(session, text));
  }

  /**
   * What each harness the session can switch to offers (models, reasoning levels). Asking may
   * start a guest's adapter; `known` starts nothing and leaves out what is not known yet.
   */
  async settings(known = false): Promise<SessionSettingsPayload | undefined> {
    const session = this.sessionId;
    if (session === undefined) return undefined;
    const reply = await this.need().sessionSettings(session, known);
    if (this.sessionId === session) {
      this.offered = known ? mergeKnown(this.offered, reply.harnesses) : reply.harnesses;
    }
    return reply;
  }

  /** The name a harness (by default the session's) gives a value of a setting, else the value. */
  label(
    category: "model" | "thought_level" | "mode",
    value: string,
    harness = this.status.harness,
  ): string {
    return optionLabel(this.offered, harness, category, value);
  }

  /** Asks what the harnesses offer, once for the settings that arrive together (a replay). */
  private askOffered(): void {
    if (this.offeredScheduled) return;
    this.offeredScheduled = true;
    setTimeout(() => {
      this.offeredScheduled = false;
      void this.settings(true).catch(() => {});
    }, OFFERED_DELAY_MS);
  }

  /** Cancels the running turn. */
  async cancel(): Promise<void> {
    if (this.sessionId === undefined) return;
    await this.guard(this.need().cancel(this.sessionId));
  }

  /** A button in view item `ref` was pressed (`args` carries e.g. a note for the agent). */
  async action(ref: string, action: string, args?: unknown): Promise<void> {
    if (this.sessionId === undefined) return;
    await this.guard(this.need().action(this.sessionId, ref, action, args));
  }

  /** Shift-Tab: the next approvals mode. */
  async cycleApprovals(): Promise<void> {
    await this.send(`/approvals ${nextApprovals(this.status.approvals)}`);
  }

  /**
   * Follows one agent's view (a subagent or a guest) in the shown session, for the agents list.
   * Returns the function that stops it.
   */
  followAgent(agent: string, onItems: (items: readonly ViewItem[]) => void): Unsubscribe {
    const id = this.sessionId;
    if (id === undefined) return () => {};
    const view = new ViewState();
    return this.need().subscribeView(id, {
      agent,
      onSnapshot: (snapshot) => {
        view.applySnapshot(snapshot);
        onItems([...view.items]);
      },
      onPatch: (patch) => {
        if (view.applyPatch(patch).length > 0) onItems([...view.items]);
      },
    });
  }

  /** Every event of the shown session from now on, also after switching sessions. */
  onRecord(listener: RecordListener): Unsubscribe {
    this.recordListeners.add(listener);
    return () => this.recordListeners.delete(listener);
  }

  /**
   * The daemon asks this window for a secret (an API key, a login code). Only the listener may
   * keep it, and not beyond the answer.
   */
  onSecretRequest(listener: SecretListener): Unsubscribe {
    this.secretListeners.add(listener);
    return () => this.secretListeners.delete(listener);
  }

  /** Answers a secret request: the value, or none when cancelled. Nothing is kept or shown. */
  answerSecret(id: string, value?: string): void {
    try {
      this.need().answerSecret(id, value);
    } catch (error) {
      this.error = message(error);
    }
  }

  private async refreshBranch(): Promise<void> {
    this.branch = await host.branch(this.cwd).catch(() => null);
  }

  private async guard(request: Promise<void>): Promise<void> {
    try {
      await request;
      this.error = undefined;
    } catch (error) {
      this.error = message(error);
    }
  }

  private need(): Client {
    if (!this.client) throw new Error("not connected to the suite daemon");
    return this.client;
  }

  // --- Pictures (the chat) ---

  /** Has the daemon download a web image into its asset store; the asset's handle. */
  async fetchAsset(url: string): Promise<string> {
    return (await this.need().fetchAsset(url)).handle;
  }

  /** The daemon's address as of the last (re)connect; assets are fetched from it. */
  private daemonAddress?: DaemonAddress;

  /**
   * The URL the window can load an `asset://` handle from, or undefined for any other source.
   * It contains the daemon's token: hand it to an `Image` only, never log or show it.
   */
  assetSrc(src: string): string | undefined {
    return this.daemonAddress === undefined ? undefined : assetUrl(this.daemonAddress, src);
  }
}

export const suite = new Suite();

/** Known options fill in a harness that had none; they never take away what a full ask found. */
function mergeKnown(
  before: readonly HarnessSettings[],
  asked: readonly HarnessSettings[],
): HarnessSettings[] {
  return asked.map((entry) => {
    const had = before.find((old) => old.name === entry.name);
    if (entry.options.length === 0 && had !== undefined) return { ...entry, options: had.options };
    return entry;
  });
}
