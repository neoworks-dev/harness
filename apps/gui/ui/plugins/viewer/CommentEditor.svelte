<script lang="ts">
  // The small editor under the selected lines: what to say about them, as in a pull request review:
  // a comment, a question (the agent answers and changes nothing) or a request to rewrite them.
  // Enter adds it, and it waits in the input as a chip that goes with the next message; Esc drops
  // it; Ctrl-1, 2, 3 (or the chips) choose the kind.
  import { Button, Input, Text, View } from "@neoworks/svelte-gpui/ui";
  import type { KeyEvent } from "@neoworks/svelte-gpui/ui";
  import { kindLabel, rangeLabel, type CommentKind } from "../../lib/comments.ts";
  import { useCtx } from "../../kernel/slots/context.ts";
  import { EDITOR_HEIGHT } from "./rows.ts";
  import { viewer } from "./state.svelte.ts";

  const KINDS: readonly CommentKind[] = ["comment", "question", "rewrite"];
  const PLACEHOLDER: Record<CommentKind, string> = {
    comment: "Say something about these lines",
    question: "Ask about these lines",
    rewrite: "How should these lines be rewritten?",
  };

  const { daemon } = useCtx();
  const p = $derived(daemon.palette);

  let input: ReturnType<typeof Input> | undefined = $state();
  const place = $derived(viewer.place);
  const empty = $derived(viewer.draft.trim() === "");

  // The editor has the keyboard from the moment it is drawn: a tick later, so the key that opened
  // it (c, ?, r) is over and its character not typed into it.
  $effect(() => {
    const field = input;
    if (field === undefined) return;
    const timer = setTimeout(() => field.focus(), 0);
    return () => clearTimeout(timer);
  });

  function key(event: KeyEvent): void {
    if (event.key === "Escape") {
      event.preventDefault();
      viewer.clearSelection();
    } else if (event.key === "Enter" && !event.shift && !event.alt) {
      event.preventDefault();
      viewer.submit();
    } else if (event.ctrl && ["1", "2", "3"].includes(event.key)) {
      event.preventDefault();
      viewer.kind = KINDS[Number(event.key) - 1]!;
    }
  }
</script>

<View style={{ height: EDITOR_HEIGHT, paddingX: 12, paddingY: 8, background: p.bg }}>
  <View
    style={{ flex: 1, minHeight: 0, gap: 6, padding: 8, radius: 8, background: p.surface, borderWidth: 1, borderColor: p.border }}
  >
    <View style={{ direction: "row", align: "center", gap: 4 }}>
      {#each KINDS as kind, index}
        <Button
          onpress={() => (viewer.kind = kind)}
          style={{
            paddingX: 8,
            height: 20,
            radius: 5,
            align: "center",
            background: viewer.kind === kind ? p.accent : "transparent",
            color: viewer.kind === kind ? p.bg : p.muted,
          }}
          hover={{ color: viewer.kind === kind ? p.bg : p.fg }}
        >
          <Text style={{ fontSize: 11 }}>{kindLabel(kind)} <Text style={{ opacity: 0.6 }}>^{index + 1}</Text></Text>
        </Button>
      {/each}
      <Text style={{ flex: 1, minWidth: 0, fontSize: 11, color: p.muted, truncate: true, textAlign: "right" }}>
        {place ? rangeLabel(place) : "the selected lines"}
      </Text>
    </View>
    <Input
      bind:this={input}
      bind:value={viewer.draft}
      multiline
      rows={2}
      placeholder={PLACEHOLDER[viewer.kind]}
      style={{ fontSize: 13, color: p.fg, flex: 1, minHeight: 0 }}
      onkey={key}
    />
    <View style={{ direction: "row", justify: "end", gap: 6 }}>
      <Button
        onpress={() => viewer.clearSelection()}
        style={{ paddingX: 10, height: 24, radius: 6, align: "center", color: p.muted }}
        hover={{ background: p.hover, color: p.fg }}
      >
        <Text style={{ fontSize: 12 }}>Cancel</Text>
      </Button>
      <Button
        onpress={() => viewer.submit()}
        disabled={empty}
        style={{ paddingX: 10, height: 24, radius: 6, align: "center", background: p.accent, color: p.bg }}
        hover={{ opacity: 0.9 }}
        disabledStyle={{ opacity: 0.5 }}
      >
        <Text style={{ fontSize: 12 }}>{kindLabel(viewer.kind)}</Text>
      </Button>
    </View>
  </View>
</View>
