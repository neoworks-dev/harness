<script lang="ts">
  // An attached image (pasted or picked) over the input: a small picture with a button in its
  // corner that takes it back. When the file can't be drawn it falls back to a chip with its name.
  import { Button, Icon, Image, Text, View } from "@neoworks/svelte-gpui/ui";
  import xIcon from "@phosphor-icons/core/regular/x.svg";
  import { useCtx } from "../../kernel/slots/context.ts";
  import { uploadedHandle } from "../../lib/upload.ts";

  let { path, onremove }: { path: string; onremove: () => void } = $props();

  /** The picture's side, in pixels. */
  const SIZE = 56;

  const { daemon } = useCtx();
  const p = $derived(daemon.palette);
  const name = $derived(path.slice(path.lastIndexOf("/") + 1));
  /** A file a browser tab uploaded loads from the daemon by its handle; the window reads the path. */
  const src = $derived.by(() => {
    const handle = uploadedHandle(path);
    return (handle === undefined ? undefined : daemon.assetSrc(handle)) ?? path;
  });
  let failed = $state(false);

  $effect(() => {
    void path;
    failed = false;
  });
</script>

{#if failed}
  <View style={{ direction: "row", align: "center", gap: 4, paddingLeft: 10, paddingRight: 4, height: 26, radius: p.radius.s, background: p.raised }}>
    <Text style={{ fontSize: 12, maxWidth: 220, truncate: true }}>{name}</Text>
    <Button onpress={onremove} style={{ size: 18, align: "center", justify: "center", radius: 4, color: p.muted }} hover={{ color: p.fg, background: p.hover }}>
      <Icon svg={xIcon} size={12} />
    </Button>
  </View>
{:else}
  <View style={{ size: SIZE, radius: 10, overflow: "hidden", background: p.raised, borderWidth: 1, borderColor: p.border }}>
    <Image {src} fit="cover" onerror={() => (failed = true)} style={{ width: SIZE - 2, height: SIZE - 2 }} />
    <Button
      onpress={onremove}
      style={{ position: "absolute", top: 3, right: 3, size: 18, radius: 9, align: "center", justify: "center", background: "rgba(0, 0, 0, 0.6)", color: "#ffffff" }}
      hover={{ background: "rgba(0, 0, 0, 0.85)" }}
    >
      <Icon svg={xIcon} size={10} />
    </Button>
  </View>
{/if}
