// Language servers by language and project root, started on first use and kept running.
// Documents are synced from disk before every request, so edits by the agent, the editor or
// anything else are seen without file.changed bookkeeping.
import { existsSync, readFileSync } from "node:fs";
import { dirname, extname, join } from "node:path";
import { pathToFileURL } from "node:url";
import { RpcConnection } from "./rpc.ts";

export interface Language {
  id: string;
  /** LSP languageId per file extension. */
  extensions: Record<string, string>;
  command: string[];
  /** Files that mark a project root, nearest first. */
  rootMarkers: string[];
}

export const LANGUAGES: Language[] = [
  {
    id: "go",
    extensions: { ".go": "go" },
    command: ["gopls"],
    rootMarkers: ["go.work", "go.mod"],
  },
  {
    id: "typescript",
    extensions: {
      ".ts": "typescript",
      ".tsx": "typescriptreact",
      ".js": "javascript",
      ".jsx": "javascriptreact",
      ".mts": "typescript",
      ".cts": "typescript",
    },
    command: ["typescript-language-server", "--stdio"],
    rootMarkers: ["tsconfig.json", "package.json"],
  },
  {
    id: "rust",
    extensions: { ".rs": "rust" },
    command: ["rust-analyzer"],
    rootMarkers: ["Cargo.toml"],
  },
];

export function languageOf(path: string): { language: Language; languageId: string } | undefined {
  const ext = extname(path).toLowerCase();
  for (const language of LANGUAGES) {
    const languageId = language.extensions[ext];
    if (languageId !== undefined) return { language, languageId };
  }
  return undefined;
}

/** The outermost directory with a root marker for go.work, else the nearest one. */
export function findRoot(path: string, markers: string[]): string {
  let dir = dirname(path);
  let found: string | undefined;
  for (;;) {
    for (const marker of markers) {
      if (existsSync(join(dir, marker))) {
        // A workspace file wins over the module files below it.
        if (marker === "go.work") return dir;
        found ??= dir;
      }
    }
    const parent = dirname(dir);
    if (parent === dir) return found ?? dirname(path);
    dir = parent;
  }
}

export interface Diagnostic {
  range: Range;
  severity?: number;
  message: string;
  source?: string;
  code?: string | number;
}

export interface Range {
  start: { line: number; character: number };
  end: { line: number; character: number };
}

interface Doc {
  version: number;
  text: string;
}

interface Published {
  version?: number;
  diagnostics: Diagnostic[];
  at: number;
}

export class LanguageServer {
  private readonly docs = new Map<string, Doc>();
  private readonly published = new Map<string, Published>();
  private readonly waiters = new Set<() => void>();
  private ready: Promise<void>;

  constructor(
    readonly language: Language,
    readonly root: string,
    private readonly rpc: RpcConnection,
  ) {
    rpc.on("textDocument/publishDiagnostics", (params) => {
      const p = params as { uri: string; version?: number; diagnostics: Diagnostic[] };
      const entry: Published = { diagnostics: p.diagnostics, at: Date.now() };
      if (typeof p.version === "number") entry.version = p.version;
      this.published.set(p.uri, entry);
      for (const wake of this.waiters) wake();
    });
    this.ready = this.initialize();
  }

  get exited(): Promise<number> {
    return this.rpc.exited;
  }

  private async initialize(): Promise<void> {
    const rootUri = pathToFileURL(this.root).href;
    await this.rpc.request(
      "initialize",
      {
        processId: process.pid,
        rootUri,
        workspaceFolders: [{ uri: rootUri, name: this.root }],
        capabilities: {
          general: { positionEncodings: ["utf-16"] },
          textDocument: {
            synchronization: { didSave: true },
            publishDiagnostics: { versionSupport: true },
            definition: { linkSupport: false },
            references: {},
            hover: { contentFormat: ["markdown", "plaintext"] },
            rename: { prepareSupport: false },
          },
          workspace: { workspaceFolders: true, configuration: true, workspaceEdit: {} },
        },
      },
      60_000,
    );
    this.rpc.notify("initialized", {});
  }

