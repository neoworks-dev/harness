<script lang="ts">
  // Text with the words that were searched for picked out. Draws as one paragraph; give it the
  // style of the line, such as `truncate`. The host does not mark runs inside truncated text, so
  // while something is picked out the line is cut at its edge (`maxLines: 1`) without the ellipsis.
  import { Text } from "@neoworks/svelte-gpui/ui";
  import type { Style } from "@neoworks/svelte-gpui/ui";
  import { useCtx } from "../../kernel/slots/context.ts";
  import { highlight } from "./search.ts";

  let { text, terms, style }: { text: string; terms: readonly string[]; style?: Style } = $props();
  const { daemon } = useCtx();
  const p = $derived(daemon.palette);
  const runs = $derived(highlight(text, terms));
  const lineStyle = $derived(
    style?.truncate && runs.some((run) => run.hit)
      ? { ...style, truncate: false, maxLines: 1 }
      : style,
  );
</script>

<Text style={lineStyle}>{#each runs as run}<Text style={run.hit ? { color: p.accent, fontWeight: "semibold" } : undefined}>{run.text}</Text>{/each}</Text>
