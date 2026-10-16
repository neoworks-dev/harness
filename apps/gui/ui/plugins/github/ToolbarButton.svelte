<script lang="ts">
  // The GitHub view's toggle (Ctrl-R) in the strip at the window's right edge.
  import githubLogo from "@phosphor-icons/core/regular/github-logo.svg";
  import ToolButton from "../../widgets/ToolButton.svelte";
  import { github } from "./state.svelte.ts";
</script>

<ToolButton svg={githubLogo} active={github.shown} onpress={() => (github.shown = !github.shown)} />
