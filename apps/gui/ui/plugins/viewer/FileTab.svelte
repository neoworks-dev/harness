<script lang="ts">
  // A file opened from the diff: its text as it is on disk, with line numbers (pressing one selects
  // the line for a comment). Long files show their first lines, with a notice.
  import { untrack } from "svelte";
  import { List, Text, View } from "@neoworks/svelte-gpui/ui";
  import clockwise from "@phosphor-icons/core/regular/arrow-clockwise.svg";
  import wrapIcon from "@phosphor-icons/core/regular/arrow-u-down-left.svg";
  import { useCtx } from "../../kernel/slots/context.ts";
  import FileIcon from "../../widgets/FileIcon.svelte";
  import CommentEditor from "./CommentEditor.svelte";
  import CopyPath from "./CopyPath.svelte";
  import IconButton from "./IconButton.svelte";
  import FileLine from "./FileLine.svelte";
  import { numberWidth } from "./look.ts";
  import { span } from "./selection.ts";
  import { FILE_LINES, fileView, viewer } from "./state.svelte.ts";

  type Entry =
    | { kind: "line"; key: string; index: number; text: string }
    | { kind: "editor"; key: string };

  let { path }: { path: string } = $props();

  const { daemon } = useCtx();
  const p = $derived(daemon.palette);
  const text = $derived(viewer.texts[path]);
  const slash = $derived(path.lastIndexOf("/"));
  const base = $derived.by((): Entry[] => {
    if (text?.status !== "ready") return [];
    return text.lines.map((line, index) => ({ kind: "line", key: `l${index}`, index, text: line }));
  });
  const entries = $derived.by((): Entry[] => {
    const { selection } = viewer;
    if (!viewer.editing || selection === undefined || selection.view !== fileView(path)) return base;
    const at = span(selection).to + 1;
    return [...base.slice(0, at), { kind: "editor", key: "editor" }, ...base.slice(at)];
  });
  const notice = $derived.by(() => {
    if (text === undefined || text.status === "loading") return "Reading the file…";
    if (text.status === "error") return text.message;
    if (text.total === 0) return "The file is empty.";
    return undefined;
  });
  const cut = $derived(text?.status === "ready" && text.total > FILE_LINES ? text.total : undefined);
  let list: ReturnType<typeof List<Entry>> | undefined = $state();

  // A new selection opens the comment's editor: bring it into view.
  $effect(() => {
    void viewer.selection;
    untrack(() => {
      const at = entries.findIndex((entry) => entry.kind === "editor");
      if (at >= 0) setTimeout(() => list?.scrollToIndex(at, { align: "nearest" }), 60);
    });
  });

  const digits = $derived(text?.status === "ready" ? String(text.lines.length).length : 1);
</script>

<View
  style={{
    direction: "row",
    align: "center",
    gap: 8,
    height: 36,
    paddingX: 12,
    shrink: 0,
    borderTopWidth: 1,
    borderBottomWidth: 1,
    borderColor: p.border,
  }}
>
  <FileIcon {path} size={15} />
  <View style={{ direction: "row", flex: 1, minWidth: 0 }}>
    {#if slash >= 0}
      <Text style={{ fontSize: 12, minWidth: 0, shrink: 1, truncate: true, color: p.muted }}>{path.slice(0, slash + 1)}</Text>
    {/if}
    <Text style={{ fontSize: 12, shrink: 0 }}>{path.slice(slash + 1)}</Text>
  </View>
  <CopyPath {path} />
  <IconButton svg={clockwise} label="Reload" onpress={() => void viewer.readFile(path)} />
  <IconButton svg={wrapIcon} label="Word wrap" active={viewer.wrap} onpress={() => (viewer.wrap = !viewer.wrap)} />
</View>
{#if notice !== undefined}
  <View style={{ flex: 1, align: "center", justify: "center", padding: 24 }}>
    <Text style={{ color: p.muted, fontSize: 13, textAlign: "center" }}>{notice}</Text>
  </View>
{:else}
  {#if cut !== undefined}
    <View style={{ paddingX: 12, paddingY: 6, background: p.surface, borderBottomWidth: 1, borderColor: p.border }}>
      <Text style={{ fontSize: 12, color: p.muted }}>
        The file has {cut} lines; the first {FILE_LINES} are shown.
      </Text>
    </View>
  {/if}
  <List bind:this={list} items={entries} key={(entry) => entry.key} style={{ flex: 1, minHeight: 0 }}>
    {#snippet row(entry: Entry)}
      {#if entry.kind === "line"}
        <FileLine {path} index={entry.index} text={entry.text} width={numberWidth(digits)} />
      {:else}
        <CommentEditor />
      {/if}
    {/snippet}
  </List>
{/if}
