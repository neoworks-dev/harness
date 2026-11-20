<script lang="ts">
  // A picture of the chat over the whole window, as large as fits, with the chat's other pictures
  // to step through: the arrows at its sides (or ← →), a strip of them under it. Esc, the close
  // button or a click beside the picture closes it. Drawn in `shell.overlay`; nothing while closed.
  import { Button, Icon, Image, Text, View } from "@neoworks/svelte-gpui/ui";
  import caretLeft from "@phosphor-icons/core/regular/caret-left.svg";
  import caretRight from "@phosphor-icons/core/regular/caret-right.svg";
  import xIcon from "@phosphor-icons/core/regular/x.svg";
  import { useCtx } from "../../kernel/slots/context.ts";
  import { gallery } from "./gallery.svelte.ts";

  /** The side of a picture in the strip, in pixels. */
  const THUMB = 56;

  const { daemon } = useCtx();
  const p = $derived(daemon.palette);
  const current = $derived(gallery.current);
  const many = $derived(gallery.pictures.length > 1);

  // Another session shown: its chat has other pictures.
  $effect(() => {
    void daemon.sessionId;
    gallery.close();
  });

  let failed = $state(false);
  $effect(() => {
    void current?.src;
    failed = false;
  });

  const control = { size: 40, radius: 20, align: "center", justify: "center", color: "#ffffff", background: "rgba(255, 255, 255, 0.08)" } as const;
  const controlHover = { background: "rgba(255, 255, 255, 0.18)" };
</script>

{#if current}
  <View style={{ position: "absolute", inset: 0, background: "rgba(0, 0, 0, 0.92)", padding: p.space.l, gap: p.space.m }}>
    <View style={{ direction: "row", align: "center", gap: p.space.m }}>
      <Text style={{ flex: 1, minWidth: 0, truncate: true, color: "#ffffff" }}>
        {current.name}{#if many}<Text style={{ color: "rgba(255, 255, 255, 0.6)" }}>{`  ${gallery.index + 1} / ${gallery.pictures.length}`}</Text>{/if}
      </Text>
      <Button onpress={() => gallery.close()} style={control} hover={controlHover}>
        <Icon svg={xIcon} size={18} />
      </Button>
    </View>

    <View style={{ flex: 1, minHeight: 0, direction: "row", align: "center", gap: p.space.m }}>
      {#if many}
        <Button onpress={() => gallery.step(-1)} style={control} hover={controlHover}>
          <Icon svg={caretLeft} size={20} />
        </Button>
      {/if}
      <!-- Beside the picture is the backdrop: a click there closes, as in a lightbox. -->
      <Button onpress={() => gallery.close()} style={{ flex: 1, minWidth: 0, height: "full", align: "center", justify: "center" }}>
        {#if failed}
          <Text style={{ color: "rgba(255, 255, 255, 0.7)" }}>[image {current.name}] could not be loaded</Text>
        {:else}
          <Image src={current.src} fit="contain" style={{ width: "full", height: "full" }} onerror={() => (failed = true)} />
        {/if}
      </Button>
      {#if many}
        <Button onpress={() => gallery.step(1)} style={control} hover={controlHover}>
          <Icon svg={caretRight} size={20} />
        </Button>
      {/if}
    </View>

    {#if current.caption}
      <Text style={{ color: "rgba(255, 255, 255, 0.7)", textAlign: "center" }}>{current.caption}</Text>
    {/if}

    {#if many}
      <View style={{ direction: "row", justify: "center", gap: p.space.s, overflow: "hidden" }}>
        {#each gallery.pictures as picture, index (index)}
          <Button
            onpress={() => (gallery.index = index)}
            style={{ size: THUMB + 4, radius: p.radius.s, borderWidth: 2, borderColor: index === gallery.index ? p.accent : "transparent", overflow: "hidden", opacity: index === gallery.index ? 1 : 0.6 }}
            hover={{ opacity: 1 }}
          >
            <Image src={picture.src} fit="cover" style={{ width: THUMB, height: THUMB }} />
          </Button>
        {/each}
      </View>
    {/if}
  </View>
{/if}
