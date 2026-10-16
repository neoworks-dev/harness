<script lang="ts">
  // One session in the sidebar: its title and changed lines, then what was said last and how
  // long ago. The shown session has a bar in the margin; a running one a spinner; one that
  // finished unseen a green dot and a bold title; one that another session delegated to a ↳.
  import { Text, View } from "@neoworks/svelte-gpui/ui";
  import type { SessionInfo } from "@suite/protocol/types";
  import Spinner from "../../widgets/Spinner.svelte";
  import { useCtx } from "../../kernel/slots/context.ts";
  import { age, changedLines, previewOf, sessionTitle } from "./sessions.ts";

  interface Props {
    info: SessionInfo;
    /** Delegated to by another session. */
    child: boolean;
    current: boolean;
    finished: boolean;
    /** The keys are on this row. */
    selected: boolean;
    nowMs: number;
    onpress: () => void;
  }

  let { info, child, current, finished, selected, nowMs, onpress }: Props = $props();
  const { daemon } = useCtx();
  const p = $derived(daemon.palette);
  const changed = $derived(changedLines(info));
  const preview = $derived(previewOf(info));
  const emphasis = $derived(current || finished);
</script>

<View
  {onpress}
  hover={{ background: p.band }}
  style={{
    direction: "row",
    paddingRight: p.space.m,
    paddingY: p.space.s,
    borderLeftWidth: 2,
    borderColor: current ? p.accent : "transparent",
    background: selected ? p.band : "transparent",
    cursor: "pointer",
  }}
>
  <View style={{ width: 20, align: "center", color: finished ? p.success : p.accent }}>
    {#if info.state === "running"}
      <Spinner />
    {:else if finished}
      <Text>●</Text>
    {/if}
  </View>
  <View style={{ flex: 1, minWidth: 0 }}>
    <View style={{ direction: "row", gap: p.space.s }}>
      <Text
        style={{
          flex: 1,
          minWidth: 0,
          truncate: true,
          fontWeight: emphasis ? "bold" : "normal",
          color: current ? p.accent : p.fg,
          paddingLeft: child ? p.space.m : 0,
        }}>{child ? "↳ " : ""}{sessionTitle(info)}</Text
      >
      {#if changed.added > 0 || changed.removed > 0}
        <Text style={{ fontSize: p.text.body - 2 }}>
          {#if changed.added > 0}<Text style={{ color: p.success }}>+{changed.added}</Text>{/if}
          {#if changed.added > 0 && changed.removed > 0}{" "}{/if}
          {#if changed.removed > 0}<Text style={{ color: p.danger }}>−{changed.removed}</Text>{/if}
        </Text>
      {/if}
    </View>
    <View style={{ direction: "row", gap: p.space.s, paddingLeft: child ? p.space.m : 0 }}>
      <Text style={{ flex: 1, minWidth: 0, truncate: true, color: p.muted, fontSize: p.text.body - 2 }}>
        {#if info.queued > 0}<Text style={{ color: p.accent }}>{info.queued} queued</Text>{preview === "" ? "" : " · "}{/if}{preview}
      </Text>
      <Text style={{ color: p.muted, fontSize: p.text.body - 2 }}>{age(info.lastActivity, nowMs)}</Text>
    </View>
  </View>
</View>
