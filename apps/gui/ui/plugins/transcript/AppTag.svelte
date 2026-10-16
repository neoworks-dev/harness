<script lang="ts">
  // The app a user message came from ("photo-editor", "browser"), muted at its right. Messages
  // typed in this window need no label.
  import { Text } from "@neoworks/svelte-gpui/ui";
  import type { ViewItem } from "@suite/protocol/types";
  import { useCtx } from "../../kernel/slots/context.ts";

  let { item }: { item: ViewItem } = $props();

  const { daemon } = useCtx();
  const p = $derived(daemon.palette);
  const from = $derived(
    item.label !== undefined && item.label !== daemon.appId ? item.label : undefined,
  );
</script>

{#if from}<Text style={{ color: p.muted, marginLeft: p.space.m }}>{from}</Text>{/if}
