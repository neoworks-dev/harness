<script lang="ts">
  // A project's favicon, or a badge with its initials while it has none (or it fails to load).
  import { Image, Text, View } from "@neoworks/svelte-gpui/ui";
  import { useCtx } from "../../kernel/slots/context.ts";
  import { initials } from "./favicon.ts";

  let { name, src, size = 16 }: { name: string; src: string | null | undefined; size?: number } =
    $props();

  const { daemon } = useCtx();
  const p = $derived(daemon.palette);
  let failed = $state(false);

  $effect(() => {
    void src;
    failed = false;
  });
</script>

{#if src && !failed}
  <Image {src} fit="contain" onerror={() => (failed = true)} style={{ size, radius: 3 }} />
{:else}
  <View
    style={{
      size,
      align: "center",
      justify: "center",
      radius: 4,
      background: p.raised,
      borderWidth: 1,
      borderColor: p.border,
    }}
  >
    <Text style={{ fontSize: Math.round(size * 0.45), fontWeight: "bold", color: p.muted }}
      >{initials(name)}</Text
    >
  </View>
{/if}
