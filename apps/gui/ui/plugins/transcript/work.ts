// What a tool call, a thought or a plan is, as the chat's work rows show it: which kind of work
// (a command, a read, an edit…), the label of its collapsed row, the parts its expanded form
// needs, and the summary of a group of them. The view items say little (a title, a state, a
// result), so the harness's own facts about the call (`ToolFacts`: its kind, tool and input,
// from the session's events) come first and the title and the result are read when they are
// missing. An unknown tool is a wrench with its own title. Plain functions, tested without a
// window.
import type { Component, DiffComponent, ViewItem } from "@neoworks-dev/neo-protocol/types";
import { diffStats } from "../../lib/diff.ts";
import { blocksText, componentText, hasButtons, oneLine } from "./components.ts";
import { count } from "./format.ts";
import type { ToolFacts } from "./timeline.ts";

export type WorkKind =
  | "command"
  | "read"
  | "edit"
  | "search"
  | "web"
  | "plan"
  | "agent"
  | "tool"
  | "thought"
  | "question";

/** The changes one tool call, or a whole turn, made to one file. */
export interface FileChange {
  /** As the tool wrote it, normally absolute. */
  path: string;
  added: number;
  removed: number;
  /** Absent while the call has not said what it changed. */
  diff?: DiffComponent;
}

/** One line of a plan. */
export interface PlanLine {
  text: string;
  done: boolean;
  active: boolean;
}

/** One row of work, ready to draw. */
export interface WorkEntry {
  /** The item's ref. */
  id: string;
  kind: WorkKind;
  item: ViewItem;
  /** What the collapsed row says. */
  label: string;
  /** Still going: its state is pending or running while the turn is. */
  live: boolean;
  failed: boolean;
  /** When it started, in Unix milliseconds. */
  at?: number;
  /** Commands: what was run, what it printed and how it exited. */
  command?: string;
  output?: string;
  exit?: number;
  /** Reads, relative to the session's directory. */
  paths?: string[];
  /** Edits. */
  files?: FileChange[];
  plan?: PlanLine[];
  /** Other tools: the arguments as JSON. */
  input?: string;
  /** Subagents: where the work stands, in a word. */
  status?: string;
}

