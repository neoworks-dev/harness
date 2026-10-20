// Review of edits that were written without asking (design §9, approvals). In edits mode, after
// the turn: one card offers to keep them or put the files back (a choice: Keep, Revert all,
// Revert <file> when there are several files). In review mode, after each edit an ACP harness
// made (neo's own tools ask themselves, see permissions.ts): a card with the diff and Keep or
// Revert. Reverting restores each file to what it was before, unless it changed again since,
// and tells the agent (edits_reverted).
import { readFile, rm, writeFile } from "node:fs/promises";
import { isAbsolute, relative } from "node:path";
import type {
  DiffContent,
  PermissionOption,
  SessionEvent,
  SessionEventRecord,
  ToolCallContent,
} from "@suite/protocol/types";
import type { Asker } from "./asker.ts";
import type { Logger } from "./log.ts";
import type { Approvals } from "./settings.ts";
import type { SessionStore } from "./store.ts";

/** Above this many files the card only offers Keep and Revert all. */
const MAX_FILE_OPTIONS = 10;
/** How many file names the card's title lists. */
const TITLE_FILES = 5;
const BOM = "﻿";

/** The options of a review-mode card: the edit is written already. */
export const KEEP_OR_REVERT: PermissionOption[] = [
  { id: "keep", name: "Keep", kind: "allow_once" },
  { id: "revert", name: "Revert", kind: "reject_once" },
];

/** One file's edits over a turn. */
export interface FileEdit {
  /** The content before the turn's first edit; absent: the turn created the file. */
  before?: string;
  /** The content after its last edit. */
  after: string;
}

interface Call {
  agent?: string;
  status?: string;
  diffs: DiffContent[];
  counted: boolean;
}

/** The edits of the turn in progress. */
interface Turn {
  files: Map<string, FileEdit>;
  calls: Map<string, Call>;
  /** Tool calls the user was asked about (they were not "applied without asking"). */
  asked: Set<string>;
}

export interface ReviewOptions {
  store: SessionStore;
  asker: Asker;
  approvals: (session: string) => Approvals;
  /** A file was put back (or deleted): tell the editors. */
  fileChanged: (path: string, session: string) => void;
  log: Logger;
}

/** "Edited a.ts, b.ts — keep it?" */
export function editedTitle(names: string[]): string {
  return `Edited ${names.join(", ")} — keep it?`;
}

/** The agent of a guest's tool call id (`claude:t1`, `claude-main:t1`). */
function guestOf(toolCallId: string): string {
  const colon = toolCallId.indexOf(":");
  return colon > 0 ? toolCallId.slice(0, colon).replace(/-main$/, "") : "agent";
}

/** Each file's first before and last after over the diffs, added to `into`. */
function fold(diffs: DiffContent[], into = new Map<string, FileEdit>()): Map<string, FileEdit> {
  for (const diff of diffs) {
    const known = into.get(diff.path);
    if (known) known.after = diff.newText;
    else into.set(diff.path, { before: diff.oldText, after: diff.newText });
  }
  return into;
}

/** Editors keep a BOM and CRLF out of the diffs; compare and restore without them. */
function normalize(text: string): string {
  return (text.startsWith(BOM) ? text.slice(1) : text).replaceAll("\r\n", "\n");
}

export class Review {
  private readonly turns = new Map<string, Turn>();

  constructor(private readonly options: ReviewOptions) {
    options.store.onAppend((record) => this.onEvent(record));
  }

  private turn(session: string): Turn {
    let turn = this.turns.get(session);
    if (!turn) {
      turn = { files: new Map(), calls: new Map(), asked: new Set() };
      this.turns.set(session, turn);
    }
    return turn;
  }

  private onEvent({ session, event }: SessionEventRecord): void {
    switch (event.type) {
      case "user_message":
        this.turns.delete(session);
        break;
      case "permission_request":
        if (event.toolCallId !== undefined) this.turn(session).asked.add(event.toolCallId);
        break;
      case "tool_call":
        this.track(session, event.toolCallId, event.status, undefined, event.agent);
        break;
      case "tool_call_update":
        this.track(session, event.toolCallId, event.status, event.content);
        break;
      case "turn_ended": {
        const turn = this.turns.get(session);
        this.turns.delete(session);
        if (!turn || turn.files.size === 0) break;
        // Not inside the append that is still notifying listeners: the card is an append too.
        const files = new Map(turn.files);
        queueMicrotask(() => {
          this.review(session, files).catch((err: unknown) => {
            this.options.log.warn(`${session}: edit review failed: ${String(err)}`);
          });
        });
        break;
      }
      default:
        break;
    }
  }

