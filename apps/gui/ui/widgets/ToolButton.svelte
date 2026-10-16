<script lang="ts">
  // A square icon button for the toolbars around the conversation, lit while what it toggles is on.
  import { Button, Icon } from "@neoworks/svelte-gpui/ui";
  import { useCtx } from "../kernel/slots/context.ts";

  let { svg, onpress, active = false }: { svg: string; onpress: () => void; active?: boolean } = $props();

  const { daemon } = useCtx();
  const p = $derived(daemon.palette);
</script>

<Button
  {onpress}
  style={{
    size: 32,
    radius: p.radius.s,
    align: "center",
    justify: "center",
    color: active ? p.fg : p.side.icon,
    background: active ? p.side.active : "transparent",
  }}
  hover={{ background: p.side.hover, color: p.fg }}
>
  <Icon {svg} size={18} />
</Button>
