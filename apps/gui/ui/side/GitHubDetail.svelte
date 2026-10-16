<script lang="ts">
  // One issue or pull request: who, when, labels, and for a pull request its branches, size,
  // review decision, checks and files; then the description and the conversation. Markdown is
  // shown as the text it is.
  import { Button, Scroll, Text, View } from "@neoworks/svelte-gpui/ui";
  import type { ScrollPosition } from "@neoworks/svelte-gpui/ui";
  import {
    checkCounts,
    readable,
    shortAge,
    timeline,
    type Detail,
  } from "../lib/github.ts";
  import { useCtx } from "../kernel/slots/context.ts";
  import { suite } from "../state/suite.svelte.ts";
  import { github } from "./github.svelte.ts";
  import { checkColor, checkMark, stateColor } from "./github-look.ts";

  interface Props {
    detail: Detail;
    nowMs: number;
    /** Scroll with the keys: the parent asks for an offset. */
    onscroll: (position: ScrollPosition) => void;
    scroller?: ReturnType<typeof Scroll>;
  }

  let { detail, nowMs, onscroll, scroller = $bindable() }: Props = $props();
  const { diff } = useCtx();

  const p = $derived(suite.palette);
  const counts = $derived(checkCounts(detail.checks));
  const conversation = $derived(timeline(detail));
  const body = $derived(readable(detail.body));
  const isPr = $derived(detail.kind === "pr");
</script>

{#snippet heading(text: string)}
  <Text style={{ color: p.muted, fontWeight: "bold", marginTop: p.space.m }}>{text}</Text>
{/snippet}

<Scroll bind:this={scroller} {onscroll} style={{ flex: 1, minHeight: 0 }}>
  <View style={{ padding: p.space.l, gap: p.space.s }}>
    <Text style={{ fontSize: p.text.title, fontWeight: "bold" }}>#{detail.number} {detail.title}</Text>
    <View style={{ direction: "row", gap: p.space.m, wrap: true }}>
      <Text style={{ color: stateColor(detail.state, p), fontWeight: "bold" }}>{detail.state}</Text>
      <Text style={{ color: p.muted }}>
        {detail.author}{detail.createdMs > 0 ? ` opened ${shortAge(detail.createdMs, nowMs)} ago` : ""}
      </Text>
      {#if detail.labels.length > 0}
        <Text style={{ color: p.accent }}>{detail.labels.join(", ")}</Text>
      {/if}
    </View>

    {#if isPr}
      <Text style={{ color: p.muted }}>
        {detail.head} → {detail.base} ·
        <Text style={{ color: p.success }}>+{detail.additions}</Text>
        <Text style={{ color: p.danger }}>{" "}−{detail.deletions}</Text>
        in {detail.files.length} files{detail.decision ? ` · ${detail.decision.toLowerCase().replaceAll("_", " ")}` : ""}
      </Text>
      {#if detail.checks.length > 0}
        <Text style={{ color: p.muted }}>
          checks:
          <Text style={{ color: p.success }}>{counts.pass} passed</Text>,
          <Text style={{ color: counts.fail > 0 ? p.danger : p.muted }}>{counts.fail} failed</Text>,
          {counts.pending} pending
        </Text>
        {#each detail.checks.filter((check) => check.state === "fail") as check (check.name)}
          <Text style={{ color: checkColor(check.state, p) }}>  {checkMark(check.state)} {check.name}</Text>
        {/each}
      {/if}
    {/if}

    <View style={{ direction: "row", gap: p.space.m, marginTop: p.space.s }}>
      <Button
        onpress={() => void github.send(detail.kind, detail.number)}
        style={{ paddingX: p.space.m, paddingY: p.space.s, radius: 4, background: p.accent, color: p.bg }}
        hover={{ opacity: 0.85 }}
      >
        <Text style={{ fontWeight: "bold" }}>Send to the model</Text>
      </Button>
      {#if isPr}
        <Button
          onpress={() => void github.showDiff(detail.number, diff)}
          style={{ paddingX: p.space.m, paddingY: p.space.s, radius: 4, background: p.surface }}
          hover={{ background: p.band }}
        >
          <Text>View diff</Text>
        </Button>
      {/if}
    </View>

    {@render heading("Description")}
    {#if body === ""}
      <Text style={{ color: p.muted }}>No description.</Text>
    {:else}
      <Text pre>{body}</Text>
    {/if}

    {#if isPr && detail.files.length > 0}
      {@render heading(`Files (${detail.files.length})`)}
      {#each detail.files as file (file.path)}
        <View style={{ direction: "row", gap: p.space.m }}>
          <Text style={{ flex: 1, minWidth: 0, truncate: true }}>{file.path}</Text>
          <Text style={{ color: p.success }}>+{file.additions}</Text>
          <Text style={{ color: p.danger }}>−{file.deletions}</Text>
        </View>
      {/each}
    {/if}

    {#if conversation.length > 0}
      {@render heading(`Conversation (${conversation.length})`)}
      {#each conversation as entry, index (index)}
        <View style={{ gap: 2, paddingLeft: p.space.m, borderLeftWidth: 2, borderColor: p.border, marginTop: p.space.s }}>
          <Text style={{ color: p.muted }}>
            <Text style={{ color: p.fg, fontWeight: "bold" }}>{entry.author}</Text>
            {entry.atMs > 0 ? ` · ${shortAge(entry.atMs, nowMs)} ago` : ""}{entry.what ? ` · ${entry.what}` : ""}
          </Text>
          {#if readable(entry.body) !== ""}<Text pre>{readable(entry.body)}</Text>{/if}
        </View>
      {/each}
    {/if}
  </View>
</Scroll>
