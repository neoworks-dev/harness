// A bigger working tree for measuring the viewer:
//   SCENE=side SCENE_SETUP='bun run apps/gui/scripts/scene-big.ts' apps/gui/scripts/shot.sh …
// commits nine files of a few hundred lines in different languages, then edits every fourth
// function of each, so the diff has thousands of lines and many hunks. Runs in the repository.
import { mkdirSync, writeFileSync } from "node:fs";
import { dirname } from "node:path";

const COUNT = Number(process.env.BIG_FUNCTIONS ?? 40);

type Maker = (i: number) => string[];

const makers: Record<string, Maker> = {
  "src/server/handlers.ts": (i) => [
    `/** Handles request number ${i}. */`,
    `export async function handle${i}(request: Request, state: State): Promise<Response> {`,
    `  const url = new URL(request.url);`,
    `  const limit = Number(url.searchParams.get("limit") ?? ${i + 10});`,
    `  if (limit > 1000) {`,
    `    return new Response("too many", { status: 400 });`,
    `  }`,
    `  const rows = await state.db.query("select * from items where id > ?", [${i}]);`,
    `  const items = rows.map((row) => ({ id: row.id, name: String(row.name).trim() }));`,
    `  return Response.json({ items: items.slice(0, limit), total: items.length });`,
    `}`,
    ``,
  ],
  "src/components/Card.svelte": (i) => [
    `<!-- card ${i} -->`,
    `<div class="card card-${i}" onclick={() => select(${i})}>`,
    `  {#each items as item (item.id)}`,
    `    <span class:active={item.id === ${i}}>{item.label} ({count})</span>`,
    `  {/each}`,
    `  {#if open}<Details id={${i}} onclose={() => (open = false)} />{/if}`,
    `</div>`,
    ``,
  ],
  "crates/core/src/engine.rs": (i) => [
    `/// Runs step ${i} of the pipeline.`,
    `pub fn step_${i}(input: &[u8], opts: &Options) -> Result<Vec<u8>, Error> {`,
    `    let mut out = Vec::with_capacity(input.len() + ${i});`,
    `    for (n, byte) in input.iter().enumerate() {`,
    `        if opts.skip.contains(&n) { continue; }`,
    `        out.push(byte.wrapping_add(${i % 7} as u8));`,
    `    }`,
    `    // "done" is a string with a // inside`,
    `    Ok(out)`,
    `}`,
    ``,
  ],
  "cmd/serve/main.go": (i) => [
    `// Serve${i} answers requests for shard ${i}.`,
    `func (s *Server) Serve${i}(ctx context.Context, req *Request) (*Reply, error) {`,
    `\tif req == nil {`,
    `\t\treturn nil, errors.New("nil request")`,
    `\t}`,
    `\tvalues := make(map[string]int, ${i + 4})`,
    `\tfor _, key := range req.Keys {`,
    `\t\tvalues[key] += len(key) * ${i}`,
    `\t}`,
    `\treturn &Reply{Values: values, Shard: ${i}}, nil`,
    `}`,
    ``,
  ],
  "tools/convert.py": (i) => [
    `def convert_${i}(path: str, *, strict: bool = False) -> dict:`,
    `    """Converts file ${i}."""`,
    `    with open(path, encoding="utf-8") as handle:`,
    `        data = json.load(handle)`,
    `    result = {k: v for k, v in data.items() if not k.startswith("_")}`,
    `    if strict and len(result) < ${i}:`,
    `        raise ValueError(f"too few keys in {path}")  # ${i}`,
    `    return result`,
    ``,
  ],
  "styles/app.css": (i) => [
    `.card-${i} {`,
    `  display: flex;`,
    `  padding: ${i % 12}px 16px;`,
    `  color: #${(0x336699 + i * 977).toString(16).slice(0, 6)};`,
    `  background: rgba(0, 0, 0, 0.${i % 9});`,
    `}`,
    `.card-${i}:hover > .title { text-decoration: underline; }`,
    ``,
  ],
  "config/settings.json": (i) => [
    `  "feature_${i}": {`,
    `    "enabled": ${i % 2 === 0},`,
    `    "limit": ${i * 10},`,
    `    "name": "feature number ${i}",`,
    `    "tags": ["a", "b", null]`,
    `  },`,
  ],
  "docs/guide.md": (i) => [
    `## Step ${i}`,
    ``,
    `Run \`make step-${i}\` and check the **output** against the [spec](https://example.com/${i}).`,
    ``,
    `- first point of step ${i}`,
    `- second point, with *emphasis*`,
    ``,
    "```sh",
    `./run.sh --step ${i}`,
    "```",
    ``,
  ],
  "scripts/deploy.sh": (i) => [
    `deploy_${i}() {`,
    `  local target="$1"`,
    `  if [ -z "$target" ]; then echo "usage: deploy_${i} TARGET" >&2; return 1; fi`,
    `  for host in $(cat hosts-${i}.txt); do`,
    `    ssh "$host" "systemctl restart app-${i}" || echo "failed: $host"`,
    `  done`,
    `}`,
    ``,
  ],
};

const run = (...cmd: string[]): void => {
  const done = Bun.spawnSync(cmd, { stdout: "ignore", stderr: "inherit" });
  if (done.exitCode !== 0) throw new Error(`${cmd.join(" ")} failed`);
};

const bodies = new Map<string, string[][]>();
for (const [path, make] of Object.entries(makers)) {
  const blocks = Array.from({ length: COUNT }, (_, i) => make(i + 1));
  bodies.set(path, blocks);
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, `${blocks.flat().join("\n")}\n`);
}
run("git", "add", "-A");
run(
  "git",
  "-c",
  "user.name=t",
  "-c",
  "user.email=t@t",
  "-c",
  "commit.gpgsign=false",
  "commit",
  "-q",
  "-m",
  "big",
);

// Every fourth block changes: one line edited, one added and one dropped.
for (const [path, blocks] of bodies) {
  const edited = blocks.map((block, n) => {
    if (n % 4 !== 1) return block;
    const next = [...block];
    next[2] = `${next[2] ?? ""} // changed`;
    next.splice(4, 0, "  /* inserted line */");
    next.splice(6, 1);
    return next;
  });
  writeFileSync(path, `${edited.flat().join("\n")}\n`);
}
