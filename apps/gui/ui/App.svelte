<script lang="ts">
  // The window is whatever the plugins register into the root slot (the shell, in practice).
  import { root } from "./kernel/context.ts";
  import Keys from "./kernel/Keys.svelte";
  import Slot from "./kernel/slots/Slot.svelte";

  $effect(() => {
    void root.daemon.start();
  });
</script>

<Keys />
<Slot name="root" />
