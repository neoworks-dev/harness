<script lang="ts">
  // A CommandOutput: `$ command` with its exit code and time, then the end of its output.
  import { Text, View } from "@neoworks/svelte-gpui/ui";
  import { useCtx } from "../../../kernel/slots/context.ts";
  import CodeBlock from "../CodeBlock.svelte";
  import { num, str } from "./props.ts";
  import type { NodeProps } from "./Node.svelte";

  /** Lines of output shown from its end. */
  const TAIL = 12;

  let { node }: NodeProps = $props();
  const { daemon } = useCtx();
  const p = $derived(daemon.palette);
  const exit = $derived(num(node, "exit"));
  const lines = $derived((str(node, "output") ?? "").replace(/\n+$/, "").split("\n"));
  const hidden = $derived(Math.max(0, lines.length - TAIL));
  const tail = $derived(lines.slice(hidden).join("\n"));
</script>

<View style={{ gap: 4 }}>
  <View style={{ direction: "row", gap: 8, align: "center" }}>
    <Text style={{ fontFamily: "mono", color: p.muted }}>$</Text>
    <Text style={{ fontFamily: "mono", fontWeight: "medium", color: p.fg, flex: 1, minWidth: 0, truncate: true }}>{str(node, "command") ?? ""}</Text>
    {#if exit !== undefined}
      <Text style={{ color: exit === 0 ? p.success : p.danger, nowrap: true }}>{exit === 0 ? "✓" : "✗"} exit {exit}</Text>
    {/if}
    {#if str(node, "duration")}
      <Text style={{ color: p.muted, nowrap: true }}>{str(node, "duration")}</Text>
    {/if}
  </View>
  {#if hidden > 0}
    <Text style={{ color: p.muted, fontSize: p.text.body - 1 }}>… {hidden} lines above</Text>
  {/if}
  {#if tail.trim() !== ""}
    <CodeBlock code={tail} />
  {/if}
</View>
