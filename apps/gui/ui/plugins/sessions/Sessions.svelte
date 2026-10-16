<script lang="ts">
  // The sessions sidebar (Ctrl-S): every session grouped by repository and, under it, by
  // worktree, newest first. Pressing a row shows that session. F6 gives the sidebar the keyboard:
  // arrows move, [ and ] jump between groups, Enter opens, Esc gives the keyboard back.
  import { untrack } from "svelte";
  import { Button, List, Text, View } from "@neoworks/svelte-gpui/ui";
  import type { KeyEvent } from "@neoworks/svelte-gpui/ui";
  import { clock } from "../../widgets/clock.svelte.ts";
  import { useCtx } from "../../kernel/slots/context.ts";
  import SessionRow from "./SessionRow.svelte";
  import { groupSessions, jumpGroup, step, type Entry } from "./sessions.ts";
  import { sessions } from "./state.svelte.ts";

  /** Rows a PgUp / PgDn moves. */
  const PAGE = 5;
  const WIDTH = 270;

  const { daemon } = useCtx();

  const p = $derived(daemon.palette);
  const grouped = $derived(groupSessions(daemon.sessions, sessions.places));
  let list: ReturnType<typeof List<Entry>> | undefined = $state();
  let keys: ReturnType<typeof View> | undefined = $state();
  /** The row the keys are on while the sidebar has the keyboard. */
  let selected = $state<string>();

  // Ages tick while the sidebar is drawn.
  $effect(() => {
    if (sessions.open) return clock.join();
  });

  // The keys start on the shown session.
  $effect(() => {
    if (!sessions.focused || !sessions.open) return;
    untrack(() => {
      selected = daemon.sessionId ?? grouped.order[0];
      keys?.focus();
    });
  });

  function indexOfSession(id: string): number {
    return grouped.entries.findIndex((entry) => entry.kind === "session" && entry.key === id);
  }

  /** Brings a session into view, with the headers above it when it starts its group. */
  function reveal(id: string | undefined, up: boolean): void {
    if (id === undefined) return;
    let index = indexOfSession(id);
    if (index < 0) return;
    if (up) {
      while (index > 0 && grouped.entries[index - 1]?.kind !== "session") index -= 1;
    }
    list?.scrollToIndex(index, { align: "nearest" });
  }

  // The shown session stays in view.
  $effect(() => {
    const id = daemon.sessionId;
    if (id !== undefined && grouped.order.includes(id)) untrack(() => reveal(id, false));
  });

  function moveTo(id: string | undefined, up: boolean): void {
    if (id === undefined) return;
    selected = id;
    reveal(id, up);
  }

  function leave(): void {
    sessions.focused = false;
  }

  function open(id: string): void {
    if (id !== daemon.sessionId) void daemon.open(id);
  }

  function key(event: KeyEvent): void {
    if (event.ctrl || event.alt || event.meta) return;
    switch (event.key) {
      case "ArrowUp":
      case "k":
        moveTo(step(grouped.order, selected, -1), true);
        break;
      case "ArrowDown":
      case "j":
        moveTo(step(grouped.order, selected, 1), false);
        break;
      case "PageUp":
        moveTo(step(grouped.order, selected, -PAGE), true);
        break;
      case "PageDown":
        moveTo(step(grouped.order, selected, PAGE), false);
        break;
      case "[":
        moveTo(jumpGroup(grouped, selected, -1), true);
        break;
      case "]":
        moveTo(jumpGroup(grouped, selected, 1), false);
        break;
      case "Home":
      case "g":
        moveTo(grouped.order[0], true);
        break;
      case "End":
      case "G":
        moveTo(grouped.order[grouped.order.length - 1], false);
        break;
      case "Enter":
        if (selected !== undefined) open(selected);
        leave();
        break;
      case "Escape":
      case "q":
      case "h":
      case "ArrowLeft":
        leave();
        break;
      default:
        return;
    }
    event.preventDefault();
  }
</script>

{#if sessions.open}
  <View
    style={{
      width: WIDTH,
      shrink: 0,
      background: p.surface,
      borderRightWidth: 1,
      borderColor: p.border,
    }}
  >
    <View
      style={{
        direction: "row",
        align: "center",
        gap: p.space.s,
        paddingX: p.space.m,
        paddingY: p.space.m,
      }}
    >
      <Text
        style={{
          flex: 1,
          minWidth: 0,
          color: sessions.focused ? p.accent : p.muted,
          fontWeight: sessions.focused ? "bold" : "normal",
        }}
      >
        Sessions · {daemon.sessions.length}{#if sessions.finished.size > 0}<Text style={{ color: p.success }}>{" · "}{sessions.finished.size} done</Text>{/if}
      </Text>
      <Button
        onpress={() => void daemon.newSession()}
        style={{ paddingX: p.space.m, paddingY: 2, radius: 4, color: p.accent }}
        hover={{ background: p.band }}
      >
        <Text>+ New</Text>
      </Button>
    </View>

    <View bind:this={keys} focusable onkey={key} onblur={leave} style={{ flex: 1, minHeight: 0 }}>
      <List
        bind:this={list}
        items={[...grouped.entries]}
        key={(entry: Entry) => entry.key}
        style={{ flex: 1, minHeight: 0 }}
      >
        {#snippet row(entry: Entry)}
          {#if entry.kind === "cluster"}
            <Text
              style={{
                paddingX: p.space.m,
                paddingTop: p.space.m,
                paddingBottom: 2,
                color: p.muted,
                fontWeight: "bold",
                truncate: true,
              }}>{entry.name}</Text
            >
          {:else if entry.kind === "group"}
            <Text
              style={{ paddingLeft: p.space.l, paddingY: 2, color: p.muted, truncate: true }}
              >⎇ {entry.name}</Text
            >
          {:else}
            <SessionRow
              info={entry.info}
              child={entry.child}
              current={entry.key === daemon.sessionId}
              finished={sessions.finished.has(entry.key)}
              selected={sessions.focused && selected === entry.key}
              nowMs={clock.now}
              onpress={() => open(entry.key)}
            />
          {/if}
        {/snippet}
      </List>
    </View>
  </View>
{/if}
