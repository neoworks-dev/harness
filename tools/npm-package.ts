// Builds a workspace package for npm: `bun run tools/npm-package.ts sdk-ts` writes sdk-ts/dist/,
// which is what gets published (`npm publish sdk-ts/dist --access public`). In the repo the
// packages stay TypeScript sources; on npm they are JavaScript with type declarations, since Node
// strips no types inside node_modules.
//
// - JavaScript: one ESM bundle per entry (Bun.build), dependencies left as imports; JSON is inlined.
// - Types: `tsc` declarations, with `./x.ts` imports rewritten to `./x.js`.
// - package.json: the source's name, version and dependencies (`workspace:*` becomes `^<version>`
//   of that package), and exports pointing at dist.
import {
  cpSync,
  existsSync,
  mkdirSync,
  readdirSync,
  readFileSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { join, relative } from "node:path";

/** What a package publishes: export path → its entry for Bun/Node and, if it differs, browsers. */
interface Spec {
  exports: Record<string, { default: string; browser?: string; typesOnly?: boolean }>;
}

const SPECS: Record<string, Spec> = {
  "sdk-ts": {
    exports: { ".": { default: "src/index.ts", browser: "src/browser.ts" } },
  },
  protocol: {
    exports: {
      "./types": { default: "gen/types.ts", typesOnly: true },
      "./validate": { default: "src/validate.ts" },
      "./core-tools": { default: "src/core-tools.ts" },
    },
  },
};

const ROOT = join(import.meta.dir, "..");

interface PackageJson {
  name: string;
  version?: string;
  description?: string;
  license?: string;
  workspaces?: string[];
  dependencies?: Record<string, string>;
  [key: string]: unknown;
}

function isPackageJson(value: unknown): value is PackageJson {
  return (
    typeof value === "object" && value !== null && "name" in value && typeof value.name === "string"
  );
}

function readJson(path: string): PackageJson {
  const parsed: unknown = JSON.parse(readFileSync(path, "utf8"));
  if (!isPackageJson(parsed)) throw new Error(`${path} is not a package.json`);
  return parsed;
}

/** Every workspace package's version, by name, to replace `workspace:*`. */
function workspaceVersions(): Map<string, string> {
  const versions = new Map<string, string>();
  for (const dir of readJson(join(ROOT, "package.json")).workspaces ?? []) {
    const file = join(ROOT, dir, "package.json");
    if (!existsSync(file)) continue;
    const pkg = readJson(file);
    if (pkg.version) versions.set(pkg.name, pkg.version);
  }
  return versions;
}

/** `foo/bar.ts` → `bar` (the name its output files get). */
function stem(entry: string): string {
  return entry.replace(/^.*\//, "").replace(/\.ts$/, "");
}

function walk(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    return statSync(path).isDirectory() ? walk(path) : [path];
  });
}

async function main(): Promise<void> {
  const name = process.argv[2];
  const spec = name ? SPECS[name] : undefined;
  if (!name || !spec) {
    process.stderr.write(`usage: bun run tools/npm-package.ts <${Object.keys(SPECS).join("|")}>\n`);
    process.exit(2);
  }
  const dir = join(ROOT, name);
  const source = readJson(join(dir, "package.json"));
  if (!source.version) throw new Error(`${name}/package.json has no version`);
  const external = Object.keys(source.dependencies ?? {});
  const out = join(dir, "dist");
  rmSync(out, { recursive: true, force: true });
  mkdirSync(out, { recursive: true });

  // JavaScript
  for (const entry of Object.values(spec.exports)) {
    const builds: [string, "node" | "browser"][] = [];
    if (!entry.typesOnly) builds.push([entry.default, "node"]);
    if (entry.browser) builds.push([entry.browser, "browser"]);
    for (const [file, target] of builds) {
      const result = await Bun.build({
        entrypoints: [join(dir, file)],
        outdir: out,
        target,
        format: "esm",
        external: external.flatMap((dep) => [dep, `${dep}/*`]),
      });
      if (!result.success) throw new AggregateError(result.logs, `bundling ${file} failed`);
    }
  }

  // Types: tsc over the package, with a config of its own next to it.
  const types = join(out, "types");
  const config = join(dir, "tsconfig.npm.json");
  writeFileSync(
    config,
    JSON.stringify({
      extends: relative(dir, join(ROOT, "tsconfig.json")),
      compilerOptions: {
        noEmit: false,
        declaration: true,
        emitDeclarationOnly: true,
        outDir: relative(dir, types),
        rootDir: ".",
        paths: {},
      },
      include: [
        ...new Set(Object.values(spec.exports).flatMap((e) => [e.default, e.browser ?? e.default])),
      ],
    }),
  );
  try {
    const tsc = Bun.spawnSync(["bunx", "tsc", "-p", config], {
      cwd: ROOT,
      stdout: "inherit",
      stderr: "inherit",
    });
    if (tsc.exitCode !== 0) throw new Error("tsc failed");
  } finally {
    rmSync(config);
  }
  for (const file of walk(types)) {
    if (!file.endsWith(".d.ts")) continue;
    const text = readFileSync(file, "utf8").replace(
      /(from\s+["']\.{1,2}\/[^"']+)\.ts(["'])/g,
      "$1.js$2",
    );
    writeFileSync(file, text);
  }

  // package.json
  const typesOf = (entry: string): string =>
    `./${relative(out, join(types, entry.replace(/\.ts$/, ".d.ts")))}`;
  const exports: Record<string, unknown> = {};
  for (const [path, entry] of Object.entries(spec.exports)) {
    if (entry.typesOnly) {
      exports[path] = { types: typesOf(entry.default) };
      continue;
    }
    const node = { types: typesOf(entry.default), default: `./${stem(entry.default)}.js` };
    exports[path] = entry.browser
      ? {
          browser: { types: typesOf(entry.browser), default: `./${stem(entry.browser)}.js` },
          ...node,
        }
      : node;
  }
  const versions = workspaceVersions();
  const dependencies = Object.fromEntries(
    Object.entries(source.dependencies ?? {}).map(([dep, range]) => {
      if (!range.startsWith("workspace:")) return [dep, range];
      const version = versions.get(dep);
      if (!version) throw new Error(`${dep} has no version to publish against`);
      return [dep, `^${version}`];
    }),
  );
  const published: PackageJson = {
    name: source.name,
    version: source.version,
    ...(source.description && { description: source.description }),
    license: source.license ?? "MIT",
    type: "module",
    exports,
    ...(Object.keys(dependencies).length > 0 && { dependencies }),
  };
  writeFileSync(join(out, "package.json"), `${JSON.stringify(published, null, 2)}\n`);
  if (existsSync(join(dir, "README.md"))) cpSync(join(dir, "README.md"), join(out, "README.md"));
  cpSync(join(ROOT, "LICENSE"), join(out, "LICENSE"));
  process.stdout.write(
    `${source.name}@${source.version} → ${relative(ROOT, out)} (npm publish ${relative(ROOT, out)} --access public)\n`,
  );
}

await main();
