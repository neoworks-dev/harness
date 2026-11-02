<script lang="ts">
  // A Diff: the unified diff the model wrote, drawn like an edit's diff under its path.
  import { Text, View } from "@neoworks/svelte-gpui/ui";
  import { parseUnified } from "../../../lib/unified.ts";
  import { useCtx } from "../../../kernel/slots/context.ts";
  import DiffView from "../DiffView.svelte";
  import { str } from "./props.ts";
  import type { NodeProps } from "./Node.svelte";

  let { node }: NodeProps = $props();
  const { daemon } = useCtx();
  const p = $derived(daemon.palette);
  const parsed = $derived(parseUnified(str(node, "diff") ?? "")[0]);
  const path = $derived(str(node, "path") ?? parsed?.path ?? "");
</script>

<View style={{ gap: 4 }}>
  {#if path !== ""}
    <Text style={{ fontFamily: "mono", fontWeight: "medium", color: p.fg }}>{path}</Text>
  {/if}
  <DiffView diff={{ type: "diff", path, lines: parsed?.lines ?? [] }} />
</View>
