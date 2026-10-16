<script lang="ts">
  // One entry of the chat, drawn by its kind: the user's message on a band, the agent's reply
  // behind a bullet, thoughts muted, tool calls as a header and a result, notices as one line.
  import { Text, View } from "@neoworks/svelte-gpui/ui";
  import type { ViewItem } from "@suite/protocol/types";
  import { hasButtons } from "../lib/components.ts";
  import { suite } from "../state/suite.svelte.ts";
  import { ui } from "../state/ui.svelte.ts";
  import Component from "./Component.svelte";
  import { opened } from "./opened.svelte.ts";
  import Spinner from "./Spinner.svelte";
  import Summary from "./Summary.svelte";

  let { item }: { item: ViewItem } = $props();
  const p = $derived(suite.palette);

  /** Lines of a diff or code a question shows before its buttons (Ctrl-O shows all). */
  const PREVIEW_LINES = 12;
  /** Width of the glyph column (bullet, spinner, ✓) that results and text hang from. */
  const GUTTER = 20;

  const question = $derived(hasButtons(item.component));
  const open = $derived(ui.expanded || opened.has(item.ref));
  /** The input side is showing this card: the chat keeps a stub. */
  const stub = $derived(ui.panelRef === item.ref && !ui.expanded);
  /** Messages typed in this window need no label. */
  const from = $derived(item.label !== undefined && item.label !== suite.appId ? item.label : undefined);

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

<View style={{ paddingTop: p.space.m, paddingBottom: p.space.s }}>
  {#if item.kind === "user"}
    <View style={{ direction: "row", background: p.band, paddingX: p.space.l, paddingY: p.space.m }}>
      <Text style={{ width: GUTTER, color: p.accent, fontWeight: "bold" }}>›</Text>
      <View style={{ flex: 1, minWidth: 0 }}>
        <Component component={item.component} ref={item.ref} />
      </View>
      {#if from}<Text style={{ color: p.muted, marginLeft: p.space.m }}>{from}</Text>{/if}
    </View>
  {:else if item.kind === "agent"}
    <View style={{ paddingX: p.space.l, gap: p.space.s }}>
      {#if item.label}
        <Text style={{ color: p.accent, fontWeight: "bold" }}>{item.label}</Text>
      {/if}
      {#if item.component.type === "text"}
        <View style={{ direction: "row" }}>
          <Text style={{ width: GUTTER }}>●</Text>
          <View style={{ flex: 1, minWidth: 0 }}>
            <Component component={item.component} ref={item.ref} />
          </View>
        </View>
      {:else}
        <Component component={item.component} ref={item.ref} limit={ui.expanded ? undefined : PREVIEW_LINES} />
      {/if}
    </View>
  {:else if item.kind === "thought"}
    <View style={{ direction: "row", paddingX: p.space.l, color: p.muted, italic: true }}>
      <Text style={{ width: GUTTER, italic: false }}>∴</Text>
      <View style={{ flex: 1, minWidth: 0 }}>
        <Component component={item.component} ref={item.ref} />
      </View>
    </View>
  {:else if item.kind === "notice"}
    <View style={{ paddingX: p.space.l, paddingLeft: p.space.l + GUTTER, color: item.state === "failed" ? p.danger : p.muted }}>
      <Component component={item.component} ref={item.ref} />
    </View>
  {:else if stub}
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
            limit={question && !ui.expanded ? PREVIEW_LINES : undefined}
          />
        {:else}
          <View onpress={() => opened.toggle(item.ref)} style={{ cursor: "pointer", color: p.muted }}>
            <Summary component={item.component} ref={item.ref} />
          </View>
        {/if}
      </View>
    </View>
  {/if}
</View>
