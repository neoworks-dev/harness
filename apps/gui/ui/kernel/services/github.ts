// The `github` service: the host's GitHub commands. They only read; the app never writes to
// GitHub. The view's own state is in plugins/github.
import { Service, type Context } from "@neoworks/extension-system";
import * as host from "../host/github.ts";

declare module "@neoworks/extension-system" {
  interface Context {
    github: GithubService;
  }
}

export class GithubService extends Service {
  constructor(ctx: Context) {
    super(ctx, "github");
  }

  repo = host.repo;
  issues = host.issues;
  prs = host.prs;
  issue = host.issue;
  pr = host.pr;
  prDiff = host.prDiff;
  issueText = host.issueText;
  prText = host.prText;
}
