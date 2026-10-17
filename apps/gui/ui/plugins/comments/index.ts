// Comments on lines of code: the viewer writes them, the input shows them as chips and sends them
// with the next message. This plugin only holds them between the two, as the `comments` service.
import type { Context } from "@neoworks/extension-system";
import { CommentsService } from "./service.ts";
import { comments } from "./state.svelte.ts";

export const commentsStore = {
  name: "comments",
  inject: ["daemon"],
  apply(ctx: Context) {
    const { daemon } = ctx;
    ctx.effect(() => comments.start(daemon), "comments state");
    ctx.plugin(CommentsService);
  },
};
