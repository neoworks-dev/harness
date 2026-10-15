// Generates TS and Go types from protocol/schemas/*.schema.json, plus the JSON example files.
//
//   bun run protocol/codegen/main.ts           write generated files
//   bun run protocol/codegen/main.ts --check   fail if any generated file is stale
//
// Supported schema subset (anything else is an error, so the Rust generator in Phase 3 sees the
// same shapes):
//   - every named type lives in `$defs`; names are globally unique across files
//   - objects: `properties` + `required`, `additionalProperties: false`; no inline objects
//   - maps: `type: object` with `additionalProperties: <schema>` and no properties
//   - string enums as named defs (no inline enums)
//   - tagged unions: `oneOf` of `$ref`s to object defs + `discriminator.propertyName`; each
//     variant declares the tag as a required `const` string and belongs to exactly one union
//   - `{}` (or a schema with only a description) is opaque JSON
//   - optional = not in `required`; never `null`
import { readdirSync, readFileSync, writeFileSync, existsSync } from "node:fs";
import { join, resolve } from "node:path";
import * as prettier from "prettier";
import { generateGo } from "./go.ts";
import { generateTs } from "./ts.ts";
import { loadSchemas } from "./model.ts";
import { examples } from "../examples/examples.ts";

const root = resolve(import.meta.dir, "..");
const check = process.argv.includes("--check");

const model = loadSchemas(join(root, "schemas"));

const prettierConfig = await prettier.resolveConfig(join(root, "gen", "types.ts"));

async function formatTs(src: string): Promise<string> {
  return prettier.format(src, { ...prettierConfig, parser: "typescript" });
}

function formatGo(src: string): string {
  const res = Bun.spawnSync(["gofmt"], { stdin: new TextEncoder().encode(src) });
  if (res.exitCode !== 0) {
    throw new Error(`gofmt failed:\n${res.stderr.toString()}\n--- source ---\n${src}`);
  }
  return res.stdout.toString();
}

async function formatJson(value: unknown): Promise<string> {
  return prettier.format(JSON.stringify(value), { ...prettierConfig, parser: "json" });
}

const outputs = new Map<string, string>();
outputs.set(join(root, "gen", "types.ts"), await formatTs(generateTs(model)));
outputs.set(join(root, "go", "types.gen.go"), formatGo(generateGo(model)));
for (const [name, value] of Object.entries(examples)) {
  outputs.set(join(root, "examples", `${name}.json`), await formatJson(value));
}

const stale: string[] = [];
for (const [path, content] of outputs) {
  const current = existsSync(path) ? readFileSync(path, "utf8") : undefined;
  if (current === content) continue;
  if (check) stale.push(path);
  else writeFileSync(path, content);
}
// Example files whose source entry was removed.
for (const file of readdirSync(join(root, "examples"))) {
  const path = join(root, "examples", file);
  if (file.endsWith(".json") && !outputs.has(path)) stale.push(path);
}

if (stale.length > 0) {
  process.stderr.write(
    `${check ? "Generated files are stale" : "Leftover files"} (run \`make codegen\`):\n${stale.map((p) => `  ${p}`).join("\n")}\n`,
  );
  process.exit(1);
}
