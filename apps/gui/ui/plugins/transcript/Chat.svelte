<script lang="ts">
  // The session's chat: its items as rows (rows.ts: a finished turn's work folded, tool calls
  // grouped), each drawn by the renderer registered for its kind (`chat.row`), following the end
  // as the agent writes.
  import { window } from "@neoworks/svelte-gpui";
  import { List } from "@neoworks/svelte-gpui/ui";
  import { useCtx } from "../../kernel/slots/context.ts";
  import Slot from "../../kernel/slots/Slot.svelte";
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
      isOpen: (key) => transcript.expanded || opened.has(`${daemon.sessionId}/${key}`),
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

  // `stickToEnd` follows new rows only once the list is at its end, which a session that has
  // just been opened (all its items at once) never is: start such a chat at its end.
  let started: string | undefined;
  $effect(() => {
    const session = daemon.sessionId;
    const count = rows.length;
    const start = `${session}@${width}`;
    if (session === undefined || count === 0 || started === start) return;
    started = start;
    list?.scrollToIndex(count - 1, { align: "end" });
  });

  // The list is built anew when the window's width changes: a List that is resized keeps stale
  // row heights, so rows overlap (svelte-gpui #31). Until that is fixed, a resize returns the
  // chat to its end.
  const width = $derived(window.size.width);
</script>

{#key width}
<List
  bind:this={list}
  items={rows}
  key={(row: Row) => row.key}
  stickToEnd
  style={{ flex: 1, minHeight: 0 }}
>
  {#snippet row(row: Row)}
    <Slot name="chat.row" key={row.kind} props={{ row }} />
  {/snippet}
</List>
{/key}
