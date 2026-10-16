import { afterEach, expect, test } from "bun:test";
import { startTestDaemon, type TestDaemon } from "../../daemon/src/testing.ts";
import { SuiteClient } from "../src/client.ts";

const cleanups: (() => Promise<void>)[] = [];
afterEach(async () => {
  for (const cleanup of cleanups.splice(0).reverse()) await cleanup();
});

test("setContext updates the app's context and is kept for reconnects", async () => {
  const daemon: TestDaemon = await startTestDaemon();
  cleanups.push(() => daemon.stop());
  const env = { SUITE_SOCKET: daemon.info.socket };
  const app = await SuiteClient.connect({
    client: "code-editor",
    env,
    presence: { app: "code-editor", kind: "app", open: true, context: { line: "1" } },
  });
  cleanups.push(() => app.close());
  await app.setContext({ file: "/site/a.ts", line: "40" });
  const listed = daemon.connectors.list().find((c) => c.app === "code-editor");
  expect(listed?.context).toEqual({ file: "/site/a.ts", line: "40" });

  const bare = await SuiteClient.connect({ client: "bare", env });
  cleanups.push(() => bare.close());
  expect(bare.setContext({ a: "b" })).rejects.toThrow("setPresence");
});
