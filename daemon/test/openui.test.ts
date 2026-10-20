import { afterEach, describe, expect, test } from "bun:test";
import type {
  Block,
  SessionEvent,
  UiDoc,
  ViewItem,
  ViewOp,
  ViewPatch,
} from "@suite/protocol/types";
import { silentLogger } from "../src/log.ts";
import { startTestDaemon, type TestDaemon } from "../src/testing.ts";
import { parseMarkdown } from "../src/view/markdown.ts";
import { SessionView } from "../src/view/session-view.ts";
import { startMcpApp, type McpApp } from "./mcp-app.ts";

const cleanups: (() => Promise<void>)[] = [];
afterEach(async () => {
  for (const cleanup of cleanups.splice(0).reverse()) await cleanup();
});

const PROGRAM = 'root = Card([h], "T")\nh = Header("Hello")';
const fence = (program: string, lang = "openui"): string => `\`\`\`${lang}\n${program}\n\`\`\``;

const docs = (blocks: Block[]): UiDoc[] =>
  blocks.filter((b) => b.type === "openui").map((b) => b.ui);

/** The blocks of an item that is text. */
function blocksOf(item: ViewItem | undefined): Block[] {
  if (item?.component.type === "text") return item.component.blocks;
  return [];
}

describe("openui fences in markdown", () => {
  const host = (index: number, program: string, closed: boolean): UiDoc => ({
    id: String(index),
    root: { component: "Header", props: { title: `${program.length}${closed ? "c" : "o"}` } },
  });

  test("a top-level fence becomes an openui block among the others; they are numbered in order", () => {
    const blocks = parseMarkdown(
      `Before.\n\n${fence("a = 1")}\n\nBetween.\n\n${fence("bb = 2", "openui-lang")}\n\nAfter.`,
      host,
    );
    expect(blocks.map((b) => b.type)).toEqual([
      "paragraph",
      "openui",
      "paragraph",
      "openui",
      "paragraph",
    ]);
    expect(docs(blocks).map((d) => d.id)).toEqual(["0", "1"]);
  });

  test("an open fence is not closed, and a half-written closing fence is not program", () => {
    const seen: [string, boolean][] = [];
    const record = (_i: number, program: string, closed: boolean): UiDoc => {
      seen.push([program, closed]);
      return { id: "0" };
    };
    parseMarkdown("```openui\nroot = A(1)\n", record);
    parseMarkdown("```openui\nroot = A(1)\n``", record);
    parseMarkdown("```openui\nroot = A(1)\n```", record);
    parseMarkdown('```openui\nroot = A("``")\n```\n\nand', record);
    expect(seen).toEqual([
      ["root = A(1)", false],
      ["root = A(1)", false],
      ["root = A(1)", true],
      ['root = A("``")', true],
    ]);
  });

  test("other fences, fences without a host and fences inside lists stay code", () => {
    expect(parseMarkdown(fence("x", "js"), host)[0]?.type).toBe("code_block");
    expect(parseMarkdown(fence(PROGRAM))[0]?.type).toBe("code_block");
    const nested = parseMarkdown(`- item\n\n  ${fence("x").replaceAll("\n", "\n  ")}`, host);
    expect(JSON.stringify(nested)).not.toContain('"type":"openui"');
  });
});

function view(events: SessionEvent[]): { view: SessionView; ops: ViewOp[][] } {
  const v = new SessionView("s", () => undefined, silentLogger);
  const ops = events.map((event, i) => v.apply({ session: "s", seq: i + 1, ts: 0, event }));
  return { view: v, ops };
}

const chunk = (text: string, messageId = "m1"): SessionEvent => ({
  type: "agent_message_chunk",
  messageId,
  text,
});

