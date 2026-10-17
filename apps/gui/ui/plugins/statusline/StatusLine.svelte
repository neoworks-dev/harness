<script lang="ts">
  // One line under the input: branch, harness and its settings, approvals mode, turn state and
  // usage. Other plugins add messages after it (the `statusline.right` slot); a lost connection
  // and the last error show here too.
  import { Text, View } from "@neoworks/svelte-gpui/ui";
  import { useCtx } from "../../kernel/slots/context.ts";
  import Slot from "../../kernel/slots/Slot.svelte";
  import { currentSettings } from "../../lib/status.ts";
  import { statusSegments, type Tone } from "./segments.ts";

  const { daemon } = useCtx();

  const p = $derived(daemon.palette);

  // Ticks once a second while a turn runs, for its elapsed time.
  let now = $state(Date.now());
  $effect(() => {
    if (!daemon.status.running) return;
    now = Date.now();
    const timer = setInterval(() => (now = Date.now()), 1000);
    return () => clearInterval(timer);
  });

  const model = $derived(currentSettings(daemon.status).model);
  const segments = $derived(
    statusSegments({
      branch: daemon.branch,
      status: daemon.status,
      model: model === undefined ? undefined : daemon.label("model", model),
      queued: daemon.session?.queued ?? 0,
      connection: daemon.connection,
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
  <Slot name="statusline.right" />
  {#if daemon.error}
    <Text style={{ color: p.danger, maxWidth: "50%", truncate: true }}>{daemon.error}</Text>
  {/if}
</View>
