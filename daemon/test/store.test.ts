import { describe, expect, test } from "bun:test";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { AgentMessageChunk, SessionEventRecord } from "@neoworks-dev/neo-protocol/types";
import { SessionStore } from "../src/store.ts";

const chunk = (text: string): AgentMessageChunk => ({
  type: "agent_message_chunk",
  messageId: "m",
  text,
});

describe("SessionStore", () => {
  test("a title knows where it came from; one without a source is the user's", () => {
    const dir = mkdtempSync(join(tmpdir(), "suite-store-"));
    const path = join(dir, "events.db");
    const store = new SessionStore(path);
    store.createSession({ id: "a", cwd: "/", title: "Given" });
    store.createSession({ id: "b", cwd: "/" });
    expect(store.getSession("a")).toMatchObject({ title: "Given" });
    expect(store.getSession("a")?.titleSource).toBeUndefined();
    expect(store.getSession("b")?.title).toBeUndefined();
    store.setTitle("b", "The start of it", "first_line");
    expect(store.getSession("b")).toMatchObject({
      title: "The start of it",
      titleSource: "first_line",
    });
    store.setTitle("b", "Written by a model", "generated");
    store.setTitle("a", "Typed by the user");
    store.close();
    const again = new SessionStore(path);
    expect(again.getSession("b")).toMatchObject({
      title: "Written by a model",
      titleSource: "generated",
    });
    expect(again.getSession("a")?.titleSource).toBeUndefined();
    again.close();
  });

  test("sequence numbers start at 1 and are gap-free per session", () => {
    const store = new SessionStore(":memory:");
    store.createSession({ id: "a", cwd: "/" });
    store.createSession({ id: "b", cwd: "/" });
    const seqs = { a: [] as number[], b: [] as number[] };
    for (let i = 0; i < 50; i++) {
      const id = i % 3 === 0 ? "b" : "a";
      seqs[id].push(store.append(id, chunk(String(i))).seq);
    }
    expect(seqs.a).toEqual(seqs.a.map((_, i) => i + 1));
    expect(seqs.b).toEqual(seqs.b.map((_, i) => i + 1));
    expect(store.getSession("a")?.lastSeq).toBe(seqs.a.length);
  });

  test("read returns events from a seq, in order, with their payloads", () => {
    const store = new SessionStore(":memory:");
    store.createSession({ id: "a", cwd: "/" });
    for (const t of ["x", "y", "z"]) store.append("a", chunk(t), 1000);
    expect(store.read("a", 2)).toEqual([
      { session: "a", seq: 2, ts: 1000, event: chunk("y") },
      { session: "a", seq: 3, ts: 1000, event: chunk("z") },
    ]);
    expect(store.read("a", 4)).toEqual([]);
  });

  test("append to an unknown session throws and writes nothing", () => {
    const store = new SessionStore(":memory:");
    expect(() => store.append("nope", chunk("x"))).toThrow();
  });

  test("listeners see each record after commit, in seq order", () => {
    const store = new SessionStore(":memory:");
    store.createSession({ id: "a", cwd: "/" });
    const seen: SessionEventRecord[] = [];
    store.onAppend((r) => {
      // Already readable when the listener runs.
      expect(store.read("a", r.seq)[0]?.seq).toBe(r.seq);
      seen.push(r);
    });
    store.append("a", chunk("1"));
    store.append("a", chunk("2"));
    expect(seen.map((r) => r.seq)).toEqual([1, 2]);
  });

  test("sessions, events and the counter survive reopening", () => {
    const path = join(mkdtempSync(join(tmpdir(), "suite-store-")), "s.db");
    const first = new SessionStore(path);
    first.createSession({ id: "a", cwd: "/w", title: "T", now: 5 });
    first.append("a", chunk("1"), 10);
    first.append("a", chunk("2"), 20);
    first.close();

    const second = new SessionStore(path);
    expect(second.getSession("a")).toEqual({
      id: "a",
      title: "T",
      cwd: "/w",
      createdAt: 5,
      lastActivity: 20,
      lastSeq: 2,
    });
    expect(second.append("a", chunk("3")).seq).toBe(3);
    expect(second.read("a").map((r) => r.seq)).toEqual([1, 2, 3]);
    second.close();
  });

  test("listSessions orders by last activity", () => {
    const store = new SessionStore(":memory:");
    store.createSession({ id: "old", cwd: "/", now: 1 });
    store.createSession({ id: "new", cwd: "/", now: 2 });
    store.append("old", chunk("x"), 3);
    expect(store.listSessions().map((s) => s.id)).toEqual(["old", "new"]);
  });

  test("lastSeqOfType", () => {
    const store = new SessionStore(":memory:");
    store.createSession({ id: "a", cwd: "/" });
    expect(store.lastSeqOfType("a", "user_message")).toBe(0);
    store.append("a", { type: "user_message", app: "cli", text: "hi" });
    store.append("a", chunk("x"));
    expect(store.lastSeqOfType("a", "user_message")).toBe(1);
  });
});
