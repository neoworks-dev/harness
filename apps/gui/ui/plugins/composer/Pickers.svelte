<script lang="ts">
  // The session's settings under the input: the model (with the harness in it), the reasoning
  // level and the approvals mode. Each opens a menu and runs the daemon's command for the pick.
  import { Icon, View } from "@neoworks/svelte-gpui/ui";
  import eye from "@phosphor-icons/core/regular/eye.svg";
  import lock from "@phosphor-icons/core/regular/lock.svg";
  import lockOpen from "@phosphor-icons/core/regular/lock-open.svg";
  import pencil from "@phosphor-icons/core/regular/pencil-simple.svg";
  import brain from "@phosphor-icons/core/regular/brain.svg";
  import { useCtx } from "../../kernel/slots/context.ts";
  import { currentSettings } from "../../lib/status.ts";
  import { approvalsTone } from "../statusline/segments.ts";
  import ModelPicker from "./ModelPicker.svelte";
  import { optionOf, settings as offered } from "./settings.svelte.ts";
  import PickerMenu, { type Choice } from "./PickerMenu.svelte";

  /** The approvals modes, as the daemon describes them (protocol: SettingChanged). */
  const SUPERVISED: Choice = {
    value: "ask",
    label: "Supervised",
    icon: lock,
    description: "Ask before edits, commands and starting a guest.",
  };
  const APPROVALS: readonly Choice[] = [
    SUPERVISED,
    { value: "review", label: "Review edits", icon: eye, description: "Write each edit, then keep or revert it. Ask for the rest." },
    { value: "edits", label: "Auto-accept edits", icon: pencil, description: "Write edits right away, revertible after the turn. Ask for the rest." },
    { value: "auto", label: "Full access", icon: lockOpen, description: "Ask for nothing, except reading secrets." },
  ];

  const { daemon } = useCtx();
  const p = $derived(daemon.palette);
  const status = $derived(daemon.status);
  const settings = $derived(currentSettings(status));
  const reasoning = $derived(optionOf(offered.harnesses.find((h) => h.current), "thought_level"));
  const levels = $derived<Choice[]>(
    (reasoning?.values ?? []).map((value) => ({ value: value.value, label: value.name, description: value.description })),
  );
  const approvals = $derived(approvalsChoice(status.approvals));
  /** Full access is red and the modes that write without asking accent, as the status line had them. */
  const tone = $derived(approvalsTone(status.approvals));

  /** The menu entry of an approvals mode; Supervised for one this list doesn't know. */
  function approvalsChoice(mode: string): Choice {
    const found = APPROVALS.find((choice) => choice.value === mode);
    if (found) return found;
    return SUPERVISED;
  }

  function run(command: string): void {
    void daemon.send(command);
  }
</script>

{#snippet separator()}
  <View style={{ width: 1, height: 16, marginX: 2, background: p.border }} />
{/snippet}

<View style={{ direction: "row", align: "center" }}>
  <ModelPicker />
  {@render separator()}
  <PickerMenu
    label={settings.effort === undefined ? (reasoning?.current ?? "Reasoning") : daemon.label("thought_level", settings.effort)}
    title="Reasoning"
    choices={levels}
    current={settings.effort ?? reasoning?.current}
    onselect={(level) => run(`/effort ${level}`)}
    onopen={() => void offered.load(daemon)}
    empty={offered.loading ? "Asking the harness…" : `${daemon.session?.harness ?? status.harness} has no reasoning levels.`}
    width={220}
  >
    {#snippet icon()}<Icon svg={brain} size={16} />{/snippet}
  </PickerMenu>
  {@render separator()}
  <PickerMenu
    label={approvals.label}
    choices={APPROVALS}
    current={status.approvals}
    onselect={(mode) => run(`/approvals ${mode}`)}
    width={340}
    color={tone === "muted" ? undefined : p[tone]}
  >
    {#snippet icon()}<Icon svg={approvals.icon ?? lock} size={16} />{/snippet}
  </PickerMenu>
</View>
