// A minimal ACP client: JSON-RPC 2.0 over an agent's stdio, one message per line. Only what the
// bridge needs (initialize, session/new, session/prompt, session/cancel, set_config_option; the
// agent's session/update and session/request_permission).
import type { Subprocess } from "bun";

/** Answers a request the agent sends us; throwing returns a JSON-RPC error. */
export type RequestHandler = (method: string, params: unknown) => Promise<unknown>;
export type NotificationHandler = (method: string, params: unknown) => void;

interface Pending {
  method: string;
  resolve: (value: unknown) => void;
  reject: (err: Error) => void;
}

export class AcpError extends Error {
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

/** How many of the agent's last stderr lines an exit error quotes. */
const STDERR_LINES = 20;

export interface AcpOptions {
  command: string[];
  cwd: string;
  env?: Record<string, string>;
  onRequest: RequestHandler;
  onNotification: NotificationHandler;
  /** The agent's stderr, line by line. */
  onLog?: (line: string) => void;
}

export class AcpConnection {
  private nextId = 1;
  private readonly pending = new Map<number, Pending>();
  private readonly stderr: string[] = [];
  private closed = false;
  readonly exited: Promise<number>;

  private constructor(
    private readonly proc: Subprocess<"pipe", "pipe", "pipe">,
    private readonly options: AcpOptions,
  ) {
    this.exited = proc.exited.then((code) => {
      this.closed = true;
      const tail = this.stderr.map((line) => `\n${line}`).join("");
      for (const p of this.pending.values()) {
        p.reject(new Error(`the agent exited (${code}) during ${p.method}${tail}`));
      }
      this.pending.clear();
      return code;
    });
    void this.read();
    void this.drainStderr();
  }

  static spawn(options: AcpOptions): AcpConnection {
    const proc = Bun.spawn(options.command, {
      cwd: options.cwd,
      env: { ...process.env, ...options.env },
      stdin: "pipe",
      stdout: "pipe",
      stderr: "pipe",
    });
    return new AcpConnection(proc, options);
  }

  get running(): boolean {
    return !this.closed;
  }

  request(method: string, params: unknown): Promise<unknown> {
    if (this.closed) return Promise.reject(new Error("the agent is not running"));
    const id = this.nextId++;
    return new Promise((resolve, reject) => {
      this.pending.set(id, { method, resolve, reject });
      this.write({ jsonrpc: "2.0", id, method, params });
    });
  }

  notify(method: string, params: unknown): void {
    if (!this.closed) this.write({ jsonrpc: "2.0", method, params });
  }

  kill(): void {
    this.proc.kill();
  }

  private write(message: unknown): void {
    void this.proc.stdin.write(`${JSON.stringify(message)}\n`);
    void this.proc.stdin.flush();
  }

  private async read(): Promise<void> {
    const decoder = new TextDecoder();
    let buffer = "";
    for await (const chunk of this.proc.stdout) {
      buffer += decoder.decode(chunk, { stream: true });
      let nl = buffer.indexOf("\n");
      while (nl !== -1) {
        const line = buffer.slice(0, nl).trim();
        buffer = buffer.slice(nl + 1);
        if (line !== "") this.dispatchLine(line);
        nl = buffer.indexOf("\n");
      }
    }
  }

  private async drainStderr(): Promise<void> {
    const decoder = new TextDecoder();
    let buffer = "";
    for await (const chunk of this.proc.stderr) {
      buffer += decoder.decode(chunk, { stream: true });
      const lines = buffer.split("\n");
      buffer = lines.pop() ?? "";
      for (const line of lines) {
        this.stderr.push(line);
        if (this.stderr.length > STDERR_LINES) this.stderr.shift();
        this.options.onLog?.(line);
      }
    }
  }

  private dispatchLine(line: string): void {
    let message: unknown;
    try {
      message = JSON.parse(line);
    } catch {
      this.options.onLog?.(`not JSON-RPC: ${line.slice(0, 200)}`);
      return;
    }
    if (!isRecord(message)) return;
    const { id, method } = message;
    if (typeof method === "string") {
      if (id === undefined) this.options.onNotification(method, message.params);
      else void this.answer(id, method, message.params);
      return;
    }
    if (typeof id !== "number") return;
    const pending = this.pending.get(id);
    if (!pending) return;
    this.pending.delete(id);
    if (isRecord(message.error)) {
      const text = typeof message.error.message === "string" ? message.error.message : "error";
      const code = typeof message.error.code === "number" ? message.error.code : -32603;
      pending.reject(new AcpError(`${pending.method}: ${text}`, code));
    } else {
      pending.resolve(message.result);
    }
  }

  private async answer(id: unknown, method: string, params: unknown): Promise<void> {
    try {
      const result = await this.options.onRequest(method, params);
      this.write({ jsonrpc: "2.0", id, result: result ?? null });
    } catch (err) {
      const code = err instanceof AcpError ? err.code : -32603;
      const message = err instanceof Error ? err.message : String(err);
      this.write({ jsonrpc: "2.0", id, error: { code, message } });
    }
  }
}
