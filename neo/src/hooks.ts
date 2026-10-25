// Claude Code hooks, as neo runs them (#23): commands that get an event as JSON on stdin and
// answer with an exit code and, optionally, JSON on stdout. The configuration is Claude Code's:
//
//   { "hooks": { "PreToolUse": [ { "matcher": "Bash|Edit",
//       "hooks": [ { "type": "command", "command": "…", "timeout": 10 } ] } ] } }
//
// and comes from three places, all of which run: the `hooks` key of ~/.config/neo/config.json,
// `<cwd>/.neo/settings.json`, and the `hooks/hooks.json` of each enabled plugin (plugins.ts),
// where `${CLAUDE_PLUGIN_ROOT}` is the plugin's folder.
//
// What a hook's answer means, by event:
//
//   exit 0   goes on. stdout may be JSON: `continue: false` (+ `stopReason`) stops the turn,
//            `systemMessage` is shown to the user, `decision: "block"` + `reason`, and
//            `hookSpecificOutput`: `permissionDecision` allow|deny|ask (+ reason), `updatedInput`,
//            `additionalContext`. UserPromptSubmit and SessionStart also take plain stdout as
//            context for the agent.
//   exit 2   blocks, and stderr is what the agent is told: PreToolUse stops the call,
//            PostToolUse adds it to the result, UserPromptSubmit drops the prompt (the user is
//            told), Stop and SubagentStop make the agent go on. Elsewhere stderr is the user's.
//   other    a non-blocking error: shown to the user, nothing else happens.
//
// Not supported: hook types other than "command", and `async` hooks' results (they run, in the
// background; asyncRewake does not wake the agent). The events PermissionRequest, StopFailure and
// UserPromptExpansion are ignored; neo has no compaction, so PreCompact is never run.
import { spawn } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import type { HarnessConfig } from "./config.ts";
import type { Logger } from "./logger.ts";
import { enabledPlugins, expandPluginRoot, type Plugin } from "./plugins.ts";
import { isRecord } from "./util.ts";

export const HOOK_EVENTS = [
  "PreToolUse",
  "PostToolUse",
  "PostToolUseFailure",
  "UserPromptSubmit",
  "Stop",
  "SubagentStop",
  "SessionStart",
  "SessionEnd",
  "Notification",
  "PreCompact",
] as const;
export type HookEvent = (typeof HOOK_EVENTS)[number];

/** Claude Code's default. */
const DEFAULT_TIMEOUT_S = 60;
/** What of a hook's output is kept. */
const MAX_OUTPUT = 1024 * 1024;
const KILL_GRACE_MS = 200;

export interface HookCommand {
  command: string;
  /** Seconds. */
  timeout: number;
  /** Claude Code's `if`: a permission rule like `Bash(git push *)` the call has to match. */
  condition?: string;
  /** Run in the background; its answer is not used. */
  async: boolean;
  /** Where it was written, for messages: "plugin hookify@official", ".neo/settings.json". */
  source: string;
  /** The plugin it belongs to. */
  plugin?: Plugin;
}

export interface HookGroup {
  /** Tested against the tool's name (SessionStart: its source). Absent, "" and "*": everything. */
  matcher?: string;
  hooks: HookCommand[];
}

export type HookTable = Partial<Record<HookEvent, HookGroup[]>>;

const isEvent = (name: string): name is HookEvent => HOOK_EVENTS.some((event) => event === name);

/**
 * The `hooks` object of a settings file or hooks.json. Entries that aren't a command hook are
 * left out, each with a line in `problems`.
 */
