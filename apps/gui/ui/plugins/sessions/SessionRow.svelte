<script lang="ts">
  // One session in the sidebar, as a small card: its project's icon and name and how long ago it
  // was last active, its title, then its branch and the lines it changed. The shown session sits
  // on a raised card; a running one has a spinner; one that finished unseen a green dot; one that
  // another session delegated to is indented.
  import { Icon, Text, View } from "@neoworks/svelte-gpui/ui";
  import branchIcon from "@phosphor-icons/core/regular/git-branch.svg";
  import type { SessionInfo } from "@suite/protocol/types";
  import Spinner from "../../widgets/Spinner.svelte";
  import { useCtx } from "../../kernel/slots/context.ts";
  import { blend } from "../../lib/palette.ts";
  import ProjectIcon from "./ProjectIcon.svelte";
  import { age, changedLines, sessionTitle } from "./sessions.ts";

  interface Props {
    info: SessionInfo;
    /** The repository's name. */
    project: string;
    /** The repository's favicon: null when it has none, undefined until looked up. */
    icon: string | null | undefined;
    /** The branch checked out where the session works. */
    branch: string | undefined;
    /** Delegated to by another session. */
    child: boolean;
    current: boolean;
    finished: boolean;
    /** The keys are on this row. */
    selected: boolean;
    nowMs: number;
    onpress: () => void;
  }

  let { info, project, icon, branch, child, current, finished, selected, nowMs, onpress }: Props = $props();
  const { daemon } = useCtx();
  const p = $derived(daemon.palette);
  const changed = $derived(changedLines(info));
  const emphasis = $derived(current || finished);
  /** Titles of the rows that aren't shown read a step quieter than the shown one. */
  const blendMuted = $derived(blend(p.fg, p.muted, 35));
</script>

<View
  {onpress}
  hover={{ background: current ? p.raised : p.hover }}
  style={{
    gap: 3,
    marginLeft: child ? p.space.l : 0,
    marginBottom: 2,
    paddingX: 10,
    paddingY: p.space.m,
    radius: p.radius.m,
    background: current || selected ? p.raised : "transparent",
    cursor: "pointer",
  }}
>
  <View style={{ direction: "row", align: "center", gap: p.space.s + 2 }}>
    <ProjectIcon name={project} src={icon} />
    <Text style={{ flex: 1, minWidth: 0, truncate: true, color: p.muted, fontSize: 12 }}>{project}</Text>
    <Text style={{ color: p.muted, fontSize: 12 }}>{age(info.lastActivity, nowMs)}</Text>
  </View>
  <View style={{ direction: "row", align: "center", gap: p.space.s + 2 }}>
    {#if info.state === "running"}
      <View style={{ color: p.accent }}><Spinner /></View>
    {:else if finished}
      <Text style={{ color: p.success, fontSize: 10 }}>●</Text>
    {/if}
    <Text
      style={{
        flex: 1,
        minWidth: 0,
        truncate: true,
        fontWeight: "semibold",
        color: current || finished ? p.fg : blendMuted,
      }}>{sessionTitle(info)}</Text
    >
  </View>
  <View style={{ direction: "row", align: "center", gap: p.space.s, color: p.muted }}>
    {#if branch}
      <Icon svg={branchIcon} size={12} />
      <Text style={{ flex: 1, minWidth: 0, truncate: true, fontSize: 12 }}>{branch}</Text>
    {:else}
      <View style={{ flex: 1 }} />
    {/if}
    {#if info.queued > 0}
      <Text style={{ color: p.accent, fontSize: 12 }}>{info.queued} queued</Text>
    {/if}
    {#if changed.added > 0 || changed.removed > 0}
      <Text style={{ fontSize: 12 }}>
        {#if changed.added > 0}<Text style={{ color: p.success }}>+{changed.added}</Text>{/if}
        {#if changed.added > 0 && changed.removed > 0}{" "}{/if}
        {#if changed.removed > 0}<Text style={{ color: p.danger }}>−{changed.removed}</Text>{/if}
      </Text>
    {/if}
  </View>
</View>
