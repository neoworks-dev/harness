import { describe, expect, test } from "bun:test";
import type { Block, UiNode } from "@suite/protocol/types";
import { Program, type CallTool, type ProgramEnv } from "../src/program.ts";

const markdown = (source: string): Block[] => [{ type: "paragraph", spans: [{ text: source }] }];

function program(source: string, env: Partial<ProgramEnv> = {}, streaming = false): Program {
  const p = new Program("0", { markdown, ...env });
  p.set(source, streaming);
  return p;
}

/** Every node of a doc in drawing order. */
function nodes(node: UiNode | undefined): UiNode[] {
  if (!node) return [];
  const out = [node];
  const visit = (value: unknown): void => {
    if (Array.isArray(value)) value.forEach(visit);
    else if (typeof value === "object" && value !== null) {
      if ("component" in value) out.push(...nodes(value as UiNode));
      else Object.values(value).forEach(visit);
    }
  };
  Object.values(node.props).forEach(visit);
  return out;
}

const keysOf = (p: Program): string[] =>
  nodes(p.doc().root)
    .map((n) => n.key)
    .filter((key): key is string => key !== undefined);

const byKey = (p: Program, key: string): UiNode | undefined =>
  nodes(p.doc().root).find((n) => n.key === key);

describe("evaluating", () => {
  test("positional arguments become named props, nulls and defaults stay out", () => {
    const p = program(
      [
        'root = Card([steps], "Deploy")',
        'steps = Steps([Step("Build", "2m", "done"), Step("Ship", null, "pending")])',
      ].join("\n"),
    );
    expect(p.doc()).toEqual({
      id: "0",
      root: {
        component: "Card",
        props: {
          title: "Deploy",
          children: [
            {
              component: "Steps",
              props: {
                steps: [
                  { component: "Step", props: { title: "Build", detail: "2m", status: "done" } },
                  { component: "Step", props: { title: "Ship", status: "pending" } },
                ],
              },
            },
          ],
        },
      },
    });
  });

  test("expressions, ternaries and @Each resolve to concrete values", () => {
    const p = program(
      [
        "$big = true",
        "total = @Sum([1, 2, 3])",
        'root = Stack([Metric("Total", total), Text($big ? "large" : "small")])',
        'list = List(@Each([1, 2], "n", "row " + n))',
      ].join("\n"),
    );
    const metric = nodes(p.doc().root).find((n) => n.component === "Metric");
    expect(metric?.props.value).toBe(6);
    const text = nodes(p.doc().root).find((n) => n.component === "Text");
    expect(text?.props.text).toBe("large");
  });

  test("Text and Callout carry the parsed markdown beside their text", () => {
    const p = program('root = Callout("**hi**", "danger")');
    expect(p.doc().root?.props).toEqual({
      text: "**hi**",
      tone: "danger",
      blocks: markdown("**hi**"),
    });
  });

  test("an unknown component is dropped and reported once the program is complete", () => {
    const p = program('root = Stack([Header("a"), Nope(1)])');
    expect(p.doc().root?.props.children).toHaveLength(1);
    expect(p.doc().errors?.[0]?.code).toBe("unknown-component");
  });

  test("a program with no root draws nothing and says so", () => {
    const p = program('title = Header("a")');
    expect(p.doc().root).toBeUndefined();
    expect(p.doc().errors?.map((e) => e.code)).toContain("no-root");
  });

  test("the entry may be the first Stack when there is no root statement", () => {
    const p = program('main = Stack([Header("a")])');
    expect(p.doc().root?.component).toBe("Stack");
  });
});

describe("streaming", () => {
  test("only the statement being written is partial, and errors wait", () => {
    const p = program(
      'root = Stack([a, b])\na = Header("Done")\nb = Table(["x"], [["1"], ["2',
      {},
      true,
    );
    const doc = p.doc();
    expect(doc.streaming).toBe(true);
    expect(doc.errors).toBeUndefined();
    const found = nodes(doc.root);
    expect(found.find((n) => n.component === "Header")?.partial).toBeUndefined();
    expect(found.find((n) => n.component === "Table")?.partial).toBe(true);
    expect(found.find((n) => n.component === "Stack")?.partial).toBeUndefined();
    expect(found.find((n) => n.component === "Table")?.props.rows).toEqual([["1"], ["2"]]);
  });

  test("no field or button can be used while streaming", async () => {
    const p = program('root = Stack([f, b])\nf = Input("x")\nb = Button("Go")\n', {}, true);
    expect(p.doc().root).toBeDefined();
    expect(p.input("f", "hi")).toBe(false);
    expect((await p.press("b")).refused).toBe("streaming");
  });

  test("setting the same text again keeps what was typed", () => {
    const source = 'root = Stack([f])\nf = Input("x")';
    const p = program(source);
    expect(p.input("f", "typed")).toBe(true);
    p.set(source, false);
    expect(byKey(p, "f")?.props.value).toBe("typed");
  });
});

