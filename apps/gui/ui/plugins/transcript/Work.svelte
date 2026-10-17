<script lang="ts">
  // The row of a group of work: one entry draws as itself; several under a header that says what
  // they did, which opens to the entries in a box that scrolls past a cap (a List is not nested in
  // a List; a Scroll with a height limit is). While the group runs the header says what the
  // latest of them is doing.
  import { Icon, Scroll, Text, View } from "@neoworks/svelte-gpui/ui";
  import caretDown from "@phosphor-icons/core/regular/caret-down.svg";
  import caretRight from "@phosphor-icons/core/regular/caret-right.svg";
  import { useCtx } from "../../kernel/slots/context.ts";
  import Spinner from "../../widgets/Spinner.svelte";
  import Entry from "./Entry.svelte";
  import { iconOf } from "./icons.ts";
  import ItemFrame from "./ItemFrame.svelte";
  import { GUTTER, softBorder } from "./layout.ts";
  import { opened } from "./opened.svelte.ts";
  import type { Row } from "./rows.ts";
  import { transcript } from "./state.svelte.ts";
  import { groupLabel, leadKind } from "./work.ts";

  let { row }: { row: Row } = $props();
  const it = $derived(row.kind === "work" ? row : undefined);

  const { daemon } = useCtx();
  const p = $derived(daemon.palette);

  /** The most a group's box shows before it scrolls. */
  const MAX_HEIGHT = 288;

  const only = $derived(it?.entries.length === 1 ? it?.entries[0] : undefined);
  const key = $derived(`${daemon.sessionId}/group:${it?.key}`);
  const open = $derived(transcript.expanded || opened.has(key));
  const failed = $derived(it?.entries.some((entry) => entry.failed));
</script>

{#if it}

<ItemFrame gap={only?.kind === "question" ? 16 : 6}>
  {#if only}
    <Entry entry={only} />
  {:else}
    <View
      onpress={() => opened.toggle(key)}
      hover={{ background: p.hover, color: p.fg }}
      style={{ direction: "row", align: "center", gap: 6, minHeight: 24, paddingX: 2, paddingY: 2, radius: p.radius.s, fontSize: 14, color: p.muted, cursor: "pointer" }}
    >
      <View style={{ width: GUTTER, height: 24, align: "center", justify: "center", color: failed ? p.danger : undefined, opacity: it.live || failed ? 1 : 0.7 }}>
        {#if it.live}
          <Spinner />
        {:else}
          <Icon svg={iconOf(leadKind(it.entries))} size={16} />
        {/if}
      </View>
      <Text style={{ flex: 1, minWidth: 0, truncate: true }}>{groupLabel(it.entries, it.live)}</Text>
      <Icon svg={open ? caretDown : caretRight} size={12} />
    </View>
    {#if open}
      <View style={{ marginLeft: 12, marginTop: 2, borderLeftWidth: 1, borderColor: softBorder(p) }}>
        <Scroll style={{ maxHeight: MAX_HEIGHT }}>
          <View style={{ paddingLeft: 8 }}>
            {#each it.entries as entry (entry.id)}
              <Entry {entry} grouped />
            {/each}
          </View>
        </Scroll>
      </View>
    {/if}
  {/if}
</ItemFrame>
{/if}
