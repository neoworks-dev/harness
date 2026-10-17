<script lang="ts">
  // The small button a row shows in place of its age while the pointer is on it ("Settle",
  // "Un-settle"), with its name in a tooltip.
  import { Button, Icon, Tooltip } from "@neoworks/svelte-gpui/ui";
  import { useCtx } from "../../kernel/slots/context.ts";

  let { svg, label, onpress }: { svg: string; label: string; onpress: () => void } = $props();
  const { daemon } = useCtx();
  const p = $derived(daemon.palette);
</script>

<Tooltip text={label} side="top" delay={300} anchorStyle={{ alignSelf: "center" }}>
  <Button
    {onpress}
    style={{ size: 22, radius: 6, align: "center", justify: "center", color: p.muted }}
    hover={{ background: p.border, color: p.fg }}
  >
    <Icon {svg} size={14} />
  </Button>
</Tooltip>
