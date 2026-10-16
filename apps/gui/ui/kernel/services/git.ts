// The `git` service: the host's git commands.
import { Service, type Context } from "@neoworks/extension-system";
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

  branch = host.branch;
  places = host.places;
  status = host.status;
  diff = host.diff;
}
