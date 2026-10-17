<script lang="ts">
  // The sidebar's search field: a magnifying glass, the text and, once there is some, a × that
  // clears it. Muted until it has the keyboard.
  import { Button, Icon, Input, View } from "@neoworks/svelte-gpui/ui";
  import type { KeyEvent } from "@neoworks/svelte-gpui/ui";
  import cross from "@phosphor-icons/core/regular/x.svg";
  import search from "@phosphor-icons/core/regular/magnifying-glass.svg";
  import { useCtx } from "../../kernel/slots/context.ts";

  interface Props {
    value: string;
    /** Comes before the field handles the key. */
    onkey: (event: KeyEvent) => void;
  }

  let { value = $bindable(), onkey }: Props = $props();
  const { daemon } = useCtx();
  const p = $derived(daemon.palette);
  let field: ReturnType<typeof Input> | undefined = $state();
  let focused = $state(false);

  export function focus(): void {
    field?.focus();
  }

  export function blur(): void {
    field?.blur();
  }
</script>

<View
  style={{
    flex: 1,
    minWidth: 0,
    direction: "row",
    align: "center",
    gap: 8,
    height: 32,
    paddingLeft: 10,
    paddingRight: 5,
    radius: p.radius.s,
    borderWidth: 1,
    borderColor: focused ? p.border : "transparent",
    background: focused ? p.side.active : p.side.hover,
    color: focused ? p.fg : p.side.icon,
    transition: 120,
  }}
>
  <Icon svg={search} size={16} />
  <Input
    bind:this={field}
    bind:value
    placeholder="Search"
    {onkey}
    onfocus={() => (focused = true)}
    onblur={() => (focused = false)}
    style={{ flex: 1, minWidth: 0, color: p.fg, fontSize: 13 }}
  />
  {#if value !== ""}
    <Button
      onpress={() => {
        value = "";
        field?.focus();
      }}
      style={{ size: 22, radius: 6, align: "center", justify: "center", color: p.muted }}
      hover={{ background: p.border, color: p.fg }}
    >
      <Icon svg={cross} size={14} />
    </Button>
  {/if}
</View>
