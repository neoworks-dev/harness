<script lang="ts">
  // One suggestion of a FollowUpBlock: a pill that sends its text to the assistant when pressed.
  import { Button, Text } from "@neoworks/svelte-gpui/ui";
  import type { KeyEvent } from "@neoworks/svelte-gpui/ui";
  import type { UiNode } from "@suite/protocol/types";
  import { useCtx } from "../../../kernel/slots/context.ts";
  import { useDoc } from "./context.ts";
  import { str } from "./props.ts";

  let { node }: { node: UiNode } = $props();
  const { daemon } = useCtx();
  const p = $derived(daemon.palette);
  const doc = useDoc();

  const key = $derived(node.key ?? "");
  const off = $derived(doc.streaming);
  let element: { focus(): void; blur(): void } | undefined = $state();
  $effect(() => {
    if (key === "" || off) return;
    return doc.view.register(key, () => element?.focus());
  });
</script>

<Button
  bind:this={element}
  disabled={off}
  onpress={() => doc.press(node)}
  onfocus={() => (doc.view.focus = key)}
  onblur={() => {
    if (doc.view.focus === key) doc.view.focus = undefined;
  }}
  onkey={(event: KeyEvent) => {
    if (event.key === "Escape") {
      element?.blur();
      event.preventDefault();
      event.stopPropagation();
    }
  }}
  style={{ paddingX: 14, paddingY: 6, radius: 999, borderWidth: 1, borderColor: p.border, background: p.surface, color: p.accent, transition: 120 }}
  hover={{ background: p.raised, borderColor: p.accent }}
  focus={{ borderColor: p.accent }}
  disabledStyle={{ opacity: 0.5 }}
>
  <Text style={{ color: p.accent }}>{str(node, "text") ?? ""}</Text>
</Button>
