import { afterEach, describe, expect, test } from "bun:test";
import type {
  Component,
  Envelope,
  SessionEvent,
  ToolKind,
  ViewItem,
  ViewOp,
  ViewPatch,
} from "@neoworks-dev/neo-protocol/types";
import { validateEnvelope } from "@neoworks-dev/neo-protocol/validate";
import { silentLogger } from "../src/log.ts";
import { startTestDaemon, type TestDaemon } from "../src/testing.ts";
import { SessionView } from "../src/view/session-view.ts";
import { type McpApp, startMcpApp } from "./mcp-app.ts";

const cleanups: (() => Promise<void>)[] = [];
afterEach(async () => {
  for (const cleanup of cleanups.splice(0).reverse()) await cleanup();
});

/** What a renderer does with patches: the reference implementation for the tests. */
function applyOps(items: ViewItem[], ops: ViewOp[]): ViewItem[] {
  const out = structuredClone(items);
  for (const op of ops) {
    if (op.op === "set") {
      const at = out.findIndex((i) => i.ref === op.item.ref);
      if (at === -1) out.push(structuredClone(op.item));
      else out[at] = structuredClone(op.item);
    } else if (op.op === "blocks") {
      const item = out.find((i) => i.ref === op.ref);
      if (item?.component.type === "text") {
        item.component.blocks = [...item.component.blocks.slice(0, op.from), ...op.blocks];
      }
    } else {
      const at = out.findIndex((i) => i.ref === op.ref);
      if (at !== -1) out.splice(at, 1);
    }
  }
  return out;
}

function feed(
  events: SessionEvent[],
  templates: Record<string, unknown> = {},
): { view: SessionView; ops: ViewOp[][] } {
  const view = new SessionView("s", (tool) => templates[tool], silentLogger);
  const ops = events.map((event, i) => view.apply({ session: "s", seq: i + 1, ts: 0, event }));
  return { view, ops };
}

const chunks = (messageId: string, text: string): SessionEvent[] =>
  text.match(/[\s\S]{1,7}/g)?.map((t) => ({ type: "agent_message_chunk", messageId, text: t })) ??
  [];

