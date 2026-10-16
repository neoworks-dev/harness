<script lang="ts">
  // The agent's question (a permission or a choice) answered in place of the input: its title,
  // a compact look at what it concerns, and one button per answer. The keyboard drives it:
  // Left/Right (or Up/Down) move, Enter answers, 1-9 answer directly, Tab adds a note that goes
  // with the answer, Esc puts the question aside.
  import { Button, Input, Scroll, Text, View } from "@neoworks/svelte-gpui/ui";
  import type { KeyEvent } from "@neoworks/svelte-gpui/ui";
  import { suite } from "../state/suite.svelte.ts";
  import PromptBody from "./PromptBody.svelte";
  import { composer } from "./composer.svelte.ts";

  const p = $derived(suite.palette);
  const prompt = $derived(composer.prompt);
  const focused = $derived(prompt?.buttons[composer.button]);
  const hasBody = $derived((prompt?.body.length ?? 0) > 0);

  let root: ReturnType<typeof View> | undefined = $state();
  let noteInput: ReturnType<typeof Input> | undefined = $state();

  // The keys go to the panel; when the note field opens they go there, and back after it.
  $effect(() => {
    if (composer.note === undefined) root?.focus();
    else noteInput?.focus();
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
        composer.press(composer.button);
        break;
      case "Tab":
        if (event.shift) return;
        composer.tab();
        break;
      default: {
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

  function noteKey(event: KeyEvent): void {
    if (event.key !== "Enter" || event.shift || event.alt) return;
    event.preventDefault();
    event.stopPropagation();
    composer.press(composer.button);
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
          <Text style={{ fontWeight: button.current ? 600 : 400 }}>{button.label}{button.current ? " ✓" : ""}</Text>
        </Button>
      {/each}
    </View>

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
      {#if focused?.description}
        <Text style={{ color: p.muted }}>{focused.description}</Text>
      {/if}
      <Text style={{ color: p.muted, fontSize: p.text.body - 1 }}>
        ←/→ choose · Enter answers · Tab adds a note · Esc writes a message instead
      </Text>
    {/if}
  </View>
{/if}
