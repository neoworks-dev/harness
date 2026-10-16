// Review after a turn in edits mode (design §9, approvals): edits are written right away, so
// afterwards one card offers to keep them or put the files back. The card is a choice (Keep,
// Revert all, Revert <file> when there are several files); reverting restores each file to what
// it was before the turn, unless it changed again since, and tells the agent (edits_reverted).
import { readFile, rm, writeFile } from "node:fs/promises";
import { isAbsolute, relative } from "node:path";
import type {
  DiffContent,
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

/** One file's edits over a turn. */
interface FileEdit {
  /** The content before the turn's first edit; absent: the turn created the file. */
  before?: string;
  /** The content after its last edit. */
  after: string;
}

interface Call {
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
        this.track(session, event.toolCallId, event.status, undefined);
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

  /** Counts a finished edit call's diffs, when it was applied without asking in edits mode. */
  private track(
    session: string,
    id: string,
    status: string | undefined,
    content: ToolCallContent[] | undefined,
  ): void {
    const turn = this.turn(session);
    let call = turn.calls.get(id);
    if (!call) {
      call = { diffs: [], counted: false };
      turn.calls.set(id, call);
    }
    if (status !== undefined) call.status = status;
    const diffs = (content ?? []).filter((c): c is DiffContent => c.type === "diff");
    if (diffs.length > 0) call.diffs = diffs;
    if (call.counted || call.status !== "completed" || call.diffs.length === 0) return;
    call.counted = true;
    if (turn.asked.has(id) || this.options.approvals(session) !== "edits") return;
    for (const diff of call.diffs) {
      const known = turn.files.get(diff.path);
      if (known) known.after = diff.newText;
      else turn.files.set(diff.path, { before: diff.oldText, after: diff.newText });
    }
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
async function restore(path: string, edit: FileEdit): Promise<boolean> {
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
