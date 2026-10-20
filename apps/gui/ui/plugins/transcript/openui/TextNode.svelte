<script lang="ts">
  // Markdown in a tone, as a title, as body text or as code.
  import { View } from "@neoworks/svelte-gpui/ui";
  import { useCtx } from "../../../kernel/slots/context.ts";
  import Blocks from "../Blocks.svelte";
  import { toneColor } from "./look.ts";
  import { blocks, str, tone as toneOf } from "./props.ts";
  import type { NodeProps } from "./Node.svelte";

  let { node }: NodeProps = $props();
  const { daemon } = useCtx();
  const p = $derived(daemon.palette);

  const role = $derived(str(node, "role"));
  const size = $derived.by(() => {
    if (role === "title") return p.text.title + 3;
    return role === "code" ? p.text.code : p.text.body;
  });
  const tone = $derived(toneOf(node));
</script>

<View
  style={{
    color: tone === undefined ? undefined : toneColor(tone, p),
    fontSize: size,
    fontWeight: role === "title" ? 600 : undefined,
    fontFamily: role === "code" ? "mono" : undefined,
  }}
>
  <Blocks blocks={blocks(node)} {size} />
</View>
