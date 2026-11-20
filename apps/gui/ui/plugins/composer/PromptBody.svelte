<script lang="ts">
  // What a question is about: details, a diff or code drawn as the chat draws them (coloured, with
  // line numbers) and all of it, since what is to be approved has to be readable (the panel
  // scrolls), and text. A diff also opens in the viewer beside the chat.
  import { Button, Text, View } from "@neoworks/svelte-gpui/ui";
  import type { Component } from "@neoworks-dev/neo-protocol/types";
  import { useCtx } from "../../kernel/slots/context.ts";
  import { diffStats } from "../../lib/diff.ts";
  import CodeBlock from "../transcript/CodeBlock.svelte";
  import DiffView from "../transcript/DiffView.svelte";
  import { plainText, previewLines } from "./prompt.ts";

  interface Props {
    components: readonly Component[];
  }

  let { components }: Props = $props();

  const { daemon, viewer } = useCtx();

  const p = $derived(daemon.palette);
  /** Lines of text shown; diffs and code are shown whole (up to their own caps). */
  const TEXT_LINES = 40;
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
      {@const stats = diffStats(component.lines)}
      <View style={{ direction: "row", gap: p.space.m, align: "center" }}>
        <Text style={{ color: p.muted, flex: 1, minWidth: 0, truncate: true }}>
          {component.path ?? "diff"} · <Text style={{ color: p.success }}>+{stats.added}</Text>
          <Text style={{ color: p.danger }}>{" "}−{stats.removed}</Text>
        </Text>
        <Button
          onpress={() => viewer.showDiff(component)}
          style={{ paddingX: p.space.m, paddingY: 2, radius: 5, background: p.surface, borderWidth: 1, borderColor: p.border }}
          hover={{ borderColor: p.accent }}
        >
          <Text style={{ color: p.accent }}>Open in the viewer</Text>
        </Button>
      </View>
      <DiffView diff={component} header={false} />
    {:else if component.type === "code"}
      <CodeBlock code={component.code} language={component.language} />
    {:else if component.type === "text"}
      {@const preview = previewLines(plainText(component.blocks), TEXT_LINES)}
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
