<script lang="ts">
  // One session in a column beside the tab's own: a bar with its title and a button to close it,
  // over the same page opened on that session (a frame of its own, a full client of the daemon:
  // its chat, input, status line and panels).
  import { Button, Icon, Text, View } from "@neoworks/svelte-gpui/ui";
  import xIcon from "@phosphor-icons/core/regular/x.svg";
  import { useCtx } from "../../kernel/slots/context.ts";
  import { sessionTitle } from "../../plugins/sessions/sessions.ts";
  import { columnSrc } from "./columns.ts";
  import { columns } from "./state.svelte.ts";

  let { session }: { session: string } = $props();
  const { daemon } = useCtx();
  const p = $derived(daemon.palette);
  const info = $derived(daemon.sessions.find((entry) => entry.id === session));
</script>

<View
  style={{
    flex: 1,
    minWidth: 420,
    marginLeft: 8,
    radius: 16,
    borderWidth: 1,
    borderColor: p.border,
    background: p.bg,
    overflow: "hidden",
  }}
>
  <View style={{ direction: "row", align: "center", gap: 8, height: 32, paddingLeft: 14, paddingRight: 6, shrink: 0, borderBottomWidth: 1, borderColor: p.border }}>
    <Text style={{ flex: 1, minWidth: 0, truncate: true, fontSize: 12, fontWeight: "medium", color: p.muted }}>
      {info ? sessionTitle(info) : session}
    </Text>
    <Button
      onpress={() => columns.close(session)}
      style={{ size: 24, radius: 6, align: "center", justify: "center", color: p.muted }}
      hover={{ background: p.hover, color: p.fg }}
    >
      <Icon svg={xIcon} size={13} />
    </Button>
  </View>
  <iframe
    title={info ? sessionTitle(info) : session}
    src={columnSrc(location.pathname, session)}
    style="flex: 1; min-height: 0; width: 100%; border: 0; background: transparent;"
  ></iframe>
</View>
