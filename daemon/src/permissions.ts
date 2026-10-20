// Whether the agent's own actions (bash, read, write, edit) go ahead, by the session's
// approvals mode:
//   auto    nothing asks
//   edits   edits go ahead (the review card offers to undo them after the turn), the rest asks
//   review  edits go ahead and are then kept or reverted one by one, the rest asks
//   ask     everything asks; an edit's diff is shown before anything is written
// Reads are allowed in every mode. Sensitive files (.env, keys, see sensitive.ts) are the
// exception: reading one, or running a command that mentions one, asks in every mode.
// The question is a permission_request with the content to preview. "Allow for this session"
// holds per session and kind (per file for sensitive ones), in memory only.
import type { PermissionAnswer, PermissionAsk, RequestPermission } from "@suite/harness/agent";
import type { DiffContent, PermissionOption, ToolKind } from "@suite/protocol/types";
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

  /** The hook for one session's agent (harness name `agent`), given to its tools. */
  hook(session: string, agent: string): RequestPermission {
    return async (request, signal) => {
      if (request.phase === "after") return this.afterEdit(session, agent, request, signal);
      const mode = this.options.approvals(session);
      const open = this.guard.open(session, touchesOf(request));
      if (open.length > 0) return this.askSensitive(session, agent, request, open, signal);
      if (request.kind === "read") return { allowed: true };
      if (request.kind === "edit") {
        if (mode === "review") return { allowed: true, review: true };
        if (mode === "edits" || mode === "auto") return { allowed: true };
      }
      if (mode === "auto") return { allowed: true };
      const key = `${session}\0${request.kind}`;
      if (this.allowed.has(key)) return { allowed: true };
      const question: Parameters<Asker["askWithNote"]>[1] = {
        type: "permission_request",
        agent,
        ...request,
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
