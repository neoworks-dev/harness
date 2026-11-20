<script lang="ts">
  // The panel in the conversation's place, inside its card: the input stays under it, so comments
  // can be sent from here. It slides in over where the chat was. A file opened from the chat has a
  // bar to go back (or Esc), and is reviewed with the keys, as a pull request is: j/k (or the
  // arrows) move a cursor over its lines, shift stretches it, n/N jump between the lines the edit
  // wrote, ] and [ go to the turn's next and previous file; c comments on the lines, ? asks about
  // them, r asks for them rewritten. Tab gives the keys to the input, a press on the file takes
  // them back.
  import { Button, Icon, Presence, Text, View } from "@neoworks/svelte-gpui/ui";
  import type { KeyEvent } from "@neoworks/svelte-gpui/ui";
  import { fade, fly } from "svelte/transition";
  import arrowLeft from "@phosphor-icons/core/regular/arrow-left.svg";
  import { useCtx } from "../../kernel/slots/context.ts";
  import Panel from "./Panel.svelte";
  import { viewer } from "./state.svelte.ts";

  /** Lines a PageUp / PageDown or Ctrl-U / Ctrl-D moves. */
  const PAGE = 20;

  const { daemon } = useCtx();
  const p = $derived(daemon.palette);
  let keys: ReturnType<typeof View> | undefined = $state();

  // Reviewing: the keys are the pane's.
  $effect(() => {
    if (viewer.reviewing) keys?.focus();
  });

  function key(event: KeyEvent): void {
    if (event.alt || event.meta) return;
    const shift = event.shift;
    if (event.ctrl) {
      if (event.key === "d") viewer.moveCursor(PAGE, false);
      else if (event.key === "u") viewer.moveCursor(-PAGE, false);
      else return;
      event.preventDefault();
      return;
    }
    switch (event.key) {
      case "j":
      case "ArrowDown":
        viewer.moveCursor(1, shift);
        break;
      case "J":
        viewer.moveCursor(1, true);
        break;
      case "k":
      case "ArrowUp":
        viewer.moveCursor(-1, shift);
        break;
      case "K":
        viewer.moveCursor(-1, true);
        break;
      case "PageDown":
        viewer.moveCursor(PAGE, shift);
        break;
      case "PageUp":
        viewer.moveCursor(-PAGE, shift);
        break;
      case "g":
      case "Home":
        viewer.moveCursor(-1e9, shift);
        break;
      case "G":
      case "End":
        viewer.moveCursor(1e9, shift);
        break;
      case "n":
        viewer.nextChange(1);
        break;
      case "N":
        viewer.nextChange(-1);
        break;
      case "]":
        viewer.reviewTurn(1);
        break;
      case "[":
        viewer.reviewTurn(-1);
        break;
      case "c":
      case "Enter":
        viewer.startComment("comment");
        break;
      case "?":
        viewer.startComment("question");
        break;
      case "r":
        viewer.startComment("rewrite");
        break;
      case "Tab":
        viewer.typing = true;
        break;
      case "Escape":
      case "q":
        viewer.close();
        break;
      default:
        return;
    }
    event.preventDefault();
    event.stopPropagation();
  }
</script>

<Presence in={[fly, { x: 48, duration: 180 }]} out={[fade, { duration: 120 }]} style={{ flex: 1, minHeight: 0 }}>
  {#if viewer.inPlace}
    <View style={{ direction: "row", align: "center", gap: 8, paddingX: 12, height: 36, shrink: 0 }}>
      <Button
        onpress={() => viewer.close()}
        style={{ direction: "row", align: "center", gap: 6, paddingX: 8, height: 26, radius: p.radius.s, color: p.muted }}
        hover={{ background: p.hover, color: p.fg }}
      >
        <Icon svg={arrowLeft} size={14} />
        <Text>Back to the chat</Text>
      </Button>
      <Text style={{ flex: 1, minWidth: 0, truncate: true, color: p.muted, fontSize: 12 }}>
        {viewer.typing
          ? "Esc back · press the file for its keys"
          : "Esc back · j k move · shift stretches · n next change · ] [ files · c comment · ? ask · r rewrite · Tab input"}
      </Text>
    </View>
  {/if}
  <View
    bind:this={keys}
    focusable
    onkey={key}
    onpress={() => (viewer.typing = false)}
    style={{ flex: 1, minHeight: 0 }}
  >
    <Panel />
  </View>
</Presence>
