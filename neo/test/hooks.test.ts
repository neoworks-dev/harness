import { expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { join } from "node:path";
import {
  conditionHolds,
  Hooks,
  loadHooks,
  matches,
  parseHooks,
  type HookEvent,
  type HookResult,
  type HookTable,
} from "../src/hooks.ts";
import { TEST_CONFIG } from "./fake-model.ts";
import { tempDir } from "./helpers.ts";

const FIXTURES = join(import.meta.dir, "fixtures", "plugins");

/** One hook running `command` for `event`. */
function hooksFor(
  event: HookEvent,
  command: string,
  extra: { matcher?: string; timeout?: number; condition?: string; async?: boolean } = {},
  cwd = "/tmp",
): Hooks {
  const table: HookTable = {
    [event]: [
      {
        ...(extra.matcher && { matcher: extra.matcher }),
        hooks: [
          {
            command,
            timeout: extra.timeout ?? 10,
            async: extra.async ?? false,
            source: "test",
            ...(extra.condition && { condition: extra.condition }),
          },
        ],
      },
    ],
  };
  return new Hooks(table, { sessionId: "s1", cwd, transcriptPath: "/tmp/t.jsonl" });
}

const say = (json: unknown): string => `cat >/dev/null; echo '${JSON.stringify(json)}'`;

test("the official marketplace's hooks.json files parse", () => {
  for (const name of ["ralph-loop", "hookify", "security-guidance"]) {
    const file = join(FIXTURES, name, "hooks", "hooks.json");
    const json: unknown = JSON.parse(readFileSync(file, "utf8"));
    const { table, problems } = parseHooks((json as { hooks: unknown }).hooks, name);
    expect(problems).toEqual([]);
    expect(Object.keys(table).length).toBeGreaterThan(0);
  }
  const security = parseHooks(
    JSON.parse(readFileSync(join(FIXTURES, "security-guidance", "hooks", "hooks.json"), "utf8"))
      .hooks,
    "sg",
  ).table;
  const post = security.PostToolUse ?? [];
  expect(post.map((group) => group.matcher)).toEqual(["Edit|Write|MultiEdit|NotebookEdit", "Bash"]);
  const rewake = post[1]?.hooks.find((hook) => hook.condition === "Bash(git push:*)");
  expect(rewake?.command).toContain("sg-python.sh");
});

test("parseHooks reports what it can't use and skips the rest", () => {
  const { table, problems } = parseHooks(
    {
      PreToolUse: [
        {
          hooks: [
            { type: "prompt", prompt: "x" },
            { type: "command", command: "true", timeout: 3 },
          ],
        },
        { matcher: "Bash" },
      ],
      PermissionRequest: [{ hooks: [{ type: "command", command: "true" }] }],
      Stop: "nope",
    },
    "src",
  );
  expect(table.PreToolUse).toEqual([
    { hooks: [{ command: "true", timeout: 3, async: false, source: "src" }] },
  ]);
  expect(table.Stop).toBeUndefined();
  expect(problems).toHaveLength(3);
  expect(parseHooks("x", "src").problems).toEqual(["src: hooks must be an object"]);
});

test("matchers: names, alternatives, regexes, everything", () => {
  expect(matches(undefined, "Bash")).toBe(true);
  expect(matches("*", "Bash")).toBe(true);
  expect(matches("", "Bash")).toBe(true);
  expect(matches("Bash", "Bash")).toBe(true);
  expect(matches("Bash", "BashOutput")).toBe(false);
  expect(matches("Edit|Write", "Write")).toBe(true);
  expect(matches("Edit|Write", "Read")).toBe(false);
  expect(matches("mcp__.*__download", "mcp__browser__download")).toBe(true);
  expect(matches("^claude-security:claude-security$", "claude-security:claude-security")).toBe(
    true,
  );
  expect(matches("(", "(")).toBe(true);
  expect(matches("Edit", ["MultiEdit", "Edit"])).toBe(true);
  expect(matches("Read", ["MultiEdit", "Edit"])).toBe(false);
});

test("`if` conditions follow Claude Code's permission rule syntax", () => {
  const bash = (command: string): Record<string, unknown> => ({ command });
  expect(conditionHolds("Bash(git push *)", "Bash", bash("git push origin main"))).toBe(true);
  expect(conditionHolds("Bash(git push *)", "Bash", bash("git status"))).toBe(false);
  expect(conditionHolds("Bash(git push:*)", "Bash", bash("git push"))).toBe(true);
  expect(conditionHolds("Bash(git push:*)", "Bash", bash("git pushover"))).toBe(false);
  expect(conditionHolds("Bash(git -C * commit *)", "Bash", bash("git -C x commit -m y"))).toBe(
    true,
  );
  expect(conditionHolds("Bash", "Bash", bash("anything"))).toBe(true);
  expect(conditionHolds("Edit(*.ts)", "Edit", { file_path: "a/b.ts" })).toBe(true);
  expect(conditionHolds("Edit(*.ts)", "Edit", { file_path: "a/b.js" })).toBe(false);
  expect(conditionHolds("Bash(ls)", "Read", bash("ls"))).toBe(false);
});

test("the hook gets Claude Code's JSON on stdin", async () => {
  const dir = await tempDir();
  const out = join(dir, "stdin.json");
  const hooks = hooksFor("PreToolUse", `cat > ${out}`, {}, dir);
  await hooks.run("PreToolUse", { tool_name: "Bash", tool_input: { command: "ls" } });
  expect(JSON.parse(await readFile(out, "utf8"))).toEqual({
    session_id: "s1",
    transcript_path: "/tmp/t.jsonl",
    cwd: dir,
    hook_event_name: "PreToolUse",
    tool_name: "Bash",
    tool_input: { command: "ls" },
  });
});

test("it runs in the session's directory with CLAUDE_PROJECT_DIR set", async () => {
  const dir = await tempDir();
  const hooks = hooksFor("SessionStart", 'pwd; echo "$CLAUDE_PROJECT_DIR"', {}, dir);
  const result = await hooks.run("SessionStart", { source: "startup" });
  expect(result.context).toEqual([`${dir}\n${dir}`]);
});

test("exit 0 with nothing to say goes on", async () => {
  const result = await hooksFor("PreToolUse", "cat >/dev/null").run("PreToolUse", {});
  expect(result).toEqual({ context: [], messages: [], errors: [] });
});

test("exit 2 blocks, with stderr for the agent, in the events that can be blocked", async () => {
  const blocks: [HookEvent, string | undefined][] = [
    ["PreToolUse", "no rm"],
    ["PostToolUse", "no rm"],
    ["UserPromptSubmit", "no rm"],
    ["Stop", "no rm"],
    ["SubagentStop", "no rm"],
  ];
  for (const [event, expected] of blocks) {
    const result = await hooksFor(event, "echo 'no rm' >&2; exit 2").run(event, {});
    expect(result.blocked).toBe(expected);
    expect(result.errors).toEqual([]);
  }
  // Elsewhere stderr goes to the user.
  const end = await hooksFor("SessionEnd", "echo 'bye' >&2; exit 2").run("SessionEnd", {});
  expect(end.blocked).toBeUndefined();
  expect(end.errors[0]).toContain("bye");
});

test("exit 2 without a message still blocks, and says by whom", async () => {
  const result = await hooksFor("PreToolUse", "exit 2").run("PreToolUse", {});
  expect(result.blocked).toContain("PreToolUse hook blocked");
});

test("another exit code is a non-blocking error for the user", async () => {
  const result = await hooksFor("PreToolUse", "echo oops >&2; exit 1").run("PreToolUse", {});
  expect(result.blocked).toBeUndefined();
  expect(result.decision).toBeUndefined();
  expect(result.errors).toHaveLength(1);
  expect(result.errors[0]).toContain("exited with code 1: oops");
});

test("a hook that times out is an error and doesn't hold the tool call up", async () => {
  const started = Date.now();
  const result = await hooksFor("PreToolUse", "sleep 30", { timeout: 0.3 }).run("PreToolUse", {});
  expect(Date.now() - started).toBeLessThan(5000);
  expect(result.blocked).toBeUndefined();
  expect(result.errors[0]).toContain("timed out");
});

test("aborting stops a running hook", async () => {
  const abort = new AbortController();
  const hooks = hooksFor("PreToolUse", "sleep 30");
  const pending = hooks.run("PreToolUse", {}, { signal: abort.signal });
  setTimeout(() => abort.abort(), 50);
  const result = await pending;
  expect(result.errors[0]).toContain("stopped");
});

test("a command that isn't there is an error", async () => {
  const result = await hooksFor("PreToolUse", "/nonexistent/hook").run("PreToolUse", {});
  expect(result.errors[0]).toContain("exited with code 127");
});

test("PreToolUse JSON: permissionDecision allow, deny and ask, with a reason", async () => {
  for (const decision of ["allow", "deny", "ask"] as const) {
    const json = {
      hookSpecificOutput: {
        hookEventName: "PreToolUse",
        permissionDecision: decision,
        permissionDecisionReason: `because ${decision}`,
      },
    };
    const result = await hooksFor("PreToolUse", say(json)).run("PreToolUse", {});
    expect(result.decision).toBe(decision);
    expect(result.decisionReason).toBe(`because ${decision}`);
  }
});

test("PreToolUse JSON: the older decision approve and block", async () => {
  const approve = await hooksFor("PreToolUse", say({ decision: "approve" })).run("PreToolUse", {});
  expect(approve.decision).toBe("allow");
  const block = await hooksFor("PreToolUse", say({ decision: "block", reason: "no" })).run(
    "PreToolUse",
    {},
  );
  expect(block.decision).toBe("deny");
  expect(block.decisionReason).toBe("no");
});

test("PreToolUse JSON: updatedInput and additionalContext", async () => {
  const json = {
    hookSpecificOutput: { updatedInput: { command: "ls -la" }, additionalContext: "note" },
  };
  const result = await hooksFor("PreToolUse", say(json)).run("PreToolUse", {});
  expect(result.updatedInput).toEqual({ command: "ls -la" });
  expect(result.context).toEqual(["note"]);
});

test("JSON: continue false halts, systemMessage is for the user", async () => {
  const json = { continue: false, stopReason: "enough", systemMessage: "hello user" };
  const result = await hooksFor("PostToolUse", say(json)).run("PostToolUse", {});
  expect(result.halt).toBe("enough");
  expect(result.messages).toEqual(["hello user"]);
  const bare = await hooksFor("Stop", say({ continue: false })).run("Stop", {});
  expect(bare.halt).toBe("");
});

test("JSON: decision block with a reason blocks PostToolUse, UserPromptSubmit and Stop", async () => {
  for (const event of ["PostToolUse", "UserPromptSubmit", "Stop"] as const) {
    const result = await hooksFor(event, say({ decision: "block", reason: "because" })).run(
      event,
      {},
    );
    expect(result.blocked).toBe("because");
  }
});

test("plain stdout is context for UserPromptSubmit and SessionStart only", async () => {
  const prompt = await hooksFor("UserPromptSubmit", "echo extra").run("UserPromptSubmit", {});
  expect(prompt.context).toEqual(["extra"]);
  const start = await hooksFor("SessionStart", "echo extra").run("SessionStart", {});
  expect(start.context).toEqual(["extra"]);
  const pre = await hooksFor("PreToolUse", "echo extra").run("PreToolUse", {});
  expect(pre.context).toEqual([]);
  // JSON-looking output that isn't JSON is text.
  const broken = await hooksFor("SessionStart", "echo '{nope'").run("SessionStart", {});
  expect(broken.context).toEqual(["{nope"]);
});

test("hooks run side by side and the strictest decision wins", async () => {
  const table: HookTable = {
    PreToolUse: [
      {
        hooks: [
          { command: say({ decision: "approve" }), timeout: 10, async: false, source: "a" },
          {
            command: say({ decision: "block", reason: "nope" }),
            timeout: 10,
            async: false,
            source: "b",
          },
          { command: "echo x >&2; exit 1", timeout: 10, async: false, source: "c" },
        ],
      },
    ],
  };
  const hooks = new Hooks(table, { sessionId: "s", cwd: "/tmp", transcriptPath: "/tmp/t" });
  const result = await hooks.run("PreToolUse", {});
  expect(result.decision).toBe("deny");
  expect(result.decisionReason).toBe("nope");
  expect(result.errors).toHaveLength(1);
});

test("a matcher picks the tool; the same command in two places runs once", async () => {
  const dir = await tempDir();
  const log = join(dir, "log");
  const command = `cat >/dev/null; echo ran >> ${log}`;
  const hook = { command, timeout: 10, async: false, source: "x" };
  const table: HookTable = {
    PreToolUse: [
      { matcher: "Bash", hooks: [hook] },
      { matcher: "Bash|Edit", hooks: [hook] },
      { matcher: "Write", hooks: [{ ...hook, command: `${command} # write` }] },
    ],
  };
  const hooks = new Hooks(table, { sessionId: "s", cwd: dir, transcriptPath: "/tmp/t" });
  await hooks.run("PreToolUse", {}, { matchValue: "Bash" });
  expect((await readFile(log, "utf8")).trim().split("\n")).toEqual(["ran"]);
  await hooks.run("PreToolUse", {}, { matchValue: "Read" });
  expect((await readFile(log, "utf8")).trim().split("\n")).toEqual(["ran"]);
});

test("a hook with an `if` runs only for matching calls", async () => {
  const hooks = hooksFor("PostToolUse", "echo ran >&2; exit 2", {
    matcher: "Bash",
    condition: "Bash(git push *)",
  });
  const call = (command: string): { name: string; input: Record<string, unknown> } => ({
    name: "Bash",
    input: { command },
  });
  const no = await hooks.run("PostToolUse", {}, { matchValue: "Bash", tool: call("ls") });
  expect(no.blocked).toBeUndefined();
  const yes = await hooks.run("PostToolUse", {}, { matchValue: "Bash", tool: call("git push x") });
  expect(yes.blocked).toBe("ran");
});

test("an async hook is started and not waited for", async () => {
  const dir = await tempDir();
  const flag = join(dir, "flag");
  const hooks = hooksFor("Stop", `sleep 0.3; touch ${flag}`, { async: true }, dir);
  const started = Date.now();
  const result = await hooks.run("Stop", {});
  expect(Date.now() - started).toBeLessThan(250);
  expect(result).toEqual({ context: [], messages: [], errors: [] });
});

test("${CLAUDE_PLUGIN_ROOT} is the plugin's folder, in the command and in the environment", async () => {
  const dir = await tempDir();
  const plugin = { id: "p@m", name: "p", marketplace: "m", root: dir, version: "1" };
  const { table } = parseHooks(
    {
      SessionStart: [
        {
          hooks: [
            { type: "command", command: 'echo "${CLAUDE_PLUGIN_ROOT}/x $CLAUDE_PLUGIN_ROOT"' },
          ],
        },
      ],
    },
    "plugin p@m",
    plugin,
  );
  const hooks = new Hooks(table, { sessionId: "s", cwd: dir, transcriptPath: "/tmp/t" });
  const result = await hooks.run("SessionStart", {});
  expect(result.context).toEqual([`${dir}/x ${dir}`]);
});

test("beforeEvent runs for events with hooks to run, and only then", async () => {
  const seen: string[] = [];
  const table: HookTable = {
    Stop: [{ hooks: [{ command: "true", timeout: 5, async: false, source: "x" }] }],
  };
  const hooks = new Hooks(
    table,
    { sessionId: "s", cwd: "/tmp", transcriptPath: "/tmp/t" },
    (event) => seen.push(event),
  );
  await hooks.run("Stop", {});
  await hooks.run("SessionStart", {});
  expect(seen).toEqual(["Stop"]);
});

test("hooks come from config.json, the project's settings and enabled plugins", async () => {
  const cwd = await tempDir();
  const pluginsDir = await tempDir();
  const root = join(pluginsDir, "cache", "m", "p", "1");
  await mkdir(join(root, "hooks"), { recursive: true });
  await mkdir(join(cwd, ".suite"), { recursive: true });
  await writeFile(
    join(root, "hooks", "hooks.json"),
    JSON.stringify({
      hooks: {
        PreToolUse: [{ matcher: "Bash", hooks: [{ type: "command", command: "plugin-hook" }] }],
      },
    }),
  );
  await writeFile(
    join(pluginsDir, "installed.json"),
    JSON.stringify({
      version: 1,
      plugins: { "p@m": { root, version: "1", installedAt: "t", enabled: true } },
    }),
  );
  await writeFile(
    join(cwd, ".suite", "settings.json"),
    JSON.stringify({
      hooks: { PreToolUse: [{ hooks: [{ type: "command", command: "project-hook" }] }] },
    }),
  );
  const config = {
    ...TEST_CONFIG,
    plugins: pluginsDir,
    hooks: { PreToolUse: [{ hooks: [{ type: "command", command: "user-hook" }] }] },
  };
  const { table, problems } = loadHooks(config, cwd);
  expect(problems).toEqual([]);
  const commands = (table.PreToolUse ?? []).flatMap((group) =>
    group.hooks.map((hook) => hook.command),
  );
  expect(commands).toEqual(["user-hook", "project-hook", "plugin-hook"]);
  const sources = (table.PreToolUse ?? []).flatMap((group) =>
    group.hooks.map((hook) => hook.source),
  );
  expect(sources).toEqual(["the hooks of config.json", ".suite/settings.json", "plugin p@m"]);
});

test("a plugin's manifest may declare hooks inline or by path", async () => {
  const pluginsDir = await tempDir();
  const root = join(pluginsDir, "p");
  await mkdir(join(root, ".claude-plugin"), { recursive: true });
  await mkdir(join(root, "extra"), { recursive: true });
  await writeFile(
    join(root, ".claude-plugin", "plugin.json"),
    JSON.stringify({
      name: "p",
      hooks: ["./extra/more.json", { Stop: [{ hooks: [{ type: "command", command: "inline" }] }] }],
    }),
  );
  await writeFile(
    join(root, "extra", "more.json"),
    JSON.stringify({
      hooks: { SessionStart: [{ hooks: [{ type: "command", command: "by-path" }] }] },
    }),
  );
  await writeFile(
    join(pluginsDir, "installed.json"),
    JSON.stringify({ version: 1, plugins: { "p@m": { root, version: "1", installedAt: "t" } } }),
  );
  const cwd = await tempDir();
  const { table } = loadHooks({ ...TEST_CONFIG, plugins: pluginsDir }, cwd);
  expect(table.SessionStart?.[0]?.hooks[0]?.command).toBe("by-path");
  expect(table.Stop?.[0]?.hooks[0]?.command).toBe("inline");
});

test("no hooks anywhere is an empty table", async () => {
  const cwd = await tempDir();
  const { table, problems } = loadHooks(TEST_CONFIG, cwd);
  expect(table).toEqual({});
  expect(problems).toEqual([]);
  const results: HookResult[] = [
    await new Hooks(table, { sessionId: "s", cwd, transcriptPath: "t" }).run("Stop", {}),
  ];
  expect(results[0]?.blocked).toBeUndefined();
});
