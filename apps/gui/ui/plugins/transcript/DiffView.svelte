<script lang="ts">
  // A diff in the chat: unified, with the added and removed colouring and line numbers. Cut to a
  // few lines unless expanded. Pressing it opens the viewer beside the chat.
  import { Text, View } from "@neoworks/svelte-gpui/ui";
  import type { DiffComponent } from "@neoworks-dev/neo-protocol/types";
  import { lineHeightOf } from "./components.ts";
  import { clip, diffStats, gutterWidth } from "../../lib/diff.ts";
  import { useCtx } from "../../kernel/slots/context.ts";
  import { diffColors } from "../../lib/diff-colors.ts";
  import { forDiff } from "../viewer/highlights.ts";
  import { kindStyles } from "../viewer/syntax-colors.ts";

  interface Props {
    diff: DiffComponent;
    /** Lines to show before "… N more lines"; all (up to a cap) when absent. */
    limit?: number;
    /** Its own line with the path and counts; off where a row above says them already. */
    header?: boolean;
  }

  /** The most lines drawn in the chat; the viewer is virtualised and shows all. */
  const CAP = 400;

  let { diff, limit, header = true }: Props = $props();
  const { daemon, viewer } = useCtx();
  const p = $derived(daemon.palette);
  const colors = $derived(diffColors(p));

  const stats = $derived(diffStats(diff.lines));
  const clipped = $derived(clip(diff.lines, limit ?? CAP));
  const numberWidth = $derived(Math.round(gutterWidth(diff.lines) * p.text.code * 0.62) + 12);
  const lineHeight = $derived(lineHeightOf(p.text.code));
  /** Highlighted as the viewer does: the old and new texts each scanned as the code they are. */
  const lights = $derived(forDiff(diff.path ?? "", diff.lines));
  const styles = $derived(kindStyles(p.syntax));

  function mark(kind: string): string {
    if (kind === "add") return "+";
    if (kind === "remove") return "−";
    return " ";
  }

  function background(kind: string): string | undefined {
    if (kind === "add") return colors.add;
    if (kind === "remove") return colors.remove;
    return undefined;
  }

  function markColor(kind: string): string {
    if (kind === "add") return p.success;
    if (kind === "remove") return p.danger;
    return colors.gutter;
  }
</script>

<View
  onpress={() => viewer.showDiff(diff)}
  style={{ background: p.surface, radius: 6, cursor: "pointer", overflow: "hidden" }}
  hover={{ background: p.band }}
>
  {#if header}
    <View style={{ direction: "row", gap: p.space.m, paddingX: p.space.l, paddingY: p.space.s, align: "baseline" }}>
      <Text style={{ fontWeight: "bold", flex: 1, minWidth: 0, truncate: true }}>{diff.path ?? "diff"}</Text>
      <Text style={{ color: p.success }}>+{stats.added}</Text>
      <Text style={{ color: p.danger }}>−{stats.removed}</Text>
      <Text style={{ color: p.muted }}>open in the viewer</Text>
    </View>
  {/if}
  {#each clipped.shown as line, index}
    {#if line.kind === "hunk"}
      <View style={{ paddingX: p.space.l, paddingY: 2 }}>
        <Text style={{ fontFamily: "mono", fontSize: p.text.code, color: p.muted }}>{line.text}</Text>
      </View>
    {:else}
      <View style={{ direction: "row", paddingX: p.space.s, background: background(line.kind) }}>
        <Text style={{ width: numberWidth, textAlign: "right", paddingRight: 6, color: colors.gutter, fontFamily: "mono", fontSize: p.text.code, lineHeight }}>{line.old ?? ""}</Text>
        <Text style={{ width: numberWidth, textAlign: "right", paddingRight: 6, color: colors.gutter, fontFamily: "mono", fontSize: p.text.code, lineHeight }}>{line.new ?? ""}</Text>
        <Text style={{ width: 18, textAlign: "center", color: markColor(line.kind), fontFamily: "mono", fontSize: p.text.code, lineHeight }}>{mark(line.kind)}</Text>
        <Text pre style={{ flex: 1, minWidth: 0, fontFamily: "mono", fontSize: p.text.code, lineHeight }}>{#each lights?.tokens(index) ?? [{ text: line.text }] as token}<Text style={styles[token.kind ?? "plain"]}>{token.text}</Text>{/each}</Text>
      </View>
    {/if}
  {/each}
  {#if clipped.hidden > 0}
    <View style={{ paddingX: p.space.l, paddingY: p.space.s }}>
      <Text style={{ color: p.muted }}>… {clipped.hidden} more lines (press to see all)</Text>
    </View>
  {/if}
</View>