export function parseHooks(
  value: unknown,
  source: string,
  plugin?: Plugin,
): { table: HookTable; problems: string[] } {
  const table: HookTable = {};
  const problems: string[] = [];
  if (!isRecord(value)) return { table, problems: [`${source}: hooks must be an object`] };
  for (const [name, groups] of Object.entries(value)) {
    if (!isEvent(name)) continue;
    if (!Array.isArray(groups)) {
      problems.push(`${source}: hooks.${name} must be a list`);
      continue;
    }
    for (const group of groups) {
      if (!isRecord(group) || !Array.isArray(group.hooks)) {
        problems.push(`${source}: a ${name} entry has no hooks list`);
        continue;
      }
      const hooks: HookCommand[] = [];
      for (const hook of group.hooks) {
        if (!isRecord(hook) || hook.type !== "command" || typeof hook.command !== "string") {
          const type = isRecord(hook) ? String(hook.type) : "?";
          problems.push(`${source}: a ${name} hook of type "${type}" is not supported`);
          continue;
        }
        const entry: HookCommand = {
          command: hook.command,
          timeout:
            typeof hook.timeout === "number" && hook.timeout > 0 ? hook.timeout : DEFAULT_TIMEOUT_S,
          async: hook.async === true,
          source,
        };
        if (typeof hook.if === "string" && hook.if !== "") entry.condition = hook.if;
        if (plugin) entry.plugin = plugin;
        hooks.push(entry);
      }
      if (hooks.length === 0) continue;
      const parsed: HookGroup = { hooks };
      if (typeof group.matcher === "string") parsed.matcher = group.matcher;
      (table[name] ??= []).push(parsed);
    }
  }
  return { table, problems };
}

/** A settings-shaped value: `{ hooks: {…} }`, or the events directly. */
function hooksOf(value: unknown): unknown {
  return isRecord(value) && "hooks" in value ? value.hooks : value;
}

function readJsonFile(path: string): unknown {
  try {
    return JSON.parse(readFileSync(path, "utf8"));
  } catch {
    return undefined;
  }
}

/** What a plugin's manifest says about its hooks: inline, or a path (or paths) in the plugin. */
function manifestHooks(plugin: Plugin): unknown[] {
  const manifest = readJsonFile(join(plugin.root, ".claude-plugin", "plugin.json"));
  if (!isRecord(manifest)) return [];
  const declared = manifest.hooks;
  if (declared === undefined) return [];
  const found: unknown[] = [];
  for (const entry of Array.isArray(declared) ? declared : [declared]) {
    if (typeof entry !== "string") {
      found.push(hooksOf(entry));
      continue;
    }
    const file = readJsonFile(join(plugin.root, entry));
    if (file !== undefined) found.push(hooksOf(file));
  }
  return found;
}

function mergeInto(into: HookTable, from: HookTable): void {
  for (const event of HOOK_EVENTS) {
    const groups = from[event];
    if (groups) (into[event] ??= []).push(...groups);
  }
}

export interface LoadedHooks {
  table: HookTable;
  problems: string[];
}

/** Every hook a session in `cwd` runs: the user's, the project's, then the plugins'. */
export function loadHooks(config: HarnessConfig, cwd: string): LoadedHooks {
  const table: HookTable = {};
  const problems: string[] = [];
  const add = (value: unknown, source: string, plugin?: Plugin): void => {
    if (value === undefined) return;
    const parsed = parseHooks(value, source, plugin);
    mergeInto(table, parsed.table);
    problems.push(...parsed.problems);
  };
  add(config.hooks, "the hooks of config.json");
  const projectFile = join(cwd, ".neo", "settings.json");
  if (existsSync(projectFile)) add(hooksOf(readJsonFile(projectFile)), ".neo/settings.json");
  for (const plugin of enabledPlugins(config.plugins)) {
    const source = `plugin ${plugin.id}`;
    const file = join(plugin.root, "hooks", "hooks.json");
    if (existsSync(file)) add(hooksOf(readJsonFile(file)), source, plugin);
    for (const inline of manifestHooks(plugin)) add(inline, source, plugin);
  }
  return { table, problems };
}

/** Whether a matcher accepts the value: letters and `|` are exact names, anything else a regex. */
export function matches(
  matcher: string | undefined,
  value: string | readonly string[] | undefined,
): boolean {
  if (matcher === undefined || matcher === "" || matcher === "*") return true;
  if (value === undefined) return true;
  if (typeof value !== "string") return value.some((one) => matches(matcher, one));
  if (/^[\w|]+$/.test(matcher)) return matcher.split("|").includes(value);
  try {
    return new RegExp(matcher).test(value);
  } catch {
    return matcher === value;
  }
}

