<script lang="ts">
  // Where the conversation is, in the bar over it: the project's icon and name, then the title.
  import { Text, View } from "@neoworks/svelte-gpui/ui";
  import { useCtx } from "../../kernel/slots/context.ts";
  import { icons } from "./icons.svelte.ts";
  import ProjectIcon from "./ProjectIcon.svelte";
  import { lastSegment, placeOf, sessionTitle } from "./sessions.ts";
  import { sessions } from "./state.svelte.ts";

  const { daemon } = useCtx();
  const p = $derived(daemon.palette);
  const session = $derived(daemon.session);
  const place = $derived(session ? placeOf(session.cwd, sessions.places) : undefined);
</script>

{#if session && place}
  <View style={{ direction: "row", align: "center", gap: 10, minWidth: 0, paddingLeft: 4 }}>
    <View style={{ direction: "row", align: "center", gap: 6, maxWidth: 180 }}>
      <ProjectIcon name={lastSegment(place.repo)} src={icons.byRepo[place.repo]} size={14} />
      <Text style={{ color: p.muted, fontWeight: "medium", truncate: true }}>{lastSegment(place.repo)}</Text>
    </View>
    <Text style={{ color: p.muted }}>/</Text>
    <Text style={{ fontWeight: "medium", truncate: true, minWidth: 0 }}>{sessionTitle(session)}</Text>
  </View>
{/if}
