<script lang="ts">
  // A picture from a path, a data: URL, a URL or the daemon's asset store, in a box of a fixed
  // size (see Component.svelte for why it is fixed). When it can't be loaded the box says so.
  // A click opens it over the window, with the chat's other pictures to step through.
  import { Button, Image, Text, View } from "@neoworks/svelte-gpui/ui";
  import type { ImageComponent } from "@neoworks-dev/neo-protocol/types";
  import { baseName, oneLine } from "./components.ts";
  import { chatPictures, galleryAt } from "./gallery.ts";
  import { gallery } from "./gallery.svelte.ts";
  import { useCtx } from "../../kernel/slots/context.ts";

  interface Props {
    image: ImageComponent;
    width: number;
    height: number;
  }

  let { image, width, height }: Props = $props();
  const { daemon } = useCtx();
  const p = $derived(daemon.palette);

  let failed = $state(false);
  // A new source gets a new chance.
  $effect(() => {
    void image.src;
    failed = false;
  });

  const name = $derived(image.alt ? oneLine(image.alt) : baseName(image.src));
  const resolve = (src: string): string => daemon.assetSrc(src) ?? src;

  function show(): void {
    const picked = { src: resolve(image.src), name, ...(image.caption ? { caption: image.caption } : {}) };
    const opened = galleryAt(chatPictures(daemon.items, resolve), picked);
    gallery.open(opened.pictures, opened.index);
  }
</script>

<View style={{ gap: p.space.s, alignSelf: "start" }}>
  {#if failed}
    <View style={{ width, height: p.space.l * 2, justify: "center", paddingX: p.space.m, background: p.surface, radius: 6 }}>
      <Text style={{ color: p.muted, truncate: true }}>[image {name}] could not be loaded</Text>
    </View>
  {:else}
    <Button onpress={show} style={{ width, height }}>
      <Image
        src={resolve(image.src)}
        fit="contain"
        style={{ width, height, radius: 6 }}
        onerror={() => (failed = true)}
      />
    </Button>
  {/if}
  {#if image.caption}
    <Text style={{ color: p.muted }}>{image.caption}</Text>
  {/if}
</View>