/**
 * Claude Code's `if`: `Tool(pattern)` or `Tool`. The pattern is a glob over the tool's main
 * argument (Bash: the command; file tools: the path), `*` standing for anything; a trailing
 * `:*` is the older form of a prefix match.
 */
export function conditionHolds(
  condition: string,
  tool: string,
  input: Record<string, unknown>,
): boolean {
  const match = /^([\w.-]+)(?:\((.*)\))?$/s.exec(condition.trim());
  if (!match) return true;
  if (match[1] !== tool) return false;
  const pattern = match[2];
  if (pattern === undefined || pattern === "" || pattern === "*") return true;
  const subject = [input.command, input.file_path, input.url, input.query].find(
    (value) => typeof value === "string",
  );
  const prefix = pattern.endsWith(":*");
  const body = prefix ? pattern.slice(0, -2) : pattern;
  const source = body
    .split("*")
    .map((part) => part.replace(/[.+?^${}()|[\]\\]/g, "\\$&"))
    .join(".*");
  if (typeof subject !== "string") return false;
  return new RegExp(prefix ? `^${source}(?:\\s|$)` : `^${source}$`, "s").test(subject);
}

/** What the hooks of one event, taken together, ask for. */
export interface HookResult {
  /** The agent is to be told this, and the action (or the stop) does not happen. */
  blocked?: string;
  /** PreToolUse: what the hooks decided about the permission question. */
  decision?: "allow" | "deny" | "ask";
  decisionReason?: string;
  /** For the agent: additionalContext, and stdout of UserPromptSubmit and SessionStart. */
  context: string[];
  /** For the user: systemMessage. */
  messages: string[];
  /** For the user: hooks that failed without blocking. */
  errors: string[];
  /** `continue: false`: the turn ends; this is the stopReason, "" without one. */
  halt?: string;
  /** PreToolUse: Claude Code's names, as the hook rewrote them. */
  updatedInput?: Record<string, unknown>;
}

const RANK = { allow: 0, ask: 1, deny: 2 } as const;

function emptyResult(): HookResult {
  return { context: [], messages: [], errors: [] };
}

function combine(results: HookResult[]): HookResult {
  const out = emptyResult();
  const blocked: string[] = [];
  for (const result of results) {
    if (result.blocked !== undefined) blocked.push(result.blocked);
    if (result.decision && (!out.decision || RANK[result.decision] > RANK[out.decision])) {
      out.decision = result.decision;
      if (result.decisionReason !== undefined) out.decisionReason = result.decisionReason;
    }
    out.context.push(...result.context);
    out.messages.push(...result.messages);
    out.errors.push(...result.errors);
    if (result.halt !== undefined) out.halt ??= result.halt;
    if (result.updatedInput) out.updatedInput = { ...out.updatedInput, ...result.updatedInput };
  }
  if (blocked.length > 0) out.blocked = blocked.join("\n");
  return out;
}

/** The command's end: its exit code, or why it has none. */
interface Finished {
  code: number | null;
  stdout: string;
  stderr: string;
  /** Spawn failed, or it ran out of time. */
  failure?: string;
}

