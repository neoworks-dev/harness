<script lang="ts">
  import { widgetBoxOf } from "@neoworks/neoshell-sdk";
  import type { ExtensionViewProps } from "@neoworks/neoshell-sdk";
  import type { AgentRow, UsageTotals } from "../src/summary.ts";
  import { costLabel, headline, meterClass, ROWS_BY_SIZE, rowsOf, statusLabel, todayLabel, toneOf } from "./view.ts";

  let { source, args }: ExtensionViewProps = $props();

  let agents = $state<AgentRow[]>([]);
  let usage = $state<UsageTotals>({ connected: false, costToday: 0, sessionsToday: 0, working: 0, asking: 0 });

  const box = $derived(widgetBoxOf(args));
  const shown = $derived(agents.slice(0, ROWS_BY_SIZE[box.size] ?? 3));

  $effect(() => source.subscribe("agents", (data) => (agents = rowsOf(data))));
  $effect(() => source.subscribe("usage", (data) => (usage = data as UsageTotals)));

  function open(row: AgentRow): void {
    void source.call("open", { session: row.id });
  }
</script>

<gtkbox class="card" orientation="vertical" spacing={8} hexpand vexpand>
  <gtkbox orientation="vertical" spacing={2}>
    <gtklabel class="card-title" halign="start" ellipsize="end">{headline(usage)}</gtklabel>
    <gtklabel class="card-hint" halign="start" ellipsize="end">{todayLabel(usage)}</gtklabel>
  </gtkbox>

  {#if shown.length === 0}
    <gtklabel class="card-hint" halign="start" valign="center" vexpand wrap xalign={0}>
      No sessions yet. Start one with neo.
    </gtklabel>
  {:else}
    <gtkbox orientation="vertical" spacing={2} vexpand>
      {#each shown as row (row.id)}
        <gtkpressable class="card-row" orientation="vertical" spacing={3} tooltip={row.preview} onpress={() => open(row)}>
          <gtkbox orientation="horizontal" spacing={8}>
            <gtkbox class={toneOf(row.status)} valign="center"></gtkbox>
            <gtklabel halign="start" hexpand ellipsize="end">{row.title}</gtklabel>
            {#if box.size !== "small"}
              <gtklabel class="card-hint" halign="end">{statusLabel(row)}</gtklabel>
              <gtklabel class="card-hint" halign="end" tabular>{costLabel(row.cost)}</gtklabel>
            {/if}
          </gtkbox>
          {#if box.size === "large"}
            {#if row.preview !== ""}
              <gtklabel class="card-hint" halign="start" ellipsize="end">{row.preview}</gtklabel>
            {/if}
            {#if row.contextFill !== null}
              <gtklevelbar
                class={meterClass(row.contextFill)}
                orientation="horizontal"
                min={0}
                max={1}
                value={row.contextFill}
                hexpand
              ></gtklevelbar>
            {/if}
          {/if}
        </gtkpressable>
      {/each}
    </gtkbox>
  {/if}
</gtkbox>
