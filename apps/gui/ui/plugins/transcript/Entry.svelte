<script lang="ts">
  // One row of work: an icon, a muted label, and at its end what it has to say (an exit code, a
  // status), its time while the pointer is over it, and a chevron if it has more to show. Pressed,
  // it opens its detail under it (Detail.svelte). A question to answer is always open, and a stub
  // while the composer answers it.
  import { Icon, Text, View } from "@neoworks/svelte-gpui/ui";
  import caretDown from "@phosphor-icons/core/regular/caret-down.svg";
  import caretRight from "@phosphor-icons/core/regular/caret-right.svg";
  import warning from "@phosphor-icons/core/regular/warning-circle.svg";
  import { useCtx } from "../../kernel/slots/context.ts";
  import Spinner from "../../widgets/Spinner.svelte";
  import Component from "./Component.svelte";
  import Detail from "./Detail.svelte";
  import { clockTime } from "./format.ts";
  import { iconOf } from "./icons.ts";
  import { GUTTER } from "./layout.ts";
  import { opened } from "./opened.svelte.ts";
  import { transcript } from "./state.svelte.ts";
  import { hasButtons, isForm } from "./components.ts";
  import type { WorkEntry } from "./work.ts";

  interface Props {
    entry: WorkEntry;
    /** Inside a group: no room above and below the row. */
    grouped?: boolean;
  }

  let { entry, grouped = false }: Props = $props();

  const { daemon } = useCtx();
  const p = $derived(daemon.palette);

  const asks = $derived(entry.kind === "question" && hasButtons(entry.item.component));
  const key = $derived(`${daemon.sessionId}/${entry.id}`);
  /** A UI with fields or buttons is there to be used: it is open. */
  const usable = $derived(isForm(entry.item.component));
  /** Always open: a question to answer, a UI to use. */
  const pinned = $derived(asks || usable);
  /**
   * Open until pressed: an edit with something to show, as in the terminal (what was written stays
   * in sight). A command is one line there, its output only when opened (or Ctrl-O): so here too.
   */
  const startsOpen = $derived(entry.kind === "edit" && (entry.files ?? []).some((file) => file.diff !== undefined));
  const toggled = $derived(opened.has(key));
  const open = $derived(
    pinned ||
      transcript.expanded ||
      (startsOpen ? !toggled : toggled) ||
      (entry.kind === "thought" && transcript.showThoughts),
  );
  const stub = $derived(asks && transcript.stubbed === entry.id && !transcript.expanded);
  let hover = $state(false);

  /** A question that waits for its answer is not work in progress. */
  const spins = $derived(entry.live && entry.kind !== "question");
  const tint = $derived.by(() => {
    if (entry.failed) return p.danger;
    if (entry.kind === "plan") return p.success;
    return undefined;
  });
  /** What a failed command exited with; a command that exited 0, or whose status is unknown, says nothing. */
  const exit = $derived(entry.kind === "command" && entry.exit !== undefined && entry.exit !== 0 ? entry.exit : undefined);
</script>

{#if entry.kind === "thought" && transcript.showThoughts}
  <!-- A thought, shown (Ctrl-T hides them): the text itself, muted, in the flow of the turn. -->
  <View style={{ paddingLeft: GUTTER + 8, paddingY: 2, color: p.muted, fontSize: 14, italic: true }}>
    <Component component={entry.item.component} ref={entry.item.ref} />
  </View>
{:else}
<View>
  <View
    onpress={pinned ? undefined : () => opened.toggle(key)}
    onhover={(h: boolean) => (hover = h)}
    hover={pinned ? undefined : { background: p.hover, color: p.fg }}
    style={{
      direction: "row",
      align: "center",
      gap: 6,
      minHeight: 24,
      paddingX: 2,
      paddingY: grouped ? 0 : 2,
      radius: p.radius.s,
      fontSize: 14,
      color: asks ? p.fg : p.muted,
      cursor: pinned ? undefined : "pointer",
    }}
  >
    <View style={{ width: GUTTER, height: 24, align: "center", justify: "center", color: tint, opacity: spins || tint ? 1 : 0.7 }}>
      {#if spins}
        <Spinner />
      {:else if entry.failed && entry.kind !== "command"}
        <Icon svg={warning} size={16} />
      {:else}
        <Icon svg={iconOf(entry.kind)} size={16} />
      {/if}
    </View>
    {#if stub}
      <Text style={{ flex: 1, minWidth: 0, truncate: true }}><Text style={{ color: p.fg, fontWeight: "medium" }}>{entry.label}</Text>{" · answer below"}</Text>
    {:else}
      <Text style={{ flex: 1, minWidth: 0, truncate: true, fontWeight: asks ? "medium" : "normal" }}>{entry.label}</Text>
    {/if}
    {#if exit !== undefined}
      <Text style={{ color: p.danger, fontSize: 12, fontFamily: "mono" }}>exit {exit}</Text>
    {:else if entry.status !== undefined}
      <Text style={{ fontSize: 12 }}>{entry.status}</Text>
    {/if}
    {#if entry.at !== undefined}
      <Text style={{ fontSize: 12, opacity: hover ? 1 : 0 }}>{clockTime(entry.at)}</Text>
    {/if}
    {#if !pinned}
      <Icon svg={open ? caretDown : caretRight} size={12} />
    {/if}
  </View>
  {#if open && !stub}
    <Detail {entry} />
  {/if}
</View>
{/if}
