<script lang="ts">
  // The session's chat: every item the daemon resolved, drawn natively, following the end as
  // the agent writes. Thoughts only show with Ctrl-T.
  import { window } from "@neoworks/svelte-gpui";
  import { List } from "@neoworks/svelte-gpui/ui";
  import type { ViewItem } from "@suite/protocol/types";
  import { suite } from "../state/suite.svelte.ts";
  import { ui } from "../state/ui.svelte.ts";
  import ChatItem from "./ChatItem.svelte";

  const items = $derived(suite.items.filter((item) => item.kind !== "thought" || ui.showThoughts));

  let list: { scrollToIndex(index: number, options: { align: "end" }): void } | undefined = $state();

  // `stickToEnd` follows new items only once the list is at its end, which a session that has
  // just been opened (all its items at once) never is: start such a chat at its end.
  let started: string | undefined;
  $effect(() => {
    const session = suite.sessionId;
    const count = items.length;
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
  {items}
  key={(item: ViewItem) => item.ref}
  stickToEnd
  style={{ flex: 1, minHeight: 0 }}
>
  {#snippet row(item: ViewItem)}
    <ChatItem {item} />
  {/snippet}
</List>
{/key}
