import { afterEach, describe, expect, test } from "bun:test";
import { mkdtempSync, readdirSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { Envelope } from "@neoworks-dev/neo-protocol/types";
import { validateEnvelope } from "@neoworks-dev/neo-protocol/validate";
import { type ClientHandle, type CommandContext, Hub } from "../src/hub.ts";
import type { Logger } from "../src/log.ts";
import { SessionManager } from "../src/sessions.ts";
import { SessionStore } from "../src/store.ts";
import { FakeAgent } from "./fake-agent.ts";

const SECRET = "sk-very-secret-value-123";

interface TestClient {
  received: Envelope[];
  send(envelope: Envelope | Record<string, unknown>): void;
  handle: ClientHandle;
  /** Frames of one topic. */
  of<T extends Envelope["topic"]>(topic: T): Extract<Envelope, { topic: T }>[];
}

interface Setup {
  dir: string;
  store: SessionStore;
  hub: Hub;
  session: string;
  /** Every line the daemon logged. */
  logged: string[];
  /** What the test command got back, in order. */
  answers: (string | undefined)[];
  /** The abort signal /ask passes. */
  abort: AbortController;
}

const dirs: string[] = [];
afterEach(() => {
  for (const dir of dirs.splice(0)) rmSync(dir, { recursive: true, force: true });
});

/** A hub with a test-only /ask command (not part of the daemon) that asks the typing client. */
function setup(): Setup {
  const dir = mkdtempSync(join(tmpdir(), "neo-secret-test-"));
  dirs.push(dir);
  const logged: string[] = [];
  const log: Logger = {
    info: (m) => logged.push(m),
    warn: (m) => logged.push(m),
    error: (m) => logged.push(m),
  };
  const store = new SessionStore(join(dir, "sessions.db"));
  const sessions = new SessionManager(store, new FakeAgent(), { defaultCwd: "/home", log });
  const answers: (string | undefined)[] = [];
  const abort = new AbortController();
  const command = (
    _session: string,
    text: string,
    context: CommandContext,
  ): Promise<void> | undefined => {
    if (!text.startsWith("/ask")) return undefined;
    const placeholder = text.split(" ")[1];
    return context
      .askSecret("DeepSeek API key", placeholder, abort.signal)
      .then((value) => void answers.push(value));
  };
  const hub = new Hub(store, sessions, { version: "test", pid: 1, startedAt: 0 }, log, { command });
  const session = sessions.create({}).id;
  return { dir, store, hub, session, logged, answers, abort };
}

function client(hub: Hub): TestClient {
  const received: Envelope[] = [];
  const handle = hub.connect({
    send(frame) {
      const result = validateEnvelope(JSON.parse(frame));
      if (!result.ok) throw new Error(`daemon sent an invalid frame: ${result.error}\n${frame}`);
      received.push(result.value);
    },
  });
  const send = (envelope: Envelope | Record<string, unknown>): void =>
    handle.receive(JSON.stringify(envelope));
  send({ topic: "bus.hello", payload: { protocol: 1, client: "test" } });
  const of = <T extends Envelope["topic"]>(topic: T): Extract<Envelope, { topic: T }>[] =>
    received.filter((e): e is Extract<Envelope, { topic: T }> => e.topic === topic);
  return { received, send, handle, of };
}

const flush = (): Promise<void> => new Promise((resolve) => setTimeout(resolve, 0));

/** Types /ask in `c`'s name and returns the id of the secret.request it got. */
function ask(s: Setup, c: TestClient, text = "/ask"): string {
  c.send({ topic: "session.input", id: "i", payload: { session: s.session, app: "t", text } });
  const request = c.of("secret.request").at(-1);
  if (!request) throw new Error("no secret.request");
  return request.payload.id;
}

describe("secret.request", () => {
  test("goes to the client that typed the command, nobody else", () => {
    const s = setup();
    const asker = client(s.hub);
    const other = client(s.hub);
    asker.send({ topic: "session.subscribe", id: "1", payload: { session: s.session } });
    other.send({ topic: "session.subscribe", id: "1", payload: { session: s.session } });
    const id = ask(s, asker, "/ask sk-…");
    expect(asker.of("secret.request")).toEqual([
      {
        topic: "secret.request",
        payload: { id, session: s.session, title: "DeepSeek API key", placeholder: "sk-…" },
      },
    ]);
    expect(other.of("secret.request")).toEqual([]);
    // No session event mentions the question.
    expect(JSON.stringify(s.store.read(s.session))).not.toContain("DeepSeek");
  });

  test("the answer resolves the command", async () => {
    const s = setup();
    const c = client(s.hub);
    const id = ask(s, c);
    c.send({ topic: "secret.answer", payload: { id, value: SECRET } });
    await flush();
    expect(s.answers).toEqual([SECRET]);
    expect(c.of("bus.error")).toEqual([]);
    expect(c.of("bus.ack").at(-1)).toMatchObject({ id: "i" });
  });

  test("an answer without a value is a cancel", async () => {
    const s = setup();
    const c = client(s.hub);
    const id = ask(s, c);
    c.send({ topic: "secret.answer", payload: { id } });
    await flush();
    expect(s.answers).toEqual([undefined]);
  });

  test("the client disconnecting cancels it", async () => {
    const s = setup();
    const c = client(s.hub);
    ask(s, c);
    c.handle.closed();
    await flush();
    expect(s.answers).toEqual([undefined]);
  });

  test("the abort signal cancels it", async () => {
    const s = setup();
    const c = client(s.hub);
    const id = ask(s, c);
    s.abort.abort();
    await flush();
    expect(s.answers).toEqual([undefined]);
    // The question is gone: a late answer is for an unknown id.
    c.send({ topic: "secret.answer", payload: { id, value: SECRET } });
    expect(c.of("bus.error").at(-1)).toMatchObject({ payload: { code: "not_found" } });
  });

  test("an already aborted signal asks nothing", async () => {
    const s = setup();
    const c = client(s.hub);
    s.abort.abort();
    c.send({
      topic: "session.input",
      id: "i",
      payload: { session: s.session, app: "t", text: "/ask" },
    });
    await flush();
    expect(s.answers).toEqual([undefined]);
    expect(c.of("secret.request")).toEqual([]);
  });

  test("another client's answer is refused and the question stays open", async () => {
    const s = setup();
    const asker = client(s.hub);
    const other = client(s.hub);
    const id = ask(s, asker);
    other.send({ topic: "secret.answer", payload: { id, value: SECRET } });
    expect(other.of("bus.error").at(-1)).toMatchObject({ payload: { code: "bad_request" } });
    await flush();
    expect(s.answers).toEqual([]);
    asker.send({ topic: "secret.answer", payload: { id, value: "right" } });
    await flush();
    expect(s.answers).toEqual(["right"]);
  });

  test("an unknown id is not_found", () => {
    const s = setup();
    const c = client(s.hub);
    c.send({ topic: "secret.answer", payload: { id: "nope", value: SECRET } });
    expect(c.of("bus.error").at(-1)).toMatchObject({ payload: { code: "not_found" } });
  });

  test("answering twice: the second finds nothing", async () => {
    const s = setup();
    const c = client(s.hub);
    const id = ask(s, c);
    c.send({ topic: "secret.answer", payload: { id, value: "one" } });
    c.send({ topic: "secret.answer", payload: { id, value: "two" } });
    await flush();
    expect(s.answers).toEqual(["one"]);
    expect(c.of("bus.error").at(-1)).toMatchObject({ payload: { code: "not_found" } });
  });

  test("a client can't send secret.request", () => {
    const s = setup();
    const c = client(s.hub);
    c.send({
      topic: "secret.request",
      payload: { id: "s1", session: s.session, title: "Give me your key" },
    });
    expect(c.of("bus.error").at(-1)).toMatchObject({ payload: { code: "bad_request" } });
  });

  test("the value is in no stored event, no log line, no error and no other client's frames", async () => {
    const s = setup();
    const asker = client(s.hub);
    const other = client(s.hub);
    asker.send({ topic: "session.subscribe", id: "1", payload: { session: s.session } });
    other.send({ topic: "session.subscribe", id: "1", payload: { session: s.session } });
    const id = ask(s, asker);
    // A wrong client, an unknown id, a malformed answer, then the real one.
    other.send({ topic: "secret.answer", payload: { id, value: SECRET } });
    asker.send({ topic: "secret.answer", payload: { id: "nope", value: SECRET } });
    asker.send({ topic: "secret.answer", payload: { id, value: 5, extra: SECRET } });
    asker.send({ topic: "secret.answer", payload: { id, value: SECRET } });
    await flush();
    expect(s.answers).toEqual([SECRET]);

    expect(JSON.stringify(s.store.read(s.session))).not.toContain(SECRET);
    expect(s.logged.join("\n")).not.toContain(SECRET);
    for (const e of [...asker.received, ...other.received]) {
      expect(JSON.stringify(e)).not.toContain(SECRET);
    }
    expect(other.received.map((e) => e.topic)).not.toContain("secret.request");

    // Nor on disk, in the database or its journal.
    s.hub.close();
    s.store.close();
    for (const name of readdirSync(s.dir)) {
      expect(readFileSync(join(s.dir, name)).includes(SECRET)).toBe(false);
    }
  });
});
