<script lang="ts">
  // The session's chat: its items as rows (rows.ts: a finished turn's work folded, tool calls
  // grouped), each drawn by the renderer registered for its kind (`chat.row`), following the end
  // as the agent writes.
  import { window } from "@neoworks/svelte-gpui";
  import { List } from "@neoworks/svelte-gpui/ui";
  import { useCtx } from "../../kernel/slots/context.ts";
  import { Follow } from "../../widgets/follow.ts";
  import Slot from "../../kernel/slots/Slot.svelte";
  import { estimateRow } from "./estimate.ts";
  import { docNav } from "./docnav.svelte.ts";
  import { opened } from "./opened.svelte.ts";
  import { buildRows, splitMessages, type Row } from "./rows.ts";
  import { transcript } from "./state.svelte.ts";
  import { timeline } from "./timeline.svelte.ts";

  const { daemon } = useCtx();

  const rows = $derived(
    splitMessages(buildRows({
      items: daemon.items,
      timeline: timeline.value,
      cwd: daemon.cwd,
      running: daemon.status.running,
      startedAt: daemon.status.turnStartedAt,
      // A finished turn's work stays in sight, as in the terminal, until its row is pressed.
      isOpen: (key) => transcript.expanded || !opened.has(`${daemon.sessionId}/${key}`),
    })),
  );

  let list: { scrollToIndex(index: number, options: { align: "end" | "nearest" }): void } | undefined = $state();

  // Ctrl-P finds its docs in these rows.
  $effect(() =>
    docNav.attach({
      session: () => daemon.sessionId,
      rows: () => rows,
      reveal: (index) => list?.scrollToIndex(index, { align: "nearest" }),
    }),
  );

  // `stickToEnd` follows new rows only once the list is at its end, which a session that has just
  // been opened (all its items at once) often is not, its rows taller than guessed: it would never
  // follow. `follow` decides by what the user did instead: the chat follows until they scroll up,
  // and again once they are back at the end. Each change of the rows goes to the end, and once
  // more after the new rows are measured.
  const follow = new Follow();
  let started: string | undefined;
  $effect(() => {
    const session = daemon.sessionId;
    const count = rows.length;
    const start = `${session}@${width}`;
    if (session === undefined || count === 0) return;
    if (started !== start) {
      started = start;
      follow.reset();
    }
    if (!follow.following) return;
    list?.scrollToIndex(count - 1, { align: "end" });
    const frame = requestAnimationFrame(() => {
      if (follow.following) list?.scrollToIndex(rows.length - 1, { align: "end" });
    });
    return () => cancelAnimationFrame(frame);
  });

  // The list is built anew when the window's width changes: a List that is resized keeps stale
  // row heights, so rows overlap (svelte-gpui #31). Until that is fixed, a resize returns the
  // chat to its end.
  const width = $derived(window.size.width);
  /** Roughly the column's width (Column.svelte: at most 736, 48 at either side), for the guesses. */
  const column = $derived(Math.min(736, Math.max(width - 96, 200)));
</script>

{#key width}
<List
  bind:this={list}
  items={rows}
  key={(row: Row) => row.key}
  estimatedRowHeight={(row: Row) => estimateRow(row, column)}
  stickToEnd
  onscroll={(position: { y: number; maxY: number }) => follow.scrolled(position.y, position.maxY)}
  style={{ flex: 1, minHeight: 0 }}
>
  {#snippet row(row: Row)}
    <Slot name="chat.row" key={row.kind} props={{ row }} />
  {/snippet}
</List>
{/key}
