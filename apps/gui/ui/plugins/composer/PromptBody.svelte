<script lang="ts">
  // What a question is about, compact: details, a few lines of a diff or code, text. A diff opens
  // side by side over the window.
  import { Button, Text, View } from "@neoworks/svelte-gpui/ui";
  import type { Component } from "@suite/protocol/types";
  import { useCtx } from "../../kernel/slots/context.ts";
  import { plainText, previewLines } from "./prompt.ts";

  interface Props {
    components: readonly Component[];
  }

  let { components }: Props = $props();

  const { daemon, diff } = useCtx();

  const p = $derived(daemon.palette);
  const PREVIEW = 8;
</script>

<View style={{ gap: p.space.s }}>
  {#each components as component}
    {#if component.type === "kv"}
      {#each component.items as item}
        <View style={{ direction: "row", gap: p.space.m }}>
          <Text style={{ color: p.muted, shrink: 0 }}>{item.key}</Text>
          <Text style={{ color: item.tone === "danger" ? p.danger : p.accent, flex: 1, minWidth: 0 }}>{item.value}</Text>
        </View>
      {/each}
    {:else if component.type === "diff"}
      {@const changed = component.lines.filter((line) => line.kind === "add" || line.kind === "remove")}
      {@const shown = changed.slice(0, PREVIEW)}
      <View style={{ direction: "row", gap: p.space.m, align: "center" }}>
        <Text style={{ color: p.muted, flex: 1, minWidth: 0, truncate: true }}>
          {component.path ?? "diff"} · +{changed.filter((line) => line.kind === "add").length} −{changed.filter(
            (line) => line.kind === "remove",
          ).length}
        </Text>
        <Button
          onpress={() => diff.show(component)}
          style={{ paddingX: p.space.m, paddingY: 2, radius: 5, background: p.surface, borderWidth: 1, borderColor: p.border }}
          hover={{ borderColor: p.accent }}
        >
          <Text style={{ color: p.accent }}>open side by side</Text>
        </Button>
      </View>
      <View style={{ background: p.surface, radius: 6, padding: p.space.m }}>
        {#each shown as line}
          <Text style={{ fontFamily: "mono", fontSize: p.text.code, color: line.kind === "add" ? p.success : p.danger, nowrap: true, truncate: true }}>
            {line.kind === "add" ? "+" : "−"} {line.text}
          </Text>
        {/each}
        {#if changed.length > shown.length}
          <Text style={{ color: p.muted }}>… {changed.length - shown.length} more changed lines</Text>
        {/if}
      </View>
    {:else if component.type === "code"}
      {@const preview = previewLines(component.code, PREVIEW)}
      <View style={{ background: p.surface, radius: 6, padding: p.space.m }}>
        {#each preview.lines as line}
          <Text style={{ fontFamily: "mono", fontSize: p.text.code, nowrap: true, truncate: true }}>{line === "" ? " " : line}</Text>
        {/each}
        {#if preview.more > 0}
          <Text style={{ color: p.muted }}>… {preview.more} more lines</Text>
        {/if}
      </View>
    {:else if component.type === "text"}
      {@const preview = previewLines(plainText(component.blocks), PREVIEW)}
      <View>
        {#each preview.lines as line}
          <Text style={{ color: component.tone === "muted" ? p.muted : p.fg }}>{line === "" ? " " : line}</Text>
        {/each}
        {#if preview.more > 0}
          <Text style={{ color: p.muted }}>… {preview.more} more lines</Text>
        {/if}
      </View>
    {:else if component.type === "file"}
      <Text style={{ color: p.accent }}>{component.name ?? component.path}</Text>
    {/if}
  {/each}
</View>
