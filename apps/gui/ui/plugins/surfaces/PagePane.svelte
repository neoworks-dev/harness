<script lang="ts">
  // A page surface in the chat's place: a bar back to the chat (or Esc) over the surface, which
  // scrolls in the conversation's column.
  import { Button, Icon, Scroll, Text, View } from "@neoworks/svelte-gpui/ui";
  import arrowLeft from "@phosphor-icons/core/regular/arrow-left.svg";
  import { useCtx } from "../../kernel/slots/context.ts";
  import Column from "../../widgets/Column.svelte";
  import { surfaces } from "./state.svelte.ts";
  import SurfaceView from "./SurfaceView.svelte";
  import { titleOf } from "./surfaces.ts";

  const { daemon } = useCtx();
  const p = $derived(daemon.palette);
  const page = $derived(daemon.surfaces.find((surface) => surface.ref === surfaces.page));
</script>

{#if page}
  <View style={{ flex: 1, minHeight: 0 }}>
    <View style={{ direction: "row", align: "center", gap: 8, paddingX: 12, height: 36, shrink: 0 }}>
      <Button
        onpress={() => (surfaces.page = undefined)}
        style={{ direction: "row", align: "center", gap: 6, paddingX: 8, height: 26, radius: p.radius.s, color: p.muted }}
        hover={{ background: p.hover, color: p.fg }}
      >
        <Icon svg={arrowLeft} size={14} />
        <Text>Back to the chat</Text>
      </Button>
      <Text style={{ flex: 1, minWidth: 0, truncate: true, fontWeight: "medium" }}>{titleOf(page)}</Text>
    </View>
    <Scroll style={{ flex: 1, minHeight: 0 }}>
      <Column style={{ paddingY: 16 }}>
        <SurfaceView surface={page} />
      </Column>
    </Scroll>
  </View>
{/if}
