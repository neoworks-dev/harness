<script lang="ts">
  // A tool call: one muted row (its state, its title) and the result, collapsed to a summary until
  // pressed. A question (buttons) is always open, and a stub while the composer answers it.
  import { Icon, Text, View } from "@neoworks/svelte-gpui/ui";
  import check from "@phosphor-icons/core/regular/check.svg";
  import circle from "@phosphor-icons/core/regular/circle.svg";
  import failed from "@phosphor-icons/core/regular/x.svg";
  import question from "@phosphor-icons/core/regular/question.svg";
  import type { ViewItem } from "@suite/protocol/types";
  import { useCtx } from "../../kernel/slots/context.ts";
  import Spinner from "../../widgets/Spinner.svelte";
  import { hasButtons } from "./components.ts";
  import Component from "./Component.svelte";
  import ItemFrame from "./ItemFrame.svelte";
  import { GUTTER, PREVIEW_LINES } from "./layout.ts";
  import { opened } from "./opened.svelte.ts";
  import { summarize } from "./summary.ts";
  import { transcript } from "./state.svelte.ts";

  let { item }: { item: ViewItem } = $props();

  const { daemon } = useCtx();
  const p = $derived(daemon.palette);

  const asks = $derived(hasButtons(item.component));
  /** The lines an edit changed, shown at the end of its row. */
  const changed = $derived.by(() => {
    let added = 0;
    let removed = 0;
    let any = false;
    for (const leaf of summarize(item.component)) {
      if (leaf.kind !== "diff") continue;
      any = true;
      added += leaf.added;
      removed += leaf.removed;
    }
    return any ? { added, removed } : undefined;
  });
  const open = $derived(transcript.expanded || opened.has(item.ref));
  /** Another pane is answering this card: the transcript keeps a stub. */
  const stub = $derived(transcript.stubbed === item.ref && !transcript.expanded);

  const stateColor = $derived.by(() => {
    if (item.state === "failed") return p.danger;
    return item.state === "running" ? p.accent : p.muted;
  });
</script>

{#snippet glyph()}
  <View style={{ width: GUTTER, height: 24, align: "center", justify: "center", color: stateColor }}>
    {#if item.state === "running"}
      <Spinner />
    {:else if asks}
      <Icon svg={question} size={16} />
    {:else if item.state === "done"}
      <Icon svg={check} size={16} />
    {:else if item.state === "failed"}
      <Icon svg={failed} size={16} />
    {:else}
      <Icon svg={circle} size={16} />
    {/if}
  </View>
{/snippet}

<ItemFrame gap={asks ? 16 : 6}>
  {#if stub}
    <View style={{ direction: "row", align: "center", color: p.muted }}>
      {@render glyph()}
      <Text><Text style={{ color: p.fg, fontWeight: "medium" }}>{item.label ?? "a question"}</Text>{" · answer below"}</Text>
    </View>
  {:else}
    <View>
      <View
        onpress={asks ? undefined : () => opened.toggle(item.ref)}
        hover={asks ? undefined : { background: p.hover }}
        style={{ direction: "row", align: "center", minHeight: 24, paddingY: 2, radius: p.radius.s, color: p.muted, cursor: asks ? undefined : "pointer" }}
      >
        {@render glyph()}
        <Text style={{ flex: 1, minWidth: 0, truncate: true, color: asks ? p.fg : p.muted, fontWeight: asks ? "medium" : "normal" }}>{item.label ?? ""}</Text>
        {#if changed}
          <Text style={{ fontFamily: "mono", fontSize: 12, paddingRight: 6 }}><Text style={{ color: p.success }}>+{changed.added}</Text>{" "}<Text style={{ color: p.danger }}>−{changed.removed}</Text></Text>
        {/if}
      </View>
      {#if asks || open}
        <View style={{ paddingLeft: GUTTER, paddingTop: 4 }}>
          <Component
            component={item.component}
            ref={item.ref}
            limit={asks && !transcript.expanded ? PREVIEW_LINES : undefined}
          />
        </View>
      {/if}
    </View>
  {/if}
</ItemFrame>
