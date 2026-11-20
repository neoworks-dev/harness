import { expect, test } from "bun:test";
import { fencePath, Highlighter, languageOf, scan, type Token } from "../ui/plugins/viewer/syntax.ts";

test("a fenced block's language is read as the viewer reads a file's", () => {
  expect(languageOf(fencePath("rust"))).toBe("rust");
  expect(languageOf(fencePath("TypeScript"))).toBe("ts");
  expect(languageOf(fencePath("ts"))).toBe("ts");
  expect(languageOf(fencePath("bash"))).toBe("shell");
  expect(languageOf(fencePath("klingon"))).toBeUndefined();
});

/** A line's runs as `kind:text`, the uncoloured ones as the text alone. */
function runs(language: string, line: string, state = ""): string[] {
  return scan(language, line, state).tokens.map((t) => (t.kind ? `${t.kind}:${t.text}` : t.text));
}

/** Every line of a text in turn, as one flat list of `kind:text`. */
function all(language: string, lines: string[]): string[] {
  const light = new Highlighter(language, lines);
  return lines.flatMap((_, i) =>
    light.tokens(i).map((t: Token) => (t.kind ? `${t.kind}:${t.text}` : t.text)),
  );
}

test("a file is told by its name", () => {
  expect(languageOf("src/app.ts")).toBe("ts");
  expect(languageOf("ui/Card.svelte")).toBe("html");
  expect(languageOf("crates/x/src/lib.rs")).toBe("rust");
  expect(languageOf("main.go")).toBe("go");
  expect(languageOf("tools/run.py")).toBe("python");
  expect(languageOf("scripts/deploy.sh")).toBe("shell");
  expect(languageOf("package.json")).toBe("json");
  expect(languageOf("style.css")).toBe("css");
  expect(languageOf("README.md")).toBe("markdown");
  expect(languageOf("Cargo.toml")).toBe("toml");
  expect(languageOf("Cargo.lock")).toBe("toml");
  expect(languageOf("LICENSE")).toBeUndefined();
  expect(languageOf("image.png")).toBeUndefined();
  expect(languageOf("Makefile")).toBeUndefined();
});

test("a language that is not known comes back as it is", () => {
  expect(scan("cobol", "MOVE 1 TO X.", "")).toEqual({
    tokens: [{ text: "MOVE 1 TO X." }],
    state: "",
  });
});

test("a very long line is left alone", () => {
  const line = `const x = "${"a".repeat(700)}";`;
  expect(scan("ts", line).tokens).toEqual([{ text: line }]);
});

test("TypeScript: keywords, names, strings, numbers and comments", () => {
  expect(runs("ts", "export const limit = 42; // the most")).toEqual([
    "keyword:export",
    " ",
    "keyword:const",
    " limit = ",
    "number:42",
    "; ",
    "comment:// the most",
  ]);
  expect(runs("ts", "function handle(request: Request): Promise<void> {")).toEqual([
    "keyword:function",
    " ",
    "function:handle",
    "(request: ",
    "type:Request",
    "): ",
    "type:Promise",
    "<",
    "type:void",
    "> {",
  ]);
  expect(runs("ts", 'const s = "a \\" b" + \'c\';')).toEqual([
    "keyword:const",
    " s = ",
    'string:"a \\" b"',
    " + ",
    "string:'c'",
    ";",
  ]);
  expect(runs("ts", "if (event.type === null) return foo.bar(1)")).toEqual([
    "keyword:if",
    " (event.type === ",
    "constant:null",
    ") ",
    "keyword:return",
    " foo.",
    "function:bar",
    "(",
    "number:1",
    ")",
  ]);
});

test("a block comment and a template string go on over lines", () => {
  expect(all("ts", ["a /* one", "two", "three */ b"])).toEqual([
    "a ",
    "comment:/* one",
    "comment:two",
    "comment:three */",
    " b",
  ]);
  expect(all("ts", ["const t = `x", "y ${z}", "w`;"])).toEqual([
    "keyword:const",
    " t = ",
    "string:`x",
    "string:y ${z}",
    "string:w`",
    ";",
  ]);
});

