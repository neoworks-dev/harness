// The `git` service: the host's git commands, and the status of the shown session's directory.
import { Service, type Context } from "@neoworks/extension-system";
import type { GitStatus } from "../../lib/files.ts";
import { git as status } from "../../side/git.svelte.ts";
import * as host from "../host/git.ts";

declare module "@neoworks/extension-system" {
  interface Context {
    git: GitService;
  }
}

export class GitService extends Service {
  constructor(ctx: Context) {
    super(ctx, "git");
  }

  /** What git says changed in the shown session's directory; absent outside a repository. */
  get status(): GitStatus | undefined {
    return status.status;
  }

  /** Asks git again now (it also asks on its own when the session's files change). */
  refresh(): Promise<void> {
    return status.refresh();
  }

  branch = host.branch;
  places = host.places;
  diff = host.diff;
}
