<script lang="ts">
  // The close button every surface gets, whatever its program draws: the daemon closes it
  // (neo.surface.close on its ref), and every client sees it go.
  import { Button, Icon } from "@neoworks/svelte-gpui/ui";
  import xIcon from "@phosphor-icons/core/regular/x.svg";
  import type { ViewItem } from "@neoworks-dev/neo-protocol/types";
  import { useCtx } from "../../kernel/slots/context.ts";

  let { surface, size = 20 }: { surface: ViewItem; size?: number } = $props();

  const { daemon } = useCtx();
  const p = $derived(daemon.palette);
</script>

<Button
  onpress={() => void daemon.action(surface.ref, "neo.surface.close")}
  style={{ size, radius: p.radius.s, align: "center", justify: "center", color: p.muted, shrink: 0 }}
  hover={{ background: p.hover, color: p.fg }}
>
  <Icon svg={xIcon} size={Math.round(size * 0.6)} />
</Button>
