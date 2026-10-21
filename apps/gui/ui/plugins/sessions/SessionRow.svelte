<script lang="ts">
  // One session in the sidebar, as a small card: its project's icon and name and, in place of how
  // long ago it was active, what it is doing (working for so long, done) or "Settle" while the
  // pointer is on it, then its title, then its branch, the lines it changed and the harness it
  // runs on. The shown session sits on a raised card; one that another session delegated to is
  // indented. In a search the words found are picked out, and a match in the last message shows it.
  import { Icon, Text, View } from "@neoworks/svelte-gpui/ui";
  import type { PointerEvent } from "@neoworks/svelte-gpui/ui";
  import branchIcon from "@phosphor-icons/core/regular/git-branch.svg";
  import check from "@phosphor-icons/core/regular/check.svg";
  import checkCircle from "@phosphor-icons/core/regular/check-circle.svg";
  import pinned from "@phosphor-icons/core/fill/push-pin-fill.svg";
  import type { SessionInfo } from "@neoworks-dev/neo-protocol/types";
  import HarnessIcon from "../../widgets/HarnessIcon.svelte";
  import Spinner from "../../widgets/Spinner.svelte";
  import { useCtx } from "../../kernel/slots/context.ts";
  import Highlight from "./Highlight.svelte";
  import ProjectIcon from "./ProjectIcon.svelte";
  import RowAction from "./RowAction.svelte";
  import { around, type Field } from "./search.ts";
  import { age, changedLines, elapsed, previewOf, sessionTitle, type Status } from "./sessions.ts";

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
    status: Status;
    pin: boolean;
    /** The keys are on this row. */
    selected: boolean;
    nowMs: number;
    /** The words of the search that found it. */
    terms?: readonly string[];
    /** What the search found it by. */
    field?: Field;
    onpress: () => void;
    onsettle: () => void;
    /** The right button went down on it. */
    oncontext: (event: PointerEvent) => void;
  }

  let {
    info,
    project,
    icon,
    branch,
    child,
    current,
    status,
    pin,
    selected,
    nowMs,
    terms = [],
    field,
    onpress,
    onsettle,
    oncontext,
  }: Props = $props();
  const { daemon } = useCtx();
  const p = $derived(daemon.palette);
  const changed = $derived(changedLines(info));
  const done = $derived(status.kind === "done");
  let hovered = $state(false);
  const titleStyle = $derived({
    flex: 1,
    minWidth: 0,
    truncate: true,
    fontWeight: "medium" as const,
    color: current || done ? "#f1f3f7" : "rgba(241, 243, 247, 0.9)",
  });
</script>

<View
  {onpress}
  onhover={(over) => (hovered = over)}
  ondown={oncontext}
  buttons={["right"]}
  hover={{ background: current ? p.side.active : p.side.hover }}
  style={{
    height: 78,
    marginLeft: child ? p.space.l : 0,
    marginBottom: 1,
    paddingX: 10,
    paddingY: 8,
    radius: p.radius.s,
    background: current ? p.side.active : selected ? p.side.hover : "transparent",
    cursor: "pointer",
  }}
>
  <View style={{ direction: "row", align: "center", gap: 6, height: 20 }}>
    <ProjectIcon name={project} src={icon} />
    <Highlight
      text={project}
      {terms}
      style={{ flex: 1, minWidth: 0, truncate: true, color: p.muted, fontSize: 12, fontWeight: "medium" }}
    />
    {#if hovered}
      <RowAction svg={check} label="Settle" onpress={onsettle} />
    {:else if status.kind === "working"}
      <View style={{ direction: "row", align: "center", gap: 4, color: p.info, fontSize: 12 }}>
        <Spinner />
        <Text style={{ fontSize: 12 }}
          >Working{#if status.since !== undefined}{" "}{elapsed(status.since, nowMs)}{/if}</Text
        >
      </View>
    {:else if status.kind === "done"}
      <View style={{ direction: "row", align: "center", gap: 4, color: p.success }}>
        <Text style={{ fontSize: 12 }}>Done</Text>
        <Icon svg={checkCircle} size={14} />
      </View>
    {:else}
      <Text style={{ color: p.muted, fontSize: 12 }}>{age(info.lastActivity, nowMs)}</Text>
    {/if}
  </View>
  <View style={{ direction: "row", align: "center", gap: 6, marginTop: 4 }}>
    <Highlight text={field === "title" ? around(sessionTitle(info), terms, 18) : sessionTitle(info)} {terms} style={titleStyle} />
    {#if pin}
      <View style={{ color: p.muted }}><Icon svg={pinned} size={12} /></View>
    {/if}
  </View>
  <View style={{ direction: "row", align: "center", gap: 6, marginTop: 2, color: "rgba(163, 163, 163, 0.4)" }}>
    {#if field === "preview"}
      <Highlight
        text={around(previewOf(info), terms)}
        {terms}
        style={{ flex: 1, minWidth: 0, truncate: true, fontSize: 12, color: p.muted }}
      />
    {:else if branch}
      <Icon svg={branchIcon} size={12} />
      <Text style={{ flex: 1, minWidth: 0, truncate: true, fontSize: 12 }}>{branch}</Text>
    {:else}
      <View style={{ flex: 1 }} />
    {/if}
    {#if info.queued > 0}
      <Text style={{ color: p.accent, fontSize: 12 }}>{info.queued} queued</Text>
    {/if}
    {#if changed.added > 0 || changed.removed > 0}
      <Text style={{ fontSize: 12, fontFamily: "mono" }}>
        {#if changed.added > 0}<Text style={{ color: "#00d492" }}>+{changed.added}</Text>{/if}
        {#if changed.added > 0 && changed.removed > 0}{" "}{/if}
        {#if changed.removed > 0}<Text style={{ color: "#ff6467" }}>−{changed.removed}</Text>{/if}
      </Text>
    {/if}
    {#if info.harness}
      <View style={{ color: p.muted }}><HarnessIcon harness={info.harness} /></View>
    {/if}
  </View>
</View>
