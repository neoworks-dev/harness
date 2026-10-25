// A throwaway daemon whose session receives a long agent reply as it streams, for measuring how
// the window keeps up (NEO_GUI_SESSION=s_stream SCENE_SCRIPT=scene-stream.ts SCENE=stream
// apps/gui/scripts/shot.sh).
// After STREAM_DELAY_MS (default 3000) it appends STREAM_CHARS (default 12000) characters of
// markdown in chunks of STREAM_CHUNK (default 12) every STREAM_EVERY_MS (default 20), then ends the
// turn. The daemon's own time per chunk goes to STREAM_OUT (default /tmp/stream-daemon.txt).
import { writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { createDaemon } from "../../../daemon/src/daemon.ts";
import { createEchoAgent } from "../../../daemon/src/echo-agent.ts";
import { silentLogger } from "../../../daemon/src/log.ts";
import { resolvePaths } from "../../../daemon/src/paths.ts";

const cwd = process.env.SCENE_CWD;
if (cwd === undefined) throw new Error("SCENE_CWD is not set");
const num = (name: string, fallback: number): number => Number(process.env[name] ?? fallback);
const delay = num("STREAM_DELAY_MS", 3000);
const chars = num("STREAM_CHARS", 12000);
const chunk = num("STREAM_CHUNK", 12);
const every = num("STREAM_EVERY_MS", 20);

const paths = resolvePaths();
const daemon = await createDaemon({
  paths,
  agent: createEchoAgent(),
  log: silentLogger,
  port: 0,
  defaultCwd: cwd,
});
// A fixed id, so the window can be pointed at it: NEO_GUI_SESSION=s_stream.
const session = daemon.store.createSession({ id: "s_stream", cwd, title: "A long answer" });
daemon.store.append(session.id, { type: "user_message", app: "desktop", text: "explain it all" });

/** A reply like a model's: prose, a list, code and a table, over and over. */
function reply(length: number): string {
  const parts: string[] = [];
  let n = 0;
  while (parts.join("").length < length) {
    n++;
    parts.push(
      `## Part ${n}\n\nThe header reads its colours from the theme, so a change to the palette ` +
        `reaches every pane at once. This paragraph is long enough to wrap a few times in a ` +
        `normal window, the way a model's explanation does when it goes on about **details** ` +
        `and \`identifiers\` and [links](https://example.com/${n}).\n\n` +
        `- the first point, with some words after it\n- the second point\n- a third one, ` +
        `which is longer and wraps onto another line in a narrow window\n\n` +
        "```ts\n" +
        `export function part${n}(value: number): number {\n  return value * ${n} + 1;\n}\n` +
        "```\n\n" +
        `| Name | Value |\n| ---- | ----- |\n| part | ${n} |\n| twice | ${2 * n} |\n\n`,
    );
  }
  return parts.join("").slice(0, length);
}

writeFileSync(join(dirname(paths.socket), "scene-ready"), "");

await Bun.sleep(delay);
const text = reply(chars);
const times: number[] = [];
const started = performance.now();
for (let at = 0; at < text.length; at += chunk) {
  const t = performance.now();
  daemon.store.append(session.id, {
    type: "agent_message_chunk",
    messageId: "m1",
    text: text.slice(at, at + chunk),
  });
  times.push(performance.now() - t);
  await Bun.sleep(every);
}
daemon.store.append(session.id, { type: "turn_ended", stopReason: "end_turn" });
const sorted = [...times].sort((a, b) => a - b);
const pct = (p: number): string => (sorted[Math.floor((sorted.length - 1) * p)] ?? 0).toFixed(2);
const total = times.reduce((a, b) => a + b, 0);
writeFileSync(
  process.env.STREAM_OUT ?? "/tmp/stream-daemon.txt",
  `chunks ${times.length}, wall ${((performance.now() - started) / 1000).toFixed(1)} s, ` +
    `daemon ${total.toFixed(0)} ms total; per chunk p50 ${pct(0.5)} ms, p90 ${pct(0.9)} ms, ` +
    `max ${pct(1)} ms; first 50 avg ${(times.slice(0, 50).reduce((a, b) => a + b, 0) / 50).toFixed(2)} ms, ` +
    `last 50 avg ${(times.slice(-50).reduce((a, b) => a + b, 0) / 50).toFixed(2)} ms\n`,
);

process.on("SIGTERM", () => void daemon.stop().then(() => process.exit(0)));
await new Promise(() => {});
