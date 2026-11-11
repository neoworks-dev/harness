<script lang="ts">
  // The messages queued behind the running turn, above the input: one muted row each, the first
  // tagged `queued`. The user's can be taken back (Up in an empty input, or a press on a row); an
  // agent's cannot.
  import { Button, Text, View } from "@neoworks/svelte-gpui/ui";
  import { useCtx } from "../../kernel/slots/context.ts";
  import { composer } from "./state.svelte.ts";

  let { onback }: { onback?: () => void } = $props();

  const { daemon } = useCtx();
  const p = $derived(daemon.palette);
  const queue = $derived(composer.queue);

  function takeBack(): void {
    void composer.unqueue().then((taken) => {
      if (taken) onback?.();
    });
  }
</script>

{#if queue.rows.length > 0}
  <View style={{ gap: 2, paddingX: 18, paddingBottom: 8 }}>
    {#each queue.rows as row, index (index)}
      {#snippet line()}
        <Text style={{ flex: 1, minWidth: 0, truncate: true, italic: true, color: p.muted }}>{row.label}</Text>
        {#if index === 0}
          <Text style={{ shrink: 0, color: p.accent }}>
            queued<Text style={{ color: p.muted }}>{queue.editable ? " · ↑ edits" : ""}</Text>
          </Text>
        {/if}
      {/snippet}
      {#if row.users}
        <Button
          onpress={takeBack}
          style={{ direction: "row", align: "center", gap: 8, radius: p.radius.s }}
          hover={{ background: p.hover }}
        >
          {@render line()}
        </Button>
      {:else}
        <View style={{ direction: "row", align: "center", gap: 8 }}>
          {@render line()}
        </View>
      {/if}
    {/each}
    {#if queue.more > 0}
      <Text style={{ color: p.muted, truncate: true }}>… +{queue.more} more queued</Text>
    {/if}
  </View>
{/if}
