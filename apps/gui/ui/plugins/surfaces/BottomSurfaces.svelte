<script lang="ts">
  // The surfaces that live over the input (`bottom`), one under the other, each under its title;
  // and a chip for each page surface that is not shown, to bring it into the chat's place.
  import { Button, Icon, Text, View } from "@neoworks/svelte-gpui/ui";
  import arrowsOut from "@phosphor-icons/core/regular/arrows-out-simple.svg";
  import { useCtx } from "../../kernel/slots/context.ts";
  import CloseButton from "./CloseButton.svelte";
  import { surfaces } from "./state.svelte.ts";
  import SurfaceView from "./SurfaceView.svelte";
  import { inPlace, titleOf } from "./surfaces.ts";

  const { daemon } = useCtx();
  const p = $derived(daemon.palette);
  const bars = $derived(inPlace(daemon.surfaces, "bottom"));
  const pages = $derived(inPlace(daemon.surfaces, "page").filter((page) => page.ref !== surfaces.page));
</script>

{#if bars.length > 0 || pages.length > 0}
  <View style={{ gap: 6, paddingBottom: 8 }}>
    {#each bars as bar (bar.ref)}
      <View style={{ gap: 4, paddingX: 12, paddingY: 8, radius: p.radius.m, background: p.surface, borderWidth: 1, borderColor: p.border }}>
        <View style={{ direction: "row", align: "center", gap: 6 }}>
          <Text style={{ flex: 1, minWidth: 0, truncate: true, fontSize: 11, color: p.muted, fontWeight: "medium" }}>{titleOf(bar)}</Text>
          <CloseButton surface={bar} />
        </View>
        <SurfaceView surface={bar} />
      </View>
    {/each}
    {#if pages.length > 0}
      <View style={{ direction: "row", gap: 6, wrap: true }}>
        {#each pages as page (page.ref)}
          <Button
            onpress={() => (surfaces.page = page.ref)}
            style={{ direction: "row", align: "center", gap: 6, paddingX: 10, height: 26, radius: 13, borderWidth: 1, borderColor: p.border, color: p.muted, fontSize: 12 }}
            hover={{ color: p.fg, background: p.hover }}
          >
            <Icon svg={arrowsOut} size={12} />
            <Text>{titleOf(page)}</Text>
          </Button>
        {/each}
      </View>
    {/if}
  </View>
{/if}