test("Rust: macros, lifetimes, attributes and types", () => {
  expect(runs("rust", "pub fn step(input: &'a [u8]) -> Result<Vec<u8>, Error> {")).toEqual([
    "keyword:pub",
    " ",
    "keyword:fn",
    " ",
    "function:step",
    "(input: &",
    "type:'a",
    " [",
    "type:u8",
    "]) -> ",
    "type:Result",
    "<",
    "type:Vec",
    "<",
    "type:u8",
    ">, ",
    "type:Error",
    "> {",
  ]);
  expect(runs("rust", 'println!("{}", x); // c')).toEqual([
    "function:println",
    "!(",
    'string:"{}"',
    ", x); ",
    "comment:// c",
  ]);
  expect(runs("rust", "#[derive(Debug)]")).toEqual(["attr:#[derive(Debug)]"]);
  expect(runs("rust", "let c = 'x';")).toEqual(["keyword:let", " c = ", "string:'x'", ";"]);
});

test("Go: functions and raw strings", () => {
  expect(runs("go", "func (s *Server) Serve(ctx context.Context) error {")).toEqual([
    "keyword:func",
    " (s *",
    "type:Server",
    ") ",
    "function:Serve",
    "(ctx context.Context) ",
    "type:error",
    " {",
  ]);
  expect(all("go", ["x := `a", "b`"])).toEqual(["x := ", "string:`a", "string:b`"]);
});

test("Python: decorators, triple quotes and comments", () => {
  expect(runs("python", "@app.route('/x')")).toEqual([
    "function:@app.route",
    "(",
    "string:'/x'",
    ")",
  ]);
  expect(runs("python", "def run(self, n=3):  # go")).toEqual([
    "keyword:def",
    " ",
    "function:run",
    "(",
    "constant:self",
    ", n=",
    "number:3",
    "):  ",
    "comment:# go",
  ]);
  expect(all("python", ['x = """a', "b", 'c""" + y'])).toEqual([
    "x = ",
    'string:"""a',
    "string:b",
    'string:c"""',
    " + y",
  ]);
});

test("shell: variables, strings and keywords", () => {
  expect(runs("shell", 'if [ -z "$target" ]; then echo "usage: $0" >&2; fi # end')).toEqual([
    "keyword:if",
    " ",
    "function:[",
    " ",
    "attr:-z",
    " ",
    'string:"$target"',
    " ]",
    "keyword:;",
    " ",
    "keyword:then",
    " ",
    "keyword:echo",
    " ",
    'string:"usage: $0"',
    " >&",
    "number:2",
    "keyword:;",
    " ",
    "keyword:fi",
    " ",
    "comment:# end",
  ]);
  expect(runs("shell", "x=$HOME/bin")).toEqual(["property:x=$HOME", "/bin"]);
});

test("shell: each command of a list, pipeline or $(…) and its flags", () => {
  expect(runs("shell", "cd ~/x && grep -rn a src | head; f=$(git log --oneline)")).toEqual([
    "keyword:cd",
    " ~/x ",
    "keyword:&&",
    " ",
    "function:grep",
    " ",
    "attr:-rn",
    " a src ",
    "keyword:|",
    " ",
    "function:head",
    "keyword:;",
    " ",
    "property:f=$(",
    "function:git",
    " log ",
    "attr:--oneline",
    "keyword:)",
  ]);
  // A line after a `\` goes on with the arguments.
  expect(all("shell", ["make \\", "  build -j4"])).toEqual([
    "function:make",
    " \\",
    "  build ",
    "attr:-j4",
  ]);
});

test("JSON: keys are not values", () => {
  expect(runs("json", '  "limit": 380, "tags": ["a", null], "on": true')).toEqual([
    "  ",
    'property:"limit"',
    ": ",
    "number:380",
    ", ",
    'property:"tags"',
    ": [",
    'string:"a"',
    ", ",
    "constant:null",
    "], ",
    'property:"on"',
    ": ",
    "constant:true",
  ]);
});

