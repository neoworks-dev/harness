<script lang="ts">
  // The commands the agent and its guests ran, in a panel beside the conversation: each a header
  // (how it stands, what it is for, how it ended and how long it took), the command, and its output
  // as it comes, mono. Follows the newest output while it is scrolled to the end.
  import { Button, Icon, List, Text, View } from "@neoworks/svelte-gpui/ui";
  import xIcon from "@phosphor-icons/core/regular/x.svg";
  import { useCtx } from "../../kernel/slots/context.ts";
  import Spinner from "../../widgets/Spinner.svelte";
  import { clock } from "../../widgets/clock.svelte.ts";
  import { duration } from "../transcript/format.ts";
  import { codeColor } from "../transcript/layout.ts";
  import { timeline } from "../transcript/timeline.svelte.ts";
  import { terminalRuns, type TerminalRun } from "./runs.ts";
  import { terminal } from "./state.svelte.ts";

  const { daemon } = useCtx();
  const p = $derived(daemon.palette);
  const mono = { fontFamily: "mono" as const, fontSize: 12, lineHeight: 17 };

  const runs = $derived(
    terminalRuns({
      items: daemon.items,
      timeline: timeline.value,
      cwd: daemon.cwd,
      running: daemon.status.running,
    }),
  );
  const anyRunning = $derived(runs.some((run) => run.state === "running"));
  $effect(() => {
    if (anyRunning) return clock.join();
  });

  let list: { scrollToIndex(index: number, options: { align: "end" }): void } | undefined = $state();
  // Opened (or another session): start at the newest command.
  let started: string | undefined;
  $effect(() => {
    const session = daemon.sessionId;
    if (session === undefined || runs.length === 0 || started === session) return;
    started = session;
    list?.scrollToIndex(runs.length - 1, { align: "end" });
  });

  function took(run: TerminalRun): string | undefined {
    if (run.startedAt === undefined) return undefined;
    return duration((run.endedAt ?? clock.now) - run.startedAt);
  }
</script>

<View style={{ flex: 1, minHeight: 0 }}>
  <View style={{ direction: "row", align: "center", height: 44, paddingLeft: 16, paddingRight: 8, shrink: 0, borderBottomWidth: 1, borderColor: p.border }}>
    <Text style={{ fontWeight: "medium" }}>Terminal</Text>
    <Text style={{ color: p.muted, marginLeft: 8 }}>{runs.length === 0 ? "" : `${runs.length} command${runs.length === 1 ? "" : "s"}`}</Text>
    <View style={{ flex: 1 }} />
    <Button
      onpress={() => (terminal.shown = false)}
      style={{ size: 28, radius: p.radius.s, align: "center", justify: "center", color: p.muted }}
      hover={{ background: p.hover, color: p.fg }}
    >
      <Icon svg={xIcon} size={14} />
    </Button>
  </View>
  {#if runs.length === 0}
    <View style={{ flex: 1, align: "center", justify: "center", padding: 24 }}>
      <Text style={{ color: p.muted, textAlign: "center" }}>No commands yet. They show here once the agent runs one.</Text>
    </View>
  {:else}
    <List
      bind:this={list}
      items={runs}
      key={(run: TerminalRun) => run.id}
      stickToEnd
      style={{ flex: 1, minHeight: 0 }}
    >
      {#snippet row(run: TerminalRun)}
        {@const time = took(run)}
        <View style={{ paddingX: 12, paddingTop: 10, paddingBottom: 4, gap: 6 }}>
          <View style={{ direction: "row", align: "center", gap: 8, paddingX: 8, paddingY: 4, radius: p.radius.s, background: p.surface }}>
            {#if run.state === "running"}
              <View style={{ color: p.accent }}><Spinner /></View>
            {:else if run.state === "failed"}
              <Text style={{ color: p.danger }}>✗</Text>
            {:else}
              <Text style={{ color: p.success }}>✓</Text>
            {/if}
            <Text style={{ flex: 1, minWidth: 0, truncate: true, color: run.description ? p.fg : p.muted }}>
              {#if run.agent}<Text style={{ color: p.accent }}>{run.agent}{" "}</Text>{/if}{run.description ?? (run.state === "running" ? "running" : "ran")}
            </Text>
            {#if run.state === "failed" && run.exit !== undefined}
              <Text style={{ color: p.danger, shrink: 0 }}>exit {run.exit}</Text>
            {/if}
            {#if time}<Text style={{ color: p.muted, shrink: 0 }}>{time}</Text>{/if}
          </View>
          <Text pre style={{ ...mono, color: codeColor(p), paddingX: 8 }}><Text style={{ color: p.muted }}>$ </Text>{run.command}</Text>
          {#if run.hidden > 0}
            <Text style={{ ...mono, color: p.muted, paddingX: 8 }}>… {run.hidden} earlier lines</Text>
          {/if}
          {#if run.output.trim() !== ""}
            <View style={{ borderLeftWidth: 2, borderColor: p.border, paddingLeft: 10, marginLeft: 8 }}>
              <Text pre style={{ ...mono, color: p.muted }}>{run.output}</Text>
            </View>
          {:else if run.state !== "running"}
            <Text style={{ ...mono, italic: true, color: p.muted, paddingX: 8 }}>no output</Text>
          {/if}
        </View>
      {/snippet}
    </List>
  {/if}
</View>
