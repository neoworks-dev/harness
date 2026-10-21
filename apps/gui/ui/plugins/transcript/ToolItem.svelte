<script lang="ts">
  // A tool call on its own, where nothing groups it (another agent's chat): the same row of work
  // the transcript draws in a group.
  import type { ViewItem } from "@neoworks-dev/neo-protocol/types";
  import { useCtx } from "../../kernel/slots/context.ts";
  import Entry from "./Entry.svelte";
  import ItemFrame from "./ItemFrame.svelte";
  import { timeline } from "./timeline.svelte.ts";
  import { classify } from "./work.ts";

  let { item }: { item: ViewItem } = $props();

  const { daemon } = useCtx();

  const entry = $derived(
    classify(item, {
      facts: timeline.value.facts.get(item.ref),
      cwd: daemon.cwd,
      active: daemon.status.running,
      at: timeline.value.stamps.get(item.ref)?.first,
    }),
  );
</script>

<ItemFrame gap={entry.kind === "question" ? 16 : 6}>
  <Entry {entry} />
</ItemFrame>
