<script lang="ts">
  // The sessions sidebar (Ctrl-S): every session grouped by repository and, under it, by
  // worktree, newest first. Pressing a row shows that session. F6 gives the sidebar the keyboard:
  // arrows move, [ and ] jump between groups, Enter opens, Esc gives the keyboard back.
  import { untrack } from "svelte";
  import { Button, Icon, List, Text, View } from "@neoworks/svelte-gpui/ui";
  import type { KeyEvent } from "@neoworks/svelte-gpui/ui";
  import { clock } from "../../widgets/clock.svelte.ts";
  import { useCtx } from "../../kernel/slots/context.ts";
  import SessionRow from "./SessionRow.svelte";
  import newSession from "@phosphor-icons/core/regular/note-pencil.svg";
  import { groupSessions, jumpGroup, lastSegment, placeOf, step, type Entry } from "./sessions.ts";
  import { icons } from "./icons.svelte.ts";
  import { sessions } from "./state.svelte.ts";

  /** Rows a PgUp / PgDn moves. */
  const PAGE = 5;
  const WIDTH = 256;

  const { daemon } = useCtx();

  const p = $derived(daemon.palette);
  const grouped = $derived(groupSessions(daemon.sessions, sessions.places));
  /** Each row names its project and branch, so the list draws the sessions without headers. */
  const rows = $derived(
    grouped.entries.filter((entry): entry is Extract<Entry, { kind: "session" }> => entry.kind === "session"),
  );

  $effect(() => {
    const repos = rows.map((entry) => placeOf(entry.info.cwd, sessions.places).repo);
    untrack(() => icons.ask(repos));
  });
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
    return rows.findIndex((entry) => entry.key === id);
  }

  /** Brings a session into view. */
  function reveal(id: string | undefined, _up: boolean): void {
    if (id === undefined) return;
    const index = indexOfSession(id);
    if (index < 0) return;
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
      background: p.side.bg,
    }}
  >
    <View
      style={{
        direction: "row",
        align: "center",
        gap: p.space.s,
        paddingLeft: 20,
        paddingRight: 12,
        height: 52,
      }}
    >
      <Text
        style={{
          flex: 1,
          minWidth: 0,
          fontWeight: "medium",
          color: sessions.focused ? p.accent : p.fg,
        }}
      >
        Sessions{#if sessions.finished.size > 0}<Text style={{ color: p.success, fontWeight: "normal" }}>{"  "}{sessions.finished.size} done</Text>{/if}
      </Text>
      <Button
        onpress={() => void daemon.newSession()}
        style={{ size: 28, align: "center", justify: "center", radius: p.radius.s, color: p.side.icon }}
        hover={{ background: p.side.hover, color: p.fg }}
      >
        <Icon svg={newSession} size={16} />
      </Button>
    </View>

    <View bind:this={keys} focusable onkey={key} onblur={leave} style={{ flex: 1, minHeight: 0 }}>
      <List
        bind:this={list}
        items={[...rows]}
        key={(entry: Entry) => entry.key}
        style={{ flex: 1, minHeight: 0, paddingX: 8 }}
      >
        {#snippet row(entry: Entry)}
          {#if entry.kind === "session"}
            {@const place = placeOf(entry.info.cwd, sessions.places)}
            <SessionRow
              info={entry.info}
              project={lastSegment(place.repo)}
              icon={icons.byRepo[place.repo]}
              branch={sessions.branches[entry.info.cwd] ?? place.worktree}
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
