<script lang="ts">
  // The sessions sidebar (Ctrl-S): every session grouped by repository and, under it, by
  // worktree, newest first. Pressing a row shows that session. F6 gives the sidebar the keyboard:
  // arrows move, [ and ] jump between groups, Enter opens, Esc gives the keyboard back.
  import { untrack } from "svelte";
  import { Button, List, Text, View } from "@neoworks/svelte-gpui/ui";
  import type { KeyEvent } from "@neoworks/svelte-gpui/ui";
  import { groupSessions, jumpGroup, step, type Entry } from "../lib/sessions.ts";
  import { suite } from "../state/suite.svelte.ts";
  import { ui } from "../state/ui.svelte.ts";
  import { clock } from "./clock.svelte.ts";
  import { finished } from "./finished.svelte.ts";
  import { places } from "./places.svelte.ts";
  import SessionRow from "./SessionRow.svelte";

  /** Rows a PgUp / PgDn moves. */
  const PAGE = 5;
  const WIDTH = 270;

  const p = $derived(suite.palette);
  const grouped = $derived(groupSessions(suite.sessions, places.found));
  let list: ReturnType<typeof List<Entry>> | undefined = $state();
  let keys: ReturnType<typeof View> | undefined = $state();
  /** The row the keys are on while the sidebar has the keyboard. */
  let selected = $state<string>();

  // The directories to look up on disk: only linked worktrees outside `.worktrees/` need it.
  $effect(() => places.ensure(suite.sessions.map((session) => session.cwd)));

  // Ages tick while the sidebar is drawn.
  $effect(() => {
    if (ui.sidebarOpen) return clock.join();
  });

  // F6 opens the sidebar and gives it the keyboard.
  function windowKey(event: KeyboardEvent): void {
    if (event.key !== "F6") return;
    event.preventDefault();
    ui.sidebarOpen = true;
    ui.sidebarFocused = !ui.sidebarFocused;
  }

  // The keys start on the shown session.
  $effect(() => {
    if (!ui.sidebarFocused || !ui.sidebarOpen) return;
    untrack(() => {
      selected = suite.sessionId ?? grouped.order[0];
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
    const id = suite.sessionId;
    if (id !== undefined && grouped.order.includes(id)) untrack(() => reveal(id, false));
  });

  function moveTo(id: string | undefined, up: boolean): void {
    if (id === undefined) return;
    selected = id;
    reveal(id, up);
  }

  function leave(): void {
    ui.sidebarFocused = false;
  }

  function open(id: string): void {
    if (id !== suite.sessionId) void suite.open(id);
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

<svelte:window onkeydown={windowKey} />

{#if ui.sidebarOpen}
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
          color: ui.sidebarFocused ? p.accent : p.muted,
          fontWeight: ui.sidebarFocused ? "bold" : "normal",
        }}
      >
        Sessions · {suite.sessions.length}{#if finished.ids.size > 0}<Text style={{ color: p.success }}>{" · "}{finished.ids.size} done</Text>{/if}
      </Text>
      <Button
        onpress={() => void suite.newSession()}
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
              current={entry.key === suite.sessionId}
              finished={finished.ids.has(entry.key)}
              selected={ui.sidebarFocused && selected === entry.key}
              nowMs={clock.now}
              onpress={() => open(entry.key)}
            />
          {/if}
        {/snippet}
      </List>
    </View>
  </View>
{/if}