  /**
   * Counts a finished edit call's diffs, when it was applied without asking: in edits mode for
   * the card after the turn, in review mode for a card of its own.
   */
  private track(
    session: string,
    id: string,
    status: string | undefined,
    content: ToolCallContent[] | undefined,
    agent?: string,
  ): void {
    const turn = this.turn(session);
    let call = turn.calls.get(id);
    if (!call) {
      call = { diffs: [], counted: false };
      turn.calls.set(id, call);
    }
    if (agent !== undefined) call.agent = agent;
    if (status !== undefined) call.status = status;
    const diffs = (content ?? []).filter((c): c is DiffContent => c.type === "diff");
    if (diffs.length > 0) call.diffs = diffs;
    if (call.counted || call.status !== "completed" || call.diffs.length === 0) return;
    call.counted = true;
    if (turn.asked.has(id)) return;
    const mode = this.options.approvals(session);
    if (mode === "review") {
      const files = fold(call.diffs);
      const agent = call.agent ?? guestOf(id);
      const diffs = call.diffs;
      queueMicrotask(() => {
        this.reviewEdit(session, agent, diffs, files).catch((err: unknown) => {
          this.options.log.warn(`${session}: edit review failed: ${String(err)}`);
        });
      });
    } else if (mode === "edits") {
      for (const [path, edit] of fold(call.diffs, turn.files)) turn.files.set(path, edit);
    }
  }

  /** Review mode: the edit is on disk; Keep, or Revert it (and tell the agent). */
  private async reviewEdit(
    session: string,
    agent: string,
    diffs: DiffContent[],
    files: Map<string, FileEdit>,
  ): Promise<void> {
    const names = [...files.keys()].map((path) => this.display(session, path));
    const picked = await this.options.asker.ask(session, {
      type: "permission_request",
      agent,
      title: editedTitle(names),
      kind: "edit",
      content: diffs,
      options: KEEP_OR_REVERT,
    });
    if (picked === "revert") await this.revert(session, files, [...files.keys()]);
  }

  private display(session: string, path: string): string {
    const cwd = this.options.store.getSession(session)?.cwd;
    if (cwd === undefined) return path;
    const rel = relative(cwd, path);
    return rel === "" || rel.startsWith("..") || isAbsolute(rel) ? path : rel;
  }

  /** Shows the card; per-file reverts come back with the files that are left. */
  private async review(session: string, files: Map<string, FileEdit>): Promise<void> {
    while (files.size > 0) {
      const paths = [...files.keys()];
      const shown = paths.map((p) => this.display(session, p));
      const noun = paths.length === 1 ? "file" : "files";
      let list = shown.slice(0, TITLE_FILES).join(", ");
      if (shown.length > TITLE_FILES) list += ` and ${shown.length - TITLE_FILES} more`;
      const options = [
        { id: "keep", name: "Keep" },
        { id: "revert", name: "Revert all" },
      ];
      if (paths.length > 1 && paths.length <= MAX_FILE_OPTIONS) {
        paths.forEach((path, i) => {
          options.push({ id: `revert:${path}`, name: `Revert ${shown[i] ?? path}` });
        });
      }
      const picked = await this.options.asker.ask(session, {
        type: "choice",
        title: `Edited ${paths.length} ${noun}: ${list}`,
        options,
      });
      if (picked === undefined || picked === "keep") return;
      if (picked === "revert") {
        await this.revert(session, files, paths);
        return;
      }
      const path = picked.slice("revert:".length);
      await this.revert(session, files, [path]);
      files.delete(path);
    }
  }

  /** Puts the files back and appends edits_reverted with what was and wasn't restored. */
  private async revert(
    session: string,
    files: Map<string, FileEdit>,
    paths: string[],
  ): Promise<void> {
    const reverted: string[] = [];
    const skipped: string[] = [];
    for (const path of paths) {
      const edit = files.get(path);
      if (!edit) continue;
      const name = this.display(session, path);
      if (await restore(path, edit)) {
        reverted.push(name);
        this.options.fileChanged(path, session);
      } else {
        skipped.push(name);
      }
    }
    const event: SessionEvent = { type: "edits_reverted", paths: reverted };
    if (skipped.length > 0) event.skipped = skipped;
    this.options.store.append(session, event);
  }
}

/**
 * Restores a file to its content before the turn, but only if it still has the turn's last
 * content. A file the turn created is deleted. Returns whether it was put back.
 */
export async function restore(path: string, edit: FileEdit): Promise<boolean> {
  let current: string;
  try {
    current = await readFile(path, "utf8");
  } catch {
    return false;
  }
  if (normalize(current) !== normalize(edit.after)) return false;
  if (edit.before === undefined) {
    await rm(path);
    return true;
  }
  // Keep the file's own line endings and BOM (the edit tool hides them from the diffs).
  let text = normalize(edit.before);
  if (current.includes("\r\n")) text = text.replaceAll("\n", "\r\n");
  if (current.startsWith(BOM)) text = BOM + text;
  await writeFile(path, text);
  return true;
}
