// The delegate tool of the session's agent (whatever harness it runs on). Starting a subagent is
// the user's decision: each delegation first asks "Start a subagent?" (start it, allow it for
// the rest of the session, use another harness instead, or reject), with the harness, model,
// effort and directory that will run and the task. Without model and effort in the call the
// subagent runs with the settings the user chose for it, else its own defaults. A model or
// effort the harness lists and doesn't know is refused before anything is asked.
//
// Without cwd or worktree the subagent works inside the session (its events carry its name in
// the log; the session's view draws one tool item). With one, it runs as a session of its own in
// that directory, on the delegated harness, which the sidebar groups under its worktree; the
// parent waits for that session's turn and returns its final reply.
//
// With `background` the call returns once the user allowed it, with the subagent's id, and the
// subagent runs on beside the agent: it can be written to and can write (messages.ts), and its
// final reply reaches the agent as a message. Such a subagent is stopped by `x` in the subagents
// list and when the turn of the session is cancelled (the user said stop), not when the turn
// ends; whoever stopped it hears so with its next message, without a turn being started for it.
// A subagent also keeps running while messages arrive: a turn that ended with messages waiting
// goes on with them before the subagent is over. One subagent per harness and directory runs at
// a time, which is how a message finds its recipient.
import { homedir } from "node:os";
import type { AgentInfo, AgentUpdate, ConfigCategory, ConfigOption } from "@neoworks-dev/neo/agent";
import { findAgent, listAgents } from "@neoworks-dev/neo/agents";
import type { DelegateArgs, ModelsArgs } from "@suite/protocol/core-tools";
import type { CallToolResult } from "@modelcontextprotocol/sdk/types.js";
import type {
  PermissionDetail,
  PermissionOption,
  SessionEvent,
  SessionEventRecord,
  StopReason,
} from "@suite/protocol/types";
import type { Asker } from "../asker.ts";
import { MESSAGES_APP, type Member, type Messenger } from "../messages.ts";
import type { SessionManager } from "../sessions.ts";
import { type Approvals, CATEGORIES, type HarnessSettings } from "../settings.ts";
import type { SessionStore } from "../store.ts";
import type { GuestManager } from "./guests.ts";
import type { RunOptions } from "./run.ts";
import { NEO } from "./subagent.ts";
import { createWorktree, resolveTarget, type Target } from "./worktree.ts";

/** Valid names listed in a refusal before it says how many more there are. */
const LISTED = 40;

export interface DelegateParts {
  guests: GuestManager;
  asker: Asker;
  sessions: SessionManager;
  store: SessionStore;
  messenger: Messenger;
  settings: (session: string, guest: string) => HarnessSettings;
  append: (session: string, event: SessionEvent) => void;
  /** Who asks: the harness the session runs on. */
  main: (session: string) => string;
  /** The session's approvals mode; a session made for a delegation starts in the same one. */
  approvals: (session: string) => Approvals;
  cwd: (session: string) => string;
  /** Whether the user lets this session start guests without asking (/approvals auto). */
  autoApprove?: (session: string) => boolean;
  /** For showing directories as ~/…; tests give theirs. */
  home?: string;
}

/** What the daemon exposes of the tool: the gateway calls `run`, `x` in a pane calls `cancel`. */
export interface Delegate {
  run: (session: string, args: DelegateArgs, signal?: AbortSignal) => Promise<CallToolResult>;
  /** The models tool: what each harness takes for model and effort. */
  models: (session: string, args: ModelsArgs) => Promise<CallToolResult>;
  /** Stops one running subagent (its id is in subagent_started); false when it isn't running. */
  cancel: (session: string, id: string) => boolean;
  /** Stops the session's background subagents (its turn was cancelled). */
  stopBackground: (session: string) => void;
  /** Stops every subagent and waits for them to wind up (the daemon is stopping). */
  shutdown: (timeoutMs: number) => Promise<void>;
}

interface Result {
  text: string;
  stopReason: StopReason;
  error?: string;
}

function text(value: string, isError = false): CallToolResult {
  const result: CallToolResult = { content: [{ type: "text", text: value }] };
  if (isError) result.isError = true;
  return result;
}

/** Which harness runs and with which settings of its own for this task. */
interface Decision {
  agent: string;
  settings: HarnessSettings;
  /** What the user added to their answer. */
  note?: string;
}

