// Whether the agent's own actions (bash, read, write, edit) go ahead, by the session's
// approvals mode:
//   auto    nothing asks
//   edits   edits go ahead (the review card offers to undo them after the turn), the rest asks
//   review  edits go ahead and are then kept or reverted one by one, the rest asks
//   ask     everything asks; an edit's diff is shown before anything is written
// Reads are allowed in every mode. Sensitive files (.env, keys, see sensitive.ts) are the
// exception: reading one, or running a command that mentions one, asks in every mode. In auto
// mode nobody may be watching: unanswered for 3 minutes, that question is declined with a note
// and the agent goes on without the file (see secretTimeout).
// The question is a permission_request with the content to preview. "Allow for this session"
// holds per session and kind (per file for sensitive ones), in memory only.
import type { PermissionAnswer, PermissionAsk, RequestPermission } from "@neoworks-dev/neo/agent";
import type {
  DiffContent,
  PermissionOption,
  PermissionTimeout,
  ToolKind,
} from "@neoworks-dev/neo-protocol/types";
import type { Answer, Asker } from "./asker.ts";
import { editedTitle, KEEP_OR_REVERT, restore } from "./review.ts";
import { displayPath, sensitiveInCommand, sensitivePath } from "./sensitive.ts";
import type { Approvals } from "./settings.ts";

/** An execute request's title: the command itself is the content, shown as code. */
export const RUN_TITLE = "Run this command?";

const OPTIONS: PermissionOption[] = [
  { id: "allow", name: "Allow", kind: "allow_once" },
  { id: "always", name: "Allow for this session", kind: "allow_always" },
  { id: "reject", name: "Reject", kind: "reject_once" },
];

export interface PermissionsOptions {
  asker: Asker;
  approvals: (session: string) => Approvals;
  /** The session's working directory. */
  cwd: (session: string) => string;
  /** A file was written or put back: tell the editors. */
  fileChanged: (path: string, session: string) => void;
  /** For tests: the home directory the sensitive paths are under. */
  home?: string;
}

/** What a request is about, for the sensitive-file check. */
export interface Touches {
  kind: ToolKind;
  /** The files it reads. */
  paths: string[];
  /** The shell command it runs. */
  command?: string;
}

/**
 * The sensitive-file check, shared with the guests (their requests come from the adapters, not
 * from the hook): reads and commands that touch a sensitive file ask in every mode.
 */
export interface Guard {
  /** The sensitive files the request touches that the user hasn't allowed for the session. */
  open(session: string, touches: Touches): string[];
  /** The question about them. */
  title(session: string, kind: ToolKind, paths: string[]): string;
  /** The user allowed them for the rest of the session. */
  remember(session: string, paths: string[]): void;
}

/** How long auto mode waits for an answer about a sensitive file before declining. */
export const SECRET_TIMEOUT_MS = 3 * 60_000;

/**
 * Auto mode's question about a sensitive file declines itself after SECRET_TIMEOUT_MS: the user
 * left the session to run on its own. Undefined in other modes, or without a reject option.
 */
export function secretTimeout(
  mode: Approvals | undefined,
  options: PermissionOption[],
  now = Date.now(),
): PermissionTimeout | undefined {
  if (mode !== "auto") return undefined;
  const reject =
    options.find((o) => o.kind === "reject_once") ??
    options.find((o) => o.kind.startsWith("reject"));
  if (reject === undefined) return undefined;
  return {
    at: now + SECRET_TIMEOUT_MS,
    option: reject.id,
    note: "Nobody answered within 3 minutes, so this was declined. Go on without the file, and don't try to read it another way.",
  };
}

function answerOf({ option, note }: Answer): PermissionAnswer {
  const answer: PermissionAnswer = { allowed: option === "allow" || option === "always" };
  if (note !== undefined) answer.note = note;
  return answer;
}

/** The files a tool's request reads: a read's path, a command's words. */
function touchesOf(request: PermissionAsk): Touches {
  const touches: Touches = { kind: request.kind, paths: [] };
  if (request.kind === "read" && request.path !== undefined) touches.paths.push(request.path);
  if (request.kind === "execute") touches.command = request.title;
  return touches;
}

export class Permissions {
  /** session\0kind pairs (session\0path\0file for sensitive files) allowed for the session. */
  private readonly allowed = new Set<string>();

  constructor(private readonly options: PermissionsOptions) {}

  readonly guard: Guard = {
    open: (session, touches) => {
      const { home } = this.options;
      const cwd = this.options.cwd(session);
      const found = new Set<string>();
      for (const path of touches.paths) {
        const sensitive = sensitivePath(path, cwd, home);
        if (sensitive !== undefined) found.add(sensitive);
      }
      if (touches.command !== undefined) {
        for (const path of sensitiveInCommand(touches.command, cwd, home)) found.add(path);
      }
      return [...found].filter((path) => !this.allowed.has(this.pathKey(session, path)));
    },
    title: (session, kind, paths) => {
      const cwd = this.options.cwd(session);
      const names = paths.map((path) => displayPath(path, cwd, this.options.home)).join(", ");
      return kind === "read" ? `Read ${names}?` : `This command touches ${names}`;
    },
    remember: (session, paths) => {
      for (const path of paths) this.allowed.add(this.pathKey(session, path));
    },
  };

