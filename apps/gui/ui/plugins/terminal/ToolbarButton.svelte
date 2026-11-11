<script lang="ts">
  // Shows or hides the terminal, in the strip at the window's right edge.
  import terminalIcon from "@phosphor-icons/core/regular/terminal-window.svg";
  import ToolButton from "../../widgets/ToolButton.svelte";
  import { terminal } from "./state.svelte.ts";
</script>

<ToolButton svg={terminalIcon} active={terminal.shown} onpress={() => terminal.toggle()} />
