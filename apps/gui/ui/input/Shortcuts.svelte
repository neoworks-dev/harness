<script lang="ts">
  // The window's keyboard shortcuts, in one place. They reach the window when the focused input
  // or panel does not use the key; `preventDefault` keeps a focused input from typing it too.
  import { suite } from "../state/suite.svelte.ts";
  import { ui } from "../state/ui.svelte.ts";
  import { composer } from "./composer.svelte.ts";

  const MODIFIER_KEYS = new Set(["Control", "Shift", "Alt", "Meta"]);

  function toggle(event: KeyboardEvent, change: () => void): void {
    event.preventDefault();
    change();
  }

  function keydown(event: KeyboardEvent): void {
    if (MODIFIER_KEYS.has(event.key)) return;
    const ctrl = event.ctrlKey && !event.altKey && !event.metaKey;
    const key = event.key.toLowerCase();

    // Ctrl-C is left to the input too, so copying a selection still works.
    if (ctrl && key === "c") {
      composer.ctrlC();
      return;
    }
    composer.otherKey();

    if (event.key === "Tab" && event.shiftKey) {
      toggle(event, () => void suite.cycleApprovals());
    } else if (event.key === "Escape") {
      // Something that took Escape itself (a popover, the diff overlay) has already closed.
      if (event.defaultPrevented) return;
      if (composer.escape()) event.preventDefault();
      else if (suite.status.running) toggle(event, () => void suite.cancel());
    } else if (!ctrl || event.shiftKey) {
      return;
    } else if (key === "o") {
      toggle(event, () => (ui.expanded = !ui.expanded));
    } else if (key === "t") {
      toggle(event, () => (ui.showThoughts = !ui.showThoughts));
    } else if (key === "s") {
      toggle(event, () => (ui.sidebarOpen = !ui.sidebarOpen));
    } else if (key === "x") {
      toggle(event, () => (ui.agentsFocused = !ui.agentsFocused));
    } else if (key === "r") {
      toggle(event, () => (ui.githubOpen = !ui.githubOpen));
    } else if (key === "n") {
      toggle(event, () => void suite.newSession());
    } else if (key === "p") {
      toggle(event, () => composer.reopenPrompts());
    }
  }
</script>

<svelte:window onkeydown={keydown} />