describe("keys", () => {
  test("a statement's own component, or the one inline in it, is keyed by the statement", () => {
    const p = program(
      [
        "root = Stack([a, b, c])",
        'a = Button("A")',
        'b = FormControl("B", Input("b"))',
        'c = Buttons([Button("C1"), Button("C2")])',
      ].join("\n"),
    );
    expect(keysOf(p)).toEqual(["a", "b", "c.buttons.0", "c.buttons.1"]);
  });

  test("components made by @Each are keyed by their place", () => {
    const p = program('root = Stack(@Each(["x", "y"], "t", Button(t)))');
    expect(keysOf(p)).toEqual(["root.children.0", "root.children.1"]);
  });
});

describe("inputs", () => {
  const source = [
    '$env = "staging"',
    'root = Form("f", [env, pick, on, level, when, note], Buttons([go]))',
    'env = RadioGroup("env", [Option("staging"), Option("prod", "Production")], null, $env)',
    'pick = Select("pick", [Option("a"), Option("b")], "Pick one", { required: true })',
    'on = CheckBox("on", "Agree", { required: true })',
    'level = Slider("level", 0, 10, 1)',
    'when = DatePicker("when")',
    'note = Input("note", "x", "text", { minLength: 3 })',
    'go = Button("Go", @ToAssistant("Go!", { id: 7 }))',
  ].join("\n");

  test("a field holds what was typed; a bound field writes its $variable", () => {
    const p = program(source);
    expect(byKey(p, "env")?.props.value).toBe("staging");
    expect(p.input("env", "prod")).toBe(true);
    expect(byKey(p, "env")?.props.value).toBe("prod");
    expect(byKey(p, "on")?.props.value).toBe(false);
    expect(p.input("on", true)).toBe(true);
    expect(byKey(p, "on")?.props.value).toBe(true);
    expect(p.input("level", 99)).toBe(true);
    expect(byKey(p, "level")?.props.value).toBe(10);
  });

  test("values of the wrong kind or outside the options are refused", () => {
    const p = program(source);
    expect(p.input("pick", "zzz")).toBe(false);
    expect(p.input("on", "yes")).toBe(false);
    expect(p.input("level", "3")).toBe(false);
    expect(p.input("nope", "x")).toBe(false);
    expect(byKey(p, "pick")?.props.value).toBeUndefined();
  });

  test("required fields are marked", () => {
    const p = program(source);
    expect(byKey(p, "pick")?.props.required).toBe(true);
    expect(byKey(p, "note")?.props.required).toBeUndefined();
  });

  test("a failing form refuses the press, sets the errors, and clears them as fields are fixed", async () => {
    const p = program(source);
    p.input("note", "ab");
    const result = await p.press("go");
    expect(result).toEqual({ refused: "invalid", assistant: [] });
    expect(byKey(p, "pick")?.props.error).toBe("This field is required");
    expect(byKey(p, "on")?.props.error).toBe("This field is required");
    expect(byKey(p, "note")?.props.error).toBe("Must be at least 3 characters");
    expect(byKey(p, "when")?.props.error).toBeUndefined();
    p.input("note", "abc");
    expect(byKey(p, "note")?.props.error).toBeUndefined();
    expect(byKey(p, "pick")?.props.error).toBe("This field is required");
  });

  test("a date has to be YYYY-MM-DD", async () => {
    const p = program(source);
    p.input("pick", "a");
    p.input("on", true);
    p.input("when", "tomorrow");
    expect((await p.press("go")).refused).toBe("invalid");
    expect(byKey(p, "when")?.props.error).toBe("Use the form YYYY-MM-DD");
    p.input("when", "2026-10-21");
    expect((await p.press("go")).refused).toBeUndefined();
  });

  test("a passing form sends its message with the form's values and the context", async () => {
    const p = program(source);
    p.input("pick", "b");
    p.input("on", true);
    p.input("env", "prod");
    const result = await p.press("go");
    expect(result.refused).toBeUndefined();
    expect(result.assistant).toEqual([
      {
        message: "Go!",
        label: "Go",
        form: "f",
        context: { id: 7 },
        values: { env: "prod", pick: "b", on: true, level: null, when: null, note: null },
      },
    ]);
  });
});

