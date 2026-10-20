import { afterEach, describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
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

describe("streaming a whole message", () => {
  /** What a renderer does with the ops: set an item, replace a text item's tail of blocks. */
  function replay(ops: ViewOp[][]): ViewItem[] {
    const items: ViewItem[] = [];
    for (const op of ops.flat()) {
      if (op.op === "set") {
        const at = items.findIndex((i) => i.ref === op.item.ref);
        if (at === -1) items.push(structuredClone(op.item));
        else items[at] = structuredClone(op.item);
      } else if (op.op === "blocks") {
        const item = items.find((i) => i.ref === op.ref);
        if (item?.component.type === "text") {
          item.component.blocks = [...item.component.blocks.slice(0, op.from), ...op.blocks];
        }
      }
    }
    return items;
  }

  for (const name of ["openui-chat", "openui-form", "openui-streaming"]) {
    for (const size of [1, 3, 11]) {
      test(`${name} in pieces of ${size} characters ends as it would arrive whole`, () => {
        const fixture = JSON.parse(
          readFileSync(join(import.meta.dir, "..", "..", "fixtures", name, "fixture.json"), "utf8"),
        );
        const text: string = fixture.events[0].text;
        const pieces = text.match(new RegExp(`[\\s\\S]{1,${size}}`, "g")) ?? [];
        const events: SessionEvent[] = [
          ...pieces.map((piece): SessionEvent => chunk(piece, "m1")),
          { type: "turn_ended", stopReason: "end_turn" },
        ];
        const { view: streamed, ops } = view(events);
        const { view: whole } = view([
          chunk(text, "m1"),
          { type: "turn_ended", stopReason: "end_turn" },
        ]);
        expect(replay(ops)).toEqual(streamed.snapshot().items);
        expect(streamed.snapshot().items).toEqual(whole.snapshot().items);
      });
    }
  }
});

test("a message that starts finishes the one before it", () => {
  const { view: v, ops } = view([
    chunk('```openui\nroot = Header("a")\n', "m1"),
    chunk("Next.", "m2"),
  ]);
  expect(ops[1]).toMatchObject([{ op: "blocks", ref: "m-m1", from: 0 }, { op: "set" }]);
  const first = v.snapshot().items[0];
  expect(docs(blocksOf(first))[0]?.streaming).toBeUndefined();
});

describe("app components", () => {
  const BRIGHTNESS = {
    description: "An image with a brightness slider and live preview",
    params: { asset: "string", name: "string?" },
    openui: `
      $level  = 0
      root    = Card([pic, slider, apply], args.name)
      pic     = Image(preview.asset, args.name)
      slider  = Slider("level", -100, 100, 5, $level)
      preview = Query("photo-editor.preview", { asset: args.asset, brightness: $level }, { asset: args.asset })
      save    = Mutation("photo-editor.brightness", { asset: args.asset, amount: $level })
      apply   = Button("Apply", @Run(save), "primary")
    `,
  };

  async function setup(): Promise<{
    daemon: TestDaemon;
    session: string;
    app: McpApp;
    owner: object;
  }> {
    const daemon = await startTestDaemon();
    cleanups.push(() => daemon.stop());
    const app = startMcpApp({
      tools: [
        {
          name: "preview",
          description: "A preview of an asset at a brightness",
          inputSchema: {
            type: "object",
            properties: { asset: { type: "string" }, brightness: { type: "number" } },
          },
          handler: (args) => ({
            content: [{ type: "text", text: "preview" }],
            structuredContent: { asset: `${String(args.asset)}@${String(args.brightness)}` },
          }),
        },
        {
          name: "brightness",
          description: "Change the brightness of an asset",
          inputSchema: {
            type: "object",
            properties: { asset: { type: "string" }, amount: { type: "number" } },
          },
          handler: (args) => ({
            content: [{ type: "text", text: "done" }],
            structuredContent: { asset: String(args.asset), amount: Number(args.amount) },
          }),
        },
      ],
    });
    cleanups.push(() => app.close());
    const owner = {};
    await daemon.connectors.presence(owner, {
      app: "photo-editor",
      kind: "app",
      open: true,
      mcp: app.endpoint,
      components: {
        BrightnessCard: BRIGHTNESS,
        Chip: {
          description: "A small label",
          params: { text: "string" },
          openui: 'root = Tag(args.text, "accent")',
        },
      },
    });
    return { daemon, session: daemon.focus.current().session, app, owner };
  }

  const doc = (daemon: TestDaemon, session: string): UiDoc | undefined => {
    const item = daemon.views
      .get(session)
      .snapshot()
      .items.find((i) => i.ref === "m-m1");
    return docs(blocksOf(item))[0];
  };

  const until = async (check: () => boolean): Promise<void> => {
    for (let i = 0; i < 100 && !check(); i++) await Bun.sleep(10);
  };

  const act = (daemon: TestDaemon, session: string, action: string, args: unknown): Promise<void> =>
    daemon.actions.run({ session, ref: "m-m1", action, args }, "test");

  test("a doc uses a component of a connected app: the reply shows library components, each use with a state of its own", async () => {
    const { daemon, session, app } = await setup();
    const patches: ViewPatch[] = [];
    daemon.views.get(session);
    daemon.views.onPatch((p) => patches.push(p));
    daemon.store.append(
      session,
      chunk(
        `Two photos:\n\n${fence(
          [
            "root = Stack([a, b])",
            'a = BrightnessCard("asset://one", "Sunset")',
            'b = BrightnessCard("asset://two")',
          ].join("\n"),
        )}`,
      ),
    );
    daemon.store.append(session, { type: "turn_ended", stopReason: "end_turn" });

    // The previews answer by themselves.
    await until(() => JSON.stringify(doc(daemon, session)).includes("asset://two@0"));
    const drawn = JSON.stringify(doc(daemon, session));
    expect(drawn).toContain("asset://one@0");
    expect(drawn).not.toContain("BrightnessCard");
    expect(doc(daemon, session)?.errors).toBeUndefined();
    expect(patches.some((p) => p.ops.some((o) => o.op === "set" && o.item.ref === "m-m1"))).toBe(
      true,
    );

    // Moving the first slider asks for the first photo's preview only.
    await act(daemon, session, "openui.input", { doc: "0", key: "a/slider", value: 40 });
    await until(() => JSON.stringify(doc(daemon, session)).includes("asset://one@40"));
    const sliders = JSON.stringify(doc(daemon, session)).match(
      /"component":"Slider"[^}]*"value":(-?\d+)/g,
    );
    expect(sliders?.map((s) => s.split(":").at(-1))).toEqual(["40", "0"]);
    expect(app.calls.filter((c) => c.name === "preview").map((c) => c.args)).toContainEqual({
      asset: "asset://one",
      brightness: 40,
    });
    expect(
      app.calls.filter((c) => c.name === "preview" && c.args.asset === "asset://two"),
    ).toHaveLength(1);

    // Apply runs the first use's mutation with its own level, and is recorded like a tool call.
    await act(daemon, session, "openui.press", { doc: "0", key: "a/apply" });
    await until(() => app.calls.some((c) => c.name === "brightness"));
    expect(app.calls.filter((c) => c.name === "brightness")).toEqual([
      { name: "brightness", args: { asset: "asset://one", amount: 40 } },
    ]);
    expect(daemon.store.readTypes(session, ["tool_call"]).map((r) => r.event)).toMatchObject([
      { tool: "photo-editor.brightness", rawInput: { asset: "asset://one", amount: 40 } },
    ]);
  });

  test("the components stay known after the app closes", async () => {
    const { daemon, session, owner } = await setup();
    await daemon.connectors.presence(owner, { app: "photo-editor", kind: "app", open: false });
    expect(daemon.connectors.isOpen("photo-editor")).toBe(false);
    expect([...daemon.connectors.library().components.keys()]).toEqual([
      "PhotoEditorBrightnessCard",
      "PhotoEditorChip",
      "BrightnessCard",
      "Chip",
    ]);
    expect([...daemon.connectors.openApps()]).toEqual([]);
    daemon.store.append(session, chunk(fence('root = Stack([Chip("new")])')));
    daemon.store.append(session, { type: "turn_ended", stopReason: "end_turn" });
    expect(doc(daemon, session)?.errors).toBeUndefined();
    expect(doc(daemon, session)?.root?.props.children).toMatchObject([
      { component: "Tag", props: { label: "new", tone: "accent" } },
    ]);
  });

  test("a registration is never refused: a clash is called by its qualified name, a bad one left out", async () => {
    const { daemon, session } = await setup();
    const mine = { description: "x", params: {}, openui: 'root = Text("notes")' };
    await daemon.connectors.presence(
      {},
      {
        app: "notes",
        kind: "app",
        open: true,
        components: { Card: mine, BrightnessCard: mine, "notes.Bad": mine, NoteCard: mine },
      },
    );
    expect(daemon.connectors.isOpen("notes")).toBe(true);
    expect(Object.keys(daemon.connectors.connector("notes")?.components ?? {})).toEqual([
      "Card",
      "BrightnessCard",
      "NoteCard",
    ]);
    const snapshot = daemon.connectors.snapshot().find((c) => c.app === "notes");
    expect(snapshot?.components).toEqual(["NotesCard", "NotesBrightnessCard", "NoteCard"]);
    // The plain name still draws the photo editor's, which registered it first.
    const calls =
      'root = Stack([a, b])\na = BrightnessCard("asset://x")\nb = NotesBrightnessCard()';
    daemon.store.append(session, chunk(fence(calls)));
    daemon.store.append(session, { type: "turn_ended", stopReason: "end_turn" });
    expect(doc(daemon, session)?.errors).toBeUndefined();
    expect(doc(daemon, session)?.root?.props.children).toMatchObject([
      { component: "Card" },
      { component: "Text", props: { text: "notes" } },
    ]);
  });
});
