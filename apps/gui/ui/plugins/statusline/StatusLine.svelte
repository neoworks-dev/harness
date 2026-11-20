<script lang="ts">
  // One line under the input: branch, harness and its settings, approvals mode, turn state and
  // usage. Other plugins add messages after it (the `statusline.right` slot); a lost connection
  // and the last error show here too.
  import { Icon, Text, View } from "@neoworks/svelte-gpui/ui";
  import branchIcon from "@phosphor-icons/core/regular/git-branch.svg";
  import { useCtx } from "../../kernel/slots/context.ts";
  import Slot from "../../kernel/slots/Slot.svelte";
  import { statusSegments, type Tone } from "./segments.ts";

  const { daemon } = useCtx();

  const p = $derived(daemon.palette);

  // Ticks once a second while a turn runs, for its elapsed time; once a minute otherwise, for when
  // the prompt cache goes cold.
  let now = $state(Date.now());
  $effect(() => {
    const every = daemon.status.running ? 1000 : 60_000;
    now = Date.now();
    const timer = setInterval(() => (now = Date.now()), every);
    return () => clearInterval(timer);
  });

  const segments = $derived(
    statusSegments({
      branch: daemon.branch,
      status: daemon.status,
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
  <View style={{ direction: "row", align: "center", flex: 1, minWidth: 0, fontSize: p.text.body - 1 }}>
    {#each segments as segment, index}
      {#if index > 0}<Text pre style={{ color: p.muted, shrink: 0 }}>{" · "}</Text>{/if}
      {#if segment.icon === "branch"}
        <View style={{ color: colour(segment.tone), marginRight: 4, shrink: 0 }}><Icon svg={branchIcon} size={13} /></View>
      {/if}
      <Text style={{ color: colour(segment.tone), truncate: true, minWidth: 0 }}>{segment.text}</Text>
    {/each}
  </View>
  <Slot name="statusline.right" />
  {#if daemon.error}
    <Text style={{ color: p.danger, maxWidth: "50%", truncate: true }}>{daemon.error}</Text>
  {/if}
</View>
