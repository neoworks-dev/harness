// Session store: SQLite event log, one row per event, gap-free per-session sequence numbers.
import { Database } from "bun:sqlite";
import type {
  AppComponent,
  ConnectorKind,
  LaunchSpec,
  SessionEvent,
  SessionEventRecord,
} from "@neoworks-dev/neo-protocol/types";

/**
 * Where a session's title came from, when neo made it: the start of the first message
 * (until a model has written one), or a model. A title without one was given by the user.
 */
export type TitleSource = "first_line" | "generated";

export interface SessionRow {
  id: string;
  title?: string;
  titleSource?: TitleSource;
  cwd: string;
  /** The session that delegated into this one. */
  parent?: string;
  createdAt: number;
  lastActivity: number;
  lastSeq: number;
}

interface SessionDbRow {
  id: string;
  title: string | null;
  title_source: string | null;
  parent: string | null;
  cwd: string;
  created_at: number;
  last_activity: number;
  last_seq: number;
}

interface EventDbRow {
  session: string;
  seq: number;
  ts: number;
  data: string;
}

/** What the gateway remembers about a connector between runs (schemas, version, launch). */
export interface StoredConnector {
  app: string;
  kind: ConnectorKind;
  title?: string;
  v: number;
  schemaHash: string;
  tools: StoredTool[];
  instructions?: string;
  /** What its presence said it is for, if it said. */
  description?: string;
  launch?: LaunchSpec;
  /** View templates by tool name (without the app prefix). */
  templates?: Record<string, unknown>;
  /** The neoui components it offers, by name (written `app.Name`). */
  components?: Record<string, AppComponent>;
  /** Its skills: the MCP prompts it offers that take no arguments. */
  skills?: StoredSkill[];
}

/** Instructions for work in an app, loaded by an agent when the work calls for them. */
export interface StoredSkill {
  name: string;
  /** When to load it: what the agent is told when the app connects. */
  description: string;
  /** What skills_load returns. */
  text: string;
}

export interface StoredTool {
  /** Name as the connector's MCP server knows it (without the app prefix). */
  name: string;
  description: string;
  inputSchema: Record<string, unknown>;
  outputSchema?: Record<string, unknown>;
}

export type AppendListener = (record: SessionEventRecord) => void;

const SCHEMA = `
  CREATE TABLE IF NOT EXISTS sessions (
    id            TEXT PRIMARY KEY,
    title         TEXT,
    cwd           TEXT NOT NULL,
    created_at    INTEGER NOT NULL,
    last_activity INTEGER NOT NULL,
    last_seq      INTEGER NOT NULL DEFAULT 0
  ) STRICT;
  CREATE TABLE IF NOT EXISTS events (
    session TEXT NOT NULL REFERENCES sessions(id),
    seq     INTEGER NOT NULL,
    ts      INTEGER NOT NULL,
    type    TEXT NOT NULL,
    data    TEXT NOT NULL,
    PRIMARY KEY (session, seq)
  ) STRICT, WITHOUT ROWID;
  CREATE INDEX IF NOT EXISTS events_by_type ON events (session, type, seq);
  CREATE TABLE IF NOT EXISTS meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
  ) STRICT;
  CREATE TABLE IF NOT EXISTS connectors (
    app  TEXT PRIMARY KEY,
    data TEXT NOT NULL
  ) STRICT;
`;

function toRow(r: SessionDbRow): SessionRow {
  const row: SessionRow = {
    id: r.id,
    cwd: r.cwd,
    createdAt: r.created_at,
    lastActivity: r.last_activity,
    lastSeq: r.last_seq,
  };
  if (r.title !== null) row.title = r.title;
  if (r.title_source === "first_line" || r.title_source === "generated") {
    row.titleSource = r.title_source;
  }
  if (r.parent !== null) row.parent = r.parent;
  return row;
}

function toRecord(r: EventDbRow): SessionEventRecord {
  const event: SessionEvent = JSON.parse(r.data);
  return { session: r.session, seq: r.seq, ts: r.ts, event };
}

export class SessionStore {
  private readonly db: Database;
  private readonly listeners = new Set<AppendListener>();
  private readonly appendTx: (session: string, event: SessionEvent, ts: number) => number;
  private readonly deleteTx: (session: string) => void;

