<script lang="ts">
  // Shapes drawn over each other in one box of the width the parent gives, as tall as the first
  // shape's aspect says. Each is an SVG the Icon tints with its tone, so the lines are smooth.
  import { Icon, View } from "@neoworks/svelte-gpui/ui";
  import type { UiNode } from "@neoworks-dev/neo-protocol/types";
  import { useCtx } from "../../../kernel/slots/context.ts";
  import { toneColor } from "./look.ts";
  import { tone as toneOf } from "./props.ts";
  import { canvasHeight, shapeSvg } from "./shapes.ts";

  interface Props {
    shapes: UiNode[];
    width: number;
  }

  let { shapes, width }: Props = $props();
  const { daemon } = useCtx();
  const p = $derived(daemon.palette);

  const w = $derived(Math.max(1, Math.floor(width)));
  const height = $derived(canvasHeight(w, shapes));
</script>

<View style={{ width: w, height }}>
  {#each shapes as shape}
    <Icon
      svg={shapeSvg(shape, w, height)}
      color={toneColor(toneOf(shape), p)}
      style={{ position: "absolute", left: 0, top: 0, width: w, height }}
    />
  {/each}
</View>
