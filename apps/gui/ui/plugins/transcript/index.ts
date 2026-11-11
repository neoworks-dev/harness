// The conversation: the session's items in the shell's middle, as long as nothing else claims it.
// The items become rows (rows.ts): a finished turn's work folded, tool calls grouped. Each row
// kind is drawn by an entry in `chat.row`, and the rows that are a single item by `chat.item`
// (one entry per item kind), so a plugin can draw a kind of its own. A reply of plain text is
// the exception: each of its blocks is a `prose` row, so the list builds only those in view. The transcript keeps the
// toggles for results (Ctrl-O) and thoughts (Ctrl-T) and offers a service to the panes that draw
// items or answer them.
import type { Context } from "@neoworks/extension-system";
import "../shell/slots.ts";
import AgentItem from "./AgentItem.svelte";
import AppTag from "./AppTag.svelte";
import Chat from "./Chat.svelte";
import FallbackItem from "./FallbackItem.svelte";
import FoldRow from "./FoldRow.svelte";
import ItemRow from "./ItemRow.svelte";
import MessageRow from "./MessageRow.svelte";
import NoticeItem from "./NoticeItem.svelte";
import ProseRow from "./ProseRow.svelte";
import { TranscriptService } from "./service.ts";
import "./slots.ts";
import { transcript } from "./state.svelte.ts";
import StoppedRow from "./StoppedRow.svelte";
import { docNav } from "./docnav.svelte.ts";
import ThoughtItem from "./ThoughtItem.svelte";
import { timeline } from "./timeline.svelte.ts";
import ToolItem from "./ToolItem.svelte";
import UserItem from "./UserItem.svelte";
import Work from "./Work.svelte";
import WorkingRow from "./WorkingRow.svelte";

export const transcriptPane = {
  name: "transcript",
  inject: ["slots", "daemon", "keymap", "keyboard", "commands", "viewer"],
  apply(ctx: Context) {
    const { slots, daemon, keymap, keyboard, commands } = ctx;
    ctx.plugin(TranscriptService);
    timeline.follow(daemon);

    slots.inject("shell.main", (slot) => {
      slot.add({
        component: Chat,
        order: 100,
        select: () => true,
        slots: {
          "chat.row": { kind: "keyed", fallback: ItemRow },
          "chat.item": { kind: "keyed", fallback: FallbackItem },
        },
      });
    });

    // One renderer per kind of row; the rows that are one item (the user's message, a notice)
    // are drawn by `chat.item`.
    slots.inject("chat.row", (slot) => {
      slot.add({ component: ItemRow, key: "item" });
      slot.add({ component: MessageRow, key: "message" });
      slot.add({ component: ProseRow, key: "prose" });
      slot.add({ component: Work, key: "work" });
      slot.add({ component: FoldRow, key: "fold" });
      slot.add({ component: StoppedRow, key: "stopped" });
      slot.add({ component: WorkingRow, key: "working" });
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
    // Ctrl-P into the latest doc with fields or buttons, after the composer's own Ctrl-P (a
    // question put aside); Ctrl-N, while in one, to a newer doc and back to the input.
    keyboard.claim(() => docNav.active !== undefined);
    keymap.bind("ctrl-p", () => docNav.step(-1), { priority: -1 });
    keymap.bind("ctrl-n", () => docNav.step(1), { priority: 1 });
    keymap.bind("escape", () => {
      if (docNav.active === undefined) return false;
      docNav.leave();
    }, { priority: 1 });

    keymap.bind("ctrl-o", toggleExpanded);
    keymap.bind("ctrl-t", toggleThoughts);
    commands.register({
      name: "expand",
      desc: "toggle full tool results (ctrl-o)",
      run: toggleExpanded,
    });
    commands.register({
      name: "thoughts",
      desc: "open or close the agent's thoughts (ctrl-t)",
      run: toggleThoughts,
    });
  },
};
