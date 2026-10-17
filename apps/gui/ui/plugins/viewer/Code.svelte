<script lang="ts">
  // The text of a line of code: 13px mono on one line, cut at the edge, or wrapped when the viewer's
  // word wrap is on. With the line's runs (from the syntax highlighting) each is drawn in its colour.
  import { Text } from "@neoworks/svelte-gpui/ui";
  import { useCtx } from "../../kernel/slots/context.ts";
  import { LINE_HEIGHT } from "./rows.ts";
  import { wrappable } from "./look.ts";
  import { kindStyles } from "./syntax-colors.ts";
  import type { Token } from "./syntax.ts";
  import { viewer } from "./state.svelte.ts";

  let { text, tokens, inset = 0 }: { text: string; tokens?: Token[]; inset?: number } = $props();

  const { daemon } = useCtx();
  const styles = $derived(kindStyles(daemon.palette.syntax));
  const style = $derived({
    flex: 1,
    minWidth: 0,
    paddingLeft: inset,
    fontFamily: "mono",
    fontSize: 13,
    lineHeight: LINE_HEIGHT,
  });
</script>

{#if tokens === undefined}
  {#if viewer.wrap}
    <Text {style}>{wrappable(text)}</Text>
  {:else}
    <Text pre style={{ ...style, truncate: true }}>{text}</Text>
  {/if}
{:else if viewer.wrap}
  <Text {style}>
    {#each tokens as token, i}
      {#if token.kind}
        <Text style={styles[token.kind]}>{wrappable(token.text, i === 0)}</Text>
      {:else}
        <Text>{wrappable(token.text, i === 0)}</Text>
      {/if}
    {/each}
  </Text>
{:else}
  <Text pre style={{ ...style, truncate: true }}>
    {#each tokens as token}
      {#if token.kind}
        <Text style={styles[token.kind]}>{token.text}</Text>
      {:else}
        <Text>{token.text}</Text>
      {/if}
    {/each}
  </Text>
{/if}
