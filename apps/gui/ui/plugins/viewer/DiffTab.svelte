<script lang="ts">
  // The diff tab: the toolbar, then every changed file as a header and its lines in one list that
  // builds only the rows near the window. The header of the file at the top stays there as the
  // list scrolls, until the next file's header pushes it up.
  import { untrack } from "svelte";
  import { Text, View } from "@neoworks/svelte-gpui/ui";
  import type { Layout } from "@neoworks/svelte-gpui/ui";
  import { useCtx } from "../../kernel/slots/context.ts";
  import { diffStats } from "../../lib/diff.ts";
  import CommentEditor from "./CommentEditor.svelte";
  import DiffToolbar from "./DiffToolbar.svelte";
  import FileHeader from "./FileHeader.svelte";
  import FoldRow from "./FoldRow.svelte";
  import LineRow from "./LineRow.svelte";
  import { CHAR_WIDTH, numberWidth } from "./look.ts";
  import PairRow from "./PairRow.svelte";
  import { forDiff } from "./highlights.ts";
  import { buildRows, guessHeight, headers, HEADER_HEIGHT, maybeWraps, withEditor, type Row } from "./rows.ts";
  import { span } from "./selection.ts";
  import { diffView, viewer } from "./state.svelte.ts";
  import type { Token } from "./syntax.ts";
  import VirtualList from "./VirtualList.svelte";

  const { daemon } = useCtx();
  const p = $derived(daemon.palette);

  let list: ReturnType<typeof VirtualList<Row>> | undefined = $state();
  let width = $state(0);
  let scrollY = $state(0);

  const split = $derived(viewer.layout === "split");
  const base = $derived(
    buildRows(viewer.diffs, {
      split,
      collapsed: viewer.collapsed,
      isOpen: (path, id) => viewer.isUnfolded(path, id),
    }),
  );
  /** The file the selection is in, when its comment editor is drawn. */
  const editing = $derived.by(() => {
    const { selection } = viewer;
    if (!viewer.editing || selection === undefined) return undefined;
    const file = viewer.diffs.findIndex((candidate) => diffView(candidate.path) === selection.view);
    return file < 0 ? undefined : { file, line: span(selection).to };
  });
  const rows = $derived(editing ? withEditor(base, editing.file, editing.line) : base);
  /** How many characters of code fit a row, for guessing how tall a wrapped line is. */
  const chars = $derived.by(() => {
    if (!viewer.wrap) return Number.POSITIVE_INFINITY;
    const room = width - numberWidth(viewer.digits) * (split ? 1 : 2) - 18;
    return Math.max(10, Math.floor(room / CHAR_WIDTH / (split ? 2 : 1)));
  });
  const heads = $derived(headers(rows));
  /** The file whose header is above the window, and how far the next one pushes it up. */
  const sticky = $derived.by(() => {
    if (list === undefined || scrollY <= 0) return undefined;
    const file = rows[list.at(scrollY)]?.file;
    if (file === undefined || list.topOf(heads[file] ?? 0) >= scrollY) return undefined;
    const next = heads[file + 1];
    const push = next === undefined ? 0 : Math.min(0, list.topOf(next) - scrollY - HEADER_HEIGHT);
    return { file, push };
  });
  const stuck = $derived(sticky === undefined ? undefined : viewer.diffs[sticky.file]);

  // A file asked for (the changes card): bring its header to the top once its rows exist.
  $effect(() => {
    const target = viewer.reveal;
    if (target === undefined) return;
    const at = rows.findIndex((row) => row.kind === "file" && row.path === target.path);
    if (at >= 0) {
      untrack(() => {
        list?.scrollToIndex(at);
        viewer.reveal = undefined;
      });
    } else if (!viewer.busy && (viewer.changes !== undefined || viewer.scope === "turn")) {
      // Read, and the file is not among the changes.
      untrack(() => (viewer.reveal = undefined));
    }
  });

  // A new selection opens the comment's editor: bring it into view.
  $effect(() => {
    void viewer.selection;
    untrack(() => {
      const at = rows.findIndex((row) => row.kind === "editor");
      if (at >= 0) setTimeout(() => list?.scrollToIndex(at, { align: "nearest" }), 60);
    });
  });

  function pathOf(file: number): string {
    return viewer.diffs[file]?.path ?? "";
  }

  /** The runs of line `index` of file `file`, scanned when the row is first drawn. */
  function runsOf(file: number, index: number | undefined): Token[] | undefined {
    const diff = viewer.diffs[file];
    if (diff === undefined || index === undefined) return undefined;
    return forDiff(diff.path, diff.lines)?.tokens(index);
  }

  function sized(box: Layout): void {
    width = box.width;
  }

  /** What the list says when there is nothing to list. */
  const empty = $derived.by(() => {
    if (viewer.problem) return viewer.problem;
    if (viewer.source.kind === "patch") return "This patch has no changes.";
    if (viewer.scope === "turn") return "The agent has not edited a file in this turn.";
    if (viewer.changes === undefined) return "Reading the changes…";
    return "No uncommitted changes.";
  });
</script>

<DiffToolbar />
{#if rows.length === 0}
  <View style={{ flex: 1, align: "center", justify: "center", padding: 24 }}>
    <Text style={{ color: p.muted, fontSize: 13, textAlign: "center" }}>{empty}</Text>
  </View>
{:else}
  <View style={{ flex: 1, minHeight: 0 }} onresize={sized}>
    <VirtualList
      bind:this={list}
      items={rows}
      key={(row) => row.key}
      height={(row) => guessHeight(row, chars)}
      measure={(row) => viewer.wrap && maybeWraps(row, chars)}
      onscroll={(y) => (scrollY = y)}
      style={{ flex: 1, minHeight: 0 }}
    >
      {#snippet row(entry: Row)}
        {#if entry.kind === "file"}
          <FileHeader path={entry.path} added={entry.added} removed={entry.removed} collapsed={entry.collapsed} />
        {:else if entry.kind === "line"}
          <LineRow path={pathOf(entry.file)} index={entry.index} line={entry.line} tokens={runsOf(entry.file, entry.index)} />
        {:else if entry.kind === "pair"}
          <PairRow
            path={pathOf(entry.file)}
            left={entry.left}
            right={entry.right}
            leftTokens={runsOf(entry.file, entry.left?.index)}
            rightTokens={runsOf(entry.file, entry.right?.index)}
          />
        {:else if entry.kind === "fold"}
          <FoldRow
            text={`${entry.count} unmodified lines`}
            onpress={() => viewer.unfold(pathOf(entry.file), entry.id)}
          />
        {:else if entry.kind === "hunk"}
          <FoldRow text={entry.text} />
        {:else if entry.kind === "note"}
          <View style={{ height: 40, justify: "center", paddingX: 16 }}>
            <Text style={{ fontSize: 12, color: p.muted }}>{entry.text}</Text>
          </View>
        {:else}
          <CommentEditor />
        {/if}
      {/snippet}
    </VirtualList>
    {#if stuck && sticky}
      {@const stats = diffStats(stuck.lines)}
      <View style={{ position: "absolute", top: sticky.push, left: 0, right: 0 }}>
        <FileHeader path={stuck.path} added={stats.added} removed={stats.removed} collapsed={viewer.collapsed.has(stuck.path)} />
      </View>
    {/if}
  </View>
{/if}
