<script lang="ts">
  // Parsed markdown: paragraphs, headings, lists, quotes, code and rules, drawn row by row
  // (lib/markdown.ts flattens the nesting). Inline styles are nested `Text` runs inside the
  // paragraph's `Text`.
  import { Text, View } from "@neoworks/svelte-gpui/ui";
  import type { Block } from "@suite/protocol/types";
  import { proseLineHeight } from "./layout.ts";
  import { indentCells, markdownRows, type RowGap } from "./markdown.ts";
  import { useCtx } from "../../kernel/slots/context.ts";
  import CodeBlock from "./CodeBlock.svelte";
  import Runs from "./Runs.svelte";

  interface Props {
    blocks: readonly Block[];
    /** Font size of a paragraph; headings are sized from it. */
    size: number;
  }

  let { blocks, size }: Props = $props();
  const { daemon } = useCtx();
  const p = $derived(daemon.palette);

  const rows = $derived(markdownRows(blocks));
  /** Width of one character, to turn indents in characters into pixels. */
  const charWidth = $derived(size * 0.6);

  /** Headings are 20, 18 and then 16 pixels for text of the usual size. */
  function headingSize(level: number): number {
    const step = Math.max(size - 14, 0);
    if (level <= 1) return 20 + step;
    return (level === 2 ? 18 : 16) + step;
  }

  /** Paragraphs are 10 pixels apart, the items of a list 4. */
  function gapOf(gap: RowGap): number {
    if (gap === "loose") return 10;
    return gap === "tight" ? 4 : 0;
  }
</script>

<View>
  {#each rows as row}
    <View style={{ direction: "row", marginTop: gapOf(row.gap), color: row.quote > 0 ? p.muted : undefined }}>
      {#each { length: row.quote } as _}
        <View style={{ width: 2, background: p.border, marginRight: p.space.m }} />
      {/each}
      {#each indentCells(row) as cell}
        <Text style={{ width: Math.round(cell.chars * charWidth), lineHeight: proseLineHeight(size), color: cell.marker?.done ? p.success : p.muted }}>{cell.marker?.text ?? ""}</Text>
      {/each}
      {#if row.kind === "paragraph"}
        <Text style={{ flex: 1, minWidth: 0, lineHeight: proseLineHeight(size) }}><Runs spans={row.spans} /></Text>
      {:else if row.kind === "heading"}
        <Text style={{ flex: 1, minWidth: 0, fontSize: headingSize(row.level), fontWeight: "semibold", lineHeight: proseLineHeight(headingSize(row.level)) }}><Runs spans={row.spans} /></Text>
      {:else if row.kind === "code"}
        <View style={{ flex: 1, minWidth: 0 }}><CodeBlock code={row.code} language={row.language} fenced /></View>
      {:else}
        <View style={{ flex: 1, height: 1, background: p.border, marginY: p.space.s, alignSelf: "center" }} />
      {/if}
    </View>
  {/each}
</View>
