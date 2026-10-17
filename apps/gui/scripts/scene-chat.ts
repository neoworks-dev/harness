// A throwaway daemon with a scripted conversation, for screenshots of the transcript
// (SCENE_SCRIPT=scene-chat.ts SCENE=chat apps/gui/scripts/shot.sh): finished turns with commands,
// reads, edits, searches, a plan, a delegation, a failure and a stopped turn, with real
// timestamps, and a turn that is still running. Run by shot.sh on the temporary socket;
// SCENE_CWD is the window's directory.
import { writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { createDaemon } from "../../../daemon/src/daemon.ts";
import { createEchoAgent } from "../../../daemon/src/echo-agent.ts";
import { silentLogger } from "../../../daemon/src/log.ts";
import { resolvePaths } from "../../../daemon/src/paths.ts";
import type { ToolCallContent, ToolKind } from "@suite/protocol/types";

const cwd = process.env.SCENE_CWD;
if (cwd === undefined) throw new Error("SCENE_CWD is not set");

const paths = resolvePaths();
const daemon = await createDaemon({
  paths,
  agent: createEchoAgent(),
  log: silentLogger,
  port: 0,
  defaultCwd: process.env.SCENE_ELSEWHERE ?? cwd,
});

const main = daemon.sessions.create({ cwd, title: "Dark mode for the header" });
let clock = Date.now() - 3 * 60 * 60 * 1000;
type Event = Parameters<typeof daemon.store.append>[1];
/** Appends an event `after` seconds after the previous one. */
const at = (after: number, event: Event): void => {
  clock += after * 1000;
  daemon.store.append(main.id, event, clock);
};

const call = (id: string, title: string, kind: ToolKind, rawInput: unknown, tool: string): void => {
  at(1, { type: "tool_call", toolCallId: id, title, kind, status: "pending", rawInput, tool });
};
const done = (id: string, content?: ToolCallContent[]): void => {
  at(2, { type: "tool_call_update", toolCallId: id, status: "completed", content });
};
const say = (id: string, text: string): void => {
  at(1, { type: "agent_message_chunk", messageId: id, text });
};

// Turn 1: a long one with everything in it.
at(0, { type: "user_message", app: "desktop", text: "Refactor the header and add dark mode" });
at(2, {
  type: "agent_thought_chunk",
  messageId: "t1",
  text: "Checking how the header takes its colours. The theme has no dark palette yet.",
});
say("m1", "On it. I'll read the header first, then move its colours into the theme.");
call("r1", "read src/header.ts", "read", { path: `${cwd}/src/header.ts` }, "read");
done("r1", [{ type: "text", text: "export const header = {}" }]);
call("r2", "read src/theme.ts", "read", { path: `${cwd}/src/theme.ts` }, "read");
done("r2", [{ type: "text", text: "export const theme = {}" }]);
call("s1", "grep colours", "search", { pattern: "colour", path: `${cwd}/src` }, "grep");
done("s1", [{ type: "text", text: "src/header.ts:3: colour\nsrc/theme.ts:9: colour" }]);
at(1, {
  type: "plan",
  entries: [
    { content: "Read the header component", status: "completed" },
    { content: "Move the colours into the theme", status: "in_progress" },
    { content: "Write the dark palette", status: "pending" },
  ],
});
call("b1", "bash: bun test", "execute", { command: "bash -lc 'bun test'" }, "bash");
done("b1", [
  { type: "text", text: "bun test v1.3\n\n 12 pass\n 0 fail\nRan 12 tests across 3 files." },
]);
call("e1", "edit a.txt", "edit", { path: `${cwd}/a.txt` }, "edit");
done("e1", [
  { type: "text", text: "Successfully replaced 1 block" },
  { type: "diff", path: `${cwd}/a.txt`, oldText: "one\ntwo\n", newText: "one\nthree\n" },
]);
call("e2", "write src/dark.css", "edit", { path: `${cwd}/src/dark.css` }, "write");
done("e2", [{ type: "diff", path: `${cwd}/src/dark.css`, newText: ":root {\n  --bg: #111;\n}\n" }]);
call(
  "w1",
  "search: scrollbar-gutter browser support",
  "search",
  { query: "scrollbar-gutter support" },
  "web_search",
);
done("w1", [{ type: "text", text: "Supported in all current browsers." }]);
call("f1", "figma.export", "other", { frame: "Header" }, "figma.export");
done("f1", [{ type: "text", text: "3 frames exported" }]);
at(30, { type: "agent_thought_chunk", messageId: "t2", text: "Now the summary." });
say(
  "m2",
  [
    "## Done",
    "",
    "The header now reads its colours from the theme, and there is a dark palette.",
    "",
    "- Moved the colours into `theme.ts`",
    "- Added `src/dark.css` with the dark palette",
    "- Reserved the scrollbar gutter, see [the spec](https://example.com/scrollbar-gutter)",
    "",
    "```ts",
    "export const theme = { dark: true };",
    "```",
    "",
    "Run `bun test` to check it.",
  ].join("\n"),
);
at(60, { type: "turn_ended", stopReason: "end_turn" });

// Turn 2: a short answer with nothing to fold.
at(120, { type: "user_message", app: "desktop", text: "thanks, and the tests?" });
say("m3", "They pass: 12 tests, none failing.");
at(2, { type: "turn_ended", stopReason: "end_turn" });

// Turn 3: a long message from the user, and a command that fails.
at(300, {
  type: "user_message",
  app: "desktop",
  text: Array.from(
    { length: 12 },
    (_, i) => `Line ${i + 1} of a long request that goes on about what the footer should do.`,
  ).join("\n"),
});
call("b2", "bash: make lint", "execute", { command: "make lint" }, "bash");
at(3, {
  type: "tool_call_update",
  toolCallId: "b2",
  status: "failed",
  content: [{ type: "text", text: "src/header.ts:3:1 error unused\nExit code 2" }],
});
say("m4", "The lint failed on an unused import; I'll fix it next.");
at(1, { type: "turn_ended", stopReason: "end_turn" });

// Turn 4: a delegation, a permission asked and answered, and a stop.
at(200, { type: "user_message", app: "desktop", text: "ask pi for the palette" });
call(
  "d1",
  "pi · Write the dark palette",
  "think",
  { agent: "pi", task: "Write the dark palette" },
  "delegate",
);
done("d1", [{ type: "text", text: "Palette written to src/dark.css" }]);
at(1, {
  type: "permission_request",
  id: "perm1",
  agent: "claude",
  title: "rm -rf dist",
  options: [{ id: "a", name: "Allow", kind: "allow_once" }],
});
at(4, { type: "permission_resolved", id: "perm1", option: "a" });
at(20, { type: "turn_ended", stopReason: "cancelled" });

// Turn 5: running now, forty seconds in.
clock = Date.now() - 40 * 1000;
at(0, { type: "user_message", app: "desktop", text: "now do the footer too" });
at(2, {
  type: "agent_thought_chunk",
  messageId: "t5",
  text: "The footer shares the header's colours.",
});
call("r5", "read src/footer.ts", "read", { path: `${cwd}/src/footer.ts` }, "read");
done("r5", [{ type: "text", text: "export const footer = {}" }]);
call("b5", "bash: bun run build", "execute", { command: "bun run build" }, "bash");
at(1, { type: "tool_call_update", toolCallId: "b5", status: "in_progress" });

writeFileSync(join(dirname(paths.socket), "scene-ready"), "");

process.on("SIGTERM", () => void daemon.stop().then(() => process.exit(0)));
await new Promise(() => {});
