<script lang="ts">
  // The files panel: what git says changed in the session's directory, plus the files the agent
  // and its guests touched, tagged by who. Pressing a file shows its diff over the window.
  import { Scroll, Text, View } from "@neoworks/svelte-gpui/ui";
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

<View style={{ gap: p.space.s, minHeight: 0, flex: 1 }}>
  <View style={{ direction: "row", gap: p.space.s }}>
    <Text style={{ color: p.muted, fontWeight: "bold" }}>Files</Text>
    {#if git.status}<Text style={{ color: p.muted, flex: 1, minWidth: 0, truncate: true }}>{"· "}{branchLabel(git.status)}</Text>{/if}
  </View>
  <Scroll style={{ flex: 1, minHeight: 0 }}>
    <View>
      {#each entries as entry (entry.rel)}
        <View
          onpress={() => void showDiff(entry)}
          hover={canDiff(entry) ? { background: p.surface } : undefined}
          style={{ direction: "row", gap: p.space.m, paddingY: 1, cursor: canDiff(entry) ? "pointer" : "default" }}
        >
          <Text style={{ width: 12, color: colorOf(entry.mark) }}>{glyphOfMark(entry.mark)}</Text>
          <Text style={{ flex: 1, minWidth: 0, truncate: true, color: colorOf(entry.mark) }}>
            {entry.orig === undefined ? entry.rel : `${entry.orig} → ${entry.rel}`}{#if entry.tags.length > 0}<Text style={{ color: p.muted }}>{" "}({entry.tags.join(", ")})</Text>{/if}
          </Text>
        </View>
      {/each}
    </View>
  </Scroll>
</View>
