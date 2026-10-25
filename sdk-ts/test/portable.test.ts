// The SDK runs in browsers: its browser entry (browser.ts, the "browser" export condition) reaches
// nothing of node: or Bun. Only node.ts, index.ts and tools/ may, and a browser never loads them.
import { expect, test } from "bun:test";
import { mkdtempSync, readdirSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const SRC = join(import.meta.dir, "../src");

test("the browser entry bundles for a browser", async () => {
  // Bundled by a child process: Bun's bundler misreads files in a process that ran daemons.
  const build = Bun.spawn(
    [
      process.execPath,
      "build",
      join(SRC, "browser.ts"),
      "--target=browser",
      "--outdir",
      mkdtempSync(join(tmpdir(), "neo-bundle-")),
    ],
    { stdout: "pipe", stderr: "pipe" },
  );
  expect(await build.exited).toBe(0);
});

test("only node.ts, index.ts and tools/ import from node: or bun", async () => {
  for (const file of readdirSync(SRC)) {
    if (file === "node.ts" || file === "index.ts" || file === "tools") continue;
    const source = await Bun.file(join(SRC, file)).text();
    expect(source, file).not.toMatch(/from "(node:|bun)/);
    expect(source, file).not.toContain("process.");
  }
});
