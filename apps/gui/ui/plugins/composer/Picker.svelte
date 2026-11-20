<script lang="ts">
  // One of the composer's pickers (harness, model, approvals): an icon, the current value and a
  // caret, quiet until the pointer is over it.
  import type { Snippet } from "svelte";
  import { Button, Icon, Text } from "@neoworks/svelte-gpui/ui";
  import caret from "@phosphor-icons/core/regular/caret-down.svg";
  import { useCtx } from "../../kernel/slots/context.ts";

  let {
    label,
    onpress,
    icon,
    color,
  }: { label: string; onpress: () => void; icon?: Snippet; /** Muted when absent. */ color?: string } =
    $props();

  const { daemon } = useCtx();
  const p = $derived(daemon.palette);
</script>

<Button
  {onpress}
  style={{ height: 28, paddingX: 10, gap: 6, align: "center", radius: p.radius.s, color: color ?? p.muted }}
  hover={{ background: p.hover, color: color ?? p.fg }}
>
  {@render icon?.()}
  <Text style={{ fontWeight: "medium" }}>{label}</Text>
  <Icon svg={caret} size={14} />
</Button>
