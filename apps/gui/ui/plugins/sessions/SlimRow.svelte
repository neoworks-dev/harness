<script lang="ts">
  // A settled session: one slim line with its project's icon faded, its title muted and its age,
  // which gives way to "Un-settle" while the pointer is on it.
  import { Text, View } from "@neoworks/svelte-gpui/ui";
  import type { PointerEvent } from "@neoworks/svelte-gpui/ui";
  import undo from "@phosphor-icons/core/regular/arrow-counter-clockwise.svg";
  import type { SessionInfo } from "@suite/protocol/types";
  import { useCtx } from "../../kernel/slots/context.ts";
  import ProjectIcon from "./ProjectIcon.svelte";
  import RowAction from "./RowAction.svelte";
  import { age, sessionTitle } from "./sessions.ts";

  interface Props {
    info: SessionInfo;
    project: string;
    icon: string | null | undefined;
    current: boolean;
    /** The keys are on this row. */
    selected: boolean;
    nowMs: number;
    onpress: () => void;
    onunsettle: () => void;
    oncontext: (event: PointerEvent) => void;
  }

  let { info, project, icon, current, selected, nowMs, onpress, onunsettle, oncontext }: Props =
    $props();
  const { daemon } = useCtx();
  const p = $derived(daemon.palette);
  let hovered = $state(false);
</script>

<View
  {onpress}
  onhover={(over) => (hovered = over)}
  ondown={oncontext}
  buttons={["right"]}
  hover={{ background: current ? p.side.active : p.side.hover }}
  style={{
    direction: "row",
    align: "center",
    gap: 8,
    height: 36,
    marginBottom: 1,
    paddingX: 10,
    radius: p.radius.s,
    background: current ? p.side.active : selected ? p.side.hover : "transparent",
    cursor: "pointer",
  }}
>
  <View style={{ opacity: 0.4 }}><ProjectIcon name={project} src={icon} /></View>
  <Text style={{ flex: 1, minWidth: 0, truncate: true, color: p.muted }}>{sessionTitle(info)}</Text>
  {#if hovered}
    <RowAction svg={undo} label="Un-settle" onpress={onunsettle} />
  {:else}
    <Text style={{ color: p.muted, fontSize: 12 }}>{age(info.lastActivity, nowMs)}</Text>
  {/if}
</View>
