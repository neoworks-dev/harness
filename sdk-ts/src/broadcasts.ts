// What the daemon tells every client without being asked, kept as values you can read and watch.
import type {
  AgentFocus,
  ConnectorInfo,
  Envelope,
  FileChanged,
  SecretRequest,
  SessionInfo,
  Theme,
} from "@suite/protocol/types";
import { Callbacks, WatchedValue } from "./watched.ts";

export class Broadcasts {
  readonly focus: WatchedValue<AgentFocus | undefined>;
  readonly theme: WatchedValue<Theme | undefined>;
  readonly connectors: WatchedValue<ConnectorInfo[]>;
  readonly sessions: WatchedValue<SessionInfo[]>;
  readonly fileChanges: Callbacks<FileChanged>;
  readonly secretRequests: Callbacks<SecretRequest>;

  constructor(onError: (error: Error) => void) {
    this.focus = new WatchedValue<AgentFocus | undefined>("agent focus", undefined, onError);
    this.theme = new WatchedValue<Theme | undefined>("theme", undefined, onError);
    this.connectors = new WatchedValue<ConnectorInfo[]>("connectors", [], onError);
    this.sessions = new WatchedValue<SessionInfo[]>("sessions", [], onError);
    this.fileChanges = new Callbacks<FileChanged>(onError);
    this.secretRequests = new Callbacks<SecretRequest>(onError);
  }

  /**
   * Takes in a frame from the daemon. Replies to `connectors.list` and `session.list` count as
   * well: they arrive in order with the broadcasts, so the latest one is always the truth.
   */
  take(frame: Envelope): void {
    switch (frame.topic) {
      case "agent.focus":
        this.focus.set(frame.payload);
        return;
      case "theme":
        this.theme.set(frame.payload);
        return;
      case "connectors.changed":
      case "connectors.list_result":
        this.connectors.set(frame.payload.connectors);
        return;
      case "session.list_result":
        this.sessions.set(frame.payload.sessions);
        return;
      case "session.changed":
      case "session.info":
        this.sessions.set(withSession(this.sessions.value, frame.payload));
        return;
      case "file.changed":
        this.fileChanges.callAll(frame.payload);
        return;
      case "secret.request":
        this.secretRequests.callAll(frame.payload);
        return;
      default:
    }
  }
}

/** The list with `session` replacing the entry of the same id, or added at the end. */
function withSession(sessions: SessionInfo[], session: SessionInfo): SessionInfo[] {
  if (!sessions.some((known) => known.id === session.id)) return [...sessions, session];
  return sessions.map((known) => (known.id === session.id ? session : known));
}
