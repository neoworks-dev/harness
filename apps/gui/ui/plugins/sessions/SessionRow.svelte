<script lang="ts">
  // One session in the sidebar, in two lines. The first is its title and, at the end, how long ago
  // it was active or what it is doing (working for so long, done), or "Settle" while the pointer is
  // on it. The second is muted: a branch worth naming, the last thing said, the lines it changed and
  // the mark of the harness it runs on. Under a project's heading the project is not repeated; where
  // projects mix (pinned, a search) its icon leads the title and its name the second line. The shown
  // session sits on a raised card with a bar in the margin; one that another session delegated to
  // is indented. In a search the words found are picked out.
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
    /** Rows of other projects are around it (pinned, a search): it names its own. */
    showProject: boolean;
    /** A branch worth naming (`notableBranch`). */
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
    showProject,
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
  const untitled = $derived(!info.title?.trim());
  const preview = $derived(previewOf(info));
  let hovered = $state(false);
  const titleStyle = $derived({
    flex: 1,
    minWidth: 0,
    truncate: true,
    fontWeight: "medium" as const,
    italic: untitled,
    color: untitled ? p.muted : current || done ? "#f1f3f7" : "rgba(241, 243, 247, 0.86)",
  });
  const small = { fontSize: 12 };
</script>

<View
  {onpress}
  onhover={(over) => (hovered = over)}
  ondown={oncontext}
  buttons={["right"]}
  hover={{ background: current ? p.side.active : p.side.hover }}
  style={{
    marginLeft: child ? 14 : 0,
    marginBottom: 2,
    paddingLeft: 10,
    paddingRight: 8,
    paddingY: 7,
    gap: 3,
    radius: p.radius.s,
    borderLeftWidth: 2,
    borderColor: current ? p.accent : "transparent",
    background: current ? p.side.active : selected ? p.side.hover : "transparent",
    cursor: "pointer",
  }}
>
  <View style={{ direction: "row", align: "center", gap: 6, height: 20 }}>
    {#if child}<Text style={{ ...small, color: p.muted }}>↳</Text>{/if}
    {#if showProject}<ProjectIcon name={project} src={icon} size={14} />{/if}
    <Highlight text={field === "title" ? around(sessionTitle(info), terms, 18) : sessionTitle(info)} {terms} style={titleStyle} />
    {#if pin && !showProject}
      <View style={{ color: p.muted }}><Icon svg={pinned} size={11} /></View>
    {/if}
    {#if hovered}
      <RowAction svg={check} label="Settle" onpress={onsettle} />
    {:else if status.kind === "working"}
      <View style={{ direction: "row", align: "center", gap: 4, color: p.info }}>
        <Spinner />
        <Text style={small}>{status.since !== undefined ? elapsed(status.since, nowMs) : "Working"}</Text>
      </View>
    {:else if status.kind === "done"}
      <View style={{ direction: "row", align: "center", gap: 4, color: p.success }}>
        <Text style={small}>Done</Text>
        <Icon svg={checkCircle} size={13} />
      </View>
    {:else}
      <Text style={{ ...small, color: p.muted }}>{age(info.lastActivity, nowMs)}</Text>
    {/if}
  </View>
  <View style={{ direction: "row", align: "center", gap: 6, height: 16, color: p.muted }}>
    {#if showProject}
      <Highlight text={project} {terms} style={{ ...small, shrink: 0, maxWidth: 96, truncate: true, color: p.muted }} />
    {/if}
    {#if branch}
      <View style={{ direction: "row", align: "center", gap: 3, shrink: 0, maxWidth: 110 }}>
        <Icon svg={branchIcon} size={11} />
        <Text style={{ ...small, truncate: true }}>{branch}</Text>
      </View>
    {/if}
    {#if field === "preview"}
      <Highlight text={around(preview, terms)} {terms} style={{ ...small, flex: 1, minWidth: 0, truncate: true, color: p.muted }} />
    {:else}
      <Text style={{ ...small, flex: 1, minWidth: 0, truncate: true, color: "rgba(163, 163, 163, 0.55)" }}>{preview}</Text>
    {/if}
    {#if info.queued > 0}
      <Text style={{ ...small, color: p.accent, shrink: 0 }}>{info.queued} queued</Text>
    {/if}
    {#if changed.added > 0 || changed.removed > 0}
      <Text style={{ ...small, fontFamily: "mono", shrink: 0 }}>
        {#if changed.added > 0}<Text style={{ color: "#00d492" }}>+{changed.added}</Text>{/if}
        {#if changed.added > 0 && changed.removed > 0}{" "}{/if}
        {#if changed.removed > 0}<Text style={{ color: "#ff6467" }}>−{changed.removed}</Text>{/if}
      </Text>
    {/if}
    {#if info.harness}
      <View style={{ opacity: 0.6, shrink: 0 }}><HarnessIcon harness={info.harness} size={12} mono /></View>
    {/if}
  </View>
</View>
