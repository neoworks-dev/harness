<script lang="ts">
  // The agent's question (a permission or a choice) answered in place of the input: its title,
  // a compact look at what it concerns, and one button per answer. The keyboard drives it:
  // Left/Right (or Up/Down) move, Enter answers, 1-9 answer directly, Tab adds a note that goes
  // with the answer, Esc puts the question aside. A choice with more than 9 options is a filter
  // field and a scrolling list instead: typing narrows the list (digits too), Up/Down move over
  // the matches, Enter answers, Esc clears the filter first.
  import { Button, Input, Scroll, Text, View } from "@neoworks/svelte-gpui/ui";
  import type { KeyEvent, Layout, ScrollPosition } from "@neoworks/svelte-gpui/ui";
  import { useCtx } from "../../kernel/slots/context.ts";
  import { withTimeLeft } from "../../lib/time-left.ts";
  import { clock } from "../../widgets/clock.svelte.ts";
  import PromptBody from "./PromptBody.svelte";
  import { scrollToRow } from "./prompt.ts";
  import { composer } from "./state.svelte.ts";

  const { daemon } = useCtx();
  const p = $derived(daemon.palette);
  const prompt = $derived(composer.prompt);
  const focused = $derived(prompt?.buttons[composer.button]);
  const hasBody = $derived((prompt?.body.length ?? 0) > 0);

  // A question that answers itself counts down on the button it picks.
  const counting = $derived(prompt?.buttons.some((b) => b.deadline !== undefined) === true);
  $effect(() => {
    if (counting) return clock.join();
  });

  let root: ReturnType<typeof View> | undefined = $state();
  let noteInput: ReturnType<typeof Input> | undefined = $state();
  let filterInput: ReturnType<typeof Input> | undefined = $state();
  let scroller: ReturnType<typeof Scroll> | undefined = $state();
  let viewport = $state(0);
  let top = 0;

  /** A row of the filtered list, and how many show before it scrolls. */
  const ROW = 28;
  const VISIBLE_ROWS = 8;

  // The keys go to the panel; when the note field opens they go there, and back after it. A
  // filtered choice keeps them in its filter field.
  $effect(() => {
    if (composer.note !== undefined) noteInput?.focus();
    else if (composer.filtering) filterInput?.focus();
    else root?.focus();
  });

  // Scrolls the focused row of the filtered list into view.
  $effect(() => {
    const row = composer.shown.indexOf(composer.button);
    if (row < 0) return;
    const to = scrollToRow(row, ROW, top, viewport);
    if (to !== undefined) scroller?.scrollTo(to);
  });

  function key(event: KeyEvent): void {
    // Shortcuts with a modifier belong to the window (Ctrl-O, Shift-Tab, …).
    if (event.ctrl || event.alt || event.meta) return;
    switch (event.key) {
      case "ArrowLeft":
      case "ArrowUp":
        composer.moveButton(-1);
        break;
      case "ArrowRight":
      case "ArrowDown":
        composer.moveButton(1);
        break;
      case "Enter":
        composer.pressFocused();
        break;
      case "Tab":
        if (event.shift) return;
        composer.tab();
        break;
      default: {
        // On a filtered choice the digits are text for the filter.
        if (composer.filtering) return;
        const number = Number(event.key);
        if (Number.isInteger(number) && number >= 1 && number <= (prompt?.buttons.length ?? 0)) {
          composer.press(number - 1);
          break;
        }
        return;
      }
    }
    event.preventDefault();
    event.stopPropagation();
  }

  // Keys in the filter field: the arrows, Enter and Tab drive the list, Ctrl-U clears; the rest
  // is text for the field.
  function filterKey(event: KeyEvent): void {
    if (event.ctrl && !event.alt && !event.meta && event.key === "u") {
      composer.setFilter("");
    } else if (event.ctrl || event.alt || event.meta) {
      return;
    } else if (event.key === "ArrowUp") {
      composer.moveButton(-1);
    } else if (event.key === "ArrowDown") {
      composer.moveButton(1);
    } else if (event.key === "Enter") {
      composer.pressFocused();
    } else if (event.key === "Tab" && !event.shift) {
      composer.tab();
    } else {
      return;
    }
    event.preventDefault();
    event.stopPropagation();
  }

  function noteKey(event: KeyEvent): void {
    if (event.key !== "Enter" || event.shift || event.alt) return;
    event.preventDefault();
    event.stopPropagation();
    composer.pressFocused();
  }
