<script lang="ts">
  // A shell command as the terminal app shows one: after a muted `$`, broken after its `&&`, `||`,
  // `;` (and `|` when need be) once it is long, the parts after the first indented, and coloured
  // by the viewer's shell grammar: each command, its flags, strings and variables.
  import { Text } from "@neoworks/svelte-gpui/ui";
  import type { Style } from "@neoworks/svelte-gpui/ui";
  import { useCtx } from "../kernel/slots/context.ts";
  import { reflowCommand } from "../lib/shell.ts";
  import { forFile } from "../plugins/viewer/highlights.ts";
  import { kindStyles } from "../plugins/viewer/syntax-colors.ts";

  interface Props {
    command: string;
    /** Characters a line may take before it breaks after its operators. */
    width?: number;
    style?: Style;
  }

  let { command, width = 64, style = {} }: Props = $props();
  const { daemon } = useCtx();
  const p = $derived(daemon.palette);

  const lines = $derived(reflowCommand(command.trim(), width).split("\n"));
  const lights = $derived(forFile("command.sh", lines));
  const styles = $derived(kindStyles(p.syntax));
</script>

<Text pre style={{ fontFamily: "mono", fontSize: 12, lineHeight: 17, color: p.fg, ...style }}
  ><Text style={{ color: p.muted }}>{"$ "}</Text>{#each lines as line, index}{#if index > 0}{"\n  "}{/if}{#each lights?.tokens(index) ?? [{ text: line }] as token}<Text
        style={styles[token.kind ?? "plain"]}>{token.text}</Text
      >{/each}{/each}</Text
>