  /** `path` is a file path, or ":memory:" for tests. */
  constructor(path: string) {
    this.db = new Database(path, { create: true, strict: true });
    this.db.exec(
      "PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL; PRAGMA foreign_keys = ON;",
    );
    this.db.exec(SCHEMA);
    // Added after the first release: sessions of an older database have no parent column.
    const columns = this.db.query<{ name: string }, []>("PRAGMA table_info(sessions)").all();
    if (!columns.some((column) => column.name === "parent")) {
      this.db.exec("ALTER TABLE sessions ADD COLUMN parent TEXT");
    }
    if (!columns.some((column) => column.name === "title_source")) {
      this.db.exec("ALTER TABLE sessions ADD COLUMN title_source TEXT");
    }
    // What was there when the column came counts as seen.
    if (!columns.some((column) => column.name === "seen_seq")) {
      this.db.exec("ALTER TABLE sessions ADD COLUMN seen_seq INTEGER");
      this.db.exec("UPDATE sessions SET seen_seq = last_seq");
    }

    // The counter lives on the session row, so the next seq is derived inside the same
    // transaction that inserts the event: no gaps or reuse, also across restarts.
    const bump = this.db.query<{ last_seq: number }, [number, string]>(
      "UPDATE sessions SET last_seq = last_seq + 1, last_activity = ? WHERE id = ? RETURNING last_seq",
    );
    const insert = this.db.query(
      "INSERT INTO events (session, seq, ts, type, data) VALUES (?, ?, ?, ?, ?)",
    );
    this.appendTx = this.db.transaction((session: string, event: SessionEvent, ts: number) => {
      const bumped = bump.get(ts, session);
      if (!bumped) throw new Error(`unknown session ${session}`);
      insert.run(session, bumped.last_seq, ts, event.type, JSON.stringify(event));
      return bumped.last_seq;
    });
    const deleteEvents = this.db.query("DELETE FROM events WHERE session = ?");
    const deleteRow = this.db.query("DELETE FROM sessions WHERE id = ?");
    this.deleteTx = this.db.transaction((session: string) => {
      deleteEvents.run(session);
      deleteRow.run(session);
    });
  }

  createSession(params: {
    id: string;
    cwd: string;
    title?: string;
    parent?: string;
    now?: number;
  }): SessionRow {
    const now = params.now ?? Date.now();
    this.db
      .query(
        "INSERT INTO sessions (id, title, parent, cwd, created_at, last_activity) VALUES (?, ?, ?, ?, ?, ?)",
      )
      .run(params.id, params.title ?? null, params.parent ?? null, params.cwd, now, now);
    const row = this.getSession(params.id);
    if (!row) throw new Error(`failed to create session ${params.id}`);
    return row;
  }

  /** Without `source` the title counts as the user's own: neo never replaces it by itself. */
  setTitle(id: string, title: string, source?: TitleSource): void {
    this.db
      .query("UPDATE sessions SET title = ?, title_source = ? WHERE id = ?")
      .run(title, source ?? null, id);
  }

  setCwd(id: string, cwd: string): void {
    this.db.query("UPDATE sessions SET cwd = ? WHERE id = ?").run(cwd, id);
  }

  getSession(id: string): SessionRow | undefined {
    const r = this.db.query<SessionDbRow, [string]>("SELECT * FROM sessions WHERE id = ?").get(id);
    return r ? toRow(r) : undefined;
  }

  /** Most recently active first. */
  listSessions(): SessionRow[] {
    return this.db
      .query<SessionDbRow, []>("SELECT * FROM sessions ORDER BY last_activity DESC, id")
      .all()
      .map(toRow);
  }

  /** Appends one event and notifies listeners synchronously, after the commit. */
  append(session: string, event: SessionEvent, ts = Date.now()): SessionEventRecord {
    const seq = this.appendTx(session, event, ts);
    const record: SessionEventRecord = { session, seq, ts, event };
    for (const listener of this.listeners) listener(record);
    return record;
  }

  /** Events with seq ≥ fromSeq, in order. */
  read(session: string, fromSeq = 1): SessionEventRecord[] {
    return this.db
      .query<EventDbRow, [string, number]>(
        "SELECT session, seq, ts, data FROM events WHERE session = ? AND seq >= ? ORDER BY seq",
      )
      .all(session, fromSeq)
      .map(toRecord);
  }

