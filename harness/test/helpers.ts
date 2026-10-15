import { mkdtemp, realpath, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach } from "bun:test";

const dirs: string[] = [];

afterEach(async () => {
  await Promise.all(dirs.splice(0).map((dir) => rm(dir, { recursive: true, force: true })));
});

/** A fresh temp directory (real path), removed after the test. */
export async function tempDir(): Promise<string> {
  const dir = await realpath(await mkdtemp(join(tmpdir(), "harness-test-")));
  dirs.push(dir);
  return dir;
}

export function resultText(result: { content: { type: string; text?: string }[] }): string {
  return result.content
    .filter((part) => part.type === "text")
    .map((part) => part.text ?? "")
    .join("\n");
}

/** The items whose `type` is `type`, narrowed. */
export function ofType<T extends { type: string }, K extends T["type"]>(
  items: T[],
  type: K,
): Extract<T, { type: K }>[] {
  return items.filter((item): item is Extract<T, { type: K }> => item.type === type);
}

export async function errorMessage(promise: Promise<unknown>): Promise<string> {
  try {
    await promise;
  } catch (error) {
    return error instanceof Error ? error.message : String(error);
  }
  throw new Error("expected the promise to reject");
}
