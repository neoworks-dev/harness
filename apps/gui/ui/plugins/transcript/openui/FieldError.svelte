<script lang="ts">
  // The message a failed field shows when no FormControl is there to show it.
  import { Text } from "@neoworks/svelte-gpui/ui";
  import type { UiNode } from "@neoworks-dev/neo-protocol/types";
  import { useCtx } from "../../../kernel/slots/context.ts";
  import { inControl } from "./formcontext.ts";
  import { str } from "./props.ts";

  let { node }: { node: UiNode } = $props();
  const { daemon } = useCtx();
  const p = $derived(daemon.palette);
  const alone = !inControl();
</script>

{#if alone && str(node, "error")}
  <Text style={{ color: p.danger, fontSize: p.text.body - 1 }}>{str(node, "error")}</Text>
{/if}
