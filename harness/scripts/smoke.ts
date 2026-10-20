// Live smoke test (not part of `bun test`): one real prompt through the neo agent with the real
// config. Usage: bun run harness/scripts/smoke.ts
import { mkdtemp, readFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { loadHarnessConfig } from "../src/config.ts";
import { createNeoAgent } from "../src/neo-agent.ts";
import type { AgentUpdate } from "../src/agent.ts";

function show(update: AgentUpdate): string | undefined {
  switch (update.type) {
    case "agent_message_chunk":
      return `text: ${update.text}`;
    case "agent_thought_chunk":
      return `thought: ${update.text}`;
    case "tool_call":
      return `tool_call ${update.toolCallId} [${update.kind}] ${update.title}`;
    case "tool_call_update":
      return `tool_call_update ${update.toolCallId} ${update.status ?? "(progress)"}`;
    case "harness_message":
      return undefined;
  }
}

const config = loadHarnessConfig();
const cwd = await mkdtemp(join(tmpdir(), "harness-smoke-"));
const logger = {
  warn: (message: string) => process.stderr.write(`warn: ${message}\n`),
  error: (message: string) => process.stderr.write(`error: ${message}\n`),
};
process.stdout.write(`model ${config.provider}/${config.modelId}, cwd ${cwd}\n`);

const session = await createNeoAgent({ config, logger }).newSession({
  sessionId: "smoke",
  cwd,
  mcpServers: [],
});
let messages = 0;
const result = await session.prompt({
  text: "Create hello.txt containing hi, then change hi to hello with the edit tool",
  app: "smoke",
  pendingEvents: [],
  onUpdate: (update) => {
    if (update.type === "harness_message") messages++;
    const line = show(update);
    if (line) process.stdout.write(`${line}\n`);
  },
});
process.stdout.write(`result: ${JSON.stringify(result)}, ${messages} harness messages\n`);
const content = await readFile(join(cwd, "hello.txt"), "utf8").catch(() => "(missing)");
process.stdout.write(`hello.txt: ${JSON.stringify(content)}\n`);
