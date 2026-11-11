// The tool files of a folder, each in a worker thread of its own, so a tool that hangs or
// crashes costs only its file. A file that fails to load gives no tools and a reason; the others
// load. A worker is started again after a crash or a timeout, with its module state fresh.
import { join, resolve } from "node:path";
import type { ToolResult, ToolSpec } from "@neoworks-dev/neo-sdk";
import { fingerprint } from "./approval.ts";
import type { FromWorker, ToolMeta, ToWorker } from "./spec.ts";

/** What became of one file. */
export interface FileStatus {
  file: string;
  /** The names of the tools it gave. */
  tools: string[];
  /** Why it gave none, or why one of its tools was left out. */
  error?: string;
}

export interface Loaded {
  tools: ToolSpec[];
  files: FileStatus[];
}

/** A call that takes longer than this fails and its worker is stopped. */
export const CALL_TIMEOUT_MS = 60_000;
/** A file that has not said what it gives by now is not loading. */
const LOAD_TIMEOUT_MS = 10_000;

interface Pending {
  resolve: (result: string | ToolResult) => void;
  reject: (err: Error) => void;
  timer: ReturnType<typeof setTimeout>;
}

/** One tool file and the worker that runs it. */
class ToolFile {
  tools: ToolMeta[] = [];
  /** Why the file gave none, or left a tool out. */
  error: string | undefined;
  private worker: Worker | undefined;
  private ready: Promise<void> | undefined;
  private nextId = 0;
  private readonly pending = new Map<number, Pending>();

  constructor(
    readonly path: string,
    /** What the file was when this worker started: a different one is a different file. */
    readonly hash: string,
    private readonly timeoutMs: number,
  ) {}

  /** Starts a worker and waits until it says what the file gives. */
  start(): Promise<void> {
    if (this.ready) return this.ready;
    const worker = new Worker(new URL("./worker.ts", import.meta.url));
    this.worker = worker;
    const ready = new Promise<void>((done, fail) => {
      const timer = setTimeout(() => {
        this.stop(`took longer than ${LOAD_TIMEOUT_MS / 1000}s to load`);
        fail(new Error(this.error));
      }, LOAD_TIMEOUT_MS);
      worker.onmessage = (event: MessageEvent<FromWorker>): void => {
        const message = event.data;
        switch (message.type) {
          case "ready":
            clearTimeout(timer);
            this.tools = message.tools;
            this.error = message.problems.length > 0 ? message.problems.join("; ") : undefined;
            done();
            return;
          case "failed-load":
            clearTimeout(timer);
            this.tools = [];
            this.error = message.message;
            // Nothing to keep running.
            this.stop(message.message);
            fail(new Error(message.message));
            return;
          case "result":
          case "failed":
            this.settle(message);
        }
      };
      worker.onerror = (event: ErrorEvent): void => {
        clearTimeout(timer);
        const why = `crashed: ${event.message}`;
        if (this.worker === worker) this.stop(why);
        fail(new Error(why));
      };
      worker.addEventListener("close", () => {
        clearTimeout(timer);
        if (this.worker === worker) this.stop("its worker ended");
        fail(new Error("its worker ended"));
      });
    });
    // A start that failed is asked again by the next call, not remembered.
    ready.catch(() => undefined);
    this.ready = ready;
    this.post({ type: "load", path: this.path });
    return ready;
  }

  private post(message: ToWorker): void {
    this.worker?.postMessage(message);
  }

  private settle(message: Extract<FromWorker, { type: "result" | "failed" }>): void {
    const call = this.pending.get(message.id);
    if (!call) return;
    this.pending.delete(message.id);
    clearTimeout(call.timer);
    if (message.type === "result") call.resolve(message.result as string | ToolResult);
    else call.reject(new Error(message.message));
  }

  /** Runs one of the file's tools, starting its worker again first if it ended. */
  async call(name: string, args: Record<string, unknown>): Promise<string | ToolResult> {
    await this.start();
    const id = ++this.nextId;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.stop(`${name} took longer than ${this.timeoutMs / 1000}s`);
      }, this.timeoutMs);
      this.pending.set(id, { resolve, reject, timer });
      this.post({ type: "call", id, name, args });
    });
  }

  /** Ends the worker; every call it still had fails with `why`. The next call starts a new one. */
  stop(why: string): void {
    const worker = this.worker;
    this.worker = undefined;
    this.ready = undefined;
    worker?.terminate();
    for (const [id, call] of this.pending) {
      clearTimeout(call.timer);
      call.reject(new Error(`${why}; the tool's worker was stopped`));
      this.pending.delete(id);
    }
  }
}

/** The tool files of one folder. */
export class ToolFiles {
  private readonly files = new Map<string, ToolFile>();

  constructor(
    private readonly dir: string,
    /** Names the service gives its own tools: a file may not use them. */
    private readonly reserved: ReadonlySet<string> = new Set(),
    private readonly timeoutMs = CALL_TIMEOUT_MS,
    /**
     * Which files the user allowed, by name and the hash they had (see approval.ts), read at each
     * reload. Without it every file runs; with it, only the files that are in it as they are now.
     */
    private readonly approved?: () => Record<string, string>,
  ) {}

  /**
   * Reads the folder again: a file that is new or changed is loaded in a new worker, one that is
   * the same keeps its worker (and the state it holds), one that is gone, or not allowed as it is
   * now, loses it.
   */
  async reload(): Promise<Loaded> {
    const prints = fingerprint(this.dir);
    const allowed = this.approved?.();
    const names = Object.keys(prints);
    const running = names.filter((name) => allowed === undefined || allowed[name] === prints[name]);
    const wanted = new Set(running);
    for (const [name, file] of this.files) {
      if (wanted.has(name)) continue;
      file.stop("the file is gone, or not allowed as it is");
      this.files.delete(name);
    }
    await Promise.all(running.map((name) => this.refresh(name, prints[name] ?? "")));

    const out: Loaded = { tools: [], files: [] };
    const taken = new Set(this.reserved);
    for (const name of names) {
      const file = this.files.get(name);
      if (!file) {
        out.files.push({
          file: name,
          tools: [],
          error:
            allowed?.[name] === undefined
              ? "not allowed yet: the user has not approved this file"
              : "not allowed as it is now: it changed since the user approved it",
        });
        continue;
      }
      const status: FileStatus = { file: name, tools: [] };
      const problems: string[] = [];
      if (file.error !== undefined) problems.push(file.error);
      for (const meta of file.tools) {
        if (taken.has(meta.name)) {
          problems.push(`${meta.name}: the name is taken`);
          continue;
        }
        taken.add(meta.name);
        status.tools.push(meta.name);
        out.tools.push({
          ...meta,
          handler: (args) => file.call(meta.name, args),
        });
      }
      if (problems.length > 0) status.error = problems.join("; ");
      out.files.push(status);
    }
    return out;
  }

  private async refresh(name: string, hash: string): Promise<void> {
    const path = resolve(join(this.dir, name));
    const before = this.files.get(name);
    if (before && before.hash === hash) return;
    before?.stop("the file changed");
    const file = new ToolFile(path, hash, this.timeoutMs);
    this.files.set(name, file);
    await file.start().catch(() => undefined);
  }

  /** Ends every worker. */
  close(): void {
    for (const file of this.files.values()) file.stop("the service is stopping");
    this.files.clear();
  }
}
