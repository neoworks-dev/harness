<script lang="ts">
  // A section of the side panel on a rounded card: a heading with an icon and what goes at its
  // end, then the content.
  import type { Snippet } from "svelte";
  import { Icon, Text, View } from "@neoworks/svelte-gpui/ui";
  import { useCtx } from "../kernel/slots/context.ts";

  interface Props {
    title: string;
    /** SVG markup. */
    icon: string;
    /** Drawn at the end of the heading, such as a count or the branch. */
    end?: Snippet;
    children: Snippet;
    grow?: boolean;
  }

  let { title, icon, end, children, grow = false }: Props = $props();

  const { daemon } = useCtx();
  const p = $derived(daemon.palette);
</script>

<View
  style={{
    gap: 8,
    padding: 12,
    minHeight: 0,
    shrink: 1,
    flex: grow ? 1 : undefined,
    background: p.bg,
    radius: 14,
    borderWidth: 1,
    borderColor: p.border,
  }}
>
  <View style={{ direction: "row", align: "center", gap: 8, color: p.muted }}>
    <Icon svg={icon} size={16} />
    <Text style={{ flex: 1, minWidth: 0, fontWeight: "medium", color: p.fg }}>{title}</Text>
    {@render end?.()}
  </View>
  {@render children()}
</View>
