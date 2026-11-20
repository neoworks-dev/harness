<script lang="ts">
  // The background commands under the input, with the agents, as term lists them: one row each while it runs (and a
  // few seconds after it ends), with its last line of output and how long it has run. Pressing a
  // row shows it on its own in the terminal panel; its × stops it.
  import { Button, Icon, Text, View } from "@neoworks/svelte-gpui/ui";
  import xIcon from "@phosphor-icons/core/regular/x.svg";
  import { useCtx } from "../../kernel/slots/context.ts";
  import Spinner from "../../widgets/Spinner.svelte";
  import { clock } from "../../widgets/clock.svelte.ts";
  import { duration } from "../transcript/format.ts";
  import { terminalData } from "./data.svelte.ts";
  import { isListed, lastLine } from "./jobs.ts";
  import { terminal } from "./state.svelte.ts";

  const { daemon } = useCtx();
  const p = $derived(daemon.palette);
  /** Rows shown before the rest is folded into "+N more". */
  const MAX_ROWS = 4;

  const listed = $derived(
    terminalData.runs.flatMap((run) => {
      const job = terminalData.jobs[run.id];
      return job !== undefined && isListed(job, clock.now) ? [{ run, job }] : [];
    }),
  );
  // Ages tick, and an ended row leaves after a few seconds, while there is a row.
  const any = $derived(Object.values(terminalData.jobs).some((job) => isListed(job, Date.now())));
  $effect(() => {
    if (any) return clock.join();
  });

  async function stop(id: string): Promise<void> {
    const why = await terminalData.stop(id);
    if (why !== undefined) daemon.error = `Could not stop it: ${why}`;
  }
</script>

{#if listed.length > 0}
  <View style={{ gap: 2, marginX: 16, marginTop: 6 }}>
    {#each listed.slice(0, MAX_ROWS) as { run, job } (run.id)}
      {@const last = lastLine(job)}
      <View
        onpress={() => terminal.showJob(run.id)}
        hover={{ background: p.hover }}
        style={{ direction: "row", align: "center", gap: 8, height: 26, paddingX: 8, radius: p.radius.s, cursor: "pointer" }}
      >
        {#if job.exit === undefined}
          <View style={{ color: p.accent, shrink: 0 }}><Spinner /></View>
        {:else}
          <Text style={{ color: job.exit === 0 ? p.success : p.danger, shrink: 0 }}>{job.exit === 0 ? "✓" : "✗"}</Text>
        {/if}
        <Text style={{ fontFamily: "mono", fontSize: 12, shrink: 1, minWidth: 0, maxWidth: "45%", truncate: true }}>{run.description ?? run.command}</Text>
        <Text style={{ fontFamily: "mono", fontSize: 12, flex: 1, minWidth: 0, truncate: true, color: p.muted }}>{last ?? ""}</Text>
        <Text style={{ fontSize: 12, color: p.muted, shrink: 0 }}>{duration((job.endedAt ?? clock.now) - job.startedAt)}</Text>
        {#if job.exit === undefined}
          <Button
            onpress={() => void stop(run.id)}
            style={{ size: 20, radius: 4, align: "center", justify: "center", color: p.muted, shrink: 0 }}
            hover={{ background: p.hover, color: p.danger }}
          >
            <Icon svg={xIcon} size={12} />
          </Button>
        {/if}
      </View>
    {/each}
    {#if listed.length > MAX_ROWS}
      <Text style={{ fontSize: 12, color: p.muted, paddingX: 8 }}>+{listed.length - MAX_ROWS} more</Text>
    {/if}
  </View>
{/if}
