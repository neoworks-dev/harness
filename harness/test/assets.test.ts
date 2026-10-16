import { expect, test } from "bun:test";
import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { createBashTool } from "../src/tools/bash.ts";
import { createReadTool } from "../src/tools/read.ts";
import { createWriteTool } from "../src/tools/write.ts";
import { resolveAssetsInCommand, resolvePath } from "../src/tools/paths.ts";
import { errorMessage, resultText, tempDir } from "./helpers.ts";

const HANDLE = "asset://img-0123456789ab";

test("asset handles resolve to the stored file in paths and commands", () => {
  const assets = (h: string): string | undefined =>
    h === HANDLE ? "/data/assets/img-0123456789ab.png" : undefined;
  expect(resolvePath("/w", HANDLE, assets)).toBe("/data/assets/img-0123456789ab.png");
  expect(() => resolvePath("/w", "asset://img-ffffffffffff", assets)).toThrow("Unknown asset");
  expect(resolveAssetsInCommand(`cp ${HANDLE} logo.png`, assets)).toBe(
    "cp /data/assets/img-0123456789ab.png logo.png",
  );
  // Unknown handles stay; odd paths are quoted.
  expect(resolveAssetsInCommand("cat asset://img-ffffffffffff", assets)).toBe(
    "cat asset://img-ffffffffffff",
  );
  expect(resolveAssetsInCommand(`cp ${HANDLE} x`, () => "/a b/it's.png")).toBe(
    `cp '/a b/it'\\''s.png' x`,
  );
});

test("bash and read accept handles; write refuses them", async () => {
  const cwd = await tempDir();
  const stored = join(cwd, "stored.txt");
  writeFileSync(stored, "logo bytes", { mode: 0o400 });
  const assets = (h: string): string | undefined => (h === HANDLE ? stored : undefined);

  await createBashTool(cwd, assets).execute("t", { command: `cp ${HANDLE} copy.txt` });
  expect(readFileSync(join(cwd, "copy.txt"), "utf8")).toBe("logo bytes");
  expect(resultText(await createReadTool(cwd, assets).execute("t", { path: HANDLE }))).toContain(
    "logo bytes",
  );
  const write = createWriteTool(cwd).execute("t", { path: HANDLE, content: "x" });
  expect(await errorMessage(write)).toContain("read-only");
});
