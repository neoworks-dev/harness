// A throwaway daemon whose agent has answered with OpenUI docs, for screenshots of how the window
// draws them (SCENE_SCRIPT=scene-openui.ts SCENE=openui SCENE_DOCS=hello,metrics
// apps/gui/scripts/shot.sh). Each doc named in SCENE_DOCS (comma separated, in the order given;
// all of them when unset) is one agent message with prose around a ```openui fence. Run by shot.sh
// on the temporary socket; SCENE_CWD is the window's directory.
import { writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { createDaemon } from "../../../daemon/src/daemon.ts";
import { createEchoAgent } from "../../../daemon/src/echo-agent.ts";
import { silentLogger } from "../../../daemon/src/log.ts";
import { resolvePaths } from "../../../daemon/src/paths.ts";
import { DOCS } from "./openui-docs.ts";

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

const wanted = (process.env.SCENE_DOCS ?? Object.keys(DOCS).join(",")).split(",").filter(Boolean);
const main = daemon.sessions.create({ cwd, title: "OpenUI in the window" });
const append = (event: Parameters<typeof daemon.store.append>[1]): void => {
  daemon.store.append(main.id, event);
};

for (const name of wanted) {
  const doc = DOCS[name];
  if (doc === undefined) throw new Error(`no doc named ${name}`);
  append({ type: "user_message", app: "desktop", text: doc.ask });
  append({
    type: "agent_message_chunk",
    messageId: `m-${name}`,
    text: `${doc.before}\n\n\`\`\`openui\n${doc.program}\n\`\`\`\n\n${doc.after ?? "Tell me if you want it another way."}`,
  });
  append({ type: "turn_ended", stopReason: "end_turn" });
}

writeFileSync(join(dirname(paths.socket), "scene-ready"), "");

process.on("SIGTERM", () => void daemon.stop().then(() => process.exit(0)));
await new Promise(() => {});
