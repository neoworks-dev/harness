<script lang="ts">
  // One issue or pull request in the list: its state, title and comment count, then who opened it,
  // its labels and how long ago it changed.
  import { Text, View } from "@neoworks/svelte-gpui/ui";
  import { rowFacts, shortAge, type Summary } from "../lib/github.ts";
  import { suite } from "../state/suite.svelte.ts";
  import { stateColor } from "./github-look.ts";

  interface Props {
    row: Summary;
    selected: boolean;
    nowMs: number;
    onpress: () => void;
  }

  let { row, selected, nowMs, onpress }: Props = $props();
  const p = $derived(suite.palette);
</script>

<View
  {onpress}
  hover={{ background: p.surface }}
  style={{
    direction: "row",
    gap: p.space.m,
    paddingX: p.space.l,
    paddingY: p.space.s,
    background: selected ? p.surface : "transparent",
    borderLeftWidth: 2,
    borderColor: selected ? p.accent : "transparent",
    cursor: "pointer",
  }}
>
  <Text style={{ color: stateColor(row.state, p) }}>●</Text>
  <View style={{ flex: 1, minWidth: 0 }}>
    <Text style={{ truncate: true, fontWeight: selected ? "bold" : "normal" }}>{row.title}</Text>
    <Text style={{ truncate: true, color: p.muted, fontSize: p.text.body - 1 }}>{rowFacts(row)}</Text>
  </View>
  <Text style={{ color: p.muted, fontSize: p.text.body - 1 }}>{row.state} · {shortAge(row.updatedMs, nowMs)}</Text>
</View>