/** The result's lines with the user's note after them, when there is one. */
function withNote(lines: string[], note: string | undefined): string[] {
  if (note === undefined) return lines;
  return [...lines, `The user added: ${note}`];
}

/** What the call asks for itself: its model and effort. */
function explicitSettings(args: DelegateArgs): HarnessSettings {
  const settings: HarnessSettings = {};
  if (args.model !== undefined && args.model !== "") settings.model = args.model;
  if (args.effort !== undefined && args.effort !== "") settings.thought_level = args.effort;
  return settings;
}

const norm = (value: string): string => value.trim().toLowerCase().replace(/\s+/g, "-");

/**
 * The harness's own spelling of a model or effort the call named (case, spaces and the display
 * name don't matter), or the refusal listing what it takes. `provider/id` is left to the
 * harness: a registry that lags behind new models would call a valid one unknown.
 */
function canonical(
  options: ConfigOption[] | undefined,
  category: ConfigCategory,
  value: string,
  harness: string,
): { value: string } | { refusal: string } {
  const option = options?.find((o) => o.category === category);
  if (!option || option.values.length === 0) return { value };
  if (category === "model" && value.includes("/")) return { value };
  const wanted = norm(value);
  const hit = option.values.find(
    (v) =>
      norm(v.value) === wanted ||
      norm(v.name) === wanted ||
      norm(v.value.split("/").at(-1) ?? "") === wanted,
  );
  if (hit) return { value: hit.value };
  const words = wanted.split("-").filter((word) => word.length >= 3);
  const near = (v: string): boolean => words.some((word) => norm(v).includes(word));
  const names = option.values.map((v) => v.value);
  const ordered = [...names.filter(near), ...names.filter((v) => !near(v))];
  const shown = ordered.slice(0, LISTED).join(", ");
  let more = "";
  if (ordered.length > LISTED) more = `, … (${ordered.length - LISTED} more)`;
  const what = category === "model" ? "model" : "effort";
  return {
    refusal: `Unknown ${what} "${value}" for ${harness}. Valid ${what === "model" ? "models" : "efforts"}: ${shown}${more}.`,
  };
}

/** The final reply and how the turn that started at `after` ended, from a session's log. */
function finalReply(records: SessionEventRecord[], task: string): Result {
  const start = records.findLastIndex(
    (r) => r.event.type === "user_message" && r.event.app === "delegate" && r.event.text === task,
  );
  const messages = new Map<string, string>();
  let ended: Extract<SessionEvent, { type: "turn_ended" }> | undefined;
  for (const { event } of records.slice(Math.max(start, 0))) {
    if (event.type === "user_message" && ended) {
      // Turns that messages from other agents started belong to the delegation: the reply is
      // the last one's. Anything else the user did in that session does not.
      if (event.app !== MESSAGES_APP) break;
      ended = undefined;
      messages.clear();
    } else if (event.type === "agent_message_chunk" && event.agent === undefined) {
      messages.set(event.messageId, (messages.get(event.messageId) ?? "") + event.text);
    } else if (event.type === "turn_ended") {
      ended = event;
    }
  }
  const out: Result = {
    text: [...messages.values()].at(-1)?.trim() ?? "",
    stopReason: ended?.stopReason ?? "error",
  };
  if (ended?.error !== undefined) out.error = ended.error;
  else if (!ended) out.error = "the session ended without a reply";
  return out;
}

