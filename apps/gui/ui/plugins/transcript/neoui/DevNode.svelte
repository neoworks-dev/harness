<script lang="ts">
  // The Dev components that are rows of text: a file tree, a code reference, a commit, a test
  // run, a stack trace, a GitHub issue or pull request, checks and a timeline. An item written
  // without its list (a File, a Test…) is drawn as a list of one.
  import { Text, View } from "@neoworks/svelte-gpui/ui";
  import type { UiNode } from "@neoworks-dev/neo-protocol/types";
  import { useCtx } from "../../../kernel/slots/context.ts";
  import { fileMark, place, stateLook, statusLook, treeRows } from "./dev.ts";
  import { tint, toneColor } from "./look.ts";
  import { list, nodes, num, str, tone } from "./props.ts";
  import type { NodeProps } from "./Node.svelte";

  let { node: given }: NodeProps = $props();
  const { daemon } = useCtx();
  const p = $derived(daemon.palette);

  /** The list an item written alone belongs to, and the prop it fills there. */
  const ALONE: Record<string, [string, string]> = {
    File: ["FileTree", "files"],
    Test: ["TestRun", "tests"],
    Frame: ["StackTrace", "frames"],
    Check: ["Checks", "checks"],
    TimelineItem: ["Timeline", "items"],
  };
  const node: UiNode = $derived.by(() => {
    const alone = ALONE[given.component];
    if (!alone) return given;
    const [component, prop] = alone;
    return { component, props: { [prop]: [given] } };
  });
  const small = $derived(p.text.body - 1);

  function counts(n: UiNode): { added?: number; removed?: number } {
    const added = num(n, "added");
    const removed = num(n, "removed");
    return {
      ...(added !== undefined && added > 0 ? { added } : {}),
      ...(removed !== undefined && removed > 0 ? { removed } : {}),
    };
  }

  function joined(...words: (string | undefined)[]): string {
    return words.filter((w): w is string => w !== undefined && w !== "").join(" · ");
  }
</script>

