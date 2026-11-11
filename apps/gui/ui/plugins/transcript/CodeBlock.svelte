<script lang="ts">
  // Code or command output: mono on the surface colour, long lines scroll sideways. Capped, since
  // one text of many thousand lines makes every frame slow. A fenced block of the agent's prose
  // (`fenced`) has a header: its language, a button that wraps its lines instead of scrolling
  // them, and one that copies it.
  import { Button, Icon, Scroll, Text, View } from "@neoworks/svelte-gpui/ui";
  import wrapIcon from "@phosphor-icons/core/regular/text-align-left.svg";
  import { clip } from "../../lib/diff.ts";
  import { blend } from "../../lib/palette.ts";
  import { useCtx } from "../../kernel/slots/context.ts";
  import { forFile } from "../viewer/highlights.ts";
  import { fencePath } from "../viewer/syntax.ts";
  import { kindStyles } from "../viewer/syntax-colors.ts";
  import CopyButton from "./CopyButton.svelte";

  interface Props {
    code: string;
    /** Lines to show before "… N more lines"; all (up to a cap) when absent. */
    limit?: number;
    language?: string;
    /** With the header, and the corners of a card. */
    fenced?: boolean;
  }

  /** The most lines drawn, however much the result holds. */
  const CAP = 400;

  let { code, limit, language, fenced = false }: Props = $props();
  const { daemon } = useCtx();
  const p = $derived(daemon.palette);

  const lines = $derived(code.replace(/\n+$/, "").split("\n"));
  const clipped = $derived(clip(lines, limit ?? CAP));
  /** Highlighted as the viewer does (its syntax.ts) when the block names a language it knows. */
  const lights = $derived(language ? forFile(fencePath(language), lines) : undefined);
  const styles = $derived(kindStyles(p.syntax));
  let wrap = $state(false);

  const lineStyle = $derived({
    fontFamily: "mono" as const,
    fontSize: p.text.code,
    lineHeight: Math.round(p.text.code * 1.375),
    color: p.fg,
  });
</script>

<!-- The lines, each run of a highlighted one in its colour. -->
{#snippet body()}{#each clipped.shown as line, index}{#if index > 0}{"\n"}{/if}{#each lights?.tokens(index) ?? [{ text: line }] as token}<Text style={styles[token.kind ?? "plain"]}>{token.text}</Text>{/each}{/each}{/snippet}

<View style={{ background: p.surface, radius: p.radius.m, marginY: 4, overflow: "hidden" }}>
  {#if fenced}
    <View style={{ direction: "row", align: "center", gap: 4, paddingLeft: 14, paddingRight: 6, paddingTop: 4 }}>
      <Text style={{ flex: 1, minWidth: 0, truncate: true, fontFamily: "mono", fontSize: 11, color: blend(p.bg, p.fg, 72) }}>{language || "text"}</Text>
      <Button
        onpress={() => (wrap = !wrap)}
        style={{ size: 24, radius: 6, align: "center", justify: "center", color: wrap ? p.fg : p.muted, background: wrap ? p.hover : "transparent" }}
        hover={{ color: p.fg, background: p.hover }}
      >
        <Icon svg={wrapIcon} size={14} />
      </Button>
      <CopyButton text={code.replace(/\n+$/, "")} />
    </View>
  {/if}
  {#if wrap}
    <View style={{ padding: fenced ? [4, 14, 13, 14] : [13, 14] }}>
      <Text pre style={lineStyle}>{@render body()}</Text>
    </View>
  {:else}
    <Scroll horizontal>
      <View style={{ padding: fenced ? [4, 14, 13, 14] : [13, 14] }}>
        <Text pre style={{ ...lineStyle, nowrap: true }}>{@render body()}</Text>
      </View>
    </Scroll>
  {/if}
  {#if clipped.hidden > 0}
    <View style={{ paddingX: p.space.l, paddingBottom: p.space.m }}>
      <Text style={{ color: p.muted }}>… {clipped.hidden} more lines</Text>
    </View>
  {/if}
</View>
