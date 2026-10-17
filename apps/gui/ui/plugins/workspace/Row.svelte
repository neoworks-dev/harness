<script lang="ts">
  // One row of the card: 32px, quiet until the pointer is over it. What is inside is the caller's.
  import type { Snippet } from "svelte";
  import { Button } from "@neoworks/svelte-gpui/ui";
  import { useCtx } from "../../kernel/slots/context.ts";
  import { rowHover, rowStyle } from "./look.ts";

  interface Props {
    onpress: () => void;
    disabled?: boolean;
    children: Snippet;
  }

  let { onpress, disabled = false, children }: Props = $props();

  const { daemon } = useCtx();
  const p = $derived(daemon.palette);
</script>

<Button {onpress} {disabled} style={rowStyle(p)} hover={rowHover(p)} disabledStyle={{ opacity: 0.45 }}>
  {@render children()}
</Button>