{#snippet lineCounts(n: UiNode)}
  {@const c = counts(n)}
  {#if c.added !== undefined}
    <Text style={{ color: p.success, nowrap: true, fontFamily: "mono", fontSize: small }}>+{c.added}</Text>
  {/if}
  {#if c.removed !== undefined}
    <Text style={{ color: p.danger, nowrap: true, fontFamily: "mono", fontSize: small }}>−{c.removed}</Text>
  {/if}
{/snippet}

{#snippet pill(label: string, color: string)}
  <View style={{ alignSelf: "start", paddingX: 8, paddingY: 1, radius: 999, background: tint(p, color, 18) }}>
    <Text style={{ fontSize: 12, fontWeight: "medium", color, nowrap: true }}>{label}</Text>
  </View>
{/snippet}

{#if node.component === "FileTree"}
  <View style={{ gap: 2 }}>
    {#each treeRows(node) as row}
      <View style={{ direction: "row", gap: 6, align: "center", paddingLeft: row.depth * 14 }}>
        {#if row.kind === "dir"}
          <Text style={{ fontFamily: "mono", color: p.muted }}>▾</Text>
          <Text style={{ fontFamily: "mono", color: p.fg }}>{row.name}</Text>
        {:else}
          {@const look = fileMark(str(row.file, "status"), p)}
          {@const quiet = ["untracked", "unchanged"].includes(str(row.file, "status") ?? "")}
          <Text style={{ fontFamily: "mono", color: look.color, width: 12 }}>{look.mark}</Text>
          <Text style={{ fontFamily: "mono", color: quiet ? p.muted : p.fg, nowrap: true }}>{row.name}</Text>
          <Text style={{ color: p.muted, flex: 1, minWidth: 0, truncate: true, fontSize: small }}>{str(row.file, "note") ?? ""}</Text>
          {@render lineCounts(row.file)}
        {/if}
      </View>
    {/each}
  </View>
{:else if node.component === "CodeRef"}
  <View style={{ direction: "row", gap: 8, align: "center" }}>
    <Text style={{ fontFamily: "mono", color: p.accent, nowrap: true }}>{place(node)}</Text>
    <Text style={{ fontFamily: "mono", color: p.muted, flex: 1, minWidth: 0, truncate: true }}>{(str(node, "text") ?? "").trim()}</Text>
  </View>
{:else if node.component === "Commit"}
  <View style={{ gap: 2 }}>
    <View style={{ direction: "row", gap: 8, align: "center" }}>
      <Text style={{ fontFamily: "mono", color: p.accent, nowrap: true }}>{(str(node, "sha") ?? "").slice(0, 7)}</Text>
      <Text style={{ fontWeight: "medium", color: p.fg, flex: 1, minWidth: 0, truncate: true }}>{str(node, "title") ?? ""}</Text>
      {@render lineCounts(node)}
    </View>
    {@const files = num(node, "files")}
    {@const about = joined(str(node, "author"), str(node, "date"), files === undefined ? undefined : `${files} file${files === 1 ? "" : "s"}`)}
    {#if about !== ""}
      <Text style={{ color: p.muted, fontSize: small, paddingLeft: 62 }}>{about}</Text>
    {/if}
  </View>
{:else if node.component === "TestRun"}
  <View style={{ gap: 4 }}>
    <View style={{ direction: "row", gap: 12, align: "center", wrap: true }}>
      {#if str(node, "name")}
        <Text style={{ fontFamily: "mono", fontWeight: "medium", color: p.fg }}>{str(node, "name")}</Text>
      {/if}
      {#if num(node, "passed") !== undefined}
        <Text style={{ color: p.success, nowrap: true }}>✓ {num(node, "passed")} passed</Text>
      {/if}
      {#if (num(node, "failed") ?? 0) > 0}
        <Text style={{ color: p.danger, nowrap: true }}>✗ {num(node, "failed")} failed</Text>
      {/if}
      {#if (num(node, "skipped") ?? 0) > 0}
        <Text style={{ color: p.muted, nowrap: true }}>○ {num(node, "skipped")} skipped</Text>
      {/if}
      {#if str(node, "duration")}
        <Text style={{ color: p.muted, nowrap: true }}>{str(node, "duration")}</Text>
      {/if}
    </View>
    {#each nodes(node, "tests") as test}
      {@const look = statusLook(str(test, "status") ?? "failed", p)}
      <View style={{ gap: 2 }}>
        <View style={{ direction: "row", gap: 8, align: "center" }}>
          <Text style={{ color: look.color, width: 12 }}>{look.glyph}</Text>
          <Text style={{ fontFamily: "mono", color: p.fg, flex: 1, minWidth: 0, truncate: true }}>{str(test, "name") ?? ""}</Text>
          {#if str(test, "duration")}
            <Text style={{ color: p.muted, nowrap: true, fontSize: small }}>{str(test, "duration")}</Text>
          {/if}
        </View>
        {#if str(test, "message")}
          <Text style={{ fontFamily: "mono", color: p.danger, fontSize: small, paddingLeft: 20 }}>{str(test, "message")}</Text>
        {/if}
      </View>
    {/each}
  </View>
{:else if node.component === "StackTrace"}
  <View style={{ gap: 3 }}>
    <Text style={{ color: p.danger, fontWeight: "medium" }}>✗ {str(node, "kind") ? `${str(node, "kind")}: ` : ""}{str(node, "message") ?? ""}</Text>
    {#each nodes(node, "frames") as frame}
      <View style={{ direction: "row", gap: 8, paddingLeft: 16, align: "center" }}>
        <Text style={{ color: p.muted }}>at</Text>
        {#if str(frame, "name")}
          <Text style={{ fontFamily: "mono", color: p.fg, nowrap: true }}>{str(frame, "name")}</Text>
        {/if}
        <Text style={{ fontFamily: "mono", color: p.accent, flex: 1, minWidth: 0, truncate: true }}>{place(frame)}</Text>
      </View>
    {/each}
  </View>
{:else if node.component === "Issue" || node.component === "PullRequest"}
  {@const state = stateLook(node, p)}
  <View style={{ gap: 4 }}>
    <View style={{ direction: "row", gap: 8, align: "center" }}>
      {@render pill(state.label, state.color)}
      {#if num(node, "number") !== undefined}
        <Text style={{ color: p.muted, nowrap: true }}>#{num(node, "number")}</Text>
      {/if}
      <Text style={{ fontWeight: "medium", color: p.fg, flex: 1, minWidth: 0 }}>{str(node, "title") ?? ""}</Text>
    </View>
    <View style={{ direction: "row", gap: 10, align: "center", wrap: true }}>
      {#if node.component === "PullRequest"}
        {@const branch = str(node, "branch")}
        {@const base = str(node, "base")}
        <Text style={{ color: p.muted, fontSize: small }}>{joined(branch && base ? `${branch} → ${base}` : branch, str(node, "repo"), str(node, "author"))}</Text>
        {#if str(node, "checks")}
          {@const checks = str(node, "checks")}
          <Text style={{ fontSize: small, nowrap: true, color: checks === "passing" ? p.success : checks === "failing" ? p.danger : p.muted }}>{checks === "passing" ? "✓ checks pass" : checks === "failing" ? "✗ checks fail" : "◐ checks running"}</Text>
        {/if}
        {#if str(node, "review")}
          {@const review = str(node, "review")}
          <Text style={{ fontSize: small, nowrap: true, color: review === "approved" ? p.success : review === "changes" ? p.danger : p.muted }}>{review === "approved" ? "✓ approved" : review === "changes" ? "✗ changes requested" : "○ review pending"}</Text>
        {/if}
        {@render lineCounts(node)}
      {:else}
        {@const comments = num(node, "comments")}
        <Text style={{ color: p.muted, fontSize: small }}>{joined(str(node, "repo"), str(node, "author"), comments ? `${comments} comment${comments === 1 ? "" : "s"}` : undefined)}</Text>
        {#each list(node, "labels").filter((l): l is string => typeof l === "string") as label}
          {@render pill(label, p.muted)}
        {/each}
      {/if}
    </View>
  </View>
{:else if node.component === "Checks"}
  <View style={{ gap: 3 }}>
    {#each nodes(node, "checks") as check}
      {@const look = statusLook(str(check, "status") ?? "pending", p)}
      <View style={{ direction: "row", gap: 8, align: "center" }}>
        <Text style={{ color: look.color, width: 12 }}>{look.glyph}</Text>
        <Text style={{ color: str(check, "status") === "failed" ? p.danger : str(check, "status") === "skipped" ? p.muted : p.fg, flex: 1, minWidth: 0, truncate: true }}>{str(check, "name") ?? ""}</Text>
        {#if str(check, "duration")}
          <Text style={{ color: p.muted, nowrap: true, fontSize: small }}>{str(check, "duration")}</Text>
        {/if}
      </View>
    {/each}
  </View>
{:else if node.component === "Timeline"}
  {@const items = nodes(node, "items")}
  <View>
    {#each items as item, index}
      <View style={{ direction: "row", gap: 10 }}>
        <View style={{ align: "center", width: 10 }}>
          <View style={{ width: 9, height: 9, radius: 999, marginTop: 5, background: toneColor(tone(item) ?? "accent", p) }} />
          {#if index < items.length - 1}
            <View style={{ width: 1.5, flex: 1, minHeight: 10, background: p.border, marginY: 2 }} />
          {/if}
        </View>
        <View style={{ flex: 1, minWidth: 0, paddingBottom: index < items.length - 1 ? 10 : 0, gap: 1 }}>
          <View style={{ direction: "row", gap: 8 }}>
            {#if str(item, "time")}
              <Text style={{ color: p.muted, nowrap: true }}>{str(item, "time")}</Text>
            {/if}
            <Text style={{ fontWeight: "medium", color: p.fg, flex: 1, minWidth: 0 }}>{str(item, "title") ?? ""}</Text>
          </View>
          {#if str(item, "detail")}
            <Text style={{ color: p.muted, fontSize: small }}>{str(item, "detail")}</Text>
          {/if}
        </View>
      </View>
    {/each}
  </View>
{/if}
