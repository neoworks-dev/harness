// neo's hashline read, edit and write on the gateway's MCP server, for an ACP harness that works
// with them in place of its own (Claude Code, with /settings claude-tools neo). They run in the
// session's directory and ask the way neo's own do: an edit with its diff, through the session's
// approvals mode. A file they change is announced like the agents' edits (file.changed).
import type { RequestPermission } from "@neoworks-dev/neo/agent";
import {
  type AgentTool,
  createFileTools,
  type FileChangeDetails,
  runFileTool,
} from "@neoworks-dev/neo/file-tools";
import type { CallToolResult, Tool } from "@modelcontextprotocol/sdk/types.js";

export const FILE_TOOL_NAMES = ["read", "edit", "write"];

export interface FileToolsParts {
  /** The session's directory. */
  cwd: (session: string) => string;
  resolveAsset?: (handle: string) => string | undefined;
  /** How `agent` asks in the session (see Permissions.hook). */
  ask: (session: string, agent: string) => RequestPermission;
  /** A file the tools wrote. */
  changed: (path: string, session: string) => void;
}

function isChange(details: unknown): details is FileChangeDetails {
  return (
    typeof details === "object" &&
    details !== null &&
    typeof (details as { path?: unknown }).path === "string" &&
    typeof (details as { newText?: unknown }).newText === "string"
  );
}

export class FileTools {
  private calls = 0;

  constructor(private readonly parts: FileToolsParts) {}

  /** The tools as MCP lists them (their schemas don't depend on the directory). */
  list(): Tool[] {
    return createFileTools("/").map((tool) => ({
      name: tool.name,
      description: tool.description,
      inputSchema: { type: "object" as const, ...(tool.parameters as Record<string, unknown>) },
    }));
  }

  /** Runs `name` for `agent` in the session; a failure is the result's text, marked. */
  async call(
    name: string,
    args: Record<string, unknown>,
    signal: AbortSignal | undefined,
    session: string,
    agent: string,
  ): Promise<CallToolResult> {
    const { cwd, resolveAsset, ask, changed } = this.parts;
    const tools = createFileTools(cwd(session), resolveAsset, ask(session, agent));
    const tool: AgentTool | undefined = tools.find((t) => t.name === name);
    if (!tool) return { content: [{ type: "text", text: `Unknown tool ${name}.` }], isError: true };
    this.calls += 1;
    try {
      const result = await runFileTool(tool, `files-${this.calls}`, args, signal);
      if (isChange(result.details)) changed(result.details.path, session);
      return {
        content: result.content.map((c) =>
          c.type === "text"
            ? { type: "text" as const, text: c.text }
            : { type: "image" as const, data: c.data, mimeType: c.mimeType },
        ),
      };
    } catch (err) {
      const text = err instanceof Error ? err.message : String(err);
      return { content: [{ type: "text", text }], isError: true };
    }
  }
}
