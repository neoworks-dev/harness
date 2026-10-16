<script lang="ts">
  // A composer picker that opens a menu above itself: an optional heading, then the choices, each
  // with an icon, a description and a badge if it has them. The current one is highlighted.
  import type { Snippet } from "svelte";
  import { Button, Icon, Popover, Text, View } from "@neoworks/svelte-gpui/ui";
  import { useCtx } from "../../kernel/slots/context.ts";
  import Picker from "./Picker.svelte";

  export interface Choice {
    value: string;
    label: string;
    description?: string;
    /** SVG markup. */
    icon?: string;
    /** A short tag after the label, such as "Default". */
    badge?: string;
  }

  interface Props {
    label: string;
    choices: readonly Choice[];
    current: string | undefined;
    onselect: (value: string) => void;
    /** A heading over the choices. */
    title?: string;
    /** The picker's own icon. */
    icon?: Snippet;
    width?: number;
  }

  let { label, choices, current, onselect, title, icon, width = 260 }: Props = $props();

  const { daemon } = useCtx();
  const p = $derived(daemon.palette);
  let open = $state(false);
</script>

<Popover
  bind:open
  side="top"
  align="start"
  offset={8}
  style={{
    width,
    padding: 6,
    gap: 2,
    background: p.surface,
    radius: 14,
    borderWidth: 1,
    borderColor: p.border,
    shadow: { y: 8, blur: 24, color: "rgba(0, 0, 0, 0.5)" },
  }}
>
  {#snippet trigger({ toggle })}
    <Picker {label} onpress={toggle} {icon} />
  {/snippet}
  {#snippet children({ close })}
    {#if title}
      <Text style={{ paddingX: 10, paddingTop: 6, paddingBottom: 4, fontSize: 12, fontWeight: "medium", color: p.muted }}>{title}</Text>
    {/if}
    {#each choices as choice (choice.value)}
      <Button
        onpress={() => {
          onselect(choice.value);
          close("select");
        }}
        style={{
          direction: "column",
          align: "stretch",
          gap: 2,
          paddingX: 10,
          paddingY: 7,
          radius: p.radius.s,
          background: choice.value === current ? p.hover : "transparent",
          color: p.fg,
        }}
        hover={{ background: p.raised }}
      >
        <View style={{ direction: "row", align: "center", gap: 8 }}>
          {#if choice.icon}<Icon svg={choice.icon} size={16} />{/if}
          <Text style={{ fontWeight: "medium" }}>{choice.label}</Text>
          {#if choice.badge}
            <Text style={{ fontSize: 11, color: p.muted, paddingX: 5, paddingY: 1, radius: 4, borderWidth: 1, borderColor: p.border }}>{choice.badge}</Text>
          {/if}
        </View>
        {#if choice.description}
          <Text style={{ fontSize: 12, color: p.muted }}>{choice.description}</Text>
        {/if}
      </Button>
    {/each}
  {/snippet}
</Popover>