  /**
   * The hook for one session's agent (harness name `agent`), given to its tools. `onDecline`
   * runs when the user refused (rejected, or reverted an edit) without a note saying what to do
   * instead: the turn stops there, the model doesn't go on (see daemon.ts).
   */
  hook(session: string, agent: string, onDecline?: () => void): RequestPermission {
    const ask = this.ask(session, agent);
    return async (request, signal) => {
      const answer = await ask(request, signal);
      // Every refusal is the user's answer, but one to a question that was dropped.
      if (!answer.allowed && answer.note === undefined && signal?.aborted !== true) onDecline?.();
      return answer;
    };
  }

  private ask(session: string, agent: string): RequestPermission {
    return async (request, signal) => {
      if (request.phase === "after") return this.afterEdit(session, agent, request, signal);
      const mode = this.options.approvals(session);
      const open = this.guard.open(session, touchesOf(request));
      if (open.length > 0) return this.askSensitive(session, agent, request, open, signal);
      // A PreToolUse hook (the user's, or a plugin's they installed) said ask: it asks in every
      // mode, the sensitive-file check above aside. It said allow: the mode's question is not
      // asked, but review mode still lets the user take an edit back.
      const forced = request.hook === "ask";
      if (request.kind === "read" && !forced) return { allowed: true };
      if (request.kind === "edit") {
        if (mode === "review") return { allowed: true, review: true };
        if ((mode === "edits" || mode === "auto") && !forced) return { allowed: true };
      }
      if (request.hook === "allow") return { allowed: true };
      if (mode === "auto" && !forced) return { allowed: true };
      const key = `${session}\0${request.kind}`;
      if (this.allowed.has(key) && !forced) return { allowed: true };
      const asked = { ...request };
      delete asked.hook;
      const question: Parameters<Asker["askWithNote"]>[1] = {
        type: "permission_request",
        agent,
        ...asked,
        options: OPTIONS,
      };
      if (request.kind === "execute" && !request.content?.length) {
        question.title = RUN_TITLE;
        question.content = [{ type: "text", text: request.title }];
      }
      const answer = await this.options.asker.askWithNote(session, question, signal);
      if (answer.option === "always") this.allowed.add(key);
      return answerOf(answer);
    };
  }

  private pathKey(session: string, path: string): string {
    return `${session}\0path\0${path}`;
  }

  /** Reading or running something that touches a sensitive file: asks in every mode. */
  private async askSensitive(
    session: string,
    agent: string,
    request: PermissionAsk,
    paths: string[],
    signal?: AbortSignal,
  ): Promise<PermissionAnswer> {
    const question: Parameters<Asker["askWithNote"]>[1] = {
      type: "permission_request",
      agent,
      title: this.guard.title(session, request.kind, paths),
      kind: request.kind,
      options: OPTIONS,
    };
    if (request.toolCallId !== undefined) question.toolCallId = request.toolCallId;
    if (request.kind === "execute") question.content = [{ type: "text", text: request.title }];
    const timeout = secretTimeout(this.options.approvals(session), OPTIONS);
    if (timeout !== undefined) question.timeout = timeout;
    const answer = await this.options.asker.askWithNote(session, question, signal);
    if (answer.option === "always") this.guard.remember(session, paths);
    return answerOf(answer);
  }

  /**
   * Review mode, after the tool wrote an edit: Keep or Revert. Revert puts the file back (if it
   * still has what the edit wrote) before the tool tells the model.
   */
  private async afterEdit(
    session: string,
    agent: string,
    request: PermissionAsk,
    signal?: AbortSignal,
  ): Promise<PermissionAnswer> {
    const diffs = (request.content ?? []).filter((c): c is DiffContent => c.type === "diff");
    for (const diff of diffs) this.options.fileChanged(diff.path, session);
    const cwd = this.options.cwd(session);
    const names = diffs.map((diff) => displayPath(diff.path, cwd, this.options.home));
    const question: Parameters<Asker["askWithNote"]>[1] = {
      type: "permission_request",
      agent,
      title: editedTitle(names),
      kind: "edit",
      content: diffs,
      options: KEEP_OR_REVERT,
    };
    if (request.toolCallId !== undefined) question.toolCallId = request.toolCallId;
    const { option, note } = await this.options.asker.askWithNote(session, question, signal);
    const answer: PermissionAnswer = { allowed: option !== "revert" };
    if (note !== undefined) answer.note = note;
    if (option !== "revert") return answer;
    let reverted = true;
    for (const diff of diffs) {
      const done = await restore(diff.path, { before: diff.oldText, after: diff.newText });
      if (done) this.options.fileChanged(diff.path, session);
      else reverted = false;
    }
    answer.reverted = reverted;
    return answer;
  }
}
