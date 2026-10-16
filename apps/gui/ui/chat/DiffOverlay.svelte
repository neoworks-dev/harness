<script lang="ts">
  // A diff over the whole window, side by side: the old text on the left, the new on the right,
  // changed lines opposite each other. One list holds both sides, so they scroll together.
  // Esc or the close button puts it away.
  import { Button, List, Text, View } from "@neoworks/svelte-gpui/ui";
  import type { DiffComponent } from "@suite/protocol/types";
  import { diffStats, gutterWidth, sideBySide, type SideCell, type SideRow } from "../lib/diff.ts";
  import { diffColors } from "../lib/diff-colors.ts";
  import { suite } from "../state/suite.svelte.ts";
  import { ui } from "../state/ui.svelte.ts";

  const ROW_HEIGHT = 20;

  const p = $derived(suite.palette);
  const colors = $derived(diffColors(p));
  const diff = $derived<DiffComponent | undefined>(ui.diff);
  const rows = $derived.by(() => {
    if (diff === undefined) return [];
    return sideBySide(diff.lines);
  });
  const stats = $derived(diff === undefined ? { added: 0, removed: 0 } : diffStats(diff.lines));
  const numberWidth = $derived(
    Math.round((diff === undefined ? 1 : gutterWidth(diff.lines)) * p.text.code * 0.62) + 14,
  );

  function close(): void {
    ui.diff = undefined;
  }

  function background(cell: SideCell | undefined): string | undefined {
    if (cell?.kind === "add") return colors.add;
    if (cell?.kind === "remove") return colors.remove;
    return undefined;
  }

  function markColor(cell: SideCell): string {
    if (cell.kind === "add") return p.success;
    if (cell.kind === "remove") return p.danger;
    return p.muted;
  }
</script>

<svelte:window
  onkeydown={(event) => {
    if (diff !== undefined && event.key === "Escape") {
      event.preventDefault();
      close();
    }
  }}
/>

{#snippet side(cell: SideCell | undefined)}
  <View style={{ flex: 1, minWidth: 0, direction: "row", height: ROW_HEIGHT, align: "center", overflow: "hidden", background: background(cell) ?? p.bg }}>
    {#if cell}
      <Text style={{ width: numberWidth, textAlign: "right", paddingRight: p.space.m, color: p.muted, fontFamily: "mono", fontSize: p.text.code }}>{cell.no ?? ""}</Text>
      <Text style={{ width: 14, color: markColor(cell), fontFamily: "mono", fontSize: p.text.code }}>{cell.kind === "add" ? "+" : cell.kind === "remove" ? "−" : ""}</Text>
      <Text pre style={{ flex: 1, minWidth: 0, fontFamily: "mono", fontSize: p.text.code, truncate: true }}>{cell.text}</Text>
    {/if}
  </View>
{/snippet}

{#if diff}
  <View style={{ position: "absolute", inset: 0, background: p.bg }}>
    <View
      style={{ direction: "row", align: "center", gap: p.space.m, paddingX: p.space.l, paddingY: p.space.m, background: p.surface, borderBottomWidth: 1, borderColor: p.border }}
    >
      <Text style={{ fontWeight: "bold", flex: 1, minWidth: 0, truncate: true }}>{diff.path ?? "diff"}</Text>
      <Text style={{ color: p.success }}>+{stats.added}</Text>
      <Text style={{ color: p.danger }}>−{stats.removed}</Text>
      <Button
        onpress={close}
        style={{ paddingX: p.space.m, paddingY: p.space.s, radius: 6, background: p.band }}
        hover={{ background: p.border }}
      >
        <Text>Close · Esc</Text>
      </Button>
    </View>
    <View style={{ direction: "row", background: p.surface, borderBottomWidth: 1, borderColor: p.border }}>
      <Text style={{ flex: 1, minWidth: 0, paddingX: p.space.l, paddingY: p.space.s, color: p.muted }}>before</Text>
      <View style={{ width: 1, background: p.border }} />
      <Text style={{ flex: 1, minWidth: 0, paddingX: p.space.l, paddingY: p.space.s, color: p.muted }}>after</Text>
    </View>
    <List items={rows} rowHeight={ROW_HEIGHT} style={{ flex: 1, minHeight: 0 }}>
      {#snippet row(entry: SideRow)}
        {#if entry.kind === "hunk"}
          <View style={{ height: ROW_HEIGHT, justify: "center", paddingX: p.space.l, background: p.surface }}>
            <Text style={{ fontFamily: "mono", fontSize: p.text.code, color: p.muted, truncate: true }}>{entry.text}</Text>
          </View>
        {:else}
          <View style={{ direction: "row" }}>
            {@render side(entry.left)}
            <View style={{ width: 1, background: p.border }} />
            {@render side(entry.right)}
          </View>
        {/if}
      {/snippet}
    </List>
  </View>
{/if}
