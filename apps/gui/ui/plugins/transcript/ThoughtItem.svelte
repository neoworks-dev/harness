<script lang="ts">
  // The agent's thoughts on their own (another agent's chat): a brain and a one-line preview,
  // opened to the thought itself.
  import type { ViewItem } from "@neoworks-dev/neo-protocol/types";
  import { useCtx } from "../../kernel/slots/context.ts";
  import Entry from "./Entry.svelte";
  import ItemFrame from "./ItemFrame.svelte";
  import { classify } from "./work.ts";

  let { item }: { item: ViewItem } = $props();

  const { daemon } = useCtx();

  const entry = $derived(classify(item, { cwd: daemon.cwd, active: false }));
</script>

<ItemFrame gap={6}>
  <Entry {entry} />
</ItemFrame>