function run(
  hook: HookCommand,
  stdin: string,
  cwd: string,
  signal: AbortSignal | undefined,
): Promise<Finished> {
  const command = hook.plugin ? expandPluginRoot(hook.command, hook.plugin) : hook.command;
  const env: NodeJS.ProcessEnv = { ...process.env, CLAUDE_PROJECT_DIR: cwd };
  if (hook.plugin) env.CLAUDE_PLUGIN_ROOT = hook.plugin.root;
  return new Promise((resolve) => {
    const shell = existsSync("/bin/bash") ? "/bin/bash" : "sh";
    const child = spawn(shell, ["-c", command], {
      cwd,
      env,
      detached: true, // its own group, so a timeout takes what the command started with it
      stdio: ["pipe", "pipe", "pipe"],
    });
    let stdout = "";
    let stderr = "";
    let failure: string | undefined;
    let done = false;
    const kill = (): void => {
      try {
        if (child.pid !== undefined) process.kill(-child.pid, "SIGKILL");
      } catch {
        // already gone
      }
    };
    const finish = (code: number | null): void => {
      if (done) return;
      done = true;
      clearTimeout(timer);
      signal?.removeEventListener("abort", onAbort);
      const out: Finished = { code, stdout, stderr };
      if (failure !== undefined) out.failure = failure;
      resolve(out);
    };
    const onAbort = (): void => {
      failure = "stopped";
      kill();
    };
    const timer = setTimeout(() => {
      failure = `timed out after ${hook.timeout}s`;
      kill();
    }, hook.timeout * 1000);
    signal?.addEventListener("abort", onAbort, { once: true });
    if (signal?.aborted) onAbort();
    child.stdout.setEncoding("utf8");
    child.stderr.setEncoding("utf8");
    child.stdout.on("data", (chunk: string) => {
      if (stdout.length < MAX_OUTPUT) stdout += chunk;
    });
    child.stderr.on("data", (chunk: string) => {
      if (stderr.length < MAX_OUTPUT) stderr += chunk;
    });
    child.on("error", (error) => {
      failure = `could not start: ${error.message}`;
      finish(null);
    });
    // A command that never reads its input closes the pipe on us; that is not an error.
    child.stdin.on("error", () => {});
    child.stdin.end(stdin);
    child.on("close", (code) => finish(code));
    // A background process keeping the pipes open must not hold the answer back.
    child.on("exit", (code) => setTimeout(() => finish(code), KILL_GRACE_MS));
  });
}

const TEXT_CONTEXT: readonly HookEvent[] = ["UserPromptSubmit", "SessionStart"];
const BLOCKS_WITH_EXIT_2: readonly HookEvent[] = [
  "PreToolUse",
  "PostToolUse",
  "PostToolUseFailure",
  "UserPromptSubmit",
  "Stop",
  "SubagentStop",
];

function label(hook: HookCommand): string {
  const first = hook.command.split("\n")[0] ?? "";
  const short = first.length > 60 ? `${first.slice(0, 59)}…` : first;
  return `${hook.source}: ${short}`;
}

/** What one hook's end means for the event. */
function interpret(event: HookEvent, hook: HookCommand, end: Finished): HookResult {
  const result = emptyResult();
  const stderr = end.stderr.trim();
  if (end.failure !== undefined) {
    result.errors.push(`${event} hook failed (${end.failure}) · ${label(hook)}`);
    return result;
  }
  if (end.code === 2) {
    if (BLOCKS_WITH_EXIT_2.includes(event)) {
      result.blocked = stderr || `A ${event} hook blocked this (${label(hook)}).`;
    } else if (stderr !== "") {
      result.errors.push(`${event} hook · ${label(hook)}: ${stderr}`);
    }
    return result;
  }
  if (end.code !== 0) {
    const code = end.code === null ? "was killed" : `exited with code ${end.code}`;
    let detail = "";
    if (stderr !== "") detail = `: ${stderr.split("\n")[0]}`;
    result.errors.push(`${event} hook ${code}${detail} · ${label(hook)}`);
    return result;
  }
  const stdout = end.stdout.trim();
  if (stdout === "") return result;
  let json: unknown;
  if (stdout.startsWith("{")) {
    try {
      json = JSON.parse(stdout);
    } catch {
      // Not JSON after all: plain text below.
    }
  }
  if (!isRecord(json)) {
    if (TEXT_CONTEXT.includes(event)) result.context.push(stdout);
    return result;
  }
  return interpretJson(event, json, result);
}

