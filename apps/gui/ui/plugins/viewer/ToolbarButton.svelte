<script lang="ts">
  // Shows or hides the viewer, in the strip at the window's right edge.
  import diffIcon from "@phosphor-icons/core/regular/git-diff.svg";
  import ToolButton from "../../widgets/ToolButton.svelte";
  import { viewer } from "./state.svelte.ts";
</script>

<ToolButton svg={diffIcon} active={viewer.shown} onpress={() => viewer.toggle()} />
