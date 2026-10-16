<script lang="ts">
  // The session's settings under the input: its harness, the harness's model and effort if chosen,
  // and the approvals mode. Each runs the daemon's command for it, which asks in the chat.
  import { Icon, View } from "@neoworks/svelte-gpui/ui";
  import lock from "@phosphor-icons/core/regular/lock.svg";
  import lockOpen from "@phosphor-icons/core/regular/lock-open.svg";
  import gauge from "@phosphor-icons/core/regular/gauge.svg";
  import { useCtx } from "../../kernel/slots/context.ts";
  import { currentSettings } from "../../lib/status.ts";
  import HarnessIcon from "../../widgets/HarnessIcon.svelte";
  import Picker from "./Picker.svelte";

  const { daemon } = useCtx();
  const p = $derived(daemon.palette);
  const status = $derived(daemon.status);
  const settings = $derived(currentSettings(status));

  function run(command: string): void {
    void daemon.send(command);
  }
</script>

{#snippet separator()}
  <View style={{ width: 1, height: 16, marginX: 2, background: p.border }} />
{/snippet}

<View style={{ direction: "row", align: "center" }}>
  <Picker label={settings.model ?? status.harness} onpress={() => run(settings.model ? "/model" : "/harness")}>
    {#snippet icon()}<HarnessIcon harness={status.harness} size={16} />{/snippet}
  </Picker>
  {#if settings.effort}
    {@render separator()}
    <Picker label={settings.effort} onpress={() => run("/effort")}>
      {#snippet icon()}<Icon svg={gauge} size={16} />{/snippet}
    </Picker>
  {/if}
  {@render separator()}
  <Picker label={status.approvals} onpress={() => run("/approvals")}>
    {#snippet icon()}<Icon svg={status.approvals === "auto" ? lockOpen : lock} size={16} />{/snippet}
  </Picker>
</View>
