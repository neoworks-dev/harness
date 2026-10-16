<script lang="ts">
  // Code or command output: mono on the surface colour, long lines scroll sideways. Capped, since
  // one text of many thousand lines makes every frame slow.
  import { Scroll, Text, View } from "@neoworks/svelte-gpui/ui";
  import { clip } from "../../lib/diff.ts";
  import { useCtx } from "../../kernel/slots/context.ts";

  interface Props {
    code: string;
    /** Lines to show before "… N more lines"; all (up to a cap) when absent. */
    limit?: number;
  }

  /** The most lines drawn, however much the result holds. */
  const CAP = 400;

  let { code, limit }: Props = $props();
  const { daemon } = useCtx();
  const p = $derived(daemon.palette);

  const lines = $derived(code.replace(/\n+$/, "").split("\n"));
  const clipped = $derived(clip(lines, limit ?? CAP));
  const text = $derived(clipped.shown.join("\n"));
</script>

<View style={{ background: p.surface, radius: p.radius.m, marginY: 4 }}>
  <Scroll horizontal>
    <View style={{ padding: [13, 14] }}>
      <Text
        pre
        style={{
          fontFamily: "mono",
          fontSize: p.text.code,
          lineHeight: Math.round(p.text.code * 1.375),
          color: p.fg,
          nowrap: true,
        }}>{text}</Text>
    </View>
  </Scroll>
  {#if clipped.hidden > 0}
    <View style={{ paddingX: p.space.l, paddingBottom: p.space.m }}>
      <Text style={{ color: p.muted }}>… {clipped.hidden} more lines</Text>
    </View>
  {/if}
</View>
