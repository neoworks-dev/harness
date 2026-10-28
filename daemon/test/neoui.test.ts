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
} from "@neoworks-dev/neo-protocol/types";
import { silentLogger } from "../src/log.ts";
import { startTestDaemon, type TestDaemon } from "../src/testing.ts";
import { parseMarkdown } from "../src/view/markdown.ts";
import { SessionView } from "../src/view/session-view.ts";
import { startMcpApp, type McpApp } from "./mcp-app.ts";

const cleanups: (() => Promise<void>)[] = [];
afterEach(async () => {
  for (const cleanup of cleanups.splice(0).reverse()) await cleanup();
});

const PROGRAM = '<Card title="T"><Header title="Hello"/></Card>';
const fence = (program: string, lang = "neoui"): string => `\`\`\`${lang}\n${program}\n\`\`\``;

const docs = (blocks: Block[]): UiDoc[] =>
  blocks.filter((b) => b.type === "neoui").map((b) => b.ui);

/** The blocks of an item that is text. */
function blocksOf(item: ViewItem | undefined): Block[] {
  if (item?.component.type === "text") return item.component.blocks;
  return [];
}

describe("neoui fences in markdown", () => {
  const host = (index: number, program: string, closed: boolean): UiDoc => ({
    id: String(index),
    root: { component: "Header", props: { title: `${program.length}${closed ? "c" : "o"}` } },
  });

  test("a top-level fence becomes a neoui block among the others; they are numbered in order", () => {
    const blocks = parseMarkdown(
      `Before.\n\n${fence("<A/>")}\n\nBetween.\n\n${fence("<B/>", "neoui title")}\n\nAfter.`,
      host,
    );
    expect(blocks.map((b) => b.type)).toEqual([
      "paragraph",
      "neoui",
      "paragraph",
      "neoui",
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
    parseMarkdown("```neoui\n<A b={1}/>\n", record);
    parseMarkdown("```neoui\n<A b={1}/>\n``", record);
    parseMarkdown("```neoui\n<A b={1}/>\n```", record);
    parseMarkdown('```neoui\n<A b="``"/>\n```\n\nand', record);
    expect(seen).toEqual([
      ["<A b={1}/>", false],
      ["<A b={1}/>", false],
      ["<A b={1}/>", true],
      ['<A b="``"/>', true],
    ]);
  });

  test("other fences, fences without a host and fences inside lists stay code", () => {
    expect(parseMarkdown(fence("x", "js"), host)[0]?.type).toBe("code_block");
    // Programs in OpenUI, the language before neoui, are not read any more.
    expect(parseMarkdown(fence("root = Card([])", "openui"), host)[0]?.type).toBe("code_block");
    expect(parseMarkdown(fence("root = Card([])", "openui-lang"), host)[0]?.type).toBe(
      "code_block",
    );
    expect(parseMarkdown(fence(PROGRAM))[0]?.type).toBe("code_block");
    const nested = parseMarkdown(`- item\n\n  ${fence("x").replaceAll("\n", "\n  ")}`, host);
    expect(JSON.stringify(nested)).not.toContain('"type":"neoui"');
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

describe("a streamed message with neoui", () => {
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
    const open = `Here:\n\n\`\`\`neoui\n${PROGRAM}\n`;
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
    expect(JSON.stringify(v.snapshot())).not.toContain('"type":"neoui"');
  });

  test("a doc with a broken program draws nothing and says why", () => {
    const { view: v } = view([
      chunk(fence("<Nope/>")),
      { type: "turn_ended", stopReason: "end_turn" },
    ]);
    const ui = docs(blocksOf(v.snapshot().items[0]))[0];
    expect(ui?.root).toBeUndefined();
    expect(ui?.errors?.[0]?.code).toBe("unknown-component");
  });
});

describe("app templates in neoui", () => {
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

  test("the tool's data is read as `data`", () => {
    const templates = {
      "stats.show": {
        neoui: '<Card><Metric label="Total" value={data.total}/><Text>{data.note}</Text></Card>',
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
    expect(component?.type).toBe("neoui");
    const root = component?.type === "neoui" ? component.ui.root : undefined;
    expect(root?.props.children).toMatchObject([
      { component: "Metric", props: { value: 7 } },
      { component: "Text", props: { text: "**ok**", blocks: [{ type: "paragraph" }] } },
    ]);
  });

  test("a template that draws nothing falls back to the generic card", () => {
    const v = new SessionView(
      "s",
      () => ({ neoui: '<Query id="q" hx-get="stats.total"/>' }),
      silentLogger,
    );
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
        '<Form name="ask">',
        '  <FormControl label="Name"><Input name="name" required/></FormControl>',
        '  <Buttons><Button id="go" ui-ask="Here you go" ui-context={{ id: 3 }}>Send</Button></Buttons>',
        "</Form>",
      ].join("\n"),
    );
    const act = (action: string, args: unknown): Promise<void> =>
      daemon.actions.run({ session, ref: "m-m1", action, args }, "test");

    // Pressing with the field empty: nothing is sent, the field says why.
    await act("neoui.press", { doc: "0", key: "go" });
    expect(daemon.store.readTypes(session, ["user_message"])).toHaveLength(0);
    expect(JSON.stringify(doc(daemon, session))).toContain("This field is required");
    const failed = patches.at(-1);
    expect(failed?.ops[0]).toMatchObject({ op: "set", item: { ref: "m-m1" } });

    await act("neoui.input", { doc: "0", key: "name", value: "Ada" });
    expect(JSON.stringify(doc(daemon, session))).not.toContain("This field is required");
    expect(JSON.stringify(doc(daemon, session))).toContain('"value":"Ada"');

    await act("neoui.press", { doc: "0", key: "go" });
    await daemon.sessions.idle(session);
    const sent = daemon.store.readTypes(session, ["user_message"]).at(-1)?.event;
    expect(sent).toMatchObject({
      type: "user_message",
      text: '[ui action] The user pressed "Send": Here you go\nform ask: {"name":"Ada"}\ncontext: {"id":3}',
      shown: "Here you go\n\nName: Ada",
    });
  });

  test("a button with no ui-ask sends its label once, and the chat shows only the label", async () => {
    const { daemon, session } = await setup();
    reply(daemon, session, '<Stack><Button id="go">Run cargo clean</Button></Stack>');
    await daemon.actions.run(
      { session, ref: "m-m1", action: "neoui.press", args: { doc: "0", key: "go" } },
      "test",
    );
    await daemon.sessions.idle(session);
    expect(daemon.store.readTypes(session, ["user_message"]).at(-1)?.event).toMatchObject({
      text: '[ui action] The user pressed "Run cargo clean".',
      shown: "Run cargo clean",
    });
    const user = daemon.views
      .get(session)
      .snapshot()
      .items.find((i) => i.kind === "user");
    expect(JSON.stringify(user?.component)).toContain("Run cargo clean");
    expect(JSON.stringify(user?.component)).not.toContain("[ui action]");
  });

  test("outside a form the agent is told the variables by name", async () => {
    const { daemon, session } = await setup();
    reply(
      daemon,
      session,
      [
        "<Stack>",
        '  <Select name="env" value="staging"><Option value="staging">Staging</Option></Select>',
        '  <Button id="go" ui-ask="Deploy now">Deploy</Button>',
        "</Stack>",
      ].join("\n"),
    );
    await daemon.actions.run(
      { session, ref: "m-m1", action: "neoui.press", args: { doc: "0", key: "go" } },
      "test",
    );
    await daemon.sessions.idle(session);
    expect(daemon.store.readTypes(session, ["user_message"]).at(-1)?.event).toMatchObject({
      text: '[ui action] The user pressed "Deploy": Deploy now\nstate: {"env":"staging"}',
      shown: "Deploy now\n\nenv: Staging",
    });
  });

  test("the chat shows what was picked; a press taken back out of the queue and sent again is told whole", async () => {
    const { daemon, session } = await setup();
    reply(
      daemon,
      session,
      [
        '<Form name="pick">',
        '  <FormControl label="Tests"><CheckBoxGroup name="tests" value={["unit", "e2e"]}>',
        '    <CheckBoxItem name="unit" label="Unit"/><CheckBoxItem name="e2e" label="E2E"/>',
        "  </CheckBoxGroup></FormControl>",
        '  <SwitchGroup name="opts"><SwitchItem name="lint" label="Lint" checked/>',
        '    <SwitchItem name="fmt" label="Format"/></SwitchGroup>',
        '  <Input name="note"/>',
        '  <Buttons><Button id="go" ui-ask="Go with the selected options" ui-context={{ plan: 7 }}>Go</Button></Buttons>',
        "</Form>",
      ].join("\n"),
    );
    const press = async (): Promise<void> => {
      // A turn is running, so the press waits in the queue.
      daemon.sessions.input(session, "test", "busy");
      await daemon.actions.run(
        { session, ref: "m-m1", action: "neoui.press", args: { doc: "0", key: "go" } },
        "test",
      );
    };
    const last = (): SessionEvent | undefined =>
      daemon.store.readTypes(session, ["user_message"]).at(-1)?.event;
    const shown = "Go with the selected options\n\nTests: Unit, E2E · opts: Lint";
    const told =
      '[ui action] The user pressed "Go": Go with the selected options\n' +
      'form pick: {"tests":["unit","e2e"],"opts":{"lint":true,"fmt":false},"note":null}\n' +
      'context: {"plan":7}';

    await press();
    expect(daemon.sessions.unqueue(session, "test")).toEqual([{ app: "test", text: shown }]);
    // Sent again as it came back, with a line typed under it.
    daemon.sessions.input(session, "test", `${shown}\n\nand be quick`);
    await daemon.sessions.idle(session);
    expect(last()).toMatchObject({
      text: `${told}\n\nand be quick`,
      shown: `${shown}\n\nand be quick`,
    });

    // Edited, it goes as written.
    await press();
    daemon.sessions.unqueue(session, "test");
    daemon.sessions.input(session, "test", "Go with unit only");
    await daemon.sessions.idle(session);
    expect(last()).toMatchObject({ text: "Go with unit only" });
    expect(last()).not.toHaveProperty("shown");
  });

  test("actions on a doc that is still streaming are ignored; bad ones are rejected", async () => {
    const { daemon, session } = await setup();
    daemon.store.append(session, chunk('```neoui\n<Stack><Input name="f"/>\n'));
    const run = (action: string, args: unknown, ref = "m-m1"): Promise<void> =>
      daemon.actions.run({ session, ref, action, args }, "test");
    await run("neoui.input", { doc: "0", key: "f", value: "x" });
    expect(JSON.stringify(doc(daemon, session))).not.toContain('"value"');
    expect(run("neoui.input", { doc: "7", key: "f", value: "x" })).rejects.toThrow("no neoui doc");
    expect(run("neoui.press", { key: "f" })).rejects.toThrow("needs args");
    expect(run("neoui.press", { doc: "0", key: "f" }, "nope")).rejects.toThrow("no neoui doc");
  });

  test("a Query reads through the gateway; a post is recorded like a tool call and the query asks again", async () => {
    const { daemon, session, app } = await setup();
    const patches: ViewPatch[] = [];
    daemon.views.get(session);
    daemon.views.onPatch((p) => patches.push(p));
    reply(
      daemon,
      session,
      [
        "<Stack>",
        '  <State name="round" value={0}/>',
        '  <Query id="data" hx-get="notes.list" hx-vals={{ round: $round }} default={{ notes: [] }}/>',
        '  <List items={pluck(data.notes, "text")}/>',
        '  <Button id="go" hx-post="notes.add" hx-vals={{ text: "bread" }} ui-set={{ round: $round + 1 }}>Add</Button>',
        "</Stack>",
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
      { session, ref: "m-m1", action: "neoui.press", args: { doc: "0", key: "go" } },
      "test",
    );
    // Setting `round` made the query ask again.
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

  for (const name of ["neoui-chat", "neoui-form", "neoui-streaming"]) {
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
    chunk('```neoui\n<Header title="a"/>\n', "m1"),
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
    neoui: `
      <Card title={name}>
        <Query id="preview" hx-get="photo-editor.preview"
               hx-vals={{ asset: asset, brightness: $level }} default={{ asset: asset }}/>
        <Image src={preview.asset} alt={name}/>
        <Slider name="level" min={-100} max={100} step={5} value={0}/>
        <Button id="apply" variant="primary" hx-post="photo-editor.brightness"
                hx-vals={{ asset: asset, amount: $level }}>Apply</Button>
      </Card>`,
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
          neoui: '<Tag label={text} tone="accent"/>',
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
            "<Stack>",
            '  <photo-editor.BrightnessCard id="a" asset="asset://one" name="Sunset"/>',
            '  <photo-editor.BrightnessCard id="b" asset="asset://two"/>',
            "</Stack>",
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
    await act(daemon, session, "neoui.input", { doc: "0", key: "a/level", value: 40 });
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
    await act(daemon, session, "neoui.press", { doc: "0", key: "a/apply" });
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
      "photo-editor.BrightnessCard",
      "photo-editor.Chip",
    ]);
    expect([...daemon.connectors.openApps()]).toEqual([]);
    daemon.store.append(session, chunk(fence('<Stack><photo-editor.Chip text="new"/></Stack>')));
    daemon.store.append(session, { type: "turn_ended", stopReason: "end_turn" });
    expect(doc(daemon, session)?.errors).toBeUndefined();
    expect(doc(daemon, session)?.root?.props.children).toMatchObject([
      { component: "Tag", props: { label: "new", tone: "accent" } },
    ]);
  });

  test("a registration is never refused: nothing clashes, a bad component is left out", async () => {
    const { daemon, session } = await setup();
    const mine = { description: "x", params: {}, neoui: "<Text>notes</Text>" };
    const old = { description: "x", params: {}, neoui: 'root = Text("old")' };
    await daemon.connectors.presence(
      {},
      {
        app: "notes",
        kind: "app",
        open: true,
        components: {
          Card: mine,
          BrightnessCard: mine,
          "notes.Bad": mine,
          NoteCard: mine,
          Old: old,
        } as never,
      },
    );
    expect(daemon.connectors.isOpen("notes")).toBe(true);
    // The same names as the library's and the photo editor's register; the others are left out.
    expect(Object.keys(daemon.connectors.connector("notes")?.components ?? {})).toEqual([
      "Card",
      "BrightnessCard",
      "NoteCard",
    ]);
    const snapshot = daemon.connectors.snapshot().find((c) => c.app === "notes");
    expect(snapshot?.components).toEqual(["notes.Card", "notes.BrightnessCard", "notes.NoteCard"]);
    // Both BrightnessCards draw, and notes.Card sits beside the library's Card.
    const calls = [
      "<Stack>",
      '  <photo-editor.BrightnessCard asset="asset://x"/>',
      "  <notes.BrightnessCard/>",
      '  <Card title="mine"><notes.Card/></Card>',
      "</Stack>",
    ].join("\n");
    daemon.store.append(session, chunk(fence(calls)));
    daemon.store.append(session, { type: "turn_ended", stopReason: "end_turn" });
    expect(doc(daemon, session)?.errors).toBeUndefined();
    expect(doc(daemon, session)?.root?.props.children).toMatchObject([
      { component: "Card" },
      { component: "Text", props: { text: "notes" } },
      { component: "Card", props: { title: "mine", children: [{ component: "Text" }] } },
    ]);
  });

  test("a plain name is an error that names the apps that have it", async () => {
    const { daemon, session } = await setup();
    const mine = { description: "x", params: {}, neoui: "<Text>notes</Text>" };
    await daemon.connectors.presence(
      {},
      { app: "notes", kind: "app", open: true, components: { BrightnessCard: mine } },
    );
    daemon.store.append(session, chunk(fence('<Stack><BrightnessCard asset="x"/></Stack>')));
    daemon.store.append(session, { type: "turn_ended", stopReason: "end_turn" });
    expect(doc(daemon, session)?.errors?.[0]).toMatchObject({
      code: "unknown-component",
      message:
        "Unknown component BrightnessCard; did you mean notes.BrightnessCard or photo-editor.BrightnessCard?",
    });
  });
});
