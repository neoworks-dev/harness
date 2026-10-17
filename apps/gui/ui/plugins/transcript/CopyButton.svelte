<script lang="ts">
  // A small button that copies `text`, and says so with a ✓ for a moment.
  import { clipboard } from "@neoworks/svelte-gpui";
  import { Button, Icon } from "@neoworks/svelte-gpui/ui";
  import check from "@phosphor-icons/core/regular/check.svg";
  import copy from "@phosphor-icons/core/regular/copy.svg";
  import { useCtx } from "../../kernel/slots/context.ts";

  let { text, size = 24 }: { text: string; size?: number } = $props();

  const { daemon } = useCtx();
  const p = $derived(daemon.palette);

  let copied = $state(false);
  let timer: ReturnType<typeof setTimeout> | undefined;

  function press(): void {
    void clipboard.write(text);
    copied = true;
    clearTimeout(timer);
    timer = setTimeout(() => (copied = false), 1200);
  }
</script>

<Button
  onpress={press}
  style={{ size, radius: 6, align: "center", justify: "center", color: copied ? p.success : p.muted }}
  hover={{ color: copied ? p.success : p.fg, background: p.hover }}
>
  <Icon svg={copied ? check : copy} size={14} />
</Button>
