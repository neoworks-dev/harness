<script lang="ts">
  // Shows or hides the workspace card (Ctrl-G), in the strip at the window's right edge.
  import info from "@phosphor-icons/core/regular/info.svg";
  import ToolButton from "../../widgets/ToolButton.svelte";
  import { workspace } from "./state.svelte.ts";
</script>

<ToolButton svg={info} active={!workspace.hidden} onpress={() => (workspace.hidden = !workspace.hidden)} />