</script>

{#if prompt}
  <View bind:this={root} focusable onkey={key} style={{ gap: p.space.m }}>
    <Text style={{ fontWeight: 600, truncate: true }}>{prompt.title || "The agent asks"}</Text>

    {#if hasBody}
      <Scroll style={{ maxHeight: 180 }}>
        <PromptBody components={prompt.body} />
      </Scroll>
    {/if}

    {#if composer.filtering}
      <View style={{ direction: "row", gap: p.space.m, align: "center" }}>
        <Text style={{ color: p.accent }}>filter</Text>
        <Input
          bind:this={filterInput}
          bind:value={composer.filter}
          placeholder="type to find an option"
          style={{ flex: 1, minWidth: 0, color: p.fg }}
          oninput={(value: string) => composer.setFilter(value)}
          onkey={filterKey}
        />
      </View>
      {#if composer.shown.length === 0}
        <Text style={{ color: p.muted }}>no match</Text>
      {:else}
        <Scroll
          bind:this={scroller}
          style={{ maxHeight: ROW * VISIBLE_ROWS }}
          onscroll={(position: ScrollPosition) => (top = position.y)}
          onlayout={(box: Layout) => (viewport = box.height)}
        >
          {#each composer.shown as index (index)}
            {@const button = prompt.buttons[index]}
            {@const chosen = index === composer.button}
            {#if button}
              <View
                style={{
                  direction: "row",
                  align: "center",
                  gap: p.space.m,
                  height: ROW,
                  shrink: 0,
                  paddingX: p.space.m,
                  radius: 5,
                  background: chosen ? p.accent : "transparent",
                  cursor: "pointer",
                }}
                onpress={() => {
                  composer.button = index;
                  composer.press(index);
                }}
              >
                <Text
                  style={{
                    color: chosen ? p.bg : p.fg,
                    fontWeight: button.current ? 600 : 400,
                    minWidth: 0,
                    truncate: true,
                  }}
                >
                  {button.label}{button.current ? " ✓" : ""}
                </Text>
                {#if button.description}
                  <Text
                    style={{
                      color: chosen ? p.bg : p.muted,
                      flex: 1,
                      minWidth: 0,
                      truncate: true,
                    }}
                  >
                    {button.description}
                  </Text>
                {/if}
              </View>
            {/if}
          {/each}
        </Scroll>
      {/if}
    {:else}
      <View style={{ direction: "row", wrap: true, gap: p.space.m }}>
        {#each prompt.buttons as button, index}
          {@const chosen = index === composer.button}
          <Button
            onpress={() => {
              composer.button = index;
              composer.press(index);
            }}
            style={{
              direction: "row",
              gap: p.space.s + 2,
              paddingX: p.space.m + 2,
              paddingY: p.space.s + 1,
              radius: 6,
              borderWidth: 1,
              borderColor: chosen ? p.accent : p.border,
              background: chosen ? p.accent : p.surface,
              color: chosen ? p.bg : p.fg,
              cursor: "pointer",
            }}
          >
            {#if index < 9}
              <Text style={{ color: chosen ? p.bg : p.muted }}>{index + 1}</Text>
            {/if}
            <Text style={{ fontWeight: button.current ? 600 : 400 }}>{withTimeLeft(button.label, button.deadline, clock.now)}{button.current ? " ✓" : ""}</Text>
          </Button>
        {/each}
      </View>
    {/if}

    {#if composer.note !== undefined}
      <View style={{ direction: "row", gap: p.space.m, align: "center" }}>
        <Text style={{ color: p.accent }}>note</Text>
        <Input
          bind:this={noteInput}
          bind:value={composer.note}
          placeholder={`for the agent, Enter answers "${focused?.label ?? ""}" with it`}
          style={{ flex: 1, minWidth: 0, color: p.fg }}
          onkey={noteKey}
        />
      </View>
    {:else}
      {#if focused?.description && !composer.filtering}
        <Text style={{ color: p.muted }}>{focused.description}</Text>
      {/if}
      <Text style={{ color: p.muted, fontSize: p.text.body - 1 }}>
        {#if composer.filtering}
          type to filter · ↑/↓ choose · Enter answers · Tab adds a note · Esc {composer.filter === ""
            ? "writes a message instead"
            : "clears the filter"}
        {:else}
          ←/→ choose · Enter answers · Tab adds a note · Esc writes a message instead
        {/if}
      </Text>
    {/if}
  </View>
{/if}
