<script lang="ts">
  // Copies a path to the clipboard; the icon turns into a check for a moment.
  import { clipboard } from "@neoworks/svelte-gpui";
  import { Button, Icon, Tooltip } from "@neoworks/svelte-gpui/ui";
  import check from "@phosphor-icons/core/regular/check.svg";
  import copy from "@phosphor-icons/core/regular/copy-simple.svg";
  import { useCtx } from "../../kernel/slots/context.ts";

  const COPIED_MS = 1500;

  let { path }: { path: string } = $props();

  const { daemon } = useCtx();
  const p = $derived(daemon.palette);
  let copied = $state(false);
  let timer: ReturnType<typeof setTimeout> | undefined;

  function press(): void {
    void clipboard.write(path);
    copied = true;
    clearTimeout(timer);
    timer = setTimeout(() => (copied = false), COPIED_MS);
  }
</script>

<Tooltip text={copied ? "Copied" : "Copy path"} side="bottom" anchorStyle={{ alignSelf: "center" }}>
  <Button
    onpress={press}
    style={{ size: 22, align: "center", justify: "center", radius: 5, color: copied ? p.success : p.muted }}
    hover={{ background: p.hover, color: copied ? p.success : p.fg }}
  >
    <Icon svg={copied ? check : copy} size={13} />
  </Button>
</Tooltip>
