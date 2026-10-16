<script lang="ts">
  // Where the user types: a multiline input in a band, or, once the agent asks a question (a
  // permission or a choice) and the user has paused typing, the panel that answers it.
  import { untrack } from "svelte";
  import { Input, Overlay, Scroll, Text, View } from "@neoworks/svelte-gpui/ui";
  import type { KeyEvent, Layout } from "@neoworks/svelte-gpui/ui";
  import { suite } from "../state/suite.svelte.ts";
  import { ui } from "../state/ui.svelte.ts";
  import { composer } from "./composer.svelte.ts";
  import CommandMenu from "./CommandMenu.svelte";
  import Help from "./Help.svelte";
  import PromptPanel from "./PromptPanel.svelte";

  const p = $derived(suite.palette);
  /** The input grows with its text up to this many pixels, then scrolls. */
  const MAX_INPUT_HEIGHT = 200;
  const LINE_HEIGHT = 20;

  let input: ReturnType<typeof Input> | undefined = $state();
  let scroller: ReturnType<typeof Scroll> | undefined = $state();
  let width = $state(0);
  /** How many characters fit a row, roughly; the input does not say where its cursor is. */
  const columns = $derived(Math.floor(width / (p.text.body * 0.6)));

  const promptRef = $derived(composer.prompt?.ref);
  const showPanel = $derived(composer.engaged && composer.prompt !== undefined);

  // Another session: its own draft and history.
  $effect(() => {
    const session = suite.sessionId;
    untrack(() => void composer.switchSession(session));
  });

  // A question takes the input's place once the user has paused typing.
  $effect(() => {
    const wait = composer.syncEngaged();
    if (wait === undefined) return;
    const timer = setTimeout(() => composer.syncEngaged(), wait);
    return () => clearTimeout(timer);
  });

  // A new question starts on its current button, before it is drawn.
  $effect.pre(() => {
    void promptRef;
    untrack(() => composer.startPrompt(composer.prompt));
  });

  // The chat draws a one-line stub for the question the panel is answering.
  const answeringRef = $derived(showPanel ? promptRef : undefined);
  $effect(() => {
    if (answeringRef === undefined) return;
    ui.panelRef = answeringRef;
    return () => (ui.panelRef = undefined);
  });

  // The input has the keyboard unless the agents list or the GitHub view took it.
  $effect(() => {
    if (!showPanel && !ui.agentsFocused && !ui.githubOpen) input?.focus();
  });

  function key(event: KeyEvent): void {
    const plain = !event.ctrl && !event.alt && !event.meta;
    switch (event.key) {
      case "Enter":
        // Alt-Enter and Shift-Enter are left to the input, which breaks the line.
        if (event.alt || event.shift) return;
        event.preventDefault();
        composer.submit();
        break;
      case "Tab":
        if (event.shift || !composer.menuVisible) return;
        event.preventDefault();
        composer.complete();
        break;
      case "ArrowUp":
      case "ArrowDown": {
        if (!plain && !event.alt) return;
        const older = event.key === "ArrowUp";
        if (composer.menuVisible) {
          composer.moveMenu(older ? -1 : 1);
        } else if (!composer.walkHistory(older ? "older" : "newer", columns, event.alt)) {
          return;
        }
        event.preventDefault();
        break;
      }
      default:
        return;
    }
  }

  function typed(): void {
    composer.typed();
  }

  function layout(box: Layout): void {
    width = box.width;
  }

  /** Follows the end of the text while it grows past the cap. */
  function grown(box: Layout): void {
    if (box.height > MAX_INPUT_HEIGHT) scroller?.scrollTo(box.height);
  }
</script>

<View style={{ background: p.band, paddingX: p.space.l, paddingY: p.space.m, gap: p.space.s }}>
  {#if showPanel}
    <PromptPanel />
  {:else}
    {#if composer.waiting && composer.prompt}
      <Text style={{ color: p.muted, truncate: true }}>
        {composer.prompt.title || "A question"} is waiting. Stop typing to answer it, Esc puts it aside.
      </Text>
    {/if}
    <View style={{ direction: "row", gap: p.space.m }}>
      <Text style={{ color: p.accent, lineHeight: LINE_HEIGHT, fontWeight: 600 }}>›</Text>
      <View style={{ flex: 1, minWidth: 0 }} onlayout={layout}>
        <Scroll bind:this={scroller} style={{ maxHeight: MAX_INPUT_HEIGHT }}>
          <View onlayout={grown}>
            <Input
              bind:this={input}
              bind:value={composer.text}
              multiline
              rows={1}
              placeholder="Message the agent, / for commands"
              style={{ color: p.fg, lineHeight: LINE_HEIGHT }}
              oninput={typed}
              onkey={key}
            />
          </View>
        </Scroll>
        {#if composer.menuVisible}
          <Overlay side="top" align="start" offset={p.space.m} matchWidth>
            <CommandMenu onchosen={() => input?.focus()} />
          </Overlay>
        {/if}
        {#if composer.helpOpen}
          <Overlay side="top" align="start" offset={p.space.m} matchWidth>
            <Help />
          </Overlay>
        {/if}
      </View>
    </View>
  {/if}
</View>
