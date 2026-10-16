// Whether the agent's own actions (bash, write, edit) go ahead, by the session's approvals mode:
//   auto   nothing asks
//   edits  edits go ahead (the review card offers to undo them after the turn), the rest asks
//   ask    everything asks; an edit's diff is shown before anything is written
// The question is a permission_request with the content to preview. "Allow for this session"
// holds per session and kind, in memory only.
import type { RequestPermission } from "@suite/harness/agent";
import type { PermissionOption } from "@suite/protocol/types";
import type { Asker } from "./asker.ts";
import type { Approvals } from "./settings.ts";

const OPTIONS: PermissionOption[] = [
  { id: "allow", name: "Allow", kind: "allow_once" },
  { id: "always", name: "Allow for this session", kind: "allow_always" },
  { id: "reject", name: "Reject", kind: "reject_once" },
];

export class Permissions {
  /** session\0kind pairs the user allowed for the rest of the session. */
  private readonly allowed = new Set<string>();

  constructor(
    private readonly asker: Asker,
    private readonly approvals: (session: string) => Approvals,
  ) {}

  /** The hook for one session's agent (harness name `agent`), given to its tools. */
  hook(session: string, agent: string): RequestPermission {
    return async (request, signal) => {
      const mode = this.approvals(session);
      if (mode === "auto" || (mode === "edits" && request.kind === "edit")) return true;
      const key = `${session}\0${request.kind}`;
      if (this.allowed.has(key)) return true;
      const answer = await this.asker.ask(
        session,
        { type: "permission_request", agent, ...request, options: OPTIONS },
        signal,
      );
      if (answer === "always") this.allowed.add(key);
      return answer === "allow" || answer === "always";
    };
  }
}
