<script lang="ts">
  // A small square icon button with a tooltip, for the viewer's header and toolbars, lit while what
  // it toggles is on.
  import { Button, Icon, Tooltip } from "@neoworks/svelte-gpui/ui";
  import { useCtx } from "../../kernel/slots/context.ts";

  interface Props {
    /** SVG markup. */
    svg: string;
    label: string;
    onpress: () => void;
    active?: boolean;
    disabled?: boolean;
    size?: number;
  }

  let { svg, label, onpress, active = false, disabled = false, size = 26 }: Props = $props();

  const { daemon } = useCtx();
  const p = $derived(daemon.palette);
</script>

<Tooltip text={label} side="bottom" anchorStyle={{ alignSelf: "center" }}>
  <Button
    {onpress}
    {disabled}
    style={{
      size,
      radius: 6,
      align: "center",
      justify: "center",
      color: active ? p.fg : p.muted,
      background: active ? p.raised : "transparent",
    }}
    hover={{ background: p.hover, color: p.fg }}
    disabledStyle={{ opacity: 0.4 }}
  >
    <Icon {svg} size={15} />
  </Button>
</Tooltip>
