<script lang="ts">
  // The files panel: what git says changed in the session's directory, plus the files the agent
  // and its guests touched, tagged by who. Pressing a file shows its diff in the viewer.
  import { Icon, Scroll, Text, View } from "@neoworks/svelte-gpui/ui";
  import branchIcon from "@phosphor-icons/core/regular/git-branch.svg";
  import diffIcon from "@phosphor-icons/core/regular/git-diff.svg";
  import Card from "../../widgets/Card.svelte";
  import FileIcon from "../../widgets/FileIcon.svelte";
  import { useCtx } from "../../kernel/slots/context.ts";
  import { branchLabel } from "../../lib/git-status.ts";
  import {
    fileEntries,
    glyphOfMark,
    type FileEntry,
    type Mark,
  } from "./files.ts";
  import { git } from "./git.svelte.ts";
  import { touched } from "./state.svelte.ts";

  const { daemon, viewer } = useCtx();

  const p = $derived(daemon.palette);
  const entries = $derived(fileEntries(git.status, touched.touches.touched));

  function colorOf(mark: Mark): string {
    switch (mark) {
      case "added":
        return p.success;
      case "deleted":
      case "conflicted":
        return p.danger;
      case "modified":
      case "renamed":
        return p.accent;
      default:
        return p.muted;
    }
  }

  /** Whether git can show a diff: inside a repository, or the file is new to the agent. */
  function canDiff(entry: FileEntry): boolean {
    return !entry.rel.endsWith("/") && (git.status !== undefined || entry.mark === "added");
  }

  function showDiff(entry: FileEntry): void {
    if (canDiff(entry)) viewer.showChanges(entry.rel);
  }
</script>

<Card title="Changes" icon={diffIcon} grow>
  {#snippet end()}
    {#if git.status}
      <View style={{ direction: "row", align: "center", gap: 4, maxWidth: 140 }}>
        <Icon svg={branchIcon} size={12} />
        <Text style={{ fontSize: 12, truncate: true }}>{branchLabel(git.status)}</Text>
      </View>
    {/if}
  {/snippet}
  <Scroll style={{ flex: 1, minHeight: 0 }}>
    <View style={{ gap: 1 }}>
      {#if entries.length === 0}
        <Text style={{ color: p.muted, fontSize: 13 }}>No changes yet.</Text>
      {/if}
      {#each entries as entry (entry.rel)}
        {@const slash = entry.rel.replace(/\/$/, "").lastIndexOf("/")}
        <View
          onpress={() => showDiff(entry)}
          hover={canDiff(entry) ? { background: p.hover } : undefined}
          style={{ direction: "row", align: "center", gap: 8, height: 26, paddingX: 6, radius: p.radius.s, cursor: canDiff(entry) ? "pointer" : "default" }}
        >
          <FileIcon path={entry.rel} />
          <Text style={{ flex: 1, minWidth: 0, truncate: true, color: entry.mark === "deleted" ? p.muted : p.fg, strikethrough: entry.mark === "deleted" }}>
            {entry.orig === undefined ? entry.rel.slice(slash + 1) : `${entry.orig} → ${entry.rel}`}{#if entry.orig === undefined && slash > 0}<Text style={{ color: p.muted, fontSize: 12 }}>{"  "}{entry.rel.slice(0, slash)}</Text>{/if}
          </Text>
          {#if entry.tags.length > 0}<Text style={{ color: p.muted, fontSize: 12 }}>{entry.tags.join(", ")}</Text>{/if}
          <Text style={{ width: 12, fontFamily: "mono", fontSize: 12, textAlign: "right", color: colorOf(entry.mark) }}>{glyphOfMark(entry.mark)}</Text>
        </View>
      {/each}
    </View>
  </Scroll>
</Card>
