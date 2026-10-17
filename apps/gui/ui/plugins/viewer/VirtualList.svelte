<script lang="ts" generics="T">
  // A long list of rows of known heights, built only near the window. `List` measures every row it
  // builds, and each measure sets the others' places again, which makes a jump through a long diff
  // cost half a second. Here the heights come from `height` (a line of code is 20px, a header 36),
  // so the tops are a running sum and nothing is measured. Only a row that `measure` flags, as a
  // line may that wraps, reports what it came to, and the list moves its neighbours once for all
  // that were reported together.
  import { untrack, type Snippet } from "svelte";
  import { Scroll, View } from "@neoworks/svelte-gpui/ui";
  import type { Layout, ScrollPosition, Style } from "@neoworks/svelte-gpui/ui";
  import { indexAt, tops as place, visible } from "./window.ts";

  interface Props {
    items: readonly T[];
    /** Names a row; its measured height stays with it as the list changes. */
    key: (item: T) => string;
    /** How tall a row is, or is expected to be when `measure` says so. */
    height: (item: T) => number;
    /** The row's height is only a guess: it is measured as drawn. */
    measure?: (item: T) => boolean;
    row: Snippet<[T, number]>;
    /** Rows built past each edge of the window. */
    overscan?: number;
    onscroll?: (y: number) => void;
    style?: Style;
  }

  let { items, key, height, measure, row, overscan = 6, onscroll, style }: Props = $props();

  let scroller: ReturnType<typeof Scroll> | undefined = $state();
  let top = $state(0);
  let viewport = $state(0);
  /** What the rows that were measured came to, by key. Not reactive: `version` says when it changed. */
  const measured = new Map<string, number>();
  let version = $state(0);
  /** How much the rows above the window changed by since the last flush. */
  let above = 0;
  let flushing = false;

  const heightOf = (item: T): number => measured.get(key(item)) ?? height(item);
  /** Reading `version` as an argument is how a derived learns that `measured` changed. */
  const settled = (_version: number): void => {};
  const starts = $derived.by(() => {
    settled(version);
    return place(items.length, (index) => heightOf(items[index] as T));
  });
  const range = $derived(visible(starts, top, viewport, overscan));
  const shown = $derived.by(() => {
    const out: { item: T; index: number; key: string; top: number; height: number; flex: boolean }[] = [];
    for (let index = range.first; index < range.last; index += 1) {
      const item = items[index] as T;
      const flex = measure?.(item) ?? false;
      out.push({
        item,
        index,
        key: key(item),
        top: starts[index] ?? 0,
        height: (starts[index + 1] ?? 0) - (starts[index] ?? 0),
        flex,
      });
    }
    return out;
  });

  function scrolled(position: ScrollPosition): void {
    top = position.y;
    onscroll?.(position.y);
  }

  function laidOut(layout: Layout): void {
    viewport = layout.height;
  }

  /** A flagged row came to `size`. */
  function report(at: string, index: number, size: number): void {
    const known = untrack(() => measured.get(at) ?? height(items[index] as T));
    if (Math.abs(known - size) < 1) return;
    measured.set(at, size);
    if ((untrack(() => starts[index + 1]) ?? 0) <= top) above += size - known;
    if (flushing) return;
    flushing = true;
    // One move for the rows that reported in the same frame.
    setTimeout(() => {
      flushing = false;
      version += 1;
      if (above !== 0) {
        const by = above;
        above = 0;
        scroller?.scrollTo(Math.max(0, top + by));
      }
    }, 0);
  }

  /** The row at offset `y` of the list. */
  export function at(y: number): number {
    return indexAt(starts, y);
  }

  /** Where row `index` begins; the list's height for the one past the last. */
  export function topOf(index: number): number {
    return starts[Math.min(Math.max(index, 0), items.length)] ?? 0;
  }

  /**
   * Brings row `index` into view: to the top of the window (the default, `inset` pixels into the
   * row), or only as far as it takes.
   */
  export function scrollToIndex(
    index: number,
    options: { align?: "start" | "nearest"; inset?: number } = {},
  ): void {
    if (items.length === 0) return;
    const target = Math.min(Math.max(Math.round(index), 0), items.length - 1);
    const begin = topOf(target);
    const end = topOf(target + 1);
    const furthest = Math.max(0, topOf(items.length) - viewport);
    if (options.align === "nearest") {
      if (begin >= top && end <= top + viewport) return;
      scroller?.scrollTo(Math.min(begin < top ? begin : end - viewport, furthest));
      return;
    }
    scroller?.scrollTo(Math.min(begin + (options.inset ?? 0), furthest));
  }
</script>

<Scroll bind:this={scroller} {style} onscroll={scrolled} onlayout={laidOut}>
  <View style={{ height: starts[items.length] ?? 0, shrink: 0 }}>
    {#each shown as entry (entry.key)}
      <View
        style={{ position: "absolute", top: entry.top, left: 0, right: 0, height: entry.flex ? undefined : entry.height }}
        onresize={entry.flex ? (layout: Layout) => report(entry.key, entry.index, layout.height) : undefined}
      >
        {@render row(entry.item, entry.index)}
      </View>
    {/each}
  </View>
</Scroll>
