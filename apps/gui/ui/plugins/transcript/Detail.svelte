<script lang="ts">
  // What a work row shows when it is open, by the kind of work: the command and what it printed,
  // the paths read, each file changed with a button for its diff, the plan's lines, the arguments
  // of a tool, a thought as written. Output is mono and muted in a box that scrolls past its cap.
  import { Button, Scroll, Text, View } from "@neoworks/svelte-gpui/ui";
  import { useCtx } from "../../kernel/slots/context.ts";
  import Component from "./Component.svelte";
  import { hasButtons } from "./components.ts";
  import { tailLines } from "./format.ts";
  import NeoUi from "./neoui/NeoUi.svelte";
  import { codeColor, DETAIL_INDENT, PREVIEW_LINES } from "./layout.ts";
  import { transcript } from "./state.svelte.ts";
  import type { WorkEntry } from "./work.ts";

  let { entry }: { entry: WorkEntry } = $props();

  const { daemon, viewer } = useCtx();
  const p = $derived(daemon.palette);

  /** The most lines of output drawn; the end of it, which is where the failure is. */
  const OUTPUT_LINES = 400;
  const mono = { fontFamily: "mono" as const, fontSize: 12 };

  const output = $derived(tailLines(entry.output ?? "", OUTPUT_LINES));
</script>

{#snippet block(text: string, cap: number, tone: string, hidden = 0)}
  <View style={{ background: p.surface, radius: p.radius.s }}>
    <Scroll style={{ maxHeight: cap }}>
      <View style={{ paddingX: 10, paddingY: 6 }}>
        {#if hidden > 0}
          <Text style={{ ...mono, color: p.muted }}>… {hidden} earlier lines</Text>
        {/if}
        <Text pre style={{ ...mono, lineHeight: 17, color: tone }}>{text}</Text>
      </View>
    </Scroll>
  </View>
{/snippet}

<View style={{ paddingLeft: DETAIL_INDENT, paddingY: 4, gap: 6 }}>
  {#if entry.kind === "command"}
    <Text pre style={{ ...mono, lineHeight: 17, color: codeColor(p) }}>{entry.command}</Text>
    {#if output.text.trim() !== ""}
      {@render block(output.text, 320, p.muted, output.hidden)}
    {:else}
      <Text style={{ italic: true, color: p.muted }}>No output.</Text>
    {/if}
  {:else if entry.kind === "read"}
    {#each entry.paths ?? [] as path}
      <Text style={{ ...mono, color: codeColor(p), truncate: true }}>{path}</Text>
    {/each}
  {:else if entry.kind === "edit"}
    {#each entry.files ?? [] as file}
      <View style={{ direction: "row", align: "center", gap: 8 }}>
        <Text style={{ ...mono, flex: 1, minWidth: 0, truncate: true, color: p.muted }}>{file.path}</Text>
        {#if file.diff}
          <Text style={mono}><Text style={{ color: p.success }}>+{file.added}</Text>{" "}<Text style={{ color: p.danger }}>−{file.removed}</Text></Text>
          <Button
            onpress={() => file.diff && viewer.showDiff(file.diff)}
            style={{ paddingX: 8, paddingY: 2, radius: 6, borderWidth: 1, borderColor: p.border, fontSize: 12, color: p.fg }}
            hover={{ background: p.hover }}
          >
            <Text>Open diff</Text>
          </Button>
        {/if}
      </View>
    {/each}
  {:else if entry.kind === "plan"}
    {#each entry.plan ?? [] as line}
      <Text style={{ ...mono, color: line.active ? p.fg : p.muted }}>{line.done ? "✓" : line.active ? "◐" : "○"} {line.text}</Text>
    {/each}
  {:else if entry.kind === "thought"}
    <View style={{ color: p.muted }}>
      <Component component={entry.item.component} ref={entry.item.ref} />
    </View>
  {:else if entry.kind === "question"}
    <Component
      component={entry.item.component}
      ref={entry.item.ref}
      limit={hasButtons(entry.item.component) && !transcript.expanded ? PREVIEW_LINES : undefined}
    />
  {:else if entry.item.component.type === "neoui"}
    <NeoUi ui={entry.item.component.ui} ref={entry.item.ref} inset={DETAIL_INDENT} />
  {:else}
    {#if entry.input}
      <Text style={{ fontSize: 11, color: p.muted }}>Tool input</Text>
      {@render block(entry.input, 256, codeColor(p))}
    {/if}
    {#if output.text.trim() !== ""}
      {@render block(output.text, 256, p.muted, output.hidden)}
    {:else if !entry.input}
      <Text style={{ italic: true, color: p.muted }}>No output.</Text>
    {/if}
  {/if}
</View>