describe("actions", () => {
  test("a button without an action sends its label", async () => {
    const p = program('root = Button("Tell me more")');
    expect((await p.press("root")).assistant).toEqual([
      { message: "Tell me more", label: "Tell me more" },
    ]);
  });

  test("a button that only opens a link or copies text says so and the press is the client's", async () => {
    const p = program(
      [
        "root = Stack([a, b, c])",
        'a = Button("Docs", @OpenUrl("https://example.com/x"))',
        'b = Button("Copy", @Copy("hello"))',
        'c = Button("Bad", @OpenUrl("javascript:alert(1)"))',
      ].join("\n"),
    );
    expect(byKey(p, "a")?.props).toEqual({ label: "Docs", url: "https://example.com/x" });
    expect(byKey(p, "b")?.props).toEqual({ label: "Copy", copy: "hello" });
    expect(byKey(p, "c")?.props).toEqual({ label: "Bad" });
    expect((await p.press("a")).assistant).toEqual([]);
  });

  test("@Set and @Reset move $variables, and expressions follow", async () => {
    const p = program(
      [
        '$tab = "one"',
        'root = Stack([Text("on " + $tab), up, down])',
        'up = Button("Two", @Set($tab, "two"))',
        'down = Button("Back", @Reset($tab))',
      ].join("\n"),
    );
    const text = (): unknown => nodes(p.doc().root).find((n) => n.component === "Text")?.props.text;
    expect(text()).toBe("on one");
    await p.press("up");
    expect(text()).toBe("on two");
    await p.press("down");
    expect(text()).toBe("on one");
  });

  test("outside a form a message carries the $variables", async () => {
    const p = program(
      ['$q = "x"', 'root = Stack([Input("q", null, null, null, $q), b])', 'b = Button("Go")'].join(
        "\n",
      ),
    );
    p.input("root.children.0", "hello");
    expect((await p.press("b")).assistant[0]?.values).toEqual({ $q: "hello" });
  });
});

describe("tools", () => {
  test("a query answers later with the tool's data", async () => {
    const calls: unknown[] = [];
    const callTool: CallTool = (name, args, kind) => {
      calls.push([name, args, kind]);
      return Promise.resolve({ rows: [{ n: "a" }, { n: "b" }] });
    };
    const p = program(
      [
        'data = Query("app.rows", { limit: 2 }, { rows: [] })',
        'root = Stack(@Each(data.rows, "r", Tag(r.n)))',
      ].join("\n"),
      { callTool },
    );
    expect(p.doc().root?.props.children).toEqual([]);
    const changed = new Promise<void>((resolve) => {
      p.onChange = resolve;
    });
    await changed;
    expect(nodes(p.doc().root).filter((n) => n.component === "Tag")).toHaveLength(2);
    expect(calls).toEqual([["app.rows", { limit: 2 }, "query"]]);
  });

  test("a query in a streaming program waits", () => {
    let called = false;
    program(
      'data = Query("app.rows", {}, {})\nroot = Stack([])',
      {
        callTool: () => {
          called = true;
          return Promise.resolve({});
        },
      },
      true,
    ).doc();
    expect(called).toBe(false);
  });

  test("a mutation runs on press with the current values, and a failure stops the plan", async () => {
    const calls: unknown[] = [];
    let fail = false;
    const callTool: CallTool = (name, args, kind) => {
      calls.push([name, args, kind]);
      return fail ? Promise.reject(new Error("boom")) : Promise.resolve({ ok: true });
    };
    const p = program(
      [
        '$title = ""',
        'root = Stack([Input("title", null, null, null, $title), go])',
        'save = Mutation("app.create", { title: $title })',
        'go = Button("Create", Action([@Run(save), @Reset($title), @ToAssistant("created")]))',
      ].join("\n"),
      { callTool },
    );
    p.input("root.children.0", "Hello");
    const done = await p.press("go");
    expect(calls).toEqual([["app.create", { title: "Hello" }, "mutation"]]);
    expect(done.assistant.map((m) => m.message)).toEqual(["created"]);
    expect(byKey(p, "root.children.0")?.props.value).toBeUndefined();

    fail = true;
    p.input("root.children.0", "Again");
    const failed = await p.press("go");
    expect(failed).toEqual({ refused: "halted", assistant: [] });
    expect(byKey(p, "root.children.0")?.props.value).toBe("Again");
    expect(p.doc().errors?.[0]?.code).toBe("tool-error");
  });
});
