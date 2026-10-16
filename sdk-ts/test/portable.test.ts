// The SDK runs in browsers: nothing but node.ts may reach for node: or Bun.
import { expect, test } from "bun:test";
import { mkdtempSync, readdirSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const SRC = join(import.meta.dir, "../src");

test("the public entry bundles for a browser", async () => {
  // Bundled by a child process: Bun's bundler misreads files in a process that ran daemons.
  const build = Bun.spawn(
    [
      process.execPath,
      "build",
      join(SRC, "index.ts"),
      "--target=browser",
      "--outdir",
      mkdtempSync(join(tmpdir(), "suite-bundle-")),
    ],
    { stdout: "pipe", stderr: "pipe" },
  );
  expect(await build.exited).toBe(0);
});

test("only node.ts imports from node: or bun", async () => {
  for (const file of readdirSync(SRC)) {
    if (file === "node.ts") continue;
    const source = await Bun.file(join(SRC, file)).text();
    expect(source, file).not.toMatch(/from "(node:|bun)/);
    expect(source, file).not.toContain("process.");
  }
});
