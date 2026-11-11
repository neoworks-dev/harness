<script lang="ts">
  // The `@` menu above the input: the files and folders the word at the cursor could be, the
  // folders' names ending in `/`. Like the `/` menu, the input keeps the keyboard; this draws and
  // takes presses.
  import { Scroll, Text, View } from "@neoworks/svelte-gpui/ui";
  import type { Layout, ScrollPosition } from "@neoworks/svelte-gpui/ui";
  import { useCtx } from "../../kernel/slots/context.ts";
  import { mentions } from "./mentions.svelte.ts";

  interface Props {
    /** A row was pressed. */
    onpick: (index: number) => void;
  }

  let { onpick }: Props = $props();

  const { daemon } = useCtx();
  const p = $derived(daemon.palette);
  const ROW = 28;
  const VISIBLE_ROWS = 8;

  let scroller: ReturnType<typeof Scroll> | undefined = $state();
  let top = 0;
  let viewport = 0;

  // Scrolls the highlighted row into view.
  $effect(() => {
    const start = mentions.selected * ROW;
    if (viewport === 0) return;
    if (start < top) scroller?.scrollTo(start);
    else if (start + ROW > top + viewport) scroller?.scrollTo(start + ROW - viewport);
  });

  /** A path as its directory (muted) and its name. */
  function parts(path: string): [string, string] {
    const cut = path.replace(/\/$/, "").lastIndexOf("/") + 1;
    return [path.slice(0, cut), path.slice(cut)];
  }
</script>

<View
  style={{
    background: p.surface,
    borderWidth: 1,
    borderColor: p.border,
    radius: 8,
    padding: 4,
    shadow: { y: 8, blur: 24, color: "#00000099" },
  }}
>
  <Scroll
    bind:this={scroller}
    style={{ maxHeight: ROW * VISIBLE_ROWS }}
    onscroll={(position: ScrollPosition) => (top = position.y)}
    onlayout={(box: Layout) => (viewport = box.height)}
  >
    {#each mentions.candidates as candidate, index (candidate.path)}
      {@const [dir, name] = parts(candidate.path)}
      <View
        style={{
          direction: "row",
          align: "center",
          height: ROW,
          shrink: 0,
          paddingX: p.space.m,
          radius: 5,
          background: index === mentions.selected ? p.band : "transparent",
        }}
        onhover={(hovered: boolean) => hovered && (mentions.selected = index)}
        onpress={() => onpick(index)}
      >
        <Text style={{ minWidth: 0, shrink: 1, truncate: true, color: p.muted }}>{dir}</Text>
        <Text style={{ shrink: 0, color: candidate.dir ? p.accent : p.fg }}>{name}</Text>
      </View>
    {/each}
  </Scroll>
</View>
