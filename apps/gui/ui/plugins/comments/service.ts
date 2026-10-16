// The `comments` service: how the viewer hands comments on code to the input, which sends them
// with the next message. The viewer adds and asks what is marked; the input lists, removes and
// takes them.
import { Service, type Context } from "@neoworks/extension-system";
import type { CodeComment } from "../../lib/comments.ts";
import { comments } from "./state.svelte.ts";

declare module "@neoworks/extension-system" {
  interface Context {
    comments: CommentsService;
  }
}

export class CommentsService extends Service {
  constructor(ctx: Context) {
    super(ctx, "comments");
  }

  /** The comments waiting, oldest first; reactive. */
  get list(): readonly CodeComment[] {
    return comments.list;
  }

  add(comment: Omit<CodeComment, "id">): void {
    comments.add(comment);
  }

  remove(id: string): void {
    comments.remove(id);
  }

  /** Whether a comment waits on this line of `path`; reactive. */
  marked(path: string, side: "new" | "old", no: number): boolean {
    return comments.marked(path, side, no);
  }

  /** `text` with every waiting comment as a block after it, and the comments gone. */
  take(text: string): string {
    return comments.take(text);
  }
}