test("CSS: selectors, properties, values and comments", () => {
  expect(runs("css", ".card-3:hover > a {")).toEqual([
    "type:.card-3",
    "keyword::hover",
    " > ",
    "tag:a",
    " {",
  ]);
  expect(runs("css", "  padding: 3px 16px; color: #336699;")).toEqual([
    "  ",
    "property:padding",
    ": ",
    "number:3px",
    " ",
    "number:16px",
    "; ",
    "property:color",
    ": ",
    "number:#336699",
    ";",
  ]);
  expect(runs("css", "  background: rgba(0, 0, 0, 0.5) !important;")).toContain("function:rgba");
  expect(all("css", ["a { /* one", "two */ color: red }"])).toEqual([
    "tag:a",
    " { ",
    "comment:/* one",
    "comment:two */",
    " color: red }",
  ]);
});

test("Markdown: headings, lists, code and links", () => {
  expect(runs("markdown", "## Step 3")).toEqual(["function:## Step 3"]);
  expect(runs("markdown", "- run `make step` and [read](http://x.y)")).toEqual([
    "keyword:- ",
    "run ",
    "string:`make step`",
    " and ",
    "function:[read]",
    "comment:(http://x.y)",
  ]);
  expect(all("markdown", ["```sh", "echo # not a heading", "```", "# Title"])).toEqual([
    "comment:```sh",
    "echo # not a heading",
    "comment:```",
    "function:# Title",
  ]);
});

test("Svelte: tags, attributes and the braces of a template", () => {
  expect(
    runs("html", '<div class="card" onclick={() => select(3)}>{#if open}x{/if}</div>'),
  ).toEqual([
    "tag:<div",
    " ",
    "attr:class",
    "=",
    'string:"card"',
    " ",
    "attr:onclick",
    "={() => ",
    "function:select",
    "(",
    "number:3",
    ")}>{",
    "keyword:#if",
    " open}x{",
    "keyword:/if",
    "}",
    "tag:</div",
    ">",
  ]);
});

test("Svelte: the script and the style are read as TypeScript and CSS, to their closing tags", () => {
  const lines = [
    '<script lang="ts">',
    "  let n = 1; /* a",
    "  b */",
    "</script>",
    "<style>",
    "  a { color: red; }",
    "</style>",
    "<!-- c",
    "d -->",
  ];
  expect(all("html", lines)).toEqual([
    "tag:<script",
    " ",
    "attr:lang",
    "=",
    'string:"ts"',
    ">",
    "  ",
    "keyword:let",
    " n = ",
    "number:1",
    "; ",
    "comment:/* a",
    "comment:  b */",
    "tag:</script",
    ">",
    "tag:<style",
    ">",
    "  ",
    "tag:a",
    " { ",
    "property:color",
    ": red; }",
    "tag:</style",
    ">",
    "comment:<!-- c",
    "comment:d -->",
  ]);
});

test("a highlighter scans lines on demand and keeps what it found", () => {
  const lines = ["/* a", "b", "c */", "d"];
  const light = new Highlighter("ts", lines);
  expect(light.tokens(3).map((t) => t.text)).toEqual(["d"]);
  expect(light.pending).toBe(false);
  expect(light.tokens(1)).toEqual([{ text: "b", kind: "comment" }]);
  expect(light.tokens(1)).toBe(light.tokens(1));
  expect(light.tokens(99)).toEqual([]);
});

test("warming scans ahead a bit at a time", () => {
  const light = new Highlighter(
    "ts",
    Array.from({ length: 1000 }, (_, i) => `const x${i} = ${i};`),
  );
  expect(light.pending).toBe(true);
  light.warm(400);
  expect(light.pending).toBe(true);
  light.warm(400);
  light.warm(400);
  expect(light.pending).toBe(false);
});
