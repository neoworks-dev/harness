<script lang="ts">
  // A menu under whatever opens it, in the look of the composer's pickers: a raised card with
  // rounded rows. Up and Down, Enter and a letter work as in any menu.
  import { untrack, type Snippet } from "svelte";
  import { Menu, Popover, type MenuItem, type PopoverApi, type Style } from "@neoworks/svelte-gpui/ui";
  import { useCtx } from "../../kernel/slots/context.ts";

  interface Props {
    items: MenuItem[];
    /** What opens the menu: it gets `toggle`. */
    trigger: Snippet<[PopoverApi]>;
    /** Called each time the menu opens, e.g. to ask for its items. */
    onopen?: () => void;
    align?: "start" | "end";
    width?: number;
    /** The menu scrolls past this height. */
    maxHeight?: number;
    /** The wrapper around the trigger; it is as wide as the trigger unless this stretches it. */
    anchorStyle?: Style;
  }

  let { items, trigger, onopen, align = "end", width = 232, maxHeight, anchorStyle }: Props = $props();

  const { daemon } = useCtx();
  const p = $derived(daemon.palette);
  let open = $state(false);

  $effect(() => {
    if (open) untrack(() => onopen?.());
  });
</script>

<Popover
  bind:open
  side="bottom"
  {align}
  offset={6}
  focusPanel={false}
  {trigger}
  {anchorStyle}
  style={{
    padding: 2,
    background: p.surface,
    radius: 14,
    borderWidth: 1,
    borderColor: p.border,
    shadow: { y: 8, blur: 24, color: "rgba(0, 0, 0, 0.5)" },
  }}
>
  {#snippet children({ close })}
    <Menu
      {items}
      {width}
      {maxHeight}
      onclose={() => close("select")}
      itemStyle={{ radius: 8, paddingX: 10, fontSize: 13, fontWeight: "medium" }}
    />
  {/snippet}
</Popover>
