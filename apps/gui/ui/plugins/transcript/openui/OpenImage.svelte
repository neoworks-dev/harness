<script lang="ts">
  // A picture of an OpenUI doc in a box of a fixed size (the list measures a row once: a picture
  // that changed it after loading would overlap the next). A web image is downloaded by the daemon
  // into its asset store and read from there; until then, and when that fails, the box says so.
  import { Image, Text, View } from "@neoworks/svelte-gpui/ui";
  import { useCtx } from "../../../kernel/slots/context.ts";
  import { downloaded, pictureName, sourceOf } from "./image.ts";

  interface Props {
    src: string;
    alt?: string;
    width: number;
    height: number;
    /** Corner radius; the box's own when absent. */
    radius?: number;
  }

  let { src, alt, width, height, radius }: Props = $props();
  const { daemon } = useCtx();
  const p = $derived(daemon.palette);

  let url = $state<string>();
  let failure = $state<string>();
  const name = $derived(pictureName(src, alt));

  // What the effect reads is only the source: the state it sets is written, never read, here.
  $effect(() => {
    const source = sourceOf(src, daemon.cwd);
    url = undefined;
    failure = undefined;
    if (source.kind === "web") {
      let current = true;
      downloaded(source.url, (address) => daemon.fetchAsset(address)).then(
        (handle) => {
          if (!current) return;
          const found = daemon.assetSrc(handle);
          if (found === undefined) failure = "the daemon is not reachable";
          else url = found;
        },
        (error: unknown) => {
          if (current) failure = error instanceof Error ? error.message : String(error);
        },
      );
      return () => {
        current = false;
      };
    }
    if (source.kind === "asset") {
      const found = daemon.assetSrc(source.handle);
      if (found === undefined) failure = "the asset is not reachable";
      else url = found;
    } else if (source.kind === "file") {
      url = source.path;
    } else {
      url = source.url;
    }
    return undefined;
  });
</script>

{#if url !== undefined && failure === undefined}
  <Image
    src={url}
    fit="contain"
    style={{ width, height, radius: radius ?? p.radius.s, background: p.raised }}
    onerror={() => (failure = "could not be loaded")}
  />
{:else}
  <View style={{ width, height, align: "center", justify: "center", paddingX: p.space.m, background: p.raised, radius: radius ?? p.radius.s }}>
    <Text style={{ color: p.muted, fontSize: 12, textAlign: "center", maxLines: 3 }}>
      {failure === undefined ? `[image ${name}] loading image…` : `[image ${name}] ${failure}`}
    </Text>
  </View>
{/if}