describe("a streamed message with openui", () => {
  const text = `Here:\n\n${fence(PROGRAM)}\n\nDone.`;

  test("the doc streams until its fence closes, and only blocks from the first change are sent", () => {
    const cut = text.indexOf("Hello");
    const { view: v, ops } = view([chunk(text.slice(0, cut)), chunk(text.slice(cut))]);
    const first = v.snapshot().items[0];
    expect(ops[0]?.[0]).toMatchObject({ op: "set" });
    const second = ops[1]?.[0];
    expect(second).toMatchObject({ op: "blocks", from: 1 });
    expect(docs(blocksOf(first))[0]).toMatchObject({
      id: "0",
      root: { component: "Card" },
    });
    expect(JSON.stringify(v.snapshot())).not.toContain("streaming");
  });

  test("a doc whose fence is still open is streaming; the turn ending finishes it", () => {
    const open = `Here:\n\n\`\`\`openui\n${PROGRAM}\n`;
    const { view: v, ops } = view([chunk(open)]);
    const blocks = (): Block[] => blocksOf(v.snapshot().items[0]);
    expect(docs(blocks())[0]?.streaming).toBe(true);
    const end = v.apply({
      session: "s",
      seq: 2,
      ts: 0,
      event: { type: "turn_ended", stopReason: "end_turn" },
    });
    expect(end).toMatchObject([{ op: "blocks", from: 1 }]);
    expect(docs(blocks())[0]?.streaming).toBeUndefined();
    expect(ops).toHaveLength(1);
  });

  test("thoughts and messages between agents keep their fences as code", () => {
    const { view: v } = view([
      { type: "agent_thought_chunk", messageId: "t1", text: fence(PROGRAM) },
    ]);
    expect(JSON.stringify(v.snapshot())).not.toContain('"type":"openui"');
  });

  test("a doc with a broken program draws nothing and says why", () => {
    const { view: v } = view([
      chunk(fence("root = Nope(1)")),
      { type: "turn_ended", stopReason: "end_turn" },
    ]);
    const ui = docs(blocksOf(v.snapshot().items[0]))[0];
    expect(ui?.root).toBeUndefined();
    expect(ui?.errors?.[0]?.code).toBe("unknown-component");
  });
});

describe("app templates in OpenUI Lang", () => {
  const toolEvents = (rawOutput: unknown): SessionEvent[] => [
    {
      type: "tool_call",
      toolCallId: "t",
      tool: "stats.show",
      title: "stats.show",
      kind: "other",
      status: "pending",
    },
    { type: "tool_call_update", toolCallId: "t", status: "completed", rawOutput },
  ];

  test("the tool's data is the program's last statement", () => {
    const templates = {
      "stats.show": {
        openui: 'root = Card([Metric("Total", data.total), Text(data.note)])\ndata = {total: 0}',
      },
    };
    const v = new SessionView(
      "s",
      (tool) => templates[tool as keyof typeof templates],
      silentLogger,
    );
    toolEvents({ total: 7, note: "**ok**" }).forEach((event, i) =>
      v.apply({ session: "s", seq: i + 1, ts: 0, event }),
    );
    const component = v.snapshot().items[0]?.component;
    expect(component?.type).toBe("openui");
    const root = component?.type === "openui" ? component.ui.root : undefined;
    expect(root?.props.children).toMatchObject([
      { component: "Metric", props: { value: 7 } },
      { component: "Text", props: { text: "**ok**", blocks: [{ type: "paragraph" }] } },
    ]);
  });

  test("a template that draws nothing falls back to the generic card", () => {
    const v = new SessionView("s", () => ({ openui: 'title = Header("no root")' }), silentLogger);
    toolEvents({ total: 7 }).forEach((event, i) =>
      v.apply({ session: "s", seq: i + 1, ts: 0, event }),
    );
    expect(v.snapshot().items[0]?.component).toEqual({
      type: "kv",
      items: [{ key: "total", value: "7" }],
    });
  });
});