  /**
   * A lighter replay (session.subscribe's skipTypes and withoutResults): events of `skip` left
   * out, and the updates of tool calls of the kinds in `bare` without their content and
   * rawOutput. SQLite drops them before they are read, so they cost nothing to leave out.
   */
  readLight(
    session: string,
    fromSeq: number,
    skip: readonly string[],
    bare: readonly string[],
  ): SessionEventRecord[] {
    // Numbered: ?1 the session, ?2 fromSeq, then the skipped types, then the kinds. SQLite takes
    // an empty `IN ()`, so neither list needs a case of its own.
    const skipped = skip.map((_, i) => `?${i + 3}`).join(", ");
    const kinds = bare.map((_, i) => `?${i + 3 + skip.length}`).join(", ");
    return this.db
      .query<EventDbRow, (string | number)[]>(
        `SELECT session, seq, ts,
           CASE WHEN type = 'tool_call_update'
                 AND json_extract(data, '$.toolCallId') IN (
                   SELECT json_extract(data, '$.toolCallId') FROM events
                   WHERE session = ?1 AND type = 'tool_call' AND json_extract(data, '$.kind') IN (${kinds}))
             THEN json_remove(data, '$.content', '$.rawOutput')
             ELSE data END AS data
         FROM events WHERE session = ?1 AND seq >= ?2 AND type NOT IN (${skipped}) ORDER BY seq`,
      )
      .all(session, fromSeq, ...skip, ...bare)
      .map(toRecord);
  }

  /** Highest seq of an event of the given type, or 0. */
  lastSeqOfType(session: string, type: SessionEvent["type"]): number {
    const r = this.db
      .query<{ seq: number | null }, [string, string]>(
        "SELECT MAX(seq) AS seq FROM events WHERE session = ? AND type = ?",
      )
      .get(session, type);
    return r?.seq ?? 0;
  }

  /** Events of the given types, in seq order. */
  readTypes(session: string, types: SessionEvent["type"][]): SessionEventRecord[] {
    const placeholders = types.map(() => "?").join(", ");
    return this.db
      .query<EventDbRow, string[]>(
        `SELECT session, seq, ts, data FROM events WHERE session = ? AND type IN (${placeholders}) ORDER BY seq`,
      )
      .all(session, ...types)
      .map(toRecord);
  }

  /** Somebody looked at the session: what it holds now is seen. Whether that changed anything. */
  markSeen(session: string): boolean {
    const changes = this.db
      .query(
        "UPDATE sessions SET seen_seq = last_seq WHERE id = ? AND coalesce(seen_seq, -1) <> last_seq",
      )
      .run(session).changes;
    return changes > 0;
  }

  /** A turn ended in the session since it was last seen (`markSeen`). */
  unseen(session: string): boolean {
    const row = this.db
      .query<{ unseen: number | null }, [string]>(
        `SELECT (SELECT max(seq) FROM events WHERE session = s.id AND type = 'turn_ended')
           > coalesce(s.seen_seq, 0) AS unseen
         FROM sessions s WHERE id = ?`,
      )
      .get(session);
    return row?.unseen === 1;
  }

  /** Whether the session's log has an event of this type. */
  hasEventOfType(session: string, type: SessionEvent["type"]): boolean {
    const row = this.db
      .query<{ found: number }, [string, string]>(
        "SELECT EXISTS (SELECT 1 FROM events WHERE session = ? AND type = ?) AS found",
      )
      .get(session, type);
    return row?.found === 1;
  }

  /** Whether any session names this one as its parent. */
  hasChildren(session: string): boolean {
    const row = this.db
      .query<{ found: number }, [string]>(
        "SELECT EXISTS (SELECT 1 FROM sessions WHERE parent = ?) AS found",
      )
      .get(session);
    return row?.found === 1;
  }

  /** Deletes the session and its whole log in one transaction. */
  deleteSession(id: string): void {
    this.deleteTx(id);
  }

  saveConnector(connector: StoredConnector): void {
    this.db
      .query("INSERT OR REPLACE INTO connectors (app, data) VALUES (?, ?)")
      .run(connector.app, JSON.stringify(connector));
  }

  loadConnectors(): StoredConnector[] {
    return this.db
      .query<{ data: string }, []>("SELECT data FROM connectors ORDER BY app")
      .all()
      .map((r) => {
        const connector: StoredConnector = JSON.parse(r.data);
        return connector;
      });
  }

  getMeta(key: string): string | undefined {
    const row = this.db
      .query<{ value: string }, [string]>("SELECT value FROM meta WHERE key = ?")
      .get(key);
    return row?.value;
  }

  setMeta(key: string, value: string): void {
    this.db.query("INSERT OR REPLACE INTO meta (key, value) VALUES (?, ?)").run(key, value);
  }

  onAppend(listener: AppendListener): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  close(): void {
    this.db.close();
  }
}
