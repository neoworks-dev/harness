<script lang="ts">
  // The mark of the harness a session runs on: pi, Claude, Codex, else a robot.
  import { Icon } from "@neoworks/svelte-gpui/ui";
  import claude from "./claude.svg";
  import openai from "@phosphor-icons/core/regular/open-ai-logo.svg";
  import pi from "@phosphor-icons/core/regular/pi.svg";
  import robot from "@phosphor-icons/core/regular/robot.svg";

  let {
    harness,
    size = 14,
    mono = false,
  }: { harness: string; size?: number; /** In the text's colour, Claude's mark too. */ mono?: boolean } =
    $props();

  /** Each harness's mark and colour; Claude's in its own orange. */
  const MARKS: Record<string, { svg: string; color?: string }> = {
    pi: { svg: pi },
    claude: { svg: claude, color: "#d97757" },
    codex: { svg: openai },
  };

  const mark = $derived(MARKS[harness] ?? { svg: robot });
</script>

<Icon svg={mark.svg} {size} color={mono ? undefined : mark.color} />
