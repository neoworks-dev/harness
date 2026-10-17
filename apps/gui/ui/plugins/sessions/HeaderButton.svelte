<script lang="ts">
  // A small icon button in the sidebar's header, muted until the pointer is on it, with its name
  // in a tooltip. What it shows can be given in place of an icon, such as a project's favicon.
  import type { Snippet } from "svelte";
  import { Button, Icon, Tooltip } from "@neoworks/svelte-gpui/ui";
  import type { PointerEvent } from "@neoworks/svelte-gpui/ui";
  import { useCtx } from "../../kernel/slots/context.ts";

  interface Props {
    label: string;
    /** SVG markup, drawn at 16px unless `face` is given. */
    svg?: string;
    face?: Snippet;
    /** What it controls is on: a lit background. */
    active?: boolean;
    onpress: (event: PointerEvent) => void;
  }

  let { label, svg, face, active = false, onpress }: Props = $props();
  const { daemon } = useCtx();
  const p = $derived(daemon.palette);
</script>

<Tooltip text={label} side="bottom" anchorStyle={{ alignSelf: "center" }}>
  <Button
    {onpress}
    style={{ size: 28, radius: p.radius.s, align: "center", justify: "center", color: p.side.icon, background: active ? p.side.hover : "transparent" }}
    hover={{ background: p.side.hover, color: p.fg }}
  >
    {#if face}{@render face()}{:else if svg}<Icon {svg} size={16} />{/if}
  </Button>
</Tooltip>
