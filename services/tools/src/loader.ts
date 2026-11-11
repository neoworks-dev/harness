// The tool files of a folder, each in a worker thread of its own, so a tool that hangs or
// crashes costs only its file. A file that fails to load gives no tools and a reason; the others
// load. A call that takes too long fails, and the worker goes on with what it holds (a watcher,
// a child process): only a worker that stops answering pings (a busy loop) or crashes is ended,
// and started again with its module state fresh.
import { join, resolve } from "node:path";
import type { ToolResult, ToolSpec } from "@neoworks-dev/neo-sdk";
import { fingerprint, manifest } from "./approval.ts";
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

/** A call that takes longer than this fails; its worker goes on. */
export const CALL_TIMEOUT_MS = 60_000;
/** A worker that has not answered a ping for this long is stuck, and is ended. */
export const STUCK_MS = 10_000;
/** A file that has not said what it gives by now is not loading. */
const LOAD_TIMEOUT_MS = 10_000;

export interface ToolFilesOptions {
  /** Names the service gives its own tools: a file may not use them. */
  reserved?: ReadonlySet<string>;
  timeoutMs?: number;
  stuckMs?: number;
  /**
   * Which files the user allowed, by name and the hash they had (see approval.ts), read at each
   * reload. Without it every file runs; with it, only the files that are in it as they are now.
   */
  approved?: () => Record<string, string>;
}

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
  private heartbeat: ReturnType<typeof setInterval> | undefined;
  private lastPong = 0;
  private startedAt = 0;
  private closed = false;

  constructor(
    readonly path: string,
    private readonly root: string,
    /** What the file was when this worker started: a different one is a different file. */
    readonly hash: string,
    /** The hash of every file of the folder: what the worker may load. */
    private readonly files: Record<string, string>,
    private readonly timeoutMs: number,
    private readonly stuckMs: number,
  ) {}

  /** Starts a worker and waits until it says what the file gives. */
  start(): Promise<void> {
    if (this.ready) return this.ready;
    const worker = new Worker(new URL("./worker.ts", import.meta.url));
    this.worker = worker;
    this.startedAt = Date.now();
    const ready = new Promise<void>((done, fail) => {
      const timer = setTimeout(() => {
        this.stop(`took longer than ${LOAD_TIMEOUT_MS / 1000}s to load`);
        fail(new Error(this.error));
      }, LOAD_TIMEOUT_MS);
      worker.onmessage = (event: MessageEvent<FromWorker>): void => {
        const message = event.data;
        switch (message.type) {
          case "pong":
            this.lastPong = Date.now();
            return;
          case "ready":
            clearTimeout(timer);
            this.tools = message.tools;
            this.error = message.problems.length > 0 ? message.problems.join("; ") : undefined;
            this.watch(worker);
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
        if (this.worker === worker) this.revive(why);
        fail(new Error(why));
      };
      worker.addEventListener("close", () => {
        clearTimeout(timer);
        if (this.worker === worker) this.revive("its worker ended");
        fail(new Error("its worker ended"));
      });
    });
    // A start that failed is asked again by the next call, not remembered.
    ready.catch(() => undefined);
    this.ready = ready;
    this.post({ type: "load", path: this.path, root: this.root, files: this.files });
    return ready;
  }

  /** Pings the loaded worker; one that stops answering is stuck in a loop and is started over. */
  private watch(worker: Worker): void {
    clearInterval(this.heartbeat);
    this.lastPong = Date.now();
    const every = Math.max(10, Math.floor(this.stuckMs / 4));
    this.heartbeat = setInterval(() => {
      if (this.worker !== worker) return;
      if (Date.now() - this.lastPong > this.stuckMs) {
        this.revive(`stopped answering for ${this.stuckMs / 1000}s (a busy loop?)`);
      } else {
        this.post({ type: "ping" });
      }
    }, every);
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
      // The call fails; the worker, and what it holds, goes on. A late answer is dropped.
      const timer = setTimeout(() => {
        this.pending.delete(id);
        reject(new Error(`${name} took longer than ${this.timeoutMs / 1000}s`));
      }, this.timeoutMs);
      this.pending.set(id, { resolve, reject, timer });
      this.post({ type: "call", id, name, args });
    });
  }

  /**
   * Ends a worker that crashed or got stuck and, unless it did so right after it started (it
   * would only do it again), starts a new one at once, so a file that watches something goes on.
   */
  private revive(why: string): void {
    const quick = Date.now() - this.startedAt < this.stuckMs;
    this.stop(why);
    if (!this.closed && !quick) void this.start().catch(() => undefined);
  }

  /** Ends the worker; every call it still had fails with `why`. The next call starts a new one. */
  stop(why: string): void {
    const worker = this.worker;
    this.worker = undefined;
    this.ready = undefined;
    clearInterval(this.heartbeat);
    worker?.terminate();
    for (const [id, call] of this.pending) {
      clearTimeout(call.timer);
      call.reject(new Error(`${why}; the tool's worker was stopped`));
      this.pending.delete(id);
    }
  }

  /** Ends the worker for good: the file is gone, changed or the service stops. */
  close(why: string): void {
    this.closed = true;
    this.stop(why);
  }
}

/** The tool files of one folder. */
export class ToolFiles {
  private readonly files = new Map<string, ToolFile>();

  constructor(
    private readonly dir: string,
    private readonly options: ToolFilesOptions = {},
  ) {}

  /**
   * Reads the folder again: a file that is new or changed is loaded in a new worker, one that is
   * the same keeps its worker (and the state it holds), one that is gone, or not allowed as it is
   * now, loses it.
   */
  async reload(): Promise<Loaded> {
    const files = manifest(this.dir);
    const prints = fingerprint(this.dir, files);
    const allowed = this.options.approved?.();
    const names = Object.keys(prints);
    const running = names.filter((name) => allowed === undefined || allowed[name] === prints[name]);
    const wanted = new Set(running);
    for (const [name, file] of this.files) {
      if (wanted.has(name)) continue;
      file.close("the file is gone, or not allowed as it is");
      this.files.delete(name);
    }
    await Promise.all(running.map((name) => this.refresh(name, prints[name] ?? "", files)));

    const out: Loaded = { tools: [], files: [] };
    const taken = new Set(this.options.reserved);
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

  private async refresh(name: string, hash: string, files: Record<string, string>): Promise<void> {
    const path = resolve(join(this.dir, name));
    const before = this.files.get(name);
    if (before && before.hash === hash) return;
    before?.close("the file changed");
    const file = new ToolFile(
      path,
      resolve(this.dir),
      hash,
      files,
      this.options.timeoutMs ?? CALL_TIMEOUT_MS,
      this.options.stuckMs ?? STUCK_MS,
    );
    this.files.set(name, file);
    await file.start().catch(() => undefined);
  }

  /** Ends every worker. */
  close(): void {
    for (const file of this.files.values()) file.close("the service is stopping");
    this.files.clear();
  }
}
