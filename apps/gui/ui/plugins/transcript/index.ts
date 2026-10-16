// The conversation: the session's items in the shell's middle, as long as nothing else claims it.
// Each item kind is drawn by an entry in `chat.item`, so a plugin can draw a kind of its own;
// the transcript keeps the toggles for results (Ctrl-O) and thoughts (Ctrl-T) and offers a
// service to the panes that draw items or answer them.
import type { Context } from "@neoworks/extension-system";
import "../shell/slots.ts";
import AgentItem from "./AgentItem.svelte";
import AppTag from "./AppTag.svelte";
import Chat from "./Chat.svelte";
import FallbackItem from "./FallbackItem.svelte";
import NoticeItem from "./NoticeItem.svelte";
import { TranscriptService } from "./service.ts";
import "./slots.ts";
import { transcript } from "./state.svelte.ts";
import ThoughtItem from "./ThoughtItem.svelte";
import ToolItem from "./ToolItem.svelte";
import UserItem from "./UserItem.svelte";

export const transcriptPane = {
  name: "transcript",
  inject: ["slots", "daemon", "keymap", "commands", "diff"],
  apply(ctx: Context) {
    const { slots, keymap, commands } = ctx;
    ctx.plugin(TranscriptService);

    slots.inject("shell.main", (slot) => {
      slot.add({
        component: Chat,
        order: 100,
        select: () => true,
        slots: { "chat.item": { kind: "keyed", fallback: FallbackItem } },
      });
    });

    // One renderer per kind of item.
    slots.inject("chat.item", (slot) => {
      slot.add({ component: UserItem, key: "user", slots: { "chat.item.header": "list" } });
      slot.add({ component: AgentItem, key: "agent" });
      slot.add({ component: ThoughtItem, key: "thought" });
      slot.add({ component: ToolItem, key: "tool" });
      slot.add({ component: NoticeItem, key: "notice" });
    });
    slots.inject("chat.item.header", (slot) => {
      slot.add({ component: AppTag });
    });

    const toggleExpanded = (): void => {
      transcript.expanded = !transcript.expanded;
    };
    const toggleThoughts = (): void => {
      transcript.showThoughts = !transcript.showThoughts;
    };
    keymap.bind("ctrl-o", toggleExpanded);
    keymap.bind("ctrl-t", toggleThoughts);
    commands.register({
      name: "expand",
      desc: "toggle full tool results (ctrl-o)",
      run: toggleExpanded,
    });
    commands.register({
      name: "thoughts",
      desc: "show or hide the agent's thoughts (ctrl-t)",
      run: toggleThoughts,
    });
  },
};
