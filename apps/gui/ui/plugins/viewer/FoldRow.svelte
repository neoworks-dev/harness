<script lang="ts">
  // The place of a run of unmodified lines (pressing it shows them), or of a hunk's header in a
  // patch, which has nothing to show.
  import { Icon, Text, View } from "@neoworks/svelte-gpui/ui";
  import unfoldIcon from "@phosphor-icons/core/regular/arrows-out-line-vertical.svg";
  import { useCtx } from "../../kernel/slots/context.ts";
  import { FOLD_HEIGHT } from "./rows.ts";

  interface Props {
    /** "12 unmodified lines", or the hunk's header. */
    text: string;
    onpress?: () => void;
  }

  let { text, onpress }: Props = $props();

  const { daemon } = useCtx();
  const p = $derived(daemon.palette);
</script>

<View
  {onpress}
  style={{
    direction: "row",
    align: "center",
    justify: "center",
    gap: 6,
    height: FOLD_HEIGHT,
    background: p.surface,
    color: p.muted,
    cursor: onpress ? "pointer" : "default",
  }}
  hover={onpress ? { background: p.raised, color: p.fg } : undefined}
>
  {#if onpress}
    <Icon svg={unfoldIcon} size={12} />
    <Text style={{ fontSize: 11, shrink: 0, opacity: 0.8 }}>{text}</Text>
  {:else}
    <Text style={{ fontSize: 11, truncate: true, fontFamily: "mono", opacity: 0.8 }}>{text}</Text>
  {/if}
</View>
