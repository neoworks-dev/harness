<script lang="ts">
  // The input's short message ("press ctrl-c again to quit", "copied"), shown in the status line.
  import { Text } from "@neoworks/svelte-gpui/ui";
  import { composer } from "../../input/composer.svelte.ts";
  import { useCtx } from "../../kernel/slots/context.ts";

  const { daemon } = useCtx();
  const p = $derived(daemon.palette);
</script>

{#if composer.notice}
  <Text style={{ color: p.accent, shrink: 0 }}>{composer.notice}</Text>
{/if}