describe("using a doc", () => {
  async function setup(): Promise<{ daemon: TestDaemon; session: string; app: McpApp }> {
    const daemon = await startTestDaemon();
    cleanups.push(() => daemon.stop());
    const app = startMcpApp({
      tools: [
        {
          name: "list",
          description: "List notes",
          inputSchema: { type: "object", properties: {} },
          handler: () => ({
            content: [{ type: "text", text: "2 notes" }],
            structuredContent: { notes: [{ text: "milk" }, { text: "eggs" }] },
          }),
        },
        {
          name: "add",
          description: "Add a note",
          inputSchema: { type: "object", properties: { text: { type: "string" } } },
          handler: (args) => ({
            content: [{ type: "text", text: "added" }],
            structuredContent: { text: String(args.text) },
          }),
        },
      ],
    });
    cleanups.push(() => app.close());
    await daemon.connectors.presence(
      {},
      { app: "notes", kind: "app", open: true, mcp: app.endpoint },
    );
    return { daemon, session: daemon.focus.current().session, app };
  }

  const reply = (daemon: TestDaemon, session: string, program: string): void => {
    daemon.store.append(session, chunk(`Look:\n\n${fence(program)}`));
    daemon.store.append(session, { type: "turn_ended", stopReason: "end_turn" });
  };

  const doc = (daemon: TestDaemon, session: string): UiDoc | undefined => {
    const item = daemon.views
      .get(session)
      .snapshot()
      .items.find((i) => i.ref === "m-m1");
    return docs(blocksOf(item))[0];
  };

  test("typing and pressing: the item is patched, and the agent is told with the form's values", async () => {
    const { daemon, session } = await setup();
    const patches: ViewPatch[] = [];
    daemon.views.get(session);
    daemon.views.onPatch((p) => patches.push(p));
    reply(
      daemon,
      session,
      [
        'root = Form("ask", [name], Buttons([go]))',
        'name = FormControl("Name", Input("name", null, null, { required: true }))',
        'go = Button("Send", @ToAssistant("Here you go", { id: 3 }))',
      ].join("\n"),
    );
    const act = (action: string, args: unknown): Promise<void> =>
      daemon.actions.run({ session, ref: "m-m1", action, args }, "test");

    // Pressing with the field empty: nothing is sent, the field says why.
    await act("openui.press", { doc: "0", key: "go" });
    expect(daemon.store.readTypes(session, ["user_message"])).toHaveLength(0);
    expect(JSON.stringify(doc(daemon, session))).toContain("This field is required");
    const failed = patches.at(-1);
    expect(failed?.ops[0]).toMatchObject({ op: "set", item: { ref: "m-m1" } });

    await act("openui.input", { doc: "0", key: "name", value: "Ada" });
    expect(JSON.stringify(doc(daemon, session))).not.toContain("This field is required");
    expect(JSON.stringify(doc(daemon, session))).toContain('"value":"Ada"');

    await act("openui.press", { doc: "0", key: "go" });
    await daemon.sessions.idle(session);
    const sent = daemon.store.readTypes(session, ["user_message"]).at(-1)?.event;
    expect(sent).toMatchObject({
      type: "user_message",
      text: '[ui action] The user pressed "Send" in an OpenUI doc: Here you go\nform ask: {"name":"Ada"}\ncontext: {"id":3}',
    });
  });

  test("actions on a doc that is still streaming are ignored; bad ones are rejected", async () => {
    const { daemon, session } = await setup();
    daemon.store.append(session, chunk('```openui\nroot = Stack([f])\nf = Input("f")\n'));
    const run = (action: string, args: unknown, ref = "m-m1"): Promise<void> =>
      daemon.actions.run({ session, ref, action, args }, "test");
    await run("openui.input", { doc: "0", key: "f", value: "x" });
    expect(JSON.stringify(doc(daemon, session))).not.toContain('"value"');
    expect(run("openui.input", { doc: "7", key: "f", value: "x" })).rejects.toThrow(
      "no OpenUI doc",
    );
    expect(run("openui.press", { key: "f" })).rejects.toThrow("needs args");
    expect(run("openui.press", { doc: "0", key: "f" }, "nope")).rejects.toThrow("no OpenUI doc");
  });

  test("Query reads through the gateway; a Mutation is recorded like a tool call and refreshes", async () => {
    const { daemon, session, app } = await setup();
    const patches: ViewPatch[] = [];
    daemon.views.get(session);
    daemon.views.onPatch((p) => patches.push(p));
    reply(
      daemon,
      session,
      [
        "root = Stack([list, go])",
        'data = Query("notes.list", {}, { notes: [] })',
        'list = List(@Each(data.notes, "n", n.text))',
        'save = Mutation("notes.add", { text: "bread" })',
        'go = Button("Add", Action([@Run(save), @Run(data)]))',
      ].join("\n"),
    );
    for (let i = 0; i < 100 && !JSON.stringify(doc(daemon, session)).includes("milk"); i++) {
      await Bun.sleep(10);
    }
    expect(JSON.stringify(doc(daemon, session))).toContain('"items":["milk","eggs"]');
    // The refresh went out as a patch, and no tool call was recorded for a read.
    expect(patches.some((p) => p.ops.some((o) => o.op === "set" && o.item.ref === "m-m1"))).toBe(
      true,
    );
    expect(daemon.store.readTypes(session, ["tool_call"])).toHaveLength(0);

    await daemon.actions.run(
      { session, ref: "m-m1", action: "openui.press", args: { doc: "0", key: "go" } },
      "test",
    );
    // @Run(data) fetched the list again.
    for (let i = 0; i < 100 && app.calls.filter((c) => c.name === "list").length < 2; i++) {
      await Bun.sleep(10);
    }
    await Bun.sleep(20);
    expect(app.calls.filter((c) => c.name === "add")).toEqual([
      { name: "add", args: { text: "bread" } },
    ]);
    expect(daemon.store.readTypes(session, ["tool_call"]).map((r) => r.event)).toMatchObject([
      { tool: "notes.add", rawInput: { text: "bread" } },
    ]);
  });
});
