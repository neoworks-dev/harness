<script lang="ts">
  // One line under the input: branch, harness and its settings, approvals mode, turn state and
  // usage. A lost connection, a notice (Ctrl-C) and the last error show here too.
  import { Text, View } from "@neoworks/svelte-gpui/ui";
  import { statusSegments, type Tone } from "../lib/statusline.ts";
  import { suite } from "../state/suite.svelte.ts";
  import { composer } from "./composer.svelte.ts";

  const p = $derived(suite.palette);

  // Ticks once a second while a turn runs, for its elapsed time.
  let now = $state(Date.now());
  $effect(() => {
    if (!suite.status.running) return;
    now = Date.now();
    const timer = setInterval(() => (now = Date.now()), 1000);
    return () => clearInterval(timer);
  });

  const segments = $derived(
    statusSegments({
      branch: suite.branch,
      status: suite.status,
      queued: suite.session?.queued ?? 0,
      connection: suite.connection,
      now,
    }),
  );

  function colour(tone: Tone): string {
    return tone === "fg" ? p.fg : p[tone];
  }
</script>

<View style={{ direction: "row", gap: p.space.l, paddingX: p.space.l, paddingY: p.space.s, align: "center" }}>
  <Text style={{ flex: 1, minWidth: 0, truncate: true, fontSize: p.text.body - 1 }}>
    {#each segments as segment, index}
      {#if index > 0}<Text pre style={{ color: p.muted }}>{" · "}</Text>{/if}
      <Text style={{ color: colour(segment.tone) }}>{segment.text}</Text>
    {/each}
  </Text>
  {#if composer.notice}
    <Text style={{ color: p.accent, shrink: 0 }}>{composer.notice}</Text>
  {/if}
  {#if suite.error}
    <Text style={{ color: p.danger, maxWidth: "50%", truncate: true }}>{suite.error}</Text>
  {/if}
</View>