function interpretJson(
  event: HookEvent,
  json: Record<string, unknown>,
  result: HookResult,
): HookResult {
  if (typeof json.systemMessage === "string" && json.systemMessage !== "") {
    result.messages.push(json.systemMessage);
  }
  if (json.continue === false) {
    result.halt = "";
    if (typeof json.stopReason === "string") result.halt = json.stopReason;
  }
  const reason = typeof json.reason === "string" ? json.reason : undefined;
  const specific: Record<string, unknown> = {};
  if (isRecord(json.hookSpecificOutput)) Object.assign(specific, json.hookSpecificOutput);
  if (typeof specific.additionalContext === "string" && specific.additionalContext !== "") {
    result.context.push(specific.additionalContext);
  }
  if (event === "PreToolUse") {
    // `decision` approve|block is the older spelling of permissionDecision allow|deny.
    let decision = specific.permissionDecision ?? json.decision;
    if (decision === "approve") decision = "allow";
    if (decision === "block") decision = "deny";
    if (decision === "allow" || decision === "deny" || decision === "ask") {
      result.decision = decision;
      const why =
        typeof specific.permissionDecisionReason === "string"
          ? specific.permissionDecisionReason
          : reason;
      if (why !== undefined) result.decisionReason = why;
    }
    if (isRecord(specific.updatedInput)) result.updatedInput = specific.updatedInput;
  } else if (json.decision === "block") {
    result.blocked = reason ?? `A ${event} hook blocked this.`;
  }
  return result;
}

/** The common fields of every event's input. */
export interface HookSession {
  sessionId: string;
  cwd: string;
  /** Claude Code's JSONL transcript of the conversation; rewritten by `beforeEvent`. */
  transcriptPath: string;
}

export interface RunOptions {
  /**
   * What the matchers are tested against: the tool's name, SessionStart's source. Several names
   * when a call answers to more than one (a multi-edit is a MultiEdit and an Edit).
   */
  matchValue?: string | readonly string[];
  /** The tool call, for the hooks' `if`. */
  tool?: { name: string; input: Record<string, unknown> };
  signal?: AbortSignal;
}

export class Hooks {
  constructor(
    private readonly table: HookTable,
    private readonly session: HookSession,
    private readonly beforeEvent?: (event: HookEvent) => void,
    private readonly logger?: Logger,
  ) {}

  /** Whether anything is configured for the event. */
  has(event: HookEvent): boolean {
    return (this.table[event]?.length ?? 0) > 0;
  }

  /** The hooks whose matcher and `if` accept this. Identical commands run once. */
  private select(event: HookEvent, options: RunOptions): HookCommand[] {
    const seen = new Set<string>();
    const selected: HookCommand[] = [];
    for (const group of this.table[event] ?? []) {
      if (!matches(group.matcher, options.matchValue)) continue;
      for (const hook of group.hooks) {
        if (hook.condition !== undefined && options.tool) {
          if (!conditionHolds(hook.condition, options.tool.name, options.tool.input)) continue;
        }
        const key = `${hook.plugin?.id ?? ""}\0${hook.command}`;
        if (seen.has(key)) continue;
        seen.add(key);
        selected.push(hook);
      }
    }
    return selected;
  }

  /**
   * Runs the event's hooks side by side, each with `fields` (plus the common ones) as JSON on
   * stdin, and combines what they said. Never rejects.
   */
  async run(
    event: HookEvent,
    fields: Record<string, unknown>,
    options: RunOptions = {},
  ): Promise<HookResult> {
    const selected = this.select(event, options);
    if (selected.length === 0) return emptyResult();
    try {
      this.beforeEvent?.(event);
    } catch (error) {
      this.logger?.warn(`hooks: writing the transcript failed: ${String(error)}`);
    }
    const stdin = JSON.stringify({
      session_id: this.session.sessionId,
      transcript_path: this.session.transcriptPath,
      cwd: this.session.cwd,
      hook_event_name: event,
      ...fields,
    });
    const results = await Promise.all(
      selected.map(async (hook) => {
        const finished = run(hook, stdin, this.session.cwd, options.signal);
        if (hook.async) {
          // Runs on beside the agent; what it says is not used.
          void finished.then((end) => {
            if (end.code !== 0) this.logger?.warn(`hooks: async ${label(hook)} failed`);
          });
          return emptyResult();
        }
        return interpret(event, hook, await finished);
      }),
    );
    return combine(results);
  }
}
