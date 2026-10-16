<script lang="ts">
  // The agents working for the session, under the input (Ctrl-X gives the list the keyboard):
  // one row per subagent or guest while it runs and a few seconds after, with the harness and
  // model, what it is doing, how long it has run and its tokens. Enter opens an agent's own chat,
  // x stops it, Esc goes back to the input.
  import { untrack } from "svelte";
  import { Button, Text, View } from "@neoworks/svelte-gpui/ui";
  import type { KeyEvent } from "@neoworks/svelte-gpui/ui";
  import {
    MAX_ROWS,
    activityOf,
    compact,
    dirLabel,
    elapsed,
    firstSelection,
    modelLabel,
    moveAgent,
    shownAgents,
    type AgentRow,
  } from "./agents.ts";
  import { useCtx } from "../../kernel/slots/context.ts";
  import { clock } from "../../widgets/clock.svelte.ts";
  import { agents } from "./state.svelte.ts";

  const { daemon, agentview } = useCtx();

  const p = $derived(daemon.palette);
  const shown = $derived(shownAgents(agents.rows, clock.now));
  const visible = $derived(shown.slice(0, MAX_ROWS));
  const hidden = $derived(shown.length - visible.length);
  let keys: ReturnType<typeof View> | undefined = $state();
  let selected = $state<string>();
  /** Agents the user asked to stop, until their end arrives. */
  let stopping = $state.raw<ReadonlySet<string>>(new Set());

  // The elapsed times tick while there is a list, and the lingering rows leave on time.
  $effect(() => {
    if (agents.rows.length === 0) return;
    return clock.join();
  });

  // Ctrl-X: the keys start on the latest running agent. With nothing to select it is given back.
  $effect(() => {
    if (!agents.focused) return;
    untrack(() => {
      selected = firstSelection(shown);
      if (selected === undefined) agents.focused = false;
      else keys?.focus();
    });
  });

  function leave(): void {
    agents.focused = false;
  }

  function open(row: AgentRow): void {
    leave();
    // An agent that works as a session of its own in another directory is that session.
    if (row.child !== undefined) {
      void daemon.open(row.child);
    } else {
      agentview.open({
        harness: row.harness,
        model: row.model === undefined ? undefined : modelLabel(row.model),
        task: row.task,
      });
    }
  }

  function stop(row: AgentRow): void {
    if (row.state !== "running") return;
    stopping = new Set([...stopping, row.id]);
    // The same action the terminal app sends: the daemon ends the delegation.
    void daemon.action("agents", "suite.cancel_agent", { id: row.id });
  }

  function key(event: KeyEvent): void {
    if (event.ctrl || event.alt || event.meta) return;
    const current = shown.find((row) => row.id === selected);
    switch (event.key) {
      case "ArrowUp":
      case "k":
        selected = moveAgent(shown, selected, -1);
        break;
      case "ArrowDown":
      case "j":
        selected = moveAgent(shown, selected, 1);
        break;
      case "Enter":
        if (current) open(current);
        else leave();
        break;
      case "x":
        if (current) stop(current);
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
    event.stopPropagation();
  }

  function glyph(row: AgentRow): { mark: string; color: string } {
    switch (row.state) {
      case "running":
        return { mark: "○", color: p.accent };
      case "done":
        return { mark: "✓", color: p.success };
      case "failed":
        return { mark: "✗", color: p.danger };
      case "stopped":
        return { mark: "✗", color: p.muted };
    }
  }

  function name(row: AgentRow): string {
    let label = row.harness;
    if (row.model !== undefined) label += ` · ${modelLabel(row.model)}`;
    if (row.child !== undefined && row.cwd !== undefined) label += ` ⎇ ${dirLabel(row.cwd)}`;
    return label;
  }

  function stats(row: AgentRow): string {
    const time = elapsed((row.endedMs ?? clock.now) - row.startedMs);
    return row.tokens === undefined ? time : `${time} · ${compact(row.tokens)} tokens`;
  }

  function doing(row: AgentRow): string {
    if (row.state === "running" && stopping.has(row.id)) return "stopping…";
    return activityOf(row, daemon.sessions);
  }
</script>

{#if shown.length > 0}
  <View
    bind:this={keys}
    focusable
    onkey={key}
    onblur={leave}
    style={{
      paddingX: p.space.l,
      paddingY: p.space.s,
      borderTopWidth: 1,
      borderColor: p.border,
      fontSize: p.text.body - 1,
    }}
  >
    <View style={{ direction: "row", gap: p.space.s }}>
      <Text style={{ color: daemon.status.running ? p.accent : p.muted }}>●</Text>
      <Text>main</Text>
      <Text style={{ color: p.muted }}>· {daemon.status.harness}</Text>
    </View>
    {#each visible as row (row.id)}
      {@const mark = glyph(row)}
      <View
        onpress={() => open(row)}
        hover={{ background: p.surface }}
        style={{
          direction: "row",
          gap: p.space.m,
          paddingY: 1,
          background: agents.focused && selected === row.id ? p.surface : "transparent",
          cursor: "pointer",
        }}
      >
        <Text style={{ color: mark.color }}>{mark.mark}</Text>
        <Text style={{ shrink: 0 }}>{name(row)}</Text>
        <Text
          style={{
            flex: 1,
            minWidth: 0,
            truncate: true,
            color: row.state === "failed" ? p.danger : p.muted,
          }}>{doing(row)}</Text
        >
        <Text style={{ shrink: 0, color: p.muted }}>{stats(row)}</Text>
        {#if row.state === "running"}
          <Button
            onpress={() => stop(row)}
            style={{ paddingX: p.space.s, radius: 4, color: p.muted }}
            hover={{ color: p.danger, background: p.band }}
          >
            <Text>stop</Text>
          </Button>
        {/if}
      </View>
    {/each}
    {#if hidden > 0}
      <Text style={{ color: p.muted }}>  … +{hidden} more</Text>
    {/if}
    {#if agents.focused}
      <Text style={{ color: p.muted }}>↑/↓ select · enter open · x stop · esc back</Text>
    {/if}
  </View>
{/if}
