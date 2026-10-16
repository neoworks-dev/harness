<script lang="ts">
  // The plan: the todo list of the agent and of each guest that has one, as its events leave it.
  import { Scroll, Text, View } from "@neoworks/svelte-gpui/ui";
  import { glyphOf, ownFirst, progress } from "../lib/plan.ts";
  import { suite } from "../state/suite.svelte.ts";
  import { activity } from "./activity.svelte.ts";

  const p = $derived(suite.palette);
  const lists = $derived(ownFirst(activity.plans));
  const done = $derived(progress(lists));
</script>

<View style={{ gap: p.space.s, minHeight: 0, shrink: 1 }}>
  <View style={{ direction: "row", gap: p.space.s }}>
    <Text style={{ color: p.muted, fontWeight: "bold" }}>Plan</Text>
    <Text style={{ color: p.muted }}>· {done.done}/{done.total}</Text>
  </View>
  <Scroll style={{ minHeight: 0, shrink: 1 }}>
    <View style={{ gap: p.space.s }}>
      {#each lists as list (list.agent ?? "")}
        {#if lists.length > 1}
          <Text style={{ color: p.muted, fontWeight: "bold" }}>{list.agent ?? "agent"}</Text>
        {/if}
        {#each list.items as item, index (index)}
          {@const active = item.status === "in_progress"}
          <View style={{ direction: "row", gap: p.space.m }}>
            <Text
              style={{
                color: item.status === "completed" ? p.success : active ? p.accent : p.muted,
              }}>{glyphOf(item.status)}</Text
            >
            <Text
              style={{
                flex: 1,
                minWidth: 0,
                fontWeight: active ? "bold" : "normal",
                color: item.status === "completed" ? p.muted : p.fg,
              }}>{item.text}</Text
            >
          </View>
        {/each}
      {/each}
    </View>
  </Scroll>
</View>