describe("session view", () => {
  test("a call the interrupted turn never ended stops running with it", () => {
    const { view } = feed([
      { type: "user_message", app: "cli", text: "build" },
      {
        type: "tool_call",
        toolCallId: "t1",
        title: "Terminal",
        kind: "execute",
        status: "pending",
        rawInput: { command: "cargo build" },
      },
      { type: "turn_ended", stopReason: "cancelled" },
    ]);
    const call = view.snapshot().items.find((i) => i.ref === "c-t1");
    expect(call?.state).toBe("failed");
  });

  test("streaming text sends only changed tail blocks, and patches rebuild the snapshot", () => {
    const reply = "# Plan\n\nFirst I'll copy the file.\n\n- one\n- two\n\nDone **now**.";
    const events: SessionEvent[] = [
      { type: "user_message", app: "cli", text: "do it" },
      ...chunks("m1", reply),
    ];
    const { view, ops } = feed(events);
    const replayed = applyOps([], ops.flat());
    expect(replayed).toEqual(view.snapshot().items);
    expect(view.snapshot().items.map((i) => i.kind)).toEqual(["user", "agent"]);
    // After the heading is complete, later chunks never resend it.
    const late = ops.at(-1)?.[0];
    expect(late).toMatchObject({ op: "blocks", ref: "m-m1" });
    expect(late?.op === "blocks" && late.from).toBeGreaterThan(0);
  });

  test("a view built from the log is the one its events build one by one, and streams on alike", () => {
    const events: SessionEvent[] = [
      { type: "user_message", app: "cli", text: "do it" },
      ...chunks("m1", "# Plan\n\nFirst I'll copy the file.\n\n- one\n- two"),
      { type: "user_message", app: "cli", text: "and then?" },
      ...chunks("m2", "Then **this**.\n\n```ts\nconst a = 1;\n```"),
    ];
    const records = events.map((event, i) => ({ session: "s", seq: i + 1, ts: 0, event }));
    const { view: live } = feed(events);
    const replayed = new SessionView("s", () => undefined, silentLogger);
    replayed.replay(records);
    expect(replayed.snapshot()).toEqual(live.snapshot());

    const more: SessionEvent = { type: "agent_message_chunk", messageId: "m2", text: "\n\nDone." };
    const next = { session: "s", seq: records.length + 1, ts: 0, event: more };
    expect(replayed.apply(next)).toEqual(live.apply(next));
    expect(replayed.snapshot()).toEqual(live.snapshot());
  });

  test("tool results use the app's template, or a fallback from text, diffs or structured data", () => {
    const template = { type: "kv", items: { Notes: "{{data.count}}" } };
    const { view } = feed(
      [
        {
          type: "tool_call",
          toolCallId: "a",
          tool: "notes.count",
          title: "notes.count",
          kind: "other",
          status: "pending",
        },
        { type: "tool_call_update", toolCallId: "a", status: "completed", rawOutput: { count: 2 } },
        {
          type: "tool_call",
          toolCallId: "b",
          tool: "bash",
          title: "bash: ls",
          kind: "execute",
          status: "pending",
        },
        { type: "tool_call_update", toolCallId: "b", status: "in_progress" },
        {
          type: "tool_call_update",
          toolCallId: "b",
          status: "failed",
          content: [{ type: "text", text: "ls: no such file\n" }],
        },
        {
          type: "tool_call",
          toolCallId: "c",
          tool: "write",
          title: "write a.txt",
          kind: "edit",
          status: "pending",
        },
        {
          type: "tool_call_update",
          toolCallId: "c",
          status: "completed",
          content: [{ type: "diff", path: "/w/a.txt", newText: "hi\n" }],
        },
        {
          type: "tool_call",
          toolCallId: "d",
          tool: "notes.bad",
          title: "notes.bad",
          kind: "other",
          status: "pending",
        },
        {
          type: "tool_call_update",
          toolCallId: "d",
          status: "completed",
          rawOutput: { ok: true, n: 1 },
        },
      ],
      { "notes.count": template, "notes.bad": { type: "nonsense" } },
    );
    const [count, bash, write, bad] = view.snapshot().items;
    expect(count).toEqual({
      ref: "c-a",
      kind: "tool",
      label: "notes.count",
      state: "done",
      component: { type: "kv", items: [{ key: "Notes", value: "2" }] },
    });
    expect(bash).toMatchObject({
      state: "failed",
      component: { type: "code", code: "ls: no such file" },
    });
    expect(write?.component).toMatchObject({
      type: "diff",
      path: "/w/a.txt",
      lines: [{ kind: "hunk" }, { kind: "add", text: "hi", new: 1 }],
    });
    // A broken template falls back instead of failing the view.
    expect(bad?.component).toEqual({
      type: "kv",
      items: [
        { key: "ok", value: "true" },
        { key: "n", value: "1" },
      ],
    });
  });

  test("a user's images go where their markers are, the others after the text", () => {
    const { view } = feed([
      {
        type: "user_message",
        app: "terminal",
        text: "compare [image 2] with\n\n[image 1] please",
        images: ["/a.png", "/b.png", "/c.png"],
      },
      { type: "user_message", app: "terminal", text: "just text" },
    ]);
    const [placed, plainText] = view.snapshot().items;
    const para = (t: string): Component => ({
      type: "text",
      blocks: [{ type: "paragraph", spans: [{ text: t }] }],
    });
    expect(placed?.component).toEqual({
      type: "stack",
      gap: "s",
      children: [
        para("compare"),
        { type: "image", src: "/b.png" },
        para("with"),
        { type: "image", src: "/a.png" },
        para("please"),
        { type: "image", src: "/c.png" },
      ],
    });
    expect(plainText?.component).toEqual(para("just text"));
  });

  test("a call that runs a command carries it; a title that only repeats it gives way", () => {
    const call = (
      id: string,
      title: string,
      rawInput: unknown,
      kind: ToolKind = "execute",
    ): Extract<SessionEvent, { type: "tool_call" }> => ({
      type: "tool_call",
      toolCallId: id,
      title,
      kind,
      status: "pending",
      rawInput,
    });
    const codex: SessionEvent = {
      ...call("e", "npm test", { command: "npm test" }),
      agent: "codex",
    };
    const { view } = feed([
      call("a", "ls -la && pwd", { command: "ls -la && pwd" }),
      call("b", "bash: git status", { command: "git status --short" }),
      call("c", "Show the branch", { command: "git branch" }),
      call("d", "build", { command: ["bash", "-lc", "make build"] }),
      codex,
      call("f", "read a.txt", { path: "a.txt" }, "read"),
    ]);
    const items = view.snapshot().items.map(({ label, command }) => ({ label, command }));
    expect(items).toEqual([
      { label: undefined, command: "ls -la && pwd" },
      { label: undefined, command: "git status --short" },
      { label: "Show the branch", command: "git branch" },
      { label: "build", command: "make build" },
      { label: "codex", command: "npm test" },
      { label: "read a.txt", command: undefined },
    ]);
  });

  test("a started call is running; its description is the label until there is output", () => {
    const input = { command: "make check", description: "Run the tests" };
    const { view } = feed([
      { type: "tool_call", toolCallId: "a", title: "Terminal", kind: "execute", status: "pending" },
      {
        type: "tool_call_update",
        toolCallId: "a",
        title: "make check",
        rawInput: input,
        content: [{ type: "text", text: "Run the tests" }],
      },
    ]);
    expect(view.snapshot().items[0]).toMatchObject({
      label: "Run the tests",
      command: "make check",
      state: "running",
      component: { type: "stack", children: [] },
    });
    const done = feed([
      {
        type: "tool_call",
        toolCallId: "a",
        title: "make check",
        kind: "execute",
        status: "pending",
        rawInput: input,
      },
      {
        type: "tool_call_update",
        toolCallId: "a",
        status: "completed",
        content: [{ type: "text", text: "```console\n3 pass\n```" }],
      },
    ]).view.snapshot().items[0];
    expect(done).toMatchObject({
      state: "done",
      description: "Run the tests",
      component: { type: "code", code: "3 pass" },
    });
    expect(done?.label).toBeUndefined();
  });

  test("a command whose output is recognized shows as what it is, once it has ended", () => {
    const item = feed([
      {
        type: "tool_call",
        toolCallId: "a",
        title: "git status",
        kind: "execute",
        status: "pending",
        rawInput: { command: "git status --short" },
      },
      {
        type: "tool_call_update",
        toolCallId: "a",
        status: "completed",
        content: [{ type: "text", text: "```console\n M a.rs\n?? notes/\n```" }],
      },
    ]).view.snapshot().items[0];
    expect(item?.component).toEqual({
      type: "neoui",
      ui: {
        id: "0",
        streaming: false,
        root: {
          component: "FileTree",
          props: {
            files: [
              { component: "File", props: { path: "a.rs", status: "modified" } },
              { component: "File", props: { path: "notes/", status: "untracked" } },
            ],
          },
        },
      },
    });
  });

  test("output in one markdown code block, as Claude Code sends it, is the output", () => {
    const { view } = feed([
      {
        type: "tool_call",
        toolCallId: "a",
        title: "git status",
        kind: "execute",
        status: "pending",
      },
      {
        type: "tool_call_update",
        toolCallId: "a",
        status: "completed",
        content: [{ type: "text", text: "```console\n?? .claude/\n```" }],
      },
    ]);
    expect(view.snapshot().items[0]?.component).toEqual({ type: "code", code: "?? .claude/" });
  });

  test("a plan is one checklist per agent, replaced in place; usage is not shown", () => {
    const first: SessionEvent = {
      type: "plan",
      entries: [
        { content: "read", status: "completed", priority: "high" },
        { content: "write", status: "in_progress" },
        { content: "test", status: "pending" },
      ],
    };
    const { view, ops } = feed([
      first,
      { type: "usage", contextUsed: 10, contextSize: 100, cost: 0.1 },
      { type: "plan", agent: "claude", entries: [{ content: "fix", status: "pending" }] },
      {
        type: "plan",
        entries: [
          { content: "read", status: "completed" },
          { content: "write", status: "completed" },
          { content: "test", status: "completed" },
        ],
      },
    ]);
    expect(ops[1]).toEqual([]);
    const items = view.snapshot().items;
    expect(items.map((i) => [i.ref, i.kind, i.label, i.state])).toEqual([
      ["plan", "tool", "Plan", "done"],
      ["plan-claude", "tool", "claude: Plan", "pending"],
    ]);
    expect(items[0]?.component).toEqual({
      type: "text",
      blocks: [
        {
          type: "list",
          ordered: false,
          items: ["read", "write", "test"].map((content) => ({
            checked: true,
            blocks: [{ type: "paragraph", spans: [{ text: content }] }],
          })),
        },
      ],
    });
    // The first plan, as drawn while work was going on.
    const running = feed([first]).view.snapshot().items[0];
    expect(running?.state).toBe("running");
    expect(running?.component).toMatchObject({
      blocks: [
        {
          items: [
            { checked: true, blocks: [{ spans: [{ text: "read" }] }] },
            { checked: false, blocks: [{ spans: [{ text: "◐ write", bold: true }] }] },
            { checked: false, blocks: [{ spans: [{ text: "test" }] }] },
          ],
        },
      ],
    });
  });

  test("an empty plan removes the item", () => {
    const { view, ops } = feed([
      { type: "plan", entries: [] },
      { type: "plan", entries: [{ content: "a", status: "pending" }] },
      { type: "plan", entries: [] },
    ]);
    expect(ops.map((o) => o.map((op) => op.op))).toEqual([[], ["set"], ["remove"]]);
    expect(view.snapshot().items).toEqual([]);
  });

  test("settings show once a prompt was sent; before, they are how the session starts", () => {
    const { view } = feed([
      { type: "setting_changed", harness: "daemon", option: "harness", value: "claude" },
      { type: "setting_changed", harness: "claude", option: "model", value: "opus" },
      { type: "user_message", app: "cli", text: "hi" },
      { type: "setting_changed", harness: "claude", option: "effort", value: "high" },
    ]);
    const shown = view.snapshot().items.map((i) => JSON.stringify(i.component));
    expect(shown).toEqual([
      expect.stringContaining("hi"),
      expect.stringContaining("claude effort: high"),
    ]);
  });

  test("apps coming and going share one line until something else is said", () => {
    const { view } = feed([
      { type: "app_disconnected", app: "terminal-2" },
      { type: "app_connected", app: "terminal", kind: "app", v: 0, tools: [], reconnect: true },
      { type: "focus_changed", app: "terminal" },
      { type: "app_connected", app: "terminal-2", kind: "app", v: 0, tools: [], reconnect: true },
      { type: "user_message", app: "terminal", text: "hi" },
      { type: "app_disconnected", app: "terminal-2" },
    ]);
    const texts = view
      .snapshot()
      .items.map((i) => (i.component.type === "text" ? i.component.blocks : undefined))
      .map((blocks) => JSON.stringify(blocks));
    expect(texts[0]).toContain('"terminal is back · now in terminal · terminal-2 is back"');
    expect(texts[2]).toContain('"terminal-2 disconnected"');
    expect(texts).toHaveLength(3);
  });

  test("connector events, focus and failed turns become notices; harness messages don't show", () => {
    const { view } = feed([
      { type: "app_connected", app: "pad", kind: "app", v: 1, tools: [] },
      { type: "focus_changed", app: "pad", context: { file: "a.txt" } },
      { type: "harness_message", harness: "neo", message: {} },
      { type: "turn_ended", stopReason: "end_turn" },
      { type: "turn_ended", stopReason: "error", error: "rate limited" },
    ]);
    expect(view.snapshot().items.map((i) => [i.kind, i.component])).toEqual([
      [
        "notice",
        {
          type: "text",
          tone: "muted",
          blocks: [{ type: "paragraph", spans: [{ text: "pad connected (v1):  · now in pad" }] }],
        },
      ],
      [
        "notice",
        {
          type: "text",
          tone: "danger",
          blocks: [{ type: "paragraph", spans: [{ text: "rate limited" }] }],
        },
      ],
    ]);
  });
});