export function createDelegate(parts: DelegateParts): Delegate {
  const { guests, asker, sessions, store, messenger } = parts;
  const home = parts.home ?? homedir();
  /** session, harness, directory and settings the user allowed for the rest of the session. */
  const allowed = new Set<string>();
  /** Running subagents by id, to stop one. `done` settles when its end is logged. */
  const running = new Map<
    string,
    { session: string; stop: AbortController; background: boolean; done: Promise<unknown> }
  >();

  const shorten = (path: string): string =>
    path === home || path.startsWith(`${home}/`) ? `~${path.slice(home.length)}` : path;

  /** The harness to run (the user may pick another) and the settings to run it with. */
  async function decide(
    session: string,
    agent: string,
    task: string,
    explicit: HarnessSettings,
    target: Target | undefined,
    background: boolean,
    definition: AgentInfo | undefined,
    signal?: AbortSignal,
  ): Promise<Decision | { refusal: CallToolResult }> {
    // What will run: the call's own choices win over the user's.
    const effective = { ...parts.settings(session, agent), ...explicit };
    const where = target?.dir ?? "";
    const key = `${session}\0${agent}\0${definition?.name ?? ""}\0${where}\0${JSON.stringify(effective)}`;
    if (allowed.has(key) || parts.autoApprove?.(session) === true) {
      return { agent, settings: explicit };
    }
    const title = guests.title(agent);
    const options: PermissionOption[] = [
      { id: "allow", name: `Start ${title}`, kind: "allow_once" },
      { id: "always", name: `Allow ${title} for this session`, kind: "allow_always" },
    ];
    for (const other of guests.harnesses().filter((n) => n !== agent)) {
      options.push({ id: `use:${other}`, name: `Use ${guests.title(other)}`, kind: "allow_once" });
    }
    options.push({ id: "reject", name: "Reject", kind: "reject_once" });
    const details: PermissionDetail[] = [{ key: "Harness", value: title }];
    if (definition) details.push({ key: "Agent", value: definition.name });
    details.push(
      { key: "Model", value: effective.model ?? "default" },
      { key: "Effort", value: effective.thought_level ?? "default" },
    );
    if (effective.mode !== undefined) details.push({ key: "Mode", value: effective.mode });
    details.push({ key: "Directory", value: describeDirectory(session, target) });
    if (background) details.push({ key: "Runs", value: "in the background" });
    const { option: answer, note } = await asker.askWithNote(
      session,
      {
        type: "permission_request",
        agent: parts.main(session),
        title: "Start a subagent?",
        details,
        content: [{ type: "text", text: task }],
        options,
      },
      signal,
    );
    if (answer === undefined) return { refusal: text("No answer: the turn ended first.", true) };
    if (answer === "reject") {
      const declined = `The user declined to hand this to ${title}. Do it yourself, or ask them how to proceed.`;
      return { refusal: text(withNote([declined], note).join("\n\n"), true) };
    }
    if (answer === "always") allowed.add(key);
    const decision: Decision = { agent, settings: explicit };
    // Another harness knows other model names: the call's model and effort don't carry over.
    if (answer.startsWith("use:"))
      Object.assign(decision, { agent: answer.slice(4), settings: {} });
    if (note !== undefined) decision.note = note;
    return decision;
  }

  function describeDirectory(session: string, target: Target | undefined): string {
    if (!target) return shorten(parts.cwd(session));
    const { worktree } = target;
    if (!worktree) return shorten(target.dir);
    const how = worktree.create ? `new worktree, branch ${worktree.name}` : "worktree";
    return `${shorten(target.dir)} (${how})`;
  }

  /**
   * The subagent as part of this session: its events are labelled, the view folds them. A turn
   * that ends with messages waiting goes on with them; the subagent leaves the messenger in the
   * same breath as it finds none, so a message never lands in an inbox nobody reads.
   */
  async function inSession(
    session: string,
    member: Member,
    task: string,
    settings: HarnessSettings,
    signal: AbortSignal,
    definition?: string,
  ): Promise<Result> {
    const emit = (update: AgentUpdate): void => {
      parts.append(session, update);
    };
    const takeMessages = (): string[] => messenger.takeOf(session, member.id);
    let prompt = task;
    for (;;) {
      const runOptions: RunOptions = { emit, signal, settings, takeMessages };
      if (definition !== undefined) runOptions.definition = definition;
      const result = await guests.run(session, member.agent, prompt, runOptions);
      const out: Result = { text: result.text, stopReason: result.stopReason };
      if (result.error !== undefined) out.error = result.error;
      if (result.stopReason !== "end_turn") return out;
      const more = takeMessages();
      if (more.length === 0) {
        messenger.leave(session, member.id, "finished");
        return out;
      }
      prompt = more.join("\n\n");
    }
  }

  /** The session a delegation into this directory on this harness ran as before, if any. */
  function findChild(session: string, dir: string, agent: string): string | undefined {
    return sessions
      .list()
      .find((s) => s.parent === session && s.cwd === dir && parts.main(s.id) === agent)?.id;
  }

  /** The session a delegation into this directory on this harness runs as: reused or made. */
  function childFor(session: string, dir: string, agent: string): string {
    const found = findChild(session, dir, agent);
    if (found !== undefined) return found;
    const child = sessions.create({ cwd: dir, parent: session }).id;
    store.append(child, {
      type: "setting_changed",
      harness: "suite",
      option: "harness",
      value: agent,
    });
    const mode = parts.approvals(session);
    if (mode !== "ask") {
      store.append(child, {
        type: "setting_changed",
        harness: "suite",
        option: "approvals",
        value: mode,
      });
    }
    return child;
  }

  /**
   * The subagent as a session of its own in `dir`; waits for its turn(s) and reads the reply.
   * The session starts the turns that messages for it ask for (see SessionManager.wake).
   */
  async function inDirectory(
    session: string,
    child: string,
    member: Member,
    task: string,
    settings: HarnessSettings,
    signal: AbortSignal,
  ): Promise<Result> {
    const agent = member.agent;
    for (const [category, value] of Object.entries(settings)) {
      const option = Object.entries(CATEGORIES).find(([, c]) => c === category)?.[0];
      try {
        await sessions.configure(child, category as ConfigCategory, value, agent);
      } catch (err) {
        return {
          text: "",
          stopReason: "error",
          error: err instanceof Error ? err.message : String(err),
        };
      }
      if (option === "model" || option === "effort" || option === "mode") {
        store.append(child, { type: "setting_changed", harness: agent, option, value });
      }
    }
    const after = store.getSession(child)?.lastSeq ?? 0;
    const unmirror = asker.mirror(child, session);
    const onAbort = (): void => sessions.cancel(child);
    signal.addEventListener("abort", onAbort);
    try {
      if (signal.aborted) return { text: "", stopReason: "cancelled" };
      sessions.input(child, "delegate", task);
      for (;;) {
        await sessions.idle(child);
        // A message that came in as the last turn ended starts one more.
        if (signal.aborted || !messenger.hasPending(session, member.id)) break;
        sessions.wake(child);
        if (messenger.hasPending(session, member.id)) break;
      }
      messenger.leave(session, member.id, "finished");
      return finalReply(store.read(child, after + 1), task);
    } finally {
      signal.removeEventListener("abort", onAbort);
      unmirror();
      // Questions nobody answered are dropped with the turn.
      asker.dropAll(child);
    }
  }

  /** One harness's models and efforts, for the models tool. */
  async function harnessModels(session: string, agent: string, query?: string): Promise<string> {
    const head = `${agent} (${guests.title(agent)})`;
    let options: ConfigOption[] | undefined;
    try {
      // neo answers in-process; a guest is started (once per session) to ask it.
      options =
        agent === NEO
          ? await guests.knownOptions(session, agent)
          : await guests.configOptions(session, agent);
    } catch (err) {
      const why = err instanceof Error ? err.message : String(err);
      return `${head}: couldn't start it to ask (${why}).`;
    }
    const chosen = parts.settings(session, agent);
    const lines = [head];
    const words = (query ?? "")
      .split(/[\s,]+/)
      .map(norm)
      .filter((w) => w !== "");
    for (const [category, label] of [
      ["model", "models"],
      ["thought_level", "efforts"],
    ] as const) {
      const option = options?.find((o) => o.category === category);
      if (!option || option.values.length === 0) {
        lines.push(`  ${label}: no list; it takes what its own docs name.`);
        continue;
      }
      const matches = (v: { value: string; name: string }): boolean =>
        category !== "model" ||
        words.length === 0 ||
        words.some((w) => norm(v.value).includes(w) || norm(v.name).includes(w));
      const values = option.values.filter(matches);
      if (values.length === 0) {
        lines.push(`  ${label}: none match "${query ?? ""}" of ${option.values.length}.`);
        continue;
      }
      const shown = values.slice(0, LISTED).map((v) => {
        const marks: string[] = [];
        if (v.value === chosen[category]) marks.push("the user's choice");
        if (v.value === option.current) marks.push("default");
        let line = v.value;
        if (norm(v.name) !== norm(v.value)) line += ` ${v.name}`;
        if (marks.length > 0) line += ` [${marks.join(", ")}]`;
        return line;
      });
      if (values.length > LISTED)
        shown.push(`… ${values.length - LISTED} more (narrow with query)`);
      lines.push(`  ${label}:`, ...shown.map((v) => `    ${v}`));
    }
    if (agent === NEO) lines.push("  neo also takes any provider/model id.");
    return lines.join("\n");
  }

  const models: Delegate["models"] = async (session, { agent, query }) => {
    const harnesses = guests.harnesses();
    if (agent !== undefined && !harnesses.includes(agent)) {
      const known = harnesses.length > 0 ? harnesses.join(", ") : "none are configured";
      return text(`No guest named ${agent} (${known}).`, true);
    }
    const names = agent === undefined ? harnesses : [agent];
    if (names.length === 0) return text("No subagent harnesses are configured.", true);
    const blocks = await Promise.all(names.map((name) => harnessModels(session, name, query)));
    const how =
      "Give delegate a value as its model or effort. Without them the user's choice runs, else the default.";
    return text([...blocks, how].join("\n\n"));
  };

  const run: Delegate["run"] = async (session, args, signal) => {
    const { task } = args;
    let { agent } = args;
    const harnesses = guests.harnesses();
    // A name that is not a harness may be an agent definition: it runs on neo.
    const definitions = guests.agentDefinitions(session);
    let definition: AgentInfo | undefined;
    if (!harnesses.includes(agent)) {
      definition = findAgent(definitions, agent);
      if (definition) agent = NEO;
    }
    if (!harnesses.includes(agent)) {
      const names = [...harnesses, ...listAgents(definitions).map((d) => d.name)];
      const known = names.length > 0 ? names.join(", ") : "none are configured";
      return text(`No guest named ${args.agent} (${known}).`, true);
    }
    if (definition && (args.cwd !== undefined || args.worktree !== undefined)) {
      return text(
        `${definition.name} is an agent definition: it works in the session's directory, not in a cwd or worktree of its own.`,
        true,
      );
    }
    if (sessions.info(session)?.parent !== undefined) {
      return text("Subagents can't delegate.", true);
    }
    const resolved = await resolveTarget(parts.cwd(session), args);
    if (typeof resolved === "string") return text(resolved, true);
    const target = resolved;

    // Before asking for something that would fail: the harness's own list of names.
    // The definition's model is a person's choice, the call's own still wins over it.
    const explicit: HarnessSettings = {};
    if (definition?.model !== undefined) explicit.model = definition.model;
    Object.assign(explicit, explicitSettings(args));
    const options = await guests.knownOptions(session, agent);
    for (const category of ["model", "thought_level"] as const) {
      const wanted = explicit[category];
      if (wanted === undefined) continue;
      const checked = canonical(options, category, wanted, guests.title(agent));
      if ("refusal" in checked) return text(checked.refusal, true);
      explicit[category] = checked.value;
    }

    const background = args.background === true;
    const busy = busyWith(session, agent, target);
    if (busy !== undefined) return text(busy, true);
    const decision = await decide(
      session,
      agent,
      task,
      explicit,
      target,
      background,
      definition,
      signal,
    );
    if ("refusal" in decision) return decision.refusal;
    // Another harness knows nothing of the definition.
    if (decision.agent !== agent) definition = undefined;
    if (decision.agent !== agent) {
      const other = busyWith(session, decision.agent, target);
      if (other !== undefined) return text(other, true);
    }
    if (target?.worktree) {
      const failed = await createWorktree(target);
      if (failed !== undefined) return text(failed, true);
    }

    const id = `d_${crypto.randomUUID().slice(0, 8)}`;
    const effective = { ...parts.settings(session, decision.agent), ...decision.settings };
    const child = target ? childFor(session, target.dir, decision.agent) : undefined;
    const member: Member = { id, agent: decision.agent };
    if (child !== undefined) member.child = child;
    const started: Extract<SessionEvent, { type: "subagent_started" }> = {
      type: "subagent_started",
      id,
      agent: decision.agent,
      task,
    };
    if (definition) started.name = definition.name;
    if (effective.model !== undefined) started.model = effective.model;
    if (effective.thought_level !== undefined) started.effort = effective.thought_level;
    if (target) started.cwd = target.dir;
    if (child !== undefined) started.child = child;
    if (background) started.background = true;

    const stop = new AbortController();
    messenger.join(session, member, background);
    parts.append(session, started);
    const done = work(
      session,
      member,
      task,
      decision.settings,
      child,
      stop.signal,
      definition?.name,
    );
    running.set(id, { session, stop, background, done });
    const where: string[] = [];
    if (decision.agent !== agent) {
      where.push(`(The user chose ${guests.title(decision.agent)} instead.)`);
    }

    if (background) {
      // The reply comes as a message; a subagent the user (or a cancelled turn) stopped reports
      // that without starting a turn for it.
      void done
        .then((result) =>
          messenger.report(
            session,
            member,
            describe(result, member, target, child).join("\n\n"),
            result.stopReason !== "cancelled",
          ),
        )
        .catch(() => {
          // The store is closed: the daemon is stopping.
        });
      const lines = [
        ...where,
        `Started ${decision.agent} as ${id} in the background. Its final reply will arrive as a message (reply="true"); send_message reaches it meanwhile and wait blocks until something arrives.`,
      ];
      if (target) {
        let works = `It works in ${target.dir}`;
        if (child !== undefined) works += `, session ${child}`;
        lines.push(`${works}.`);
      }
      return text(withNote(lines, decision.note).join("\n\n"));
    }

    const onAbort = (): void => stop.abort();
    signal?.addEventListener("abort", onAbort);
    if (signal?.aborted) stop.abort();
    let result: Result;
    try {
      result = await done;
    } finally {
      signal?.removeEventListener("abort", onAbort);
    }
    const lines = [...where, ...describe(result, member, target, child)];
    return text(
      withNote(lines, decision.note).join("\n\n"),
      result.stopReason === "error" || result.stopReason === "cancelled",
    );
  };

  /** Why a delegation to this harness (and directory) can't start now, if it can't. */
  function busyWith(
    session: string,
    agent: string,
    target: Target | undefined,
  ): string | undefined {
    let child: string | undefined;
    if (target) {
      child = findChild(session, target.dir, agent);
      if (child === undefined) return undefined;
    }
    const busy = messenger.running(session, agent, child);
    if (!busy) return undefined;
    return `${guests.title(agent)} is still working on an earlier task here (${busy.id}). Send it a message with send_message, wait for it, or give this one another directory or harness.`;
  }

  /**
   * Runs the subagent and logs its start's counterpart. Never rejects (but for a closed store):
   * a failure is the result.
   */
  async function work(
    session: string,
    member: Member,
    task: string,
    settings: HarnessSettings,
    child: string | undefined,
    signal: AbortSignal,
    definition?: string,
  ): Promise<Result> {
    let result: Result;
    try {
      result =
        child === undefined
          ? await inSession(session, member, task, settings, signal, definition)
          : await inDirectory(session, child, member, task, settings, signal);
    } catch (err) {
      result = {
        text: "",
        stopReason: "error",
        error: err instanceof Error ? err.message : String(err),
      };
    }
    let how = "finished";
    if (result.stopReason === "cancelled") how = "stopped";
    else if (result.stopReason !== "end_turn") how = "failed";
    messenger.leave(session, member.id, how);
    running.delete(member.id);
    const ended: Extract<SessionEvent, { type: "subagent_ended" }> = {
      type: "subagent_ended",
      id: member.id,
      stopReason: result.stopReason,
    };
    if (result.error !== undefined) ended.error = result.error;
    parts.append(session, ended);
    return result;
  }

  /** What a subagent gives back: where it worked, its last message, how it stopped. */
  function describe(
    result: Result,
    member: Member,
    target: Target | undefined,
    child: string | undefined,
  ): string[] {
    const lines: string[] = [];
    if (target) {
      let where = target.dir;
      if (child !== undefined) where += `, session ${child}`;
      lines.push(`(Worked in ${where}.)`);
    }
    lines.push(result.text || "(no reply)");
    if (result.stopReason !== "end_turn") {
      lines.push(`[${member.agent} stopped: ${result.error ?? result.stopReason}]`);
    }
    return lines;
  }

  return {
    run,
    models,
    cancel: (session, id) => {
      const entry = running.get(id);
      if (!entry || entry.session !== session) return false;
      entry.stop.abort();
      return true;
    },
    stopBackground: (session) => {
      for (const entry of running.values()) {
        if (entry.session === session && entry.background) entry.stop.abort();
      }
    },
    shutdown: async (timeoutMs) => {
      const entries = [...running.values()];
      for (const entry of entries) entry.stop.abort();
      await Promise.race([
        Promise.allSettled(entries.map((entry) => entry.done)),
        Bun.sleep(timeoutMs),
      ]);
    },
  };
}
