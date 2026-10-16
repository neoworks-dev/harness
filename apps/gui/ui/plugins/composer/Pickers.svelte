<script lang="ts">
  // The session's settings under the input: its harness, the harness's model and effort if chosen,
  // and the approvals mode. Each runs the daemon's command for it, which asks in the chat.
  import { Icon, View } from "@neoworks/svelte-gpui/ui";
  import eye from "@phosphor-icons/core/regular/eye.svg";
  import lock from "@phosphor-icons/core/regular/lock.svg";
  import lockOpen from "@phosphor-icons/core/regular/lock-open.svg";
  import pencil from "@phosphor-icons/core/regular/pencil-simple.svg";
  import gauge from "@phosphor-icons/core/regular/gauge.svg";
  import { useCtx } from "../../kernel/slots/context.ts";
  import { currentSettings } from "../../lib/status.ts";
  import HarnessIcon from "../../widgets/HarnessIcon.svelte";
  import Picker from "./Picker.svelte";
  import PickerMenu, { type Choice } from "./PickerMenu.svelte";

  /** The approvals modes, as the daemon describes them (protocol: SettingChanged). */
  const APPROVALS: readonly Choice[] = [
    { value: "ask", label: "Supervised", icon: lock, description: "Ask before edits, commands and starting a guest." },
    { value: "review", label: "Review edits", icon: eye, description: "Write each edit, then keep or revert it. Ask for the rest." },
    { value: "edits", label: "Auto-accept edits", icon: pencil, description: "Write edits right away, revertible after the turn. Ask for the rest." },
    { value: "auto", label: "Full access", icon: lockOpen, description: "Ask for nothing, except reading secrets." },
  ];

  const { daemon } = useCtx();
  const p = $derived(daemon.palette);
  const status = $derived(daemon.status);
  const settings = $derived(currentSettings(status));
  const approvals = $derived(APPROVALS.find((mode) => mode.value === status.approvals) ?? APPROVALS[0]!);

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
  <PickerMenu
    label={approvals.label}
    choices={APPROVALS}
    current={status.approvals}
    onselect={(mode) => run(`/approvals ${mode}`)}
    width={340}
  >
    {#snippet icon()}<Icon svg={approvals.icon ?? lock} size={16} />{/snippet}
  </PickerMenu>
</View>
