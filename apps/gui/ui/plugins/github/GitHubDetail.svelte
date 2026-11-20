<script lang="ts">
  // One issue or pull request, laid out as github.com does: the title with its number, a state
  // pill and its type, then the description and the conversation as cards (author, age, the
  // markdown drawn as the chat draws it, reactions), a box to comment, close or reopen, and at the
  // side what is set: assignees, labels in their colors, type, milestone, projects, relationships,
  // and for a pull request its reviewers, branches, checks and files. Narrow, the side goes under.
  import { Button, Input, Scroll, Text, View } from "@neoworks/svelte-gpui/ui";
  import type { KeyEvent, Layout, ScrollPosition } from "@neoworks/svelte-gpui/ui";
  import Blocks from "../transcript/Blocks.svelte";
  import {
    checkCounts,
    emojiOf,
    labelLook,
    REACTIONS,
    shortAge,
    stateWord,
    timeline,
    type Detail,
    type Reaction,
  } from "./github.ts";
  import { blocksOf } from "./markdown.ts";
  import { useCtx } from "../../kernel/slots/context.ts";
  import { github } from "./state.svelte.ts";
  import { checkColor, checkMark, stateColor } from "./look.ts";

  interface Props {
    detail: Detail;
    nowMs: number;
    /** Scroll with the keys: the parent asks for an offset. */
    onscroll: (position: ScrollPosition) => void;
    scroller?: ReturnType<typeof Scroll>;
    /** The comment field gave the keys back (Esc). */
    onleave?: () => void;
  }

  let { detail, nowMs, onscroll, scroller = $bindable(), onleave }: Props = $props();
  const { daemon } = useCtx();

  /** The side column goes beside the conversation from this width on. */
  const SIDE_FROM = 860;
  const SIDE = 248;

  const p = $derived(daemon.palette);
  const counts = $derived(checkCounts(detail.checks));
  const conversation = $derived(timeline(detail));
  const isPr = $derived(detail.kind === "pr");
  const open = $derived(detail.state === "open" || detail.state === "draft");
  let width = $state(0);
  const wide = $derived(width >= SIDE_FROM);
  let draft = $state("");
  let field: ReturnType<typeof Input> | undefined = $state();
  /** Which card's reaction row is open to pick one: "item" or a comment's id. */
  let picking = $state<string>();

  $effect(() => github.ensureLabels());

  /** `c` in the detail: the keys go to the comment field. */
  export function focusComment(): void {
    field?.focus();
  }

  async function send(close: boolean): Promise<void> {
    const body = draft.trim();
    if (body !== "" && !(await github.comment(detail.kind, detail.number, body))) return;
    draft = "";
    if (close) await github.close(detail.kind, detail.number);
  }

  function fieldKey(event: KeyEvent): void {
    if (event.key === "Escape") {
      event.preventDefault();
      onleave?.();
    } else if (event.key === "Enter" && event.ctrl) {
      event.preventDefault();
      void send(false);
    }
    // Whatever it types is the field's: the view's letters (s, q, r…) do nothing here.
    event.stopPropagation();
  }

  function pill(state: string): { background: string; color: string } {
    const color = stateColor(detail.state, p);
    return { background: color, color: "#ffffff" };
  }
</script>