describe("view over the bus", () => {
  interface Client {
    received: Envelope[];
    send: (e: unknown) => void;
  }

  function connect(daemon: TestDaemon): Client {
    const received: Envelope[] = [];
    const handle = daemon.hub.connect({
      send(frame) {
        const result = validateEnvelope(JSON.parse(frame));
        if (!result.ok) throw new Error(`invalid frame: ${result.error}\n${frame}`);
        received.push(result.value);
      },
    });
    const send = (e: unknown): void => handle.receive(JSON.stringify(e));
    send({ topic: "bus.hello", payload: { protocol: 1, client: "test" } });
    return { received, send };
  }

  async function daemonWithNotes(): Promise<{ daemon: TestDaemon; app: McpApp; owner: object }> {
    const daemon = await startTestDaemon();
    cleanups.push(() => daemon.stop());
    const app = startMcpApp({
      tools: [
        {
          name: "add",
          description: "Add a note",
          inputSchema: { type: "object", properties: { text: { type: "string" } } },
          handler: (args) => ({
            content: [{ type: "text", text: "added" }],
            structuredContent: { text: String(args.text), total: 3 },
          }),
        },
      ],
    });
    cleanups.push(() => app.close());
    const owner = {};
    await daemon.connectors.presence(owner, {
      app: "notes",
      kind: "app",
      open: true,
      mcp: app.endpoint,
      templates: {
        add: {
          type: "card",
          title: "Added “{{data.text}}”",
          children: [
            { type: "text", markdown: "{{data.total}} notes" },
            {
              type: "button",
              label: "Add another",
              action: "notes.add",
              args: { text: "again" },
              key: "a",
            },
          ],
        },
      },
    });
    return { daemon, app, owner };
  }

  test("welcome brings the theme and settings; view.subscribe gives a snapshot, then patches", async () => {
    const daemon = await startTestDaemon();
    cleanups.push(() => daemon.stop());
    const c = connect(daemon);
    expect(c.received.map((e) => e.topic)).toEqual([
      "bus.welcome",
      "agent.focus",
      "theme",
      "user.settings",
    ]);

    const session = daemon.focus.current().session;
    c.send({ topic: "view.subscribe", id: "v", payload: { session } });
    expect(c.received.at(-1)).toEqual({
      topic: "view.snapshot",
      id: "v",
      payload: { session, seq: 0, items: [] },
    });
    daemon.sessions.input(session, "cli", "hello");
    await daemon.sessions.idle(session);
    const patches: ViewPatch[] = [];
    for (const e of c.received) if (e.topic === "view.patch") patches.push(e.payload);
    const items = applyOps(
      [],
      patches.flatMap((p) => p.ops),
    );
    expect(items.map((i) => i.kind)).toEqual(["user", "agent"]);
    c.send({ topic: "view.subscribe", id: "v2", payload: { session } });
    const fresh = c.received.at(-1);
    expect(fresh?.topic === "view.snapshot" && fresh.payload.items).toEqual(items);
  });

  test("ui.action runs an open app's tool and shows the result with its template", async () => {
    const { daemon, app } = await daemonWithNotes();
    const c = connect(daemon);
    const session = daemon.focus.current().session;
    c.send({ topic: "view.subscribe", id: "v", payload: { session } });
    c.send({
      topic: "ui.action",
      id: "a",
      payload: { session, ref: "x", action: "notes.add", args: { text: "milk" } },
    });
    for (let i = 0; i < 100 && !c.received.some((e) => e.topic === "bus.ack"); i++)
      await Bun.sleep(10);
    expect(app.calls).toEqual([{ name: "add", args: { text: "milk" } }]);
    const items = daemon.views.get(session).snapshot().items;
    expect(items.at(-1)).toMatchObject({
      kind: "tool",
      label: "notes.add",
      state: "done",
      component: { type: "card", title: "Added “milk”" },
    });
  });

  test("an action for a closed app goes to the agent as input; copy: is refused", async () => {
    const { daemon, app, owner } = await daemonWithNotes();
    const session = daemon.focus.current().session;
    await daemon.connectors.disconnect(owner);
    await app.close();
    const c = connect(daemon);
    // Pretend an earlier result had the button.
    daemon.store.append(session, {
      type: "tool_call",
      toolCallId: "t",
      tool: "notes.add",
      title: "notes.add",
      kind: "other",
      status: "pending",
    });
    daemon.store.append(session, {
      type: "tool_call_update",
      toolCallId: "t",
      status: "completed",
      rawOutput: { text: "x", total: 1 },
    });
    c.send({
      topic: "ui.action",
      id: "a",
      payload: { session, ref: "c-t", action: "notes.add", args: { text: "again" } },
    });
    await daemon.sessions.idle(session);
    for (let i = 0; i < 100 && !c.received.some((e) => e.topic === "bus.ack"); i++)
      await Bun.sleep(10);
    const input = daemon.store.readTypes(session, ["user_message"]).at(-1)?.event;
    expect(input).toMatchObject({
      type: "user_message",
      app: "test",
      text: '[ui action] The user pressed "Add another" (notes.add {"text":"again"}) in an earlier result; notes isn\'t open.',
    });

    c.send({ topic: "ui.action", id: "b", payload: { session, ref: "c-t", action: "copy:hi" } });
    for (let i = 0; i < 100 && !c.received.some((e) => e.topic === "bus.error"); i++)
      await Bun.sleep(10);
    expect(c.received.at(-1)).toMatchObject({
      topic: "bus.error",
      id: "b",
      payload: { code: "bad_request" },
    });
  });
});

describe("permission requests", () => {
  test("a command to run is previewed as bash code", () => {
    const { view } = feed([
      {
        type: "permission_request",
        id: "r1",
        agent: "neo",
        title: "Run this command?",
        kind: "execute",
        content: [{ type: "text", text: "cd /tmp && ls\n" }],
        options: [{ id: "allow", name: "Allow", kind: "allow_once" }],
      },
    ]);
    const [item] = view.snapshot().items;
    expect(item?.label).toBe("Run this command?");
    if (item?.component.type !== "stack") throw new Error("no preview");
    expect(item.component.children[0]).toEqual({
      type: "code",
      code: "cd /tmp && ls",
      language: "bash",
    });
  });
});
