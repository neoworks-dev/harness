<script lang="ts">
  // `/help`: the keys, above the input. Esc closes it.
  import { Text, View } from "@neoworks/svelte-gpui/ui";
  import { useCtx } from "../../kernel/slots/context.ts";
  import { composer } from "./state.svelte.ts";

  const { daemon } = useCtx();
  const p = $derived(daemon.palette);

  const KEYS: readonly (readonly [string, string])[] = [
    ["Enter", "send"],
    ["Alt-Enter, Shift-Enter", "new line"],
    ["Up, Down", "earlier inputs (Alt-Up/Down also inside longer text)"],
    ["/", "commands"],
    ["Shift-Tab", "approvals: ask, review, edits, auto"],
    ["Ctrl-O", "full tool results"],
    ["Ctrl-T", "the agent's thoughts"],
    ["Ctrl-S", "sessions sidebar"],
    ["Ctrl-X", "agents list"],
    ["Ctrl-R", "GitHub"],
    ["Ctrl-D", "the diff and files"],
    ["Ctrl-N", "new session"],
    ["Esc", "close what is open, then cancel the turn"],
    ["Ctrl-C twice", "quit; the session keeps running"],
    ["On a question: Left, Right, Enter, Tab", "choose, answer, add a note (1-9 answers directly)"],
  ];
</script>

<View
  style={{
    background: p.surface,
    borderWidth: 1,
    borderColor: p.border,
    radius: 8,
    padding: p.space.m,
    gap: 2,
    shadow: { y: 8, blur: 24, color: "#00000099" },
  }}
  onpress={() => (composer.helpOpen = false)}
>
  {#each KEYS as [keys, what]}
    <View style={{ direction: "row", gap: p.space.l }}>
      <Text style={{ color: p.accent, width: 260, shrink: 0 }}>{keys}</Text>
      <Text style={{ color: p.muted, flex: 1, minWidth: 0 }}>{what}</Text>
    </View>
  {/each}
</View>
