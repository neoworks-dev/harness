<script lang="ts">
  // The plan: the todo list of the agent and of each guest that has one, as its events leave it.
  import { Icon, Scroll, Text, View } from "@neoworks/svelte-gpui/ui";
  import checkCircle from "@phosphor-icons/core/fill/check-circle-fill.svg";
  import circle from "@phosphor-icons/core/regular/circle.svg";
  import circleHalf from "@phosphor-icons/core/regular/circle-half.svg";
  import listChecks from "@phosphor-icons/core/regular/list-checks.svg";
  import Card from "../../widgets/Card.svelte";
  import { useCtx } from "../../kernel/slots/context.ts";
  import { ownFirst, progress } from "./plan.ts";
  import { plans } from "./state.svelte.ts";

  const { daemon } = useCtx();

  const p = $derived(daemon.palette);
  const lists = $derived(ownFirst(plans.lists));
  const done = $derived(progress(lists));
</script>

{#if lists.length > 0}
  <Card title="Progress" icon={listChecks}>
    {#snippet end()}<Text style={{ fontSize: 12 }}>{done.done}/{done.total}</Text>{/snippet}
    <Scroll style={{ minHeight: 0, shrink: 1 }}>
      <View style={{ gap: 6 }}>
        {#each lists as list (list.agent ?? "")}
          {#if lists.length > 1}
            <Text style={{ color: p.muted, fontSize: 12, fontWeight: "medium", paddingTop: 2 }}>{list.agent ?? "agent"}</Text>
          {/if}
          {#each list.items as item, index (index)}
            {@const active = item.status === "in_progress"}
            {@const completed = item.status === "completed"}
            <View style={{ direction: "row", gap: 8 }}>
              <View style={{ height: 20, justify: "center", color: completed ? p.success : active ? p.accent : p.muted }}>
                <Icon svg={completed ? checkCircle : active ? circleHalf : circle} size={16} />
              </View>
              <Text
                style={{
                  flex: 1,
                  minWidth: 0,
                  lineHeight: 20,
                  fontWeight: active ? "medium" : "normal",
                  color: completed ? p.muted : p.fg,
                  strikethrough: completed,
                }}>{item.text}</Text
              >
            </View>
          {/each}
        {/each}
      </View>
    </Scroll>
  </Card>
{/if}
