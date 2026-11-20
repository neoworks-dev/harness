<script lang="ts">
  // The session's shared todo list, which the user and every agent change: the open items, then
  // the finished ones folded into "N done". Pressing an item checks it off or opens it again; the
  // list changes when the daemon's event comes back.
  import { Icon, Scroll, Text, View } from "@neoworks/svelte-gpui/ui";
  import checkSquare from "@phosphor-icons/core/fill/check-square-fill.svg";
  import square from "@phosphor-icons/core/regular/square.svg";
  import caretDown from "@phosphor-icons/core/regular/caret-down.svg";
  import caretRight from "@phosphor-icons/core/regular/caret-right.svg";
  import checklist from "@phosphor-icons/core/regular/checks.svg";
  import type { TodoItem } from "@neoworks-dev/neo-protocol/types";
  import Card from "../../widgets/Card.svelte";
  import { useCtx } from "../../kernel/slots/context.ts";
  import { todos } from "./state.svelte.ts";
  import { splitTodos, toggleArgs } from "./todos.ts";

  const { daemon } = useCtx();
  const p = $derived(daemon.palette);
  const split = $derived(splitTodos(todos.items));
  let showDone = $state(false);

  async function toggle(item: TodoItem): Promise<void> {
    try {
      await daemon.action("todos", "neo.todos", toggleArgs(item));
    } catch (error) {
      daemon.error = error instanceof Error ? error.message : String(error);
    }
  }
</script>

{#snippet row(item: TodoItem)}
  <View
    onpress={() => void toggle(item)}
    hover={{ background: p.hover }}
    style={{ direction: "row", gap: 8, paddingX: 4, paddingY: 2, radius: 6, cursor: "pointer" }}
  >
    <View style={{ height: 20, justify: "center", color: item.done ? p.success : p.muted }}>
      <Icon svg={item.done ? checkSquare : square} size={16} />
    </View>
    <Text
      style={{
        flex: 1,
        minWidth: 0,
        lineHeight: 20,
        color: item.done ? p.muted : p.fg,
        strikethrough: item.done,
      }}>{item.text}</Text
    >
  </View>
{/snippet}

{#if todos.items.length > 0}
  <Card title="Todos" icon={checklist}>
    {#snippet end()}<Text style={{ fontSize: 12 }}>{split.done.length}/{todos.items.length}</Text>{/snippet}
    <Scroll style={{ minHeight: 0, shrink: 1 }}>
      <View style={{ gap: 2 }}>
        {#each split.open as item (item.id)}
          {@render row(item)}
        {/each}
        {#if split.done.length > 0}
          <View
            onpress={() => (showDone = !showDone)}
            hover={{ color: p.fg }}
            style={{ direction: "row", align: "center", gap: 6, paddingX: 4, paddingTop: 4, color: p.muted, cursor: "pointer" }}
          >
            <Icon svg={showDone ? caretDown : caretRight} size={12} />
            <Text style={{ fontSize: 12 }}>{split.done.length} done</Text>
          </View>
          {#if showDone}
            {#each split.done as item (item.id)}
              {@render row(item)}
            {/each}
          {/if}
        {/if}
      </View>
    </Scroll>
  </Card>
{/if}
