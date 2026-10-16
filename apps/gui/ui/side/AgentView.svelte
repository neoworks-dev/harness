<script lang="ts">
  // One agent's own chat, in place of the session's: what a subagent or guest did and said,
  // read-only, drawn like the main chat. Esc, q or Enter goes back.
  import { untrack } from "svelte";
  import { Button, List, Text, View } from "@neoworks/svelte-gpui/ui";
  import type { KeyEvent } from "@neoworks/svelte-gpui/ui";
  import type { ViewItem } from "@suite/protocol/types";
  import { useCtx } from "../kernel/slots/context.ts";
  import Slot from "../kernel/slots/Slot.svelte";
  import { suite } from "../state/suite.svelte.ts";
  import { agentView } from "./agentview.svelte.ts";

  const { transcript } = useCtx();

  const p = $derived(suite.palette);
  const title = $derived(agentView.title);
  const items = $derived(transcript.visible(agentView.items));
  let keys: ReturnType<typeof View> | undefined = $state();
  let list: ReturnType<typeof List<ViewItem>> | undefined = $state();

  // The view has the keyboard from the moment it opens.
  $effect(() => {
    if (agentView.isOpen) untrack(() => keys?.focus());
  });

  function key(event: KeyEvent): void {
    if (event.ctrl || event.alt || event.meta) return;
    switch (event.key) {
      case "Escape":
      case "q":
      case "Enter":
      case "Backspace":
        agentView.close();
        break;
      case "Home":
      case "g":
        list?.scrollToIndex(0);
        break;
      case "End":
      case "G":
        list?.scrollToIndex(Math.max(0, items.length - 1), { align: "end" });
        break;
      default:
        return;
    }
    event.preventDefault();
    event.stopPropagation();
  }
</script>

{#if agentView.isOpen && title}
  <View bind:this={keys} focusable onkey={key} style={{ flex: 1, minHeight: 0 }}>
    <View
      style={{
        direction: "row",
        align: "center",
        gap: p.space.m,
        paddingX: p.space.l,
        paddingY: p.space.s,
        background: p.surface,
        borderBottomWidth: 1,
        borderColor: p.border,
      }}
    >
      <Button
        onpress={() => agentView.close()}
        style={{ paddingX: p.space.m, radius: 4, color: p.accent }}
        hover={{ background: p.band }}
      >
        <Text>◂ back</Text>
      </Button>
      <Text style={{ color: p.accent, fontWeight: "bold" }}>{title.harness}</Text>
      {#if title.model}<Text style={{ color: p.muted }}>· {title.model}</Text>{/if}
      <Text style={{ flex: 1, minWidth: 0, truncate: true, color: p.muted }}>{title.task}</Text>
    </View>
    {#if items.length === 0}
      <View style={{ flex: 1, align: "center", justify: "center" }}>
        <Text style={{ color: p.muted }}>nothing from this agent yet</Text>
      </View>
    {:else}
      <List
        bind:this={list}
        {items}
        key={(item: ViewItem) => item.ref}
        stickToEnd
        style={{ flex: 1, minHeight: 0 }}
      >
        {#snippet row(item: ViewItem)}
          <Slot name="chat.item" key={item.kind} props={{ item }} />
        {/snippet}
      </List>
    {/if}
    <View style={{ paddingX: p.space.l, paddingY: p.space.s, borderTopWidth: 1, borderColor: p.border }}>
      <Text style={{ color: p.muted }}>esc back · ctrl-o full results · ctrl-t thoughts</Text>
    </View>
  </View>
{/if}
