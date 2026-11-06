import { spawn } from "node:child_process";
import {
  appendFileSync,
  closeSync,
  createWriteStream,
  existsSync,
  fstatSync,
  openSync,
  readSync,
  rmSync,
  type WriteStream,
} from "node:fs";
import { constants, tmpdir } from "node:os";
import { join } from "node:path";
import { StringDecoder } from "node:string_decoder";
import type { AgentTool } from "../loop/index.ts";
import { Type } from "typebox";
import { MAX_BYTES, MAX_LINES, truncateTail } from "./truncate.ts";
import type { RequestPermission } from "../agent.ts";
import { type AssetResolver, resolveAssetsInCommand } from "./paths.ts";
import { askToProceed, withNote } from "./permission.ts";

const UPDATE_INTERVAL_MS = 200;
/** How often the output file is read while the command runs. */
const READ_INTERVAL_MS = 50;

const parameters = Type.Object({
  command: Type.String({ description: "The bash command to run." }),
  description: Type.String({
    description:
      'What the command does, in plain words, 5–10 of them, active voice: "List the files this ' +
      'branch changed", "Run the daemon\'s tests". The user reads this, not the command, to decide ' +
      "whether to run it, so say what it does and what it touches, not how.",
  }),
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
      this.fullOutputPath = join(tmpdir(), `neo-bash-${crypto.randomUUID()}.log`);
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

/**
 * The bash calls running in a session that the user can send to the background (Ctrl-B in
 * term): the call ends at once with where the output goes, and the command runs on.
 */
export class BackgroundCalls {
  private readonly calls = new Map<string, () => void>();

  /** `ended`: told how each command sent to the background ended, for the agent. */
  constructor(readonly ended?: (text: string) => void) {}

  /** While a call runs; the returned function forgets it. */
  add(toolCallId: string, move: () => void): () => void {
    this.calls.set(toolCallId, move);
    return () => {
      if (this.calls.get(toolCallId) === move) this.calls.delete(toolCallId);
    };
  }

  /** Sends the call to the background; false when no such call runs. */
  move(toolCallId: string): boolean {
    const move = this.calls.get(toolCallId);
    if (!move) return false;
    this.calls.delete(toolCallId);
    move();
    return true;
  }
}

/**
 * Reads what the command wrote to its output file, from where it last stopped. The command
 * writes the file itself, so the processes behind a background command can be found by it.
 */
class Tail {
  private offset = 0;
  private readonly decoder = new StringDecoder("utf8");

  constructor(private readonly fd: number) {}

  read(): string {
    const buffer = Buffer.alloc(64 * 1024);
    let text = "";
    for (;;) {
      const n = readSync(this.fd, buffer, 0, buffer.length, this.offset);
      if (n === 0) return text;
      this.offset += n;
      text += this.decoder.write(buffer.subarray(0, n));
    }
  }
}

/** The last bytes of a file, as text (the whole of a small one); "" when it can't be read. */
function fileEnd(path: string, bytes = 4 * MAX_BYTES): string {
  try {
    const fd = openSync(path, "r");
    try {
      const size = fstatSync(fd).size;
      const buffer = Buffer.alloc(Math.min(size, bytes));
      readSync(fd, buffer, 0, buffer.length, size - buffer.length);
      return buffer.toString("utf8");
    } finally {
      closeSync(fd);
    }
  } catch {
    return "";
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
  background?: BackgroundCalls,
): AgentTool<typeof parameters, BashDetails> {
  return {
    name: "bash",
    label: "bash",
    description:
      "Run a shell command with bash in the session's working directory (no need to cd into " +
      "it). asset:// handles other tools returned work as paths of the stored file " +
      "(cp asset://img-7f3 ~/site/logo.png). Returns stdout and " +
      "stderr combined. Output is cut to the last 2000 lines or 50KB; when that happens the " +
      "full output is saved to a temp file and its path is given. A non-zero exit code is " +
      "reported as an error together with the output. Commands cannot read from stdin, so " +
      "don't run interactive programs.",
    parameters,
    async execute(toolCallId, { command, description, timeout }, signal, onUpdate) {
      if (signal?.aborted) throw new Error("Command aborted before it started.");
      // The user decides by the description; the command is under it.
      const { note } = await askToProceed(
        ask,
        {
          title: description.trim() || command,
          kind: "execute",
          toolCallId,
          content: [{ type: "text", text: command }],
        },
        "The user declined to run this command.",
        signal,
      );
      const shell = existsSync("/bin/bash") ? "/bin/bash" : "sh";
      // The command writes its output to a file, read here as it grows: sent to the background
      // it goes on writing there, and that is how the processes behind it are found to stop
      // them (term's background commands).
      const outputPath = join(tmpdir(), `neo-bash-${crypto.randomUUID()}.output`);
      const writeFd = openSync(outputPath, "w");
      let child: ReturnType<typeof spawn>;
      try {
        child = spawn(shell, ["-c", resolveAssetsInCommand(command, assets)], {
          cwd,
          detached: true, // own process group, so abort and timeout can kill the whole tree
          stdio: ["ignore", writeFd, writeFd],
        });
      } finally {
        closeSync(writeFd);
      }
      const readFd = openSync(outputPath, "r");
      const tail = new Tail(readFd);

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
      const pump = (): void => {
        const text = tail.read();
        if (text !== "") onData(text);
      };
      const reader = setInterval(pump, READ_INTERVAL_MS);

      let moveToBackground = (): void => undefined;
      const backgrounded = new Promise<"background">((resolve) => {
        moveToBackground = () => resolve("background");
      });
      const forget = background?.add(toolCallId, moveToBackground);

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

      const exited = new Promise<[number | null, NodeJS.Signals | null]>((resolve, reject) => {
        child.on("error", reject);
        child.on("exit", (code, sig) => resolve([code, sig]));
      });
      let outcome: [number | null, NodeJS.Signals | null] | "background";
      try {
        outcome = await Promise.race([exited, backgrounded]);
      } finally {
        forget?.();
        clearInterval(reader);
        clearTimeout(updateTimer);
        // Sent to the background it is the user's now: a cancelled turn doesn't stop it.
        signal?.removeEventListener("abort", onAbort);
        pump();
        closeSync(readFd);
        await output.close();
      }

      if (outcome === "background") {
        clearTimeout(timeoutTimer);
        // The file ends with how the command ended, as Claude Code's background commands do,
        // and the agent hears of it with the end of the output.
        const end = (code: number): void => {
          const last = truncateTail(fileEnd(outputPath)).text.replace(/\n+$/, "") || "(no output)";
          appendFileSync(outputPath, `\n[exited with code ${code}]\n`);
          background?.ended?.(
            `The command "${description.trim() || command}" you ran, which the user sent to the background, exited with code ${code}. The end of its output:\n${last}\n\nAll of it: ${outputPath}`,
          );
        };
        exited.then(
          ([code, sig]) => end(code ?? 128 + (sig ? (constants.signals[sig] ?? 0) : 0)),
          () => end(1),
        );
        const so = `Command running in background (the user moved it there). Output is being written to: ${outputPath}\n\nSo far:\n${output.render()}`;
        return { content: [{ type: "text", text: withNote(so, note) }], details: {} };
      }
      clearTimeout(timeoutTimer);
      rmSync(outputPath, { force: true });
      const [exitCode, exitSignal] = outcome;

      const text = output.render();
      if (aborted) throw new Error(`${text}\n\nCommand aborted.`);
      if (timedOut) throw new Error(`${text}\n\nCommand timed out after ${timeout} seconds.`);
      if (exitCode !== 0) {
        const how =
          exitCode === null ? `was killed by ${exitSignal}` : `exited with code ${exitCode}`;
        throw new Error(withNote(`${text}\n\nCommand ${how}.`, note));
      }
      return {
        content: [{ type: "text", text: withNote(text, note) }],
        details: { exitCode, fullOutputPath: output.fullOutputPath },
      };
    },
  };
}