export interface ClassifyContext {
  facts?: ToolFacts;
  /** The session's directory, which paths are shown relative to. */
  cwd: string;
  /** Whether the turn the item belongs to is running (a pending item is not abandoned). */
  active: boolean;
  /** For a thought, which has no state: whether it is still being written. */
  writing?: boolean;
  at?: number;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** The first of `keys` that `input` has as a non-empty string. */
function stringOf(input: unknown, keys: readonly string[]): string | undefined {
  if (!isRecord(input)) return undefined;
  for (const key of keys) {
    const value = input[key];
    if (typeof value === "string" && value.trim() !== "") return value;
  }
  return undefined;
}

/** Strings under `keys` of `input`: a string, or a list of them. */
function stringsOf(input: unknown, keys: readonly string[]): string[] {
  if (!isRecord(input)) return [];
  const out: string[] = [];
  for (const key of keys) {
    const value = input[key];
    if (typeof value === "string" && value.trim() !== "") out.push(value);
    else if (Array.isArray(value)) {
      for (const entry of value) if (typeof entry === "string" && entry !== "") out.push(entry);
    }
  }
  return out;
}

/** `path` relative to `cwd` when it is inside it. */
export function relativePath(path: string, cwd: string): string {
  const base = cwd.replace(/\/+$/, "");
  if (base !== "" && path.startsWith(`${base}/`)) return path.slice(base.length + 1);
  return path;
}

const SHELL_WRAPPER = /^(?:\S*\/)?(?:bash|sh|zsh|dash|fish)\s+-[a-z]*c\s+([\s\S]+)$/;

function unquote(text: string): string {
  const first = text[0];
  if ((first === '"' || first === "'") && text.length >= 2 && text.endsWith(first)) {
    return text.slice(1, -1);
  }
  return text;
}

/** A command without the shell it was run through: `bash -lc "ls -la"` is `ls -la`. */
export function stripShell(command: string): string {
  const trimmed = command.trim();
  const match = SHELL_WRAPPER.exec(trimmed);
  return match?.[1] === undefined ? trimmed : unquote(match[1].trim());
}

/** The command a call's input holds: a string, or the argv a harness splits it into. */
function commandOf(input: unknown): string | undefined {
  if (!isRecord(input)) return undefined;
  for (const key of ["command", "cmd"]) {
    const value = input[key];
    if (typeof value === "string" && value.trim() !== "") return stripShell(value);
    if (Array.isArray(value) && value.every((part) => typeof part === "string") && value.length) {
      const argv = value as string[];
      const shell = /(?:^|\/)(?:bash|sh|zsh|dash|fish)$/.test(argv[0] ?? "");
      const flag = /^-[a-z]*c$/.test(argv[1] ?? "");
      return shell && flag && argv[2] !== undefined ? argv[2].trim() : argv.join(" ");
    }
  }
  return undefined;
}

/** Every component of a tree that `pick` accepts, in order. */
function collect<T>(
  component: Component,
  pick: (c: Component) => T | undefined,
  out: T[] = [],
): T[] {
  const found = pick(component);
  if (found !== undefined) out.push(found);
  if (component.type === "card" || component.type === "row" || component.type === "stack") {
    for (const child of component.children) collect(child, pick, out);
  }
  return out;
}

const PATH_KEYS = ["path", "file_path", "filePath", "file", "filename", "notebook_path"];
const PATHS_KEYS = ["paths", "files", "file_paths"];
const QUERY_KEYS = ["query", "pattern", "q", "regex", "search", "glob", "prompt"];
const TARGET_KEYS = ["path", "directory", "dir", "include", "cwd", "app"];
const URL_KEYS = ["url", "uri", "href"];

const TOOLS: Record<string, WorkKind> = {
  bash: "command",
  shell: "command",
  exec: "command",
  exec_command: "command",
  run_command: "command",
  terminal: "command",
  read: "read",
  read_file: "read",
  view: "read",
  write: "edit",
  edit: "edit",
  multiedit: "edit",
  apply_patch: "edit",
  str_replace_editor: "edit",
  notebookedit: "edit",
  grep: "search",
  glob: "search",
  search: "search",
  find: "search",
  tools_search: "search",
  websearch: "web",
  web_search: "web",
  webfetch: "web",
  web_fetch: "web",
  fetch: "web",
  todo_write: "plan",
  todowrite: "plan",
  update_plan: "plan",
  delegate: "agent",
  task: "agent",
  agent: "agent",
  subagent: "agent",
};

/** A title as `verb rest`, with the guest's name in front taken off. */
function splitTitle(label: string, agent: string | undefined): { verb: string; rest: string } {
  let text = oneLine(label);
  if (agent !== undefined && text.startsWith(`${agent}: `)) text = text.slice(agent.length + 2);
  else {
    // A guest's name when what follows is a recognisable call, not `bash: ls`.
    const match = /^([a-z][\w-]*): (.+)$/i.exec(text);
    if (match?.[1] !== undefined && match[2] !== undefined && !(match[1].toLowerCase() in TOOLS)) {
      if (kindOfTitle(match[2]) !== undefined) text = match[2];
    }
  }
  const match = /^(\S+?)(?::\s*|\s+|$)(.*)$/.exec(text);
  return { verb: (match?.[1] ?? "").toLowerCase(), rest: match?.[2] ?? "" };
}

function kindOfTitle(title: string): WorkKind | undefined {
  const verb = /^([a-z_$]+)/i.exec(title)?.[1]?.toLowerCase() ?? "";
  if (/^update todos?$/i.test(title)) return "plan";
  if (/^(?:bash|sh|shell|zsh|run|exec|execute|\$)$/.test(verb) || title.startsWith("$")) {
    return "command";
  }
  if (/^(?:read|cat|view|open)$/.test(verb)) return "read";
  if (/^(?:edit|write|create|update|delete|remove|move|rename|patch|apply)$/.test(verb)) {
    return "edit";
  }
  if (/^(?:fetch|webfetch|websearch|curl|browse)$/.test(verb)) return "web";
  if (/^(?:search|grep|glob|find|rg)$/.test(verb)) return /\bweb\b/i.test(title) ? "web" : "search";
  return undefined;
}

function looksWeb(facts: ToolFacts | undefined, label: string): boolean {
  return /web|fetch|http|url|browse/i.test(`${facts?.tool ?? ""} ${label}`);
}

/** Which kind of work a tool call is: the harness's word for it, else its title, else its result. */
function kindOfCall(facts: ToolFacts | undefined, label: string, component: Component): WorkKind {
  const tool = (facts?.tool ?? "").toLowerCase();
  const byTool = TOOLS[tool];
  if (byTool !== undefined) return byTool;
  switch (facts?.kind) {
    case "execute":
      return "command";
    case "read":
      return "read";
    case "edit":
    case "delete":
    case "move":
      return "edit";
    case "fetch":
      return "web";
    case "search":
      return looksWeb(facts, label) ? "web" : "search";
    default:
      break;
  }
  const { verb, rest } = splitTitle(label, facts?.agent);
  const byTitle = kindOfTitle(`${verb} ${rest}`);
  if (byTitle !== undefined) return byTitle;
  if (/^\S+ · /.test(label)) return "agent";
  if (collect(component, (c) => (c.type === "diff" ? c : undefined)).length > 0) return "edit";
  return "tool";
}

/** A question to answer, or an answered one: a permission request or a choice. */
export function isQuestion(item: ViewItem): boolean {
  if (item.kind !== "tool") return false;
  return item.ref.startsWith("p-") || item.ref.startsWith("q-") || hasButtons(item.component);
}

/** The agent's todo list as an item (the daemon keeps one per agent). */
export function isPlanItem(item: ViewItem): boolean {
  return item.kind === "tool" && (item.ref === "plan" || item.ref.startsWith("plan-"));
}

/**
 * The call that wrote a plan: the plan has an item of its own, so showing the call as well would
 * say the same twice.
 */
export function isPlanCall(item: ViewItem, facts: ToolFacts | undefined): boolean {
  if (item.kind !== "tool" || isPlanItem(item) || isQuestion(item)) return false;
  return kindOfCall(facts, item.label ?? "", item.component) === "plan";
}

/** The lines of a plan item, as the daemon draws them: a task list, the current one marked `◐`. */
function planLines(component: Component): PlanLine[] {
  const lines: PlanLine[] = [];
  const texts = collect(component, (c) => (c.type === "text" ? c : undefined));
  for (const text of texts) {
    for (const block of text.blocks) {
      if (block.type !== "list") continue;
      for (const entry of block.items) {
        const raw = blocksText(entry.blocks).trim();
        const active = raw.startsWith("◐");
        lines.push({
          text: active ? raw.replace(/^◐\s*/, "") : raw,
          done: entry.checked === true,
          active,
        });
      }
    }
  }
  return lines;
}

/** The changes the diffs of a result make, one per file (a file changed twice is summed). */
function changesOf(component: Component): FileChange[] {
  const diffs = collect(component, (c) => (c.type === "diff" ? c : undefined));
  return mergeChanges(
    diffs.map((diff) => ({ path: diff.path ?? "", ...diffStats(diff.lines), diff })),
  );
}

/** Changes to the same file in one: counts summed, the diffs' lines one after the other. */
export function mergeChanges(changes: readonly FileChange[]): FileChange[] {
  const byPath = new Map<string, FileChange>();
  for (const change of changes) {
    const seen = byPath.get(change.path);
    if (seen === undefined) {
      byPath.set(change.path, { ...change });
      continue;
    }
    seen.added += change.added;
    seen.removed += change.removed;
    if (change.diff !== undefined) {
      const lines = [...(seen.diff?.lines ?? []), ...change.diff.lines];
      seen.diff = { type: "diff", path: change.path, lines };
    }
  }
  return [...byPath.values()];
}

/** Every file the entries of a turn changed, in the order they were first touched. */
export function changedFiles(entries: readonly WorkEntry[]): FileChange[] {
  const all: FileChange[] = [];
  for (const entry of entries) {
    if (entry.kind === "edit" && !entry.failed) all.push(...(entry.files ?? []));
  }
  return mergeChanges(all).filter((change) => change.diff !== undefined);
}

const EXIT_CODE = /\bexit(?:ed)?(?:\s+with)?(?:\s+(?:status|code))?[\s:=]+(-?\d+)\b/gi;

/** The exit status a command's result mentions: a `kv` line, or `Exit code 1` in its output. */
function exitOf(component: Component, output: string): number | undefined {
  const kv = collect(component, (c) => (c.type === "kv" ? c : undefined));
  for (const list of kv) {
    for (const entry of list.items) {
      if (/^exit/i.test(entry.key)) {
        const code = Number.parseInt(entry.value, 10);
        if (!Number.isNaN(code)) return code;
      }
    }
  }
  let found: number | undefined;
  for (const match of output.slice(-600).matchAll(EXIT_CODE)) found = Number(match[1]);
  return found;
}

/** The label of a collapsed read: the first path, and how many more there are. */
function readLabel(paths: readonly string[]): string {
  const [first, ...more] = paths;
  if (first === undefined) return "Read file";
  return more.length === 0 ? `Read ${first}` : `Read ${first} (+${more.length} more)`;
}

/** The label of a search: what for, and where. */
function searchLabel(query: string | undefined, target: string | undefined): string {
  if (query === undefined) return "Searched code";
  return target === undefined ? `Searched ${query}` : `Searched ${query} in ${target}`;
}

function webLabel(facts: ToolFacts | undefined, rest: string): string {
  const url = stringOf(facts?.rawInput, URL_KEYS);
  if (url !== undefined) return `Fetched ${url}`;
  const query = stringOf(facts?.rawInput, QUERY_KEYS) ?? (rest === "" ? undefined : rest);
  return query === undefined ? "Searched the web" : `Searched the web for ${query}`;
}

/** The state of a subagent's work in a word. */
function statusWord(item: ViewItem, active: boolean): string {
  switch (item.state) {
    case "running":
      return "running";
    case "pending":
      return active ? "starting" : "stopped";
    case "failed":
      return "failed";
    default:
      return "done";
  }
}

/** The arguments of a call as JSON for the expanded form, cut so one call cannot fill the window. */
function inputText(input: unknown): string | undefined {
  if (input === undefined) return undefined;
  const text = typeof input === "string" ? input : JSON.stringify(input, null, 2);
  return text === undefined || text === "{}" ? undefined : text.slice(0, 4000);
}

/** The paths a call's input names, else the one its title ends with. */
function pathsOf(input: unknown, rest: string): string[] {
  const named = [...stringsOf(input, PATH_KEYS), ...stringsOf(input, PATHS_KEYS)];
  if (named.length > 0) return named;
  if (rest === "") return named;
  return [rest];
}

/** What was run: the input's command, else the result's `command` line, else the title's. */
function commandText(
  input: unknown,
  kv: readonly { key: string; value: string }[],
  verb: string,
  rest: string,
  label: string,
): string {
  const given = commandOf(input);
  if (given !== undefined) return given;
  const listed = kv.find((entry) => entry.key.toLowerCase() === "command");
  if (listed !== undefined) return listed.value;
  return /^(?:bash|sh|shell|zsh|run|exec|execute)$/.test(verb) ? rest : label;
}

/** Makes the entry of one view item: a tool call, a thought, a plan or a question. */
export function classify(item: ViewItem, context: ClassifyContext): WorkEntry {
  const { facts, cwd, active } = context;
  const label = item.label ?? "";
  const live =
    item.kind === "thought"
      ? context.writing === true
      : active && (item.state === "running" || item.state === "pending");
  const entry: WorkEntry = {
    id: item.ref,
    kind: "tool",
    item,
    label,
    live,
    failed: item.state === "failed",
    at: context.at,
  };
  if (item.kind === "thought") {
    const preview = thoughtPreview(item.component) ?? "Thought";
    return { ...entry, kind: "thought", label: live ? "Thinking" : preview };
  }
  if (isQuestion(item)) return { ...entry, kind: "question", label: label || "a question" };
  if (isPlanItem(item)) {
    const lines = planLines(item.component);
    const owner = /^(.+): /.exec(label)?.[1];
    return {
      ...entry,
      live: false,
      kind: "plan",
      label: owner === undefined ? "Plan updated" : `${owner}: Plan updated`,
      plan: lines,
    };
  }
  const kind = kindOfCall(facts, label, item.component);
  const { verb, rest } = splitTitle(label, facts?.agent);
  const input = facts?.rawInput;
  switch (kind) {
    case "command": {
      const kv = collect(item.component, (c) => (c.type === "kv" ? c : undefined)).flatMap(
        (c) => c.items,
      );
      const command = commandText(input, kv, verb, rest, label);
      const output = collect(item.component, (c) => (c.type === "code" ? c.code : undefined))
        .join("\n")
        .replace(/\n+$/, "");
      const text = output !== "" ? output : kvOutput(kv);
      const exit = exitOf(item.component, text);
      const shown = stripShell(command.replace(/^`|`$/g, ""));
      return {
        ...entry,
        kind,
        command: shown,
        label: live ? `Running ${oneLine(shown)}` : oneLine(shown),
        output: text,
        exit,
      };
    }
    case "read": {
      const paths = pathsOf(input, rest).map((path) => relativePath(path, cwd));
      return { ...entry, kind, paths, label: readLabel(paths) };
    }
    case "edit": {
      const changes = changesOf(item.component).map((c) => ({
        ...c,
        path: relativePath(c.path, cwd),
      }));
      const asked = pathsOf(input, rest)
        .slice(0, 1)
        .map((path) => ({ path: relativePath(path, cwd), added: 0, removed: 0 }));
      const files = changes.length > 0 ? changes : asked;
      return {
        ...entry,
        kind,
        files,
        label: `Changed ${count(Math.max(files.length, 1), "file")}`,
      };
    }
    case "search": {
      const query = stringOf(input, QUERY_KEYS) ?? (rest === "" ? undefined : rest);
      const target = stringOf(input, TARGET_KEYS);
      return {
        ...entry,
        kind,
        label: searchLabel(query, target === undefined ? undefined : relativePath(target, cwd)),
        output: componentText(item.component),
      };
    }
    case "web":
      return {
        ...entry,
        kind,
        label: webLabel(facts, rest),
        output: componentText(item.component),
      };
    case "agent":
      return {
        ...entry,
        kind,
        label: label || "Subagent",
        status: statusWord(item, active),
        output: componentText(item.component),
      };
    case "plan":
      return { ...entry, kind: "tool", label: label || "Plan" };
    default:
      return {
        ...entry,
        kind: "tool",
        label: label || "Tool call",
        input: inputText(input),
        output: componentText(item.component),
      };
  }
}

/** A key-value result's output lines, when a command's result has no code. */
function kvOutput(items: readonly { key: string; value: string }[]): string {
  const out = items.find((entry) => /^(?:output|stdout|stderr)$/i.test(entry.key));
  return out?.value ?? "";
}

/** A thought as one line: its first heading, else its first sentence. */
export function thoughtPreview(component: Component): string | undefined {
  const text = componentText(component).trim();
  if (text === "") return undefined;
  const first = oneLine(text.split(/\n+/)[0] ?? "")
    .replace(/^#+\s*/, "")
    .replace(/^\*\*(.+)\*\*$/, "$1");
  const sentence = /^(.+?[.!?])(?:\s|$)/.exec(first)?.[1] ?? first;
  return sentence.length > 100 ? `${sentence.slice(0, 99)}…` : sentence;
}

/** How each kind of work reads in a summary, most important first. */
const SUMMARY: readonly {
  kind: WorkKind;
  /** How many of it a group holds. */
  amount: (entries: readonly WorkEntry[]) => number;
  say: (n: number) => string;
}[] = [
  {
    kind: "command",
    amount: (entries) => entries.length,
    say: (n) => `ran ${count(n, "command")}`,
  },
  {
    kind: "edit",
    amount: (entries) => new Set(entries.flatMap((e) => (e.files ?? []).map((f) => f.path))).size,
    say: (n) => `changed ${count(n, "file")}`,
  },
  { kind: "agent", amount: (e) => e.length, say: (n) => `delegated ${count(n, "task")}` },
  {
    kind: "search",
    amount: (entries) => entries.length,
    say: (n) => `searched code ${count(n, "time")}`,
  },
  {
    kind: "web",
    amount: (entries) => entries.length,
    say: (n) => `searched the web ${count(n, "time")}`,
  },
  {
    kind: "read",
    amount: (entries) => new Set(entries.flatMap((e) => e.paths ?? [])).size,
    say: (n) => `read ${count(n, "file")}`,
  },
  { kind: "tool", amount: (e) => e.length, say: (n) => `used ${count(n, "tool")}` },
  { kind: "plan", amount: (e) => e.length, say: () => "updated the plan" },
];

/** The kind of work that stands for a group: its most important, whose icon its header wears. */
export function leadKind(entries: readonly WorkEntry[]): WorkKind {
  for (const spec of SUMMARY)
    if (entries.some((entry) => entry.kind === spec.kind)) return spec.kind;
  return entries[0]?.kind ?? "tool";
}

/** `A and B`, `A, B, and C`. */
function joinParts(parts: readonly string[]): string {
  if (parts.length <= 2) return parts.join(" and ");
  return `${parts.slice(0, -1).join(", ")}, and ${parts.at(-1)}`;
}

/**
 * What a group of entries did, from its two most important kinds of work: `Ran 3 commands and
 * changed 2 files`. Whatever else it did is `performed 4 other actions`.
 */
export function groupSummary(entries: readonly WorkEntry[]): string {
  const present = SUMMARY.map((spec) => {
    const own = entries.filter((entry) => entry.kind === spec.kind);
    return { spec, own };
  }).filter(({ own }) => own.length > 0);
  if (present.length === 0) return "Thought";
  const parts = present.slice(0, 2).map(({ spec, own }) => spec.say(spec.amount(own)));
  const rest = present.slice(2).reduce((sum, { own }) => sum + own.length, 0);
  if (rest > 0) parts.push(`performed ${count(rest, "other action")}`);
  const text = joinParts(parts);
  return text.charAt(0).toUpperCase() + text.slice(1);
}

/** The entry a running group is busy with: the latest that is live, else the last. */
export function activeEntry(entries: readonly WorkEntry[]): WorkEntry | undefined {
  for (let i = entries.length - 1; i >= 0; i -= 1) {
    const entry = entries[i];
    if (entry?.live) return entry;
  }
  return entries.at(-1);
}

/** What a group's header says: its one entry's label, the live entry's while it runs, else the summary. */
export function groupLabel(entries: readonly WorkEntry[], live: boolean): string {
  const only = entries.length === 1 ? entries[0] : undefined;
  if (only !== undefined) return only.label;
  if (live) return activeEntry(entries)?.label ?? "";
  return groupSummary(entries);
}
