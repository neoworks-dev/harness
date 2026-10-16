import { spawn } from "node:child_process";
import { createWriteStream, existsSync, type WriteStream } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { AgentTool } from "@mariozechner/pi-agent-core";
import { Type } from "typebox";
import { MAX_BYTES, MAX_LINES, truncateTail } from "./truncate.ts";
import type { RequestPermission } from "../agent.ts";
import { type AssetResolver, resolveAssetsInCommand } from "./paths.ts";
import { askToProceed } from "./permission.ts";

const UPDATE_INTERVAL_MS = 200;
// How long to wait for output pipes to close after the shell exits (a background process may
// keep them open forever).
const PIPE_GRACE_MS = 100;

const parameters = Type.Object({
  command: Type.String({ description: "The bash command to run." }),
  timeout: Type.Optional(
    Type.Number({
      description: "Kill the command after this many seconds. Default: no timeout.",
    }),
  ),
});

export interface BashDetails {
  exitCode?: number;
  /** Set when the output was truncated; holds the complete output. */
  fullOutputPath?: string;
}

/** Collects combined output. Only the tail stays in memory; large output also goes to a file. */
class Output {
  tail = "";
  fullOutputPath: string | undefined;
  private file: WriteStream | undefined;
  private bytes = 0;
  private newlines = 0;
  private endsWithNewline = true;

  push(chunk: string): void {
    this.bytes += Buffer.byteLength(chunk);
    for (const char of chunk) if (char === "\n") this.newlines++;
    this.endsWithNewline = chunk.endsWith("\n");
    const hadFile = this.file !== undefined;
    this.tail += chunk;
    if (!hadFile && (this.bytes > MAX_BYTES || this.newlines > MAX_LINES)) {
      this.fullOutputPath = join(tmpdir(), `suite-bash-${crypto.randomUUID()}.log`);
      this.file = createWriteStream(this.fullOutputPath);
      this.file.write(this.tail);
    } else {
      this.file?.write(chunk);
    }
    if (this.tail.length > 4 * MAX_BYTES) this.tail = this.tail.slice(-2 * MAX_BYTES);
  }

  get totalLines(): number {
    return this.newlines + (this.endsWithNewline ? 0 : 1);
  }

  async close(): Promise<void> {
    const file = this.file;
    if (file) await new Promise<void>((done) => file.end(done));
  }

  /** What the model sees: the tail, plus a note if the rest was cut. */
  render(): string {
    const tail = truncateTail(this.tail);
    let text = tail.text.replace(/\n+$/, "") || "(no output)";
    if (this.fullOutputPath) {
      const note = `showing the last ${tail.shownLines} of ${this.totalLines} lines`;
      text += `\n\n[Output truncated: ${note}. Full output: ${this.fullOutputPath}]`;
    }
    return text;
  }
}

function killGroup(pid: number | undefined): void {
  if (pid === undefined) return;
  try {
    process.kill(-pid, "SIGKILL");
  } catch {
    // The group is already gone.
  }
}

export function createBashTool(
  cwd: string,
  assets?: AssetResolver,
  ask?: RequestPermission,
): AgentTool<typeof parameters, BashDetails> {
  return {
    name: "bash",
    label: "bash",
    description:
      "Run a shell command with bash in the session's working directory. Returns stdout and " +
      "stderr combined. Output is cut to the last 2000 lines or 50KB; when that happens the " +
      "full output is saved to a temp file and its path is given. A non-zero exit code is " +
      "reported as an error together with the output. Commands cannot read from stdin, so " +
      "don't run interactive programs.",
    parameters,
    async execute(toolCallId, { command, timeout }, signal, onUpdate) {
      if (signal?.aborted) throw new Error("Command aborted before it started.");
      await askToProceed(
        ask,
        { title: command, kind: "execute", toolCallId },
        "The user declined to run this command.",
        signal,
      );
      const shell = existsSync("/bin/bash") ? "/bin/bash" : "sh";
      const child = spawn(shell, ["-c", resolveAssetsInCommand(command, assets)], {
        cwd,
        detached: true, // own process group, so abort and timeout can kill the whole tree
        stdio: ["ignore", "pipe", "pipe"],
      });

      const output = new Output();
      let lastUpdate = 0;
      let updateTimer: ReturnType<typeof setTimeout> | undefined;
      const sendUpdate = (): void => {
        updateTimer = undefined;
        lastUpdate = Date.now();
        const text = truncateTail(output.tail).text;
        onUpdate?.({ content: [{ type: "text", text }], details: {} });
      };
      const onData = (chunk: string): void => {
        output.push(chunk);
        if (updateTimer) return;
        const wait = lastUpdate + UPDATE_INTERVAL_MS - Date.now();
        if (wait <= 0) sendUpdate();
        else updateTimer = setTimeout(sendUpdate, wait);
      };
      child.stdout.setEncoding("utf8");
      child.stderr.setEncoding("utf8");
      child.stdout.on("data", onData);
      child.stderr.on("data", onData);

      let aborted = false;
      let timedOut = false;
      const onAbort = (): void => {
        aborted = true;
        killGroup(child.pid);
      };
      signal?.addEventListener("abort", onAbort, { once: true });
      const timeoutTimer = timeout
        ? setTimeout(() => {
            timedOut = true;
            killGroup(child.pid);
          }, timeout * 1000)
        : undefined;

      let exitCode: number | null;
      let exitSignal: NodeJS.Signals | null;
      try {
        [exitCode, exitSignal] = await new Promise<[number | null, NodeJS.Signals | null]>(
          (resolve, reject) => {
            child.on("error", reject);
            child.on("exit", (code, sig) => {
              setTimeout(() => {
                child.stdout.destroy();
                child.stderr.destroy();
                resolve([code, sig]);
              }, PIPE_GRACE_MS);
              child.on("close", () => resolve([code, sig]));
            });
          },
        );
      } finally {
        clearTimeout(timeoutTimer);
        clearTimeout(updateTimer);
        signal?.removeEventListener("abort", onAbort);
        await output.close();
      }

      const text = output.render();
      if (aborted) throw new Error(`${text}\n\nCommand aborted.`);
      if (timedOut) throw new Error(`${text}\n\nCommand timed out after ${timeout} seconds.`);
      if (exitCode !== 0) {
        const how =
          exitCode === null ? `was killed by ${exitSignal}` : `exited with code ${exitCode}`;
        throw new Error(`${text}\n\nCommand ${how}.`);
      }
      return {
        content: [{ type: "text", text }],
        details: { exitCode, fullOutputPath: output.fullOutputPath },
      };
    },
  };
}
