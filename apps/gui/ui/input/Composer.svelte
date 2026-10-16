<script lang="ts">
  // Where the user types. Enter sends; commands (`/model …`) are sent as typed.
  import { Input, View } from "@neoworks/svelte-gpui/ui";
  import { suite } from "../state/suite.svelte.ts";

  let text = $state("");
  const p = $derived(suite.palette);

  function submit(value: string): void {
    const trimmed = value.trim();
    if (trimmed === "") return;
    text = "";
    void suite.send(trimmed);
  }
</script>

<View style={{ background: p.band, paddingX: p.space.l, paddingY: p.space.m }}>
  <Input bind:value={text} placeholder="Message the agent, / for commands" onsubmit={submit} style={{ color: p.fg }} />
</View>
