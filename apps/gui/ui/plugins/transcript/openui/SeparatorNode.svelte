<script lang="ts">
  import { View } from "@neoworks/svelte-gpui/ui";
  import { useCtx } from "../../../kernel/slots/context.ts";


  const { daemon } = useCtx();
  const p = $derived(daemon.palette);
</script>

<View style={{ height: 1, background: p.border, marginY: 2 }} />
