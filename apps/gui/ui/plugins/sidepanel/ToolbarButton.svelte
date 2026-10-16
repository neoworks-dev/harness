<script lang="ts">
  // Shows or hides the side panel (plan and files), in the strip at the window's right edge.
  import listChecks from "@phosphor-icons/core/regular/list-checks.svg";
  import ToolButton from "../../widgets/ToolButton.svelte";
  import { panel } from "./state.svelte.ts";
</script>

<ToolButton svg={listChecks} active={!panel.hidden} onpress={() => (panel.hidden = !panel.hidden)} />
