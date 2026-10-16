// JSON-RPC 2.0 over a language server's stdio, with LSP's Content-Length framing.
import type { Subprocess } from "bun";

type Handler = (params: unknown) => void;

interface Pending {
  resolve: (value: unknown) => void;
  reject: (err: Error) => void;
  timer: ReturnType<typeof setTimeout>;
}

export class RpcError extends Error {
  constructor(
    message: string,
    readonly code: number,
  ) {
    super(message);
  }
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** Answers for the requests servers send to clients; anything else gets null. */
function answerServerRequest(method: string, params: unknown): unknown {
  if (method === "workspace/configuration" && isRecord(params) && Array.isArray(params.items)) {
    return params.items.map(() => null);
  }
  return null;
}

export class RpcConnection {
  private nextId = 1;
  private readonly pending = new Map<number, Pending>();
  private readonly handlers = new Map<string, Handler[]>();
  private buffer = new Uint8Array(0);
  private closed = false;
  readonly exited: Promise<number>;

  constructor(
    private readonly proc: Subprocess<"pipe", "pipe", "pipe">,
    private readonly onLog: (line: string) => void = () => {},
  ) {
    this.exited = proc.exited.then((code) => {
      this.closed = true;
      for (const p of this.pending.values()) {
        clearTimeout(p.timer);
        p.reject(new Error(`language server exited (${code})`));
      }
      this.pending.clear();
      return code;
    });
    void this.read();
    void this.drainStderr();
  }

  request(method: string, params: unknown, timeoutMs = 10_000): Promise<unknown> {
    if (this.closed) return Promise.reject(new Error("language server is not running"));
    const id = this.nextId++;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id);
        reject(new Error(`${method} timed out after ${timeoutMs} ms`));
      }, timeoutMs);
      this.pending.set(id, { resolve, reject, timer });
      this.write({ jsonrpc: "2.0", id, method, params });
    });
  }

  notify(method: string, params: unknown): void {
    if (!this.closed) this.write({ jsonrpc: "2.0", method, params });
  }

  on(method: string, handler: Handler): void {
    const list = this.handlers.get(method) ?? [];
    list.push(handler);
    this.handlers.set(method, list);
  }

  kill(): void {
    this.proc.kill();
  }

  private write(message: unknown): void {
    const body = Buffer.from(JSON.stringify(message), "utf8");
    void this.proc.stdin.write(`Content-Length: ${body.length}\r\n\r\n`);
    void this.proc.stdin.write(body);
    void this.proc.stdin.flush();
  }

  private async read(): Promise<void> {
    for await (const chunk of this.proc.stdout) {
      const merged = new Uint8Array(this.buffer.length + chunk.length);
      merged.set(this.buffer);
      merged.set(chunk, this.buffer.length);
      this.buffer = merged;
      this.drainFrames();
    }
  }

  private drainFrames(): void {
    for (;;) {
      const headerEnd = Buffer.from(this.buffer).indexOf("\r\n\r\n");
      if (headerEnd === -1) return;
      const header = Buffer.from(this.buffer.subarray(0, headerEnd)).toString("ascii");
      const match = /Content-Length:\s*(\d+)/i.exec(header);
      if (!match) {
        this.buffer = this.buffer.subarray(headerEnd + 4);
        continue;
      }
      const length = Number(match[1]);
      const start = headerEnd + 4;
      if (this.buffer.length < start + length) return;
      const body = Buffer.from(this.buffer.subarray(start, start + length)).toString("utf8");
      this.buffer = this.buffer.subarray(start + length);
      try {
        this.dispatch(JSON.parse(body));
      } catch (err) {
        this.onLog(`bad frame from language server: ${String(err)}`);
      }
    }
  }

  private dispatch(message: unknown): void {
    if (!isRecord(message)) return;
    const { id, method } = message;
    if (typeof method === "string") {
      if (id !== undefined) {
        this.write({ jsonrpc: "2.0", id, result: answerServerRequest(method, message.params) });
        return;
      }
      for (const handler of this.handlers.get(method) ?? []) handler(message.params);
      return;
    }
    if (typeof id !== "number") return;
    const pending = this.pending.get(id);
    if (!pending) return;
    this.pending.delete(id);
    clearTimeout(pending.timer);
    const { error } = message;
    if (isRecord(error)) {
      const code = typeof error.code === "number" ? error.code : -1;
      const text = typeof error.message === "string" ? error.message : "error";
      pending.reject(new RpcError(text, code));
    } else {
      pending.resolve(message.result);
    }
  }

  private async drainStderr(): Promise<void> {
    const decoder = new TextDecoder();
    for await (const chunk of this.proc.stderr) {
      for (const line of decoder.decode(chunk).split("\n")) if (line.trim()) this.onLog(line);
    }
  }
}
