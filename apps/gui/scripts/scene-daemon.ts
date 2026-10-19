// A throwaway daemon with a scripted scene, for screenshots (SCENE=side apps/gui/scripts/shot.sh):
// the echo agent as in daemon/src/main.ts, plus sessions and events appended by hand to show
// what the echo agent never does: a plan, edits, subagents, a session that finishes while another
// is shown. Run by shot.sh on the temporary socket; SCENE_CWD is the window's directory.
import { writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { createDaemon } from "../../../daemon/src/daemon.ts";
import { createEchoAgent } from "../../../daemon/src/echo-agent.ts";
import { silentLogger } from "../../../daemon/src/log.ts";
import { resolvePaths } from "../../../daemon/src/paths.ts";

const cwd = process.env.SCENE_CWD;
if (cwd === undefined) throw new Error("SCENE_CWD is not set");

const paths = resolvePaths();
const daemon = await createDaemon({
  paths,
  agent: createEchoAgent(),
  log: silentLogger,
  port: 0,
  // The empty session a daemon starts with belongs elsewhere, so the window picks the scene.
  defaultCwd: process.env.SCENE_ELSEWHERE ?? cwd,
});

const main = daemon.sessions.create({ cwd, title: "Refactor the header and add dark mode" });
const append = (event: Parameters<typeof daemon.store.append>[1]): void => {
  daemon.store.append(main.id, event);
};

append({ type: "user_message", app: "desktop", text: "Refactor the header and add dark mode" });
append({
  type: "agent_message_chunk",
  messageId: "m1",
  text: "On it. I'll start with the header.",
});
append({
  type: "plan",
  entries: [
    { content: "Read the header component", status: "completed" },
    {
      content: "Move the colours into the theme and reserve the scrollbar gutter",
      status: "in_progress",
    },
    { content: "Write the dark palette", status: "pending" },
    { content: "Run the tests", status: "pending" },
  ],
});
append({
  type: "plan",
  agent: "claude",
  entries: [{ content: "Review the contrast of the settings page", status: "in_progress" }],
});
append({
  type: "tool_call",
  toolCallId: "t1",
  title: "edit a.txt",
  kind: "edit",
  status: "pending",
});
append({
  type: "tool_call_update",
  toolCallId: "t1",
  status: "completed",
  content: [{ type: "diff", path: `${cwd}/a.txt`, oldText: "one\ntwo\n", newText: "one\nthree\n" }],
});
append({
  type: "tool_call",
  toolCallId: "g1",
  title: "edit new.txt",
  kind: "edit",
  status: "pending",
  agent: "claude",
});
append({
  type: "tool_call_update",
  toolCallId: "g1",
  status: "completed",
  content: [{ type: "diff", path: `${cwd}/new.txt`, newText: "fresh\n" }],
});

// A delegation into a worktree runs as a session of its own.
const child = daemon.sessions.create({
  cwd: `${cwd}/.worktrees/dark`,
  title: "Dark palette",
  parent: main.id,
});
daemon.sessions.input(child.id, "desktop", "wait:600000 write the dark palette");

append({
  type: "subagent_started",
  id: "d1",
  agent: "neo",
  task: "Search the web for scrollbar-gutter support",
  model: "deepseek/deepseek-v4-flash",
});
append({
  type: "tool_call",
  toolCallId: "neo:t1",
  title: "search: scrollbar-gutter browser support",
  kind: "search",
  status: "in_progress",
  agent: "neo",
});
append({ type: "usage", agent: "neo", inputTokens: 3000, outputTokens: 412 });
append({
  type: "subagent_started",
  id: "d2",
  agent: "claude",
  task: "Review the contrast",
  model: "sonnet",
});
append({
  type: "agent_thought_chunk",
  messageId: "c1",
  text: "Checking the palette",
  agent: "claude",
});
append({
  type: "subagent_started",
  id: "d3",
  agent: "codex",
  task: "Write the dark palette",
  cwd: `${cwd}/.worktrees/dark`,
  child: child.id,
});

// Another session that works in the background and finishes while this one is shown.
const background = daemon.sessions.create({
  cwd: process.env.SCENE_ELSEWHERE ?? cwd,
  title: "Update the changelog",
});
daemon.sessions.input(background.id, "desktop", "wait:3000 changelog");

// shot.sh waits for this before it opens the window, which lists the sessions once.
writeFileSync(join(dirname(paths.socket), "scene-ready"), "");

// The daemon keeps running until shot.sh stops it.
process.on("SIGTERM", () => void daemon.stop().then(() => process.exit(0)));
await new Promise(() => {});
