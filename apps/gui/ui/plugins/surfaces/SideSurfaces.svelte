<script lang="ts">
  // The surfaces that live beside the chat (`side`): each a card of the side panel, under its title.
  import { Scroll } from "@neoworks/svelte-gpui/ui";
  import squares from "@phosphor-icons/core/regular/squares-four.svg";
  import Card from "../../widgets/Card.svelte";
  import { useCtx } from "../../kernel/slots/context.ts";
  import CloseButton from "./CloseButton.svelte";
  import SurfaceView from "./SurfaceView.svelte";
  import { inPlace, titleOf } from "./surfaces.ts";

  const { daemon } = useCtx();
  const panels = $derived(inPlace(daemon.surfaces, "side"));
</script>

{#each panels as panel (panel.ref)}
  <Card title={titleOf(panel)} icon={squares}>
    {#snippet end()}<CloseButton surface={panel} />{/snippet}
    <Scroll style={{ minHeight: 0, shrink: 1 }}>
      <SurfaceView surface={panel} own />
    </Scroll>
  </Card>
{/each}
