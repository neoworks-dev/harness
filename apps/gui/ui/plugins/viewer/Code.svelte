<script lang="ts">
  // The text of a line of code: 13px mono on one line, cut at the edge, or wrapped when the viewer's
  // word wrap is on. With the line's runs (from the syntax highlighting) each is drawn in its colour,
  // and the ones marked as what changed on `mark`, the strong background of a rewritten line.
  import { Text } from "@neoworks/svelte-gpui/ui";
  import { useCtx } from "../../kernel/slots/context.ts";
  import type { Run } from "./inline.ts";
  import { LINE_HEIGHT } from "./rows.ts";
  import { wrappable } from "./look.ts";
  import { kindStyles } from "./syntax-colors.ts";
  import { viewer } from "./state.svelte.ts";

  interface Props {
    text: string;
    tokens?: readonly Run[];
    inset?: number;
    /** The background of the runs that changed. */
    mark?: string;
  }

  let { text, tokens, inset = 0, mark = "" }: Props = $props();

  const { daemon } = useCtx();
  const plain = $derived(kindStyles(daemon.palette.syntax));
  const marked = $derived(kindStyles(daemon.palette.syntax, mark));
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
      <Text style={(token.mark ? marked : plain)[token.kind ?? "plain"]}>{wrappable(token.text, i === 0)}</Text>
    {/each}
  </Text>
{:else}
  <Text pre style={{ ...style, truncate: true }}>
    {#each tokens as token}
      <Text style={(token.mark ? marked : plain)[token.kind ?? "plain"]}>{token.text}</Text>
    {/each}
  </Text>
{/if}