{#snippet section(title: string)}
  <Text style={{ fontSize: 12, fontWeight: "semibold", color: p.muted, marginTop: 14, marginBottom: 4 }}>{title}</Text>
{/snippet}

{#snippet chip(text: string, look: { background: string; border: string; color: string })}
  <View style={{ paddingX: 8, height: 20, radius: 10, align: "center", justify: "center", background: look.background, borderWidth: 1, borderColor: look.border }}>
    <Text style={{ fontSize: 12, fontWeight: "medium", color: look.color }}>{text}</Text>
  </View>
{/snippet}

{#snippet reactions(given: readonly Reaction[] | undefined, key: string, comment: number | undefined)}
  <View style={{ direction: "row", wrap: true, gap: 6, marginTop: 8, align: "center" }}>
    {#each given ?? [] as reaction (reaction.content)}
      <Button
        onpress={() => void github.react(detail.kind, detail.number, reaction.content, comment)}
        style={{ direction: "row", gap: 4, paddingX: 8, height: 24, radius: 12, align: "center", borderWidth: 1, borderColor: reaction.mine ? p.accent : p.border, background: reaction.mine ? "rgba(99, 140, 255, 0.15)" : "transparent" }}
        hover={{ borderColor: p.accent }}
      >
        <Text style={{ fontSize: 12 }}>{emojiOf(reaction.content)} {reaction.count}</Text>
      </Button>
    {/each}
    <Button
      onpress={() => (picking = picking === key ? undefined : key)}
      style={{ paddingX: 8, height: 24, radius: 12, align: "center", color: p.muted, borderWidth: 1, borderColor: p.border }}
      hover={{ color: p.fg }}
    >
      <Text style={{ fontSize: 12 }}>☺ +</Text>
    </Button>
    {#if picking === key}
      {#each REACTIONS as reaction (reaction.content)}
        <Button
          onpress={() => {
            picking = undefined;
            void github.react(detail.kind, detail.number, reaction.content, comment);
          }}
          style={{ size: 26, radius: 6, align: "center", justify: "center" }}
          hover={{ background: p.hover }}
        >
          <Text>{reaction.emoji}</Text>
        </Button>
      {/each}
    {/if}
  </View>
{/snippet}

{#snippet card(author: string, atMs: number, what: string, body: string, given: readonly Reaction[] | undefined, key: string, comment: number | undefined, reactable: boolean)}
  <View style={{ radius: 8, borderWidth: 1, borderColor: p.border, overflow: "hidden" }}>
    <View style={{ direction: "row", align: "center", gap: 6, paddingX: 14, height: 36, background: p.surface, borderBottomWidth: 1, borderColor: p.border }}>
      <Text style={{ fontWeight: "semibold" }}>{author}</Text>
      <Text style={{ color: p.muted, flex: 1, minWidth: 0, truncate: true }}>
        {what || "commented"}{atMs > 0 ? ` ${shortAge(atMs, nowMs)} ago` : ""}
      </Text>
    </View>
    <View style={{ paddingX: 14, paddingY: 10 }}>
      {#if blocksOf(body).length === 0}
        <Text style={{ color: p.muted, italic: true }}>No description provided.</Text>
      {:else}
        <Blocks blocks={blocksOf(body)} size={p.text.body} />
      {/if}
      {#if reactable}{@render reactions(given, key, comment)}{/if}
    </View>
  </View>
{/snippet}

{#snippet side()}
  <View style={{ width: wide ? SIDE : undefined, shrink: 0 }}>
    {@render section("Assignees")}
    <Text style={{ color: (detail.assignees ?? []).length > 0 ? p.fg : p.muted }}>{(detail.assignees ?? []).join(", ") || "No one"}</Text>
    {#if isPr}
      {@render section("Reviewers")}
      <Text style={{ color: (detail.reviewers ?? []).length > 0 ? p.fg : p.muted }}>{(detail.reviewers ?? []).join(", ") || "No reviews requested"}</Text>
    {/if}
    {@render section("Labels")}
    {#if detail.labels.length === 0}
      <Text style={{ color: p.muted }}>None yet</Text>
    {:else}
      <View style={{ direction: "row", wrap: true, gap: 4 }}>
        {#each detail.labels as label (label)}
          {@render chip(label, labelLook(github.labelColors[label] ?? 0x8b949e))}
        {/each}
      </View>
    {/if}
    {#if !isPr}
      {@render section("Type")}
      {#if detail.issueType}
        <View style={{ direction: "row" }}>{@render chip(detail.issueType, labelLook(0xd29922))}</View>
      {:else}
        <Text style={{ color: p.muted }}>None</Text>
      {/if}
    {/if}
    {@render section("Projects")}
    <Text style={{ color: (detail.projects ?? []).length > 0 ? p.fg : p.muted }}>{(detail.projects ?? []).join(", ") || "None yet"}</Text>
    {@render section("Milestone")}
    <Text style={{ color: detail.milestone ? p.fg : p.muted }}>{detail.milestone || "No milestone"}</Text>
    {#if (detail.parent ?? 0) > 0 || (detail.blockedBy ?? []).length > 0 || (detail.blocking ?? []).length > 0}
      {@render section("Relationships")}
      {#if (detail.parent ?? 0) > 0}<Text>Parent #{detail.parent}</Text>{/if}
      {#if (detail.blockedBy ?? []).length > 0}<Text>Blocked by {(detail.blockedBy ?? []).map((n) => `#${n}`).join(", ")}</Text>{/if}
      {#if (detail.blocking ?? []).length > 0}<Text>Blocking {(detail.blocking ?? []).map((n) => `#${n}`).join(", ")}</Text>{/if}
    {/if}
    {#if isPr}
      {@render section("Development")}
      <Text style={{ fontFamily: "mono", fontSize: 12, color: p.muted }}>{detail.head} → {detail.base}</Text>
      <Text style={{ fontFamily: "mono", fontSize: 12 }}><Text style={{ color: p.success }}>+{detail.additions}</Text>{" "}<Text style={{ color: p.danger }}>−{detail.deletions}</Text>{" "}<Text style={{ color: p.muted }}>in {detail.files.length} files</Text></Text>
      {#if detail.decision}<Text style={{ color: p.muted }}>{detail.decision.toLowerCase().replaceAll("_", " ")}</Text>{/if}
      {#if detail.checks.length > 0}
        {@render section(`Checks · ${counts.pass} passed${counts.fail > 0 ? ` · ${counts.fail} failed` : ""}${counts.pending > 0 ? ` · ${counts.pending} pending` : ""}`)}
        {#each detail.checks as check (check.name)}
          <Text style={{ fontSize: 12, truncate: true }}><Text style={{ color: checkColor(check.state, p) }}>{checkMark(check.state)}</Text> {check.name}</Text>
        {/each}
      {/if}
      {#if detail.files.length > 0}
        {@render section(`Files · ${detail.files.length}`)}
        {#each detail.files as file (file.path)}
          <View style={{ direction: "row", gap: 6 }}>
            <Text style={{ flex: 1, minWidth: 0, truncate: true, fontFamily: "mono", fontSize: 12 }}>{file.path}</Text>
            <Text style={{ fontFamily: "mono", fontSize: 12, color: p.success }}>+{file.additions}</Text>
            <Text style={{ fontFamily: "mono", fontSize: 12, color: p.danger }}>−{file.deletions}</Text>
          </View>
        {/each}
      {/if}
    {/if}
  </View>
{/snippet}

<Scroll bind:this={scroller} {onscroll} style={{ flex: 1, minHeight: 0 }}>
  <View onresize={(box: Layout) => (width = box.width)} style={{ paddingX: 24, paddingY: 20, gap: 12 }}>
    <!-- Title, number, state and what to do with it. -->
    <View style={{ direction: "row", align: "start", gap: 12 }}>
      <Text style={{ flex: 1, minWidth: 0, fontSize: 22, fontWeight: "semibold", lineHeight: 30 }}>
        {detail.title} <Text style={{ color: p.muted, fontWeight: "normal" }}>#{detail.number}</Text>
      </Text>
      <Button
        onpress={() => void github.send(detail.kind, detail.number)}
        style={{ paddingX: 12, height: 30, radius: 6, align: "center", background: p.accent, color: "#ffffff", shrink: 0 }}
        hover={{ opacity: 0.85 }}
      >
        <Text style={{ fontWeight: "medium" }}>Send to the model</Text>
      </Button>
      {#if isPr}
        <Button
          onpress={() => void github.showDiff(detail.number)}
          style={{ paddingX: 12, height: 30, radius: 6, align: "center", borderWidth: 1, borderColor: p.border, shrink: 0 }}
          hover={{ background: p.hover }}
        >
          <Text>View diff</Text>
        </Button>
      {/if}
    </View>
    <View style={{ direction: "row", align: "center", gap: 8, wrap: true }}>
      {@const look = pill(detail.state)}
      <View style={{ paddingX: 12, height: 26, radius: 13, align: "center", justify: "center", background: look.background }}>
        <Text style={{ color: look.color, fontWeight: "medium" }}>{stateWord(detail)}</Text>
      </View>
      {#if detail.issueType}{@render chip(detail.issueType, labelLook(0xd29922))}{/if}
      <Text style={{ color: p.muted }}>
        <Text style={{ color: p.fg, fontWeight: "medium" }}>{detail.author}</Text>
        {isPr ? ` wants to merge into ${detail.base} from ${detail.head}` : " opened this"}{detail.createdMs > 0 ? ` ${shortAge(detail.createdMs, nowMs)} ago` : ""} · {detail.comments.length} {detail.comments.length === 1 ? "comment" : "comments"}
      </Text>
    </View>
    <View style={{ height: 1, background: p.border }} />

    <View style={{ direction: wide ? "row" : "column", align: "start", gap: 24 }}>
      <View style={{ flex: wide ? 1 : undefined, minWidth: 0, gap: 12, alignSelf: "stretch" }}>
        {@render card(detail.author, detail.createdMs, "opened", detail.body, detail.reactions, "item", undefined, true)}
        {#each conversation as entry, index (index)}
          {@render card(entry.author, entry.atMs, entry.what, entry.body, entry.reactions, String(entry.id ?? index), entry.id, entry.id !== undefined)}
        {/each}

        <!-- Add a comment, close or reopen: as the user gh is logged in as. -->
        <View style={{ radius: 8, borderWidth: 1, borderColor: p.border, padding: 12, gap: 8 }}>
          <Text style={{ fontWeight: "semibold" }}>Add a comment</Text>
          <Input
            bind:this={field}
            bind:value={draft}
            multiline
            rows={4}
            placeholder="Leave a comment (Ctrl-Enter comments, Esc leaves)"
            onkey={fieldKey}
            style={{ minHeight: 88, padding: 8, radius: 6, background: p.surface, borderWidth: 1, borderColor: p.border, color: p.fg }}
          />
          <View style={{ direction: "row", justify: "end", gap: 8 }}>
            {#if open}
              <Button
                onpress={() => void send(true)}
                disabled={github.writing !== undefined}
                style={{ paddingX: 12, height: 30, radius: 6, align: "center", borderWidth: 1, borderColor: p.border }}
                hover={{ background: p.hover }}
                disabledStyle={{ opacity: 0.5 }}
              >
                <Text>{draft.trim() === "" ? `Close ${isPr ? "pull request" : "issue"}` : "Close with comment"}</Text>
              </Button>
            {:else}
              <Button
                onpress={() => void github.reopen(detail.kind, detail.number)}
                disabled={github.writing !== undefined}
                style={{ paddingX: 12, height: 30, radius: 6, align: "center", borderWidth: 1, borderColor: p.border }}
                hover={{ background: p.hover }}
                disabledStyle={{ opacity: 0.5 }}
              >
                <Text>Reopen</Text>
              </Button>
            {/if}
            <Button
              onpress={() => void send(false)}
              disabled={draft.trim() === "" || github.writing !== undefined}
              style={{ paddingX: 12, height: 30, radius: 6, align: "center", background: p.success, color: "#ffffff" }}
              hover={{ opacity: 0.9 }}
              disabledStyle={{ opacity: 0.5 }}
            >
              <Text style={{ fontWeight: "medium" }}>{github.writing === "comment" ? "Commenting…" : "Comment"}</Text>
            </Button>
          </View>
        </View>
      </View>
      {@render side()}
    </View>
  </View>
</Scroll>
