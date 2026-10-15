// Resolves every fixtures/<name>/fixture.json into fixtures/<name>/resolved.json (ViewItem[]),
// the input every renderer's snapshot tests draw.
//   bun run daemon/scripts/fixtures.ts           write
//   bun run daemon/scripts/fixtures.ts --check   fail if any resolved.json is stale
import { existsSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import * as prettier from "prettier";
import { resolveFixture } from "../src/view/fixtures.ts";

const dir = resolve(import.meta.dir, "..", "..", "fixtures");
const check = process.argv.includes("--check");
const config = await prettier.resolveConfig(join(dir, "x.json"));
const stale: string[] = [];

for (const name of readdirSync(dir).sort()) {
  const source = join(dir, name, "fixture.json");
  if (!existsSync(source)) continue;
  const items = resolveFixture(name, JSON.parse(readFileSync(source, "utf8")));
  const out = join(dir, name, "resolved.json");
  const content = await prettier.format(JSON.stringify(items), { ...config, parser: "json" });
  const current = existsSync(out) ? readFileSync(out, "utf8") : undefined;
  if (current === content) continue;
  if (check) stale.push(out);
  else writeFileSync(out, content);
}

if (stale.length > 0) {
  process.stderr.write(
    `Stale fixtures (run \`make codegen\`):\n${stale.map((p) => `  ${p}`).join("\n")}\n`,
  );
  process.exit(1);
}
