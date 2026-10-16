<script lang="ts">
  // A tool call: a header (its state, its title) and the result, collapsed to a few lines until
  // pressed. A question (buttons) is always open, and a stub while the composer answers it.
  import { Text, View } from "@neoworks/svelte-gpui/ui";
  import type { ViewItem } from "@suite/protocol/types";
  import { useCtx } from "../../kernel/slots/context.ts";
  import Spinner from "../../widgets/Spinner.svelte";
  import { hasButtons } from "./components.ts";
  import Component from "./Component.svelte";
  import ItemFrame from "./ItemFrame.svelte";
  import { GUTTER, PREVIEW_LINES } from "./layout.ts";
  import { opened } from "./opened.svelte.ts";
  import { transcript } from "./state.svelte.ts";
  import Summary from "./Summary.svelte";

  let { item }: { item: ViewItem } = $props();

  const { daemon } = useCtx();
  const p = $derived(daemon.palette);

  const question = $derived(hasButtons(item.component));
  const open = $derived(transcript.expanded || opened.has(item.ref));
  /** Another pane is answering this card: the transcript keeps a stub. */
  const stub = $derived(transcript.stubbed === item.ref && !transcript.expanded);

  const stateColor = $derived.by(() => {
    if (item.state === "done") return p.success;
    if (item.state === "failed") return p.danger;
    return item.state === "running" ? p.accent : p.muted;
  });
</script>

{#snippet glyph()}
  <View style={{ width: GUTTER, color: stateColor }}>
    {#if item.state === "running"}
      <Spinner />
    {:else if item.state === "done"}
      <Text>✓</Text>
    {:else if item.state === "failed"}
      <Text>✗</Text>
    {:else}
      <Text>○</Text>
    {/if}
  </View>
{/snippet}

<ItemFrame>
  {#if stub}
    <View style={{ direction: "row", paddingX: p.space.l }}>
      <Text style={{ width: GUTTER, color: p.accent }}>●</Text>
      <Text><Text style={{ fontWeight: "bold" }}>{item.label ?? "a question"}</Text><Text style={{ color: p.muted }}>{" · answer below"}</Text></Text>
    </View>
  {:else}
    <View style={{ paddingX: p.space.l }}>
      <View
        onpress={question ? undefined : () => opened.toggle(item.ref)}
        style={{ direction: "row", cursor: question ? undefined : "pointer" }}
      >
        {@render glyph()}
        <Text style={{ fontWeight: "bold", flex: 1, minWidth: 0 }}>{item.label ?? ""}</Text>
      </View>
      <View style={{ paddingLeft: GUTTER, paddingTop: 2 }}>
        {#if question || open}
          <Component
            component={item.component}
            ref={item.ref}
            limit={question && !transcript.expanded ? PREVIEW_LINES : undefined}
          />
        {:else}
          <View onpress={() => opened.toggle(item.ref)} style={{ cursor: "pointer", color: p.muted }}>
            <Summary component={item.component} ref={item.ref} />
          </View>
        {/if}
      </View>
    </View>
  {/if}
</ItemFrame>
