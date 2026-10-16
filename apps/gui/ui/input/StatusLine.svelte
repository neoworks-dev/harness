<script lang="ts">
  // One line under the input: branch, harness and its settings, approvals, turn state.
  import { Text, View } from "@neoworks/svelte-gpui/ui";
  import { currentSettings } from "../lib/status.ts";
  import { suite } from "../state/suite.svelte.ts";

  const p = $derived(suite.palette);
  const settings = $derived(currentSettings(suite.status));
  const parts = $derived(
    [suite.branch, suite.status.harness, settings.model, settings.effort, suite.status.approvals].filter(
      (part): part is string => typeof part === "string" && part !== "",
    ),
  );
</script>

<View style={{ direction: "row", gap: p.space.m, paddingX: p.space.l, paddingY: p.space.s }}>
  <Text style={{ color: p.muted, flex: 1, minWidth: 0, truncate: true }}>{parts.join(" · ")}</Text>
  {#if suite.status.running}
    <Text style={{ color: p.accent }}>working…</Text>
  {/if}
</View>
