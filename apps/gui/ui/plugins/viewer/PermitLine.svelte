<script lang="ts">
  // One line of the edit under review: its old and new numbers, the mark and the text on the green
  // or red of a change, lit when the cursor (or the visual run) is on it. A press puts the cursor
  // there, shift stretches the run to it, a double press comments on it.
  import { Text, View } from "@neoworks/svelte-gpui/ui";
  import type { DiffLine } from "@neoworks-dev/neo-protocol/types";
  import { viewerColors } from "../../lib/diff-colors.ts";
  import { useCtx } from "../../kernel/slots/context.ts";
  import Code from "./Code.svelte";
  import { withMarks } from "./inline.ts";
  import { backgrounds, markOf, numberWidth } from "./look.ts";
  import { permit } from "./permit.svelte.ts";
  import { LINE_HEIGHT } from "./rows.ts";
  import type { Token } from "./syntax.ts";

  interface Props {
    /** The line's place among all the edit's lines. */
    at: number;
    line: DiffLine;
    tokens?: Token[];
    partner?: string;
    digits: number;
    /** A comment is on this line. */
    commented: boolean;
  }

  let { at, line, tokens, partner, digits, commented }: Props = $props();

  const { daemon } = useCtx();
  const p = $derived(daemon.palette);
  const colors = $derived(viewerColors(p));
  const inRun = $derived(
    permit.visual && at >= Math.min(permit.anchor, permit.head) && at <= Math.max(permit.anchor, permit.head),
  );
  const cursor = $derived(permit.head === at);
  const look = $derived(backgrounds(colors, line.kind, inRun));
  const runs = $derived(withMarks(line.text, tokens, partner));
  const number = $derived({
    width: numberWidth(digits),
    textAlign: "right" as const,
    paddingRight: 8,
    fontFamily: "mono",
    fontSize: 12,
    lineHeight: LINE_HEIGHT,
    color: cursor ? p.fg : p.muted,
  });

  let lastPress = 0;
  function press(shift: boolean): void {
    const now = Date.now();
    const twice = !shift && now - lastPress < 350 && permit.head === at;
    lastPress = now;
    permit.press(at, shift);
    if (twice) permit.startComment();
  }
</script>

<View
  onpress={(event) => press(event.shift)}
  style={{ direction: "row", height: LINE_HEIGHT, background: look.body, cursor: "text" }}
>
  <!-- The cursor: a bar at the line's start. -->
  <View style={{ width: 3, shrink: 0, background: cursor ? p.accent : "transparent" }} />
  <View style={{ direction: "row", shrink: 0, background: look.numbers }}>
    <Text style={number}>{line.old ?? ""}</Text>
    <Text style={number}>{line.new ?? ""}</Text>
  </View>
  <Text
    style={{
      width: 18,
      shrink: 0,
      textAlign: "center",
      fontFamily: "mono",
      fontSize: 13,
      lineHeight: LINE_HEIGHT,
      color: line.kind === "add" ? p.success : p.danger,
    }}
  >{markOf(line.kind)}</Text>
  <Code text={line.text} tokens={runs} mark={look.mark} />
  {#if commented}
    <View style={{ position: "absolute", left: 6, top: 7, size: 6, radius: 3, background: p.accent }} />
  {/if}
</View>