  /** Opens or refreshes a document from disk; returns its uri and whether its text changed. */
  async sync(path: string, languageId: string): Promise<{ uri: string; changed: boolean }> {
    await this.ready;
    const uri = pathToFileURL(path).href;
    const text = readFileSync(path, "utf8");
    const doc = this.docs.get(uri);
    if (!doc) {
      this.docs.set(uri, { version: 1, text });
      this.rpc.notify("textDocument/didOpen", {
        textDocument: { uri, languageId, version: 1, text },
      });
      return { uri, changed: true };
    }
    if (doc.text === text) return { uri, changed: false };
    doc.version += 1;
    doc.text = text;
    this.rpc.notify("textDocument/didChange", {
      textDocument: { uri, version: doc.version },
      contentChanges: [{ text }],
    });
    return { uri, changed: true };
  }

  /**
   * Diagnostics for a synced document. When its text just changed, waits for the server to
   * publish for the new version (or anything newer than `since`), up to `timeoutMs`.
   */
  async diagnostics(uri: string, since: number, timeoutMs: number): Promise<Diagnostic[]> {
    const version = this.docs.get(uri)?.version;
    const fresh = (): boolean => {
      const p = this.published.get(uri);
      if (!p) return false;
      if (p.version !== undefined && version !== undefined) return p.version >= version;
      return p.at >= since;
    };
    if (!fresh()) {
      await new Promise<void>((resolve) => {
        const done = (): void => {
          clearTimeout(timer);
          this.waiters.delete(check);
          resolve();
        };
        const check = (): void => {
          if (fresh()) done();
        };
        const timer = setTimeout(done, timeoutMs);
        this.waiters.add(check);
      });
    }
    return this.published.get(uri)?.diagnostics ?? [];
  }

  text(uri: string): string | undefined {
    return this.docs.get(uri)?.text;
  }

  async request(method: string, params: unknown): Promise<unknown> {
    await this.ready;
    return this.rpc.request(method, params, 30_000);
  }

  async shutdown(): Promise<void> {
    try {
      await this.rpc.request("shutdown", null, 2000);
      this.rpc.notify("exit", null);
    } catch {
      // already gone or stuck: kill below
    }
    this.rpc.kill();
  }
}

export class ServerPool {
  private readonly servers = new Map<string, LanguageServer>();

  constructor(private readonly log: (line: string) => void = () => {}) {}

  /** The server for a file (started on first use), plus the file's languageId. */
  get(path: string): { server: LanguageServer; languageId: string } {
    const lang = languageOf(path);
    if (!lang) throw new Error(`No language server for ${extname(path) || "this file type"}.`);
    const { language, languageId } = lang;
    const root = findRoot(path, language.rootMarkers);
    const key = `${language.id}:${root}`;
    let server = this.servers.get(key);
    if (!server) {
      const [program, ...args] = language.command;
      if (program === undefined || Bun.which(program) === null) {
        throw new Error(`${program ?? language.id} isn't installed (needed for ${language.id}).`);
      }
      const proc = Bun.spawn([program, ...args], {
        cwd: root,
        stdin: "pipe",
        stdout: "pipe",
        stderr: "pipe",
      });
      const rpc = new RpcConnection(proc, (line) => this.log(`${language.id}: ${line}`));
      const started = new LanguageServer(language, root, rpc);
      this.servers.set(key, started);
      void started.exited.then((code) => {
        this.log(`${language.id} server for ${root} exited (${code})`);
        if (this.servers.get(key) === started) this.servers.delete(key);
      });
      server = started;
    }
    return { server, languageId };
  }

  running(): { language: string; root: string }[] {
    return [...this.servers.values()].map((s) => ({ language: s.language.id, root: s.root }));
  }

  async shutdown(): Promise<void> {
    await Promise.all([...this.servers.values()].map((s) => s.shutdown()));
    this.servers.clear();
  }
}
