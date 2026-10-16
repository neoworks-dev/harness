<script lang="ts">
  // Shows or hides the sessions (Ctrl-S), at the start of the bar over the conversation.
  import sidebar from "@phosphor-icons/core/regular/sidebar-simple.svg";
  import ToolButton from "../../widgets/ToolButton.svelte";
  import { sessions } from "./state.svelte.ts";
</script>

<ToolButton svg={sidebar} active={sessions.open} onpress={() => (sessions.open = !sessions.open)} />
