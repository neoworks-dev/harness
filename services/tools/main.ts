// The tools service: the tools the agent wrote for itself. Each file of a tools folder is a tool
// (or several), loaded when it is saved and offered to the agents as <app>.<name>. The daemon
// runs one instance for each folder (see daemon/src/tool-folders.ts and docs/design.md):
//   --dir D --app A --scope user|project|session --approved F [--sessions S]
// By hand, with no arguments: ~/.config/neo/tools as app "tools", every file running.
import { mkdirSync, readFileSync, watch } from "node:fs";
import { basename, dirname, join } from "node:path";
import { connect, createToolServer, daemonJsonAddress } from "@neoworks-dev/neo-sdk";
import type { ToolSpec } from "@neoworks-dev/neo-sdk";
import { neoConfigDir } from "@neoworks-dev/neo/mcp-config";
import { type Loaded, ToolFiles } from "./src/loader.ts";
import { type Scope, writingTools } from "./src/skill.ts";

const log = (line: string): void => {
  process.stderr.write(`tools: ${line}\n`);
};

/** The value after `--name`, if it was given. */
function option(name: string): string | undefined {
  const at = process.argv.indexOf(`--${name}`);
  return at === -1 ? undefined : process.argv[at + 1];
}

const app = option("app") ?? "tools";
const given = option("scope");
const scope: Scope = given === "project" || given === "session" ? given : "user";
function toolsDir(): string {
  const given = option("dir");
  if (given !== undefined) return given;
  if (process.env.NEO_TOOLS_DIR) return process.env.NEO_TOOLS_DIR;
  return join(neoConfigDir(), "tools");
}
const dir = toolsDir();
// The user's folder is made for them; a project's or a session's is made by whoever writes to it.
if (scope === "user") mkdirSync(dir, { recursive: true });

// The files run only as the user allowed them: the daemon keeps the hash of each allowed file in
// this file (see src/approval.ts). Without it (by hand) every file runs.
const approvedFile = option("approved");
let approved: Record<string, string> = {};
function readApproved(): Record<string, string> {
  try {
    const stored: unknown = JSON.parse(readFileSync(approvedFile ?? "", "utf8"));
    const next: Record<string, string> = {};
    if (typeof stored === "object" && stored !== null) {
      for (const [name, hash] of Object.entries(stored)) {
        if (typeof hash === "string") next[name] = hash;
      }
    }
    approved = next;
  } catch {
    // Missing or half written: what was allowed before stays allowed.
  }
  return approved;
}

let loaded: Loaded = { tools: [], files: [] };
const files = new ToolFiles(dir, {
  reserved: new Set(["status"]),
  ...(approvedFile !== undefined && { approved: readApproved }),
});

const status: ToolSpec = {
  name: "status",
  description:
    "The tool files in the tools folder, the tools each gave, and why a file gave none. Call it after writing or changing a tool.",
  inputSchema: { type: "object", properties: {} },
  handler() {
    const lines = loaded.files.map((f) => {
      const line = `${f.file}: ${f.tools.length > 0 ? f.tools.join(", ") : "no tools"}`;
      return f.error ? `${line} (${f.error})` : line;
    });
    return Promise.resolve({
      content: [{ type: "text", text: lines.length > 0 ? lines.join("\n") : `no files in ${dir}` }],
      structuredContent: { dir, files: loaded.files },
    });
  },
};

const whose = {
  user: "yours, for every session",
  project: "this project's",
  session: "this session's",
}[scope];
const sessions = option("sessions");
// One skill for all three folders, from the instance every session sees.
const skills = [];
if (scope === "user") skills.push(writingTools(dir, sessions));
const tools = await createToolServer({
  name: app,
  instructions: `Tools the agent wrote for itself (${whose}): one file each in ${dir}. Load the writing-tools skill of "tools" before writing one.`,
  tools: [status],
  skills,
});

async function reload(): Promise<void> {
  loaded = await files.reload();
  tools.setTools([status, ...loaded.tools]);
  for (const file of loaded.files) if (file.error) log(`${file.file}: ${file.error}`);
  log(`${loaded.tools.length} tools from ${loaded.files.length} files`);
}
await reload();

// An editor saves a file as several events: wait for them to stop.
let timer: ReturnType<typeof setTimeout> | undefined;
const changed = (): void => {
  clearTimeout(timer);
  timer = setTimeout(() => void reload(), 250);
};
// The helpers in a subfolder are part of the tools too.
const watchers = [watch(dir, { recursive: true }, changed)];
if (approvedFile !== undefined) {
  // What the user allows changes it: the tools it allows start without a restart.
  const name = basename(approvedFile);
  watchers.push(
    watch(dirname(approvedFile), (_, changedName) => {
      if (changedName?.startsWith(name)) changed();
    }),
  );
}

const neo = await connect({
  client: app,
  address: daemonJsonAddress,
  waitForDaemon: true,
  reconnect: true,
  presence: {
    app,
    kind: "service",
    title: { user: "Your tools", project: "Project tools", session: "Session tools" }[scope],
    description: `Tools the agent wrote for itself (${whose}): a file in ${dir} is a tool, loaded when it is saved.`,
    open: true,
    mcp: tools.endpoint,
    launch: {
      command: [process.execPath, "run", import.meta.path, ...process.argv.slice(2)],
    },
  },
});
log("registered; Ctrl-C to quit");

let stopping = false;
const stop = (): void => {
  if (stopping) return;
  stopping = true;
  for (const watcher of watchers) watcher.close();
  files.close();
  void Promise.all([neo.close(), tools.close()]).then(() => process.exit(0));
};
process.on("SIGINT", stop);
process.on("SIGTERM", stop);

// Started by the daemon: go when it goes (even if it was killed).
const daemonPid = Number(process.env.NEO_DAEMON_PID);
if (Number.isInteger(daemonPid) && daemonPid > 0) {
  setInterval(() => {
    try {
      process.kill(daemonPid, 0);
    } catch {
      log("daemon is gone; exiting");
      stop();
    }
  }, 1000).unref();
}
