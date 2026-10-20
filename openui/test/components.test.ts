import { describe, expect, test } from "bun:test";
import type { AppComponent, Block, UiNode } from "@suite/protocol/types";
import {
  appLibrary,
  callNames,
  checkComponents,
  instanceProgram,
  paramList,
  qualifiedName,
  type AppComponents,
} from "../src/components.ts";
import { Program, type CallTool, type ProgramEnv } from "../src/program.ts";

const markdown = (source: string): Block[] => [{ type: "paragraph", spans: [{ text: source }] }];

/** The example of the issue: an image with a slider, a live preview and an Apply. */
const BRIGHTNESS: AppComponent = {
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

const apps: AppComponents = { "photo-editor": { BrightnessCard: BRIGHTNESS } };

const ok = (name: string, component: Partial<AppComponent> = {}): Record<string, AppComponent> => ({
  [name]: {
    description: "A note",
    params: { text: "string" },
    openui: "root = Text(args.text)",
    ...component,
  },
});

describe("registering components", () => {
  /** The names of the usable components and the problems of the others. */
  const check = (components: Record<string, AppComponent>): [string[], string[]] => {
    const { usable, problems } = checkComponents(components);
    return [Object.keys(usable), problems];
  };

  test("a component of a name, a description, typed params and a program is usable", () => {
    expect(check(apps["photo-editor"] ?? {})).toEqual([["BrightnessCard"], []]);
    expect(check(ok("NoteCard"))).toEqual([["NoteCard"], []]);
  });

  test("one bad component is left out, the others stay", () => {
    expect(check({ ...ok("NoteCard"), ...ok("bad") })).toEqual([
      ["NoteCard"],
      [
        "component bad: a name is letters and digits starting with a capital (PascalCase), as lang-core reads it as a call",
      ],
    ]);
  });

  test("a name is a PascalCase identifier: lang-core reads app.Name as a subtraction and a member", () => {
    for (const name of [
      "photo-editor.Brightness",
      "photo.Brightness",
      "brightness",
      "Bright-ness",
      "1Card",
      "",
    ]) {
      expect(check(ok(name))[1][0]).toContain("PascalCase");
    }
  });

  test("a name the library, the language or another app has is no problem at registration", () => {
    expect(check({ ...ok("Card"), ...ok("Query"), ...ok("BrightnessCard") })[1]).toEqual([]);
  });

  test("params are named, typed, and the optional ones come last", () => {
    const bad = (params: Record<string, string>): string[] => check(ok("X", { params }))[1];
    expect(bad({ Asset: "string" })[0]).toContain("camelCase");
    expect(bad({ asset: "image" })[0]).toContain("not string, number, boolean or any");
    expect(bad({ asset: "string[]?", n: "number" })).toEqual([
      "component X: param n: required params come before optional ones",
    ]);
    expect(bad({ a: "string", b: "number[]", c: "boolean?", d: "any[]?" })).toEqual([]);
    expect(paramList({ asset: "string", name: "string?", tags: "string[]?" })).toBe(
      "asset: string, name?: string, tags?: string[]",
    );
  });

  test("the program has to parse, have a root and define what it uses", () => {
    const why = (openui: string): string[] => check(ok("X", { openui }))[1];
    expect(why('root = Nope("x")')[0]).toContain("Unknown component");
    expect(why('title = Text("no root")')[0]).toContain("No root");
    expect(why('root = Card([Text("a"], "b")')[0]).toContain("defining");
    expect(why("root = Stack([missing])")[0]).toContain("without defining");
    // App components are made of the library, not of each other.
    expect(why('root = NoteCard("x")')[0]).toContain("Unknown component");
    expect(check(ok("X", { description: " " }))[1]).toEqual([
      "component X: it needs a description for the agent",
    ]);
  });
});

describe("names", () => {
  test("the qualified name is the app in PascalCase, then the component's", () => {
    expect(qualifiedName("photo-editor", "BrightnessCard")).toBe("PhotoEditorBrightnessCard");
    expect(qualifiedName("terminal-2", "Chip")).toBe("Terminal2Chip");
    expect(qualifiedName("notes", "Card")).toBe("NotesCard");
  });

  test("every component has its qualified name, and its plain one when nothing earlier has it", () => {
    const lib = appLibrary({
      first: ok("NoteCard"),
      second: { ...ok("NoteCard"), ...ok("Card"), ...ok("Own") },
    });
    expect(
      [...lib.components.entries()].map(([call, r]) => `${call} → ${r.app}.${r.name}`),
    ).toEqual([
      "FirstNoteCard → first.NoteCard",
      "SecondNoteCard → second.NoteCard",
      "SecondCard → second.Card",
      "SecondOwn → second.Own",
      "NoteCard → first.NoteCard",
      "Own → second.Own",
    ]);
    // What the agent is told to call each by.
    expect(callNames(lib)).toEqual({
      first: ["NoteCard"],
      second: ["SecondNoteCard", "SecondCard", "Own"],
    });
  });

  test("the earlier app keeps the plain name whatever the app names are", () => {
    const lib = appLibrary({ zebra: ok("NoteCard"), aardvark: ok("NoteCard") });
    expect(lib.components.get("NoteCard")?.app).toBe("zebra");
    expect(lib.components.get("AardvarkNoteCard")?.app).toBe("aardvark");
  });
});

describe("the library with app components", () => {
  test("lang-core checks the calls: params by position, a name an app registered, none other", () => {
    const lib = appLibrary(apps);
    expect([...lib.components.keys()]).toEqual(["PhotoEditorBrightnessCard", "BrightnessCard"]);
    expect(lib.components.get("BrightnessCard")).toMatchObject({
      app: "photo-editor",
      params: ["asset", "name"],
    });
    const p = new Program("0", { markdown, apps: () => lib });
    p.set('root = Stack([BrightnessCard("asset://a", "Sunset"), Nope(1)])', false);
    const doc = p.doc();
    expect(doc.errors?.map((e) => e.code)).toContain("unknown-component");
    expect(Object.keys(lib.library.components)).toContain("BrightnessCard");
    expect(Object.keys(lib.library.components)).toContain("Card");
    expect(lib.library.toSpec().components.BrightnessCard?.signature).toBe(
      "BrightnessCard(asset: string, name?: string)",
    );
  });

  test("a name the library has is only the qualified one", () => {
    const lib = appLibrary({ a: ok("Card"), b: { ...ok("Mine"), ...ok("Theirs") } });
    expect([...lib.components.keys()]).toEqual(["ACard", "BMine", "BTheirs", "Mine", "Theirs"]);
  });

  test("the program of a use has its arguments as its last statement", () => {
    expect(instanceProgram("root = Text(args.text)\n\n", { text: "hi" })).toBe(
      'root = Text(args.text)\nargs = {"text":"hi"}',
    );
  });
});

// ─── Running them ───────────────────────────────────────────────────────────

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

const find = (p: Program, component: string): UiNode[] =>
  nodes(p.doc().root).filter((n) => n.component === component);

function run(source: string, env: Partial<ProgramEnv> = {}, streaming = false): Program {
  const lib = appLibrary(apps);
  const p = new Program("0", { markdown, apps: () => lib, ...env });
  p.set(source, streaming);
  return p;
}

const settle = (ms = 20): Promise<void> => new Promise((resolve) => setTimeout(resolve, ms));

describe("using a component", () => {
  test("a use is replaced by what its program draws, in library components", () => {
    const p = run('root = Stack([BrightnessCard("asset://a", "Sunset")])');
    const doc = p.doc();
    expect(doc.errors).toBeUndefined();
    const card = doc.root?.props.children;
    expect(card).toMatchObject([
      {
        component: "Card",
        props: {
          title: "Sunset",
          children: [
            { component: "Image", props: { src: "asset://a", alt: "Sunset" } },
            {
              component: "Slider",
              key: "root.children.0/slider",
              props: { min: -100, max: 100, step: 5 },
            },
            {
              component: "Button",
              key: "root.children.0/apply",
              props: { label: "Apply", variant: "primary" },
            },
          ],
        },
      },
    ]);
    // Nothing of the component is left for a renderer to know.
    expect(JSON.stringify(doc)).not.toContain("BrightnessCard");
  });

  test("two uses have a state of their own: the slider, the preview and the keys", async () => {
    const asked: [string, { asset: string; brightness: number }][] = [];
    const callTool: CallTool = (name, args) => {
      asked.push([name, args as { asset: string; brightness: number }]);
      return Promise.resolve({ asset: `${String((args as { asset: string }).asset)}@preview` });
    };
    const p = run(
      'root = Stack([a, b])\na = BrightnessCard("asset://one", "One")\nb = BrightnessCard("asset://two")',
      { callTool },
    );
    p.onChange = () => p.doc();
    p.doc();
    await settle();
    const keys = find(p, "Slider").map((n) => n.key);
    expect(keys).toEqual(["a/slider", "b/slider"]);

    expect(p.input("a/slider", 40)).toBe(true);
    const sliders = find(p, "Slider").map((n) => n.props.value);
    expect(sliders).toEqual([40, 0]);
    await settle();
    // The preview of the first use asked again with its level, the second's did not.
    expect(asked.filter(([, args]) => args.brightness === 40)).toEqual([
      ["photo-editor.preview", { asset: "asset://one", brightness: 40 }],
    ]);
    expect(find(p, "Image").map((n) => n.props.src)).toEqual([
      "asset://one@preview",
      "asset://two@preview",
    ]);
  });

  test("Apply runs the mutation with the level of its own use, and records it as a mutation", async () => {
    const calls: unknown[] = [];
    const callTool: CallTool = (name, args, kind) => {
      calls.push([name, args, kind]);
      return Promise.resolve({ asset: "x" });
    };
    const p = run(
      'root = Stack([a, b])\na = BrightnessCard("asset://one")\nb = BrightnessCard("asset://two")',
      {
        callTool,
      },
    );
    p.onChange = () => p.doc();
    p.doc();
    await settle();
    p.input("b/slider", -25);
    // The host draws the doc again after an input; that is what asks the preview for the new level.
    p.doc();
    await settle();
    calls.length = 0;
    const done = await p.press("b/apply");
    expect(done).toEqual({ assistant: [] });
    expect(calls).toEqual([
      ["photo-editor.brightness", { asset: "asset://two", amount: -25 }, "mutation"],
    ]);
    // A key no use has does nothing.
    expect(await p.press("c/apply")).toEqual({ refused: "unknown", assistant: [] });
    expect(p.input("c/slider", 1)).toBe(false);
    expect(p.input("a/nope", 1)).toBe(false);
  });

  test("a use that is streaming draws what is there and cannot be used; no query starts", () => {
    let asked = 0;
    const p = run(
      'root = Stack([BrightnessCard("asset://a", "Sun',
      { callTool: () => (asked++, Promise.resolve({})) },
      true,
    );
    const doc = p.doc();
    expect(doc.streaming).toBe(true);
    expect(find(p, "Card")[0]?.partial).toBe(true);
    expect(p.input("root.children.0/slider", 5)).toBe(false);
    expect(asked).toBe(0);
  });

  test("what the call gets wrong is an error of the doc, and the use says why it is missing", () => {
    // The app went away from the library but the doc still names it.
    const p = new Program("0", { markdown, apps: () => appLibrary({}) });
    p.set('root = Stack([BrightnessCard("asset://a")])', false);
    expect(p.doc().errors?.[0]?.code).toBe("unknown-component");
    const broken = appLibrary({
      a: ok("Broken", { openui: "root = Stack([Text(args.text)])\nx = Metric(" }),
    });
    const q = new Program("0", { markdown, apps: () => broken });
    q.set('root = Stack([Broken("hi")])', false);
    const text = JSON.stringify(q.doc().root);
    expect(text).not.toContain("Broken(");
  });

  test("a component that draws nothing is a note in its place, with the error on the doc", () => {
    const lib = appLibrary({ a: ok("Empty", { openui: 'title = Text("no root")' }) });
    const p = new Program("0", { markdown, apps: () => lib });
    p.set('root = Stack([Empty("x")])', false);
    const doc = p.doc();
    expect(doc.root?.props.children).toMatchObject([
      { component: "Callout", props: { tone: "danger" } },
    ]);
    expect(doc.errors?.[0]?.message).toContain("Empty:");
  });

  test("a use that leaves the doc takes its state with it", () => {
    const p = run(
      '$on = true\nroot = Stack([$on ? BrightnessCard("asset://a") : null, t])\nt = Switch("on", "Show", $on)',
    );
    expect(find(p, "Slider")).toHaveLength(1);
    p.input("slider-less", 1);
    p.input("t", false);
    expect(find(p, "Slider")).toHaveLength(0);
    p.input("t", true);
    expect(find(p, "Slider")).toHaveLength(1);
    p.dispose();
  });

  test("the library is asked again when the program is set: an app that connects mid-stream", () => {
    let lib = appLibrary({});
    const p = new Program("0", { markdown, apps: () => lib });
    const source = 'root = Stack([BrightnessCard("asset://a")])';
    p.set(source, false);
    expect(p.doc().errors?.[0]?.code).toBe("unknown-component");
    lib = appLibrary(apps);
    p.set(`${source} `, false);
    expect(p.doc().errors).toBeUndefined();
    expect(find(p, "Slider")).toHaveLength(1);
  });
});
