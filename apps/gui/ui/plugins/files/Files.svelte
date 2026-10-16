<script lang="ts">
  // The files panel: what git says changed in the session's directory, plus the files the agent
  // and its guests touched, tagged by who. Pressing a file shows its diff over the window.
  import { Icon, Scroll, Text, View } from "@neoworks/svelte-gpui/ui";
  import branchIcon from "@phosphor-icons/core/regular/git-branch.svg";
  import diffIcon from "@phosphor-icons/core/regular/git-diff.svg";
  import Card from "../../widgets/Card.svelte";
  import { useCtx } from "../../kernel/slots/context.ts";
  import { parseUnified } from "../../lib/unified.ts";
  import {
    branchLabel,
    fileEntries,
    glyphOfMark,
    type FileEntry,
    type Mark,
  } from "./files.ts";
  import { git } from "./git.svelte.ts";
  import { touched } from "./state.svelte.ts";

  const { daemon, diff, git: gitService } = useCtx();

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

  async function showDiff(entry: FileEntry): Promise<void> {
    if (!canDiff(entry)) return;
    const untracked = entry.mark === "untracked" || (git.status === undefined && entry.mark === "added");
    try {
      const text = await gitService.diff({ cwd: daemon.cwd, path: entry.rel, untracked });
      const lines = parseUnified(text).flatMap((file) => file.lines);
      diff.show({
        type: "diff",
        path: entry.rel,
        lines: lines.length > 0 ? lines : [{ kind: "context", text: "no changes" }],
      });
    } catch (error) {
      daemon.error = error instanceof Error ? error.message : String(error);
    }
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
          onpress={() => void showDiff(entry)}
          hover={canDiff(entry) ? { background: p.hover } : undefined}
          style={{ direction: "row", align: "center", gap: 8, height: 26, paddingX: 6, radius: p.radius.s, cursor: canDiff(entry) ? "pointer" : "default" }}
        >
          <Text style={{ width: 12, fontFamily: "mono", fontSize: 12, color: colorOf(entry.mark) }}>{glyphOfMark(entry.mark)}</Text>
          <Text style={{ flex: 1, minWidth: 0, truncate: true }}>
            {entry.orig === undefined ? entry.rel.slice(slash + 1) : `${entry.orig} → ${entry.rel}`}{#if entry.orig === undefined && slash > 0}<Text style={{ color: p.muted, fontSize: 12 }}>{"  "}{entry.rel.slice(0, slash)}</Text>{/if}
          </Text>
          {#if entry.tags.length > 0}<Text style={{ color: p.muted, fontSize: 12 }}>{entry.tags.join(", ")}</Text>{/if}
        </View>
      {/each}
    </View>
  </Scroll>
</Card>
