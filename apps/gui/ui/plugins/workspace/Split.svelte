<script lang="ts">
  // A split button: the primary part does the usual thing, the chevron at its end opens a menu of
  // the others.
  import { Button, Icon, Text, View, type MenuItem } from "@neoworks/svelte-gpui/ui";
  import caretDown from "@phosphor-icons/core/regular/caret-down.svg";
  import { useCtx } from "../../kernel/slots/context.ts";
  import Dropdown from "./Dropdown.svelte";
  import { ROW, rowHover, rowStyle } from "./look.ts";

  interface Props {
    /** SVG markup. */
    icon: string;
    label: string;
    onpress: () => void;
    disabled?: boolean;
    /** Without items there is no chevron. */
    items: MenuItem[];
  }

  let { icon, label, onpress, disabled = false, items }: Props = $props();

  const { daemon } = useCtx();
  const p = $derived(daemon.palette);
</script>

<View style={{ direction: "row", align: "center", height: ROW }}>
  <Button
    {onpress}
    {disabled}
    style={{ ...rowStyle(p), flex: 1, minWidth: 0, radius: items.length > 0 ? [8, 0, 0, 8] : 8 }}
    hover={rowHover(p)}
    disabledStyle={{ opacity: 0.45 }}
  >
    <Icon svg={icon} size={16} color={p.muted} />
    <Text style={{ flex: 1, minWidth: 0, truncate: true }}>{label}</Text>
  </Button>
  {#if items.length > 0}
    <View style={{ width: 1, height: 16, shrink: 0, background: p.border }} />
    <Dropdown {items}>
      {#snippet trigger({ toggle })}
        <Button
          onpress={toggle}
          style={{ width: ROW, height: ROW, align: "center", justify: "center", radius: [0, 8, 8, 0], color: p.muted }}
          hover={rowHover(p)}
        >
          <Icon svg={caretDown} size={14} />
        </Button>
      {/snippet}
    </Dropdown>
  {/if}
</View>
