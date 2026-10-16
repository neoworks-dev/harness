<script lang="ts">
  // The column at the right, shown while there is a plan or there are changed files: the plan
  // above, the files below.
  import { window } from "@neoworks/svelte-gpui";
  import { View } from "@neoworks/svelte-gpui/ui";
  import { fileEntries } from "../lib/files.ts";
  import { suite } from "../state/suite.svelte.ts";
  import { activity } from "./activity.svelte.ts";
  import Files from "./Files.svelte";
  import { git } from "./git.svelte.ts";
  import Plan from "./Plan.svelte";

  const WIDTH = 280;
  /** The column is only worth its width from here on. */
  const MIN_WINDOW_WIDTH = 900;

  const p = $derived(suite.palette);
  const hasPlan = $derived(activity.plans.length > 0);
  const hasFiles = $derived(fileEntries(git.status, activity.touches.touched).length > 0);
</script>

{#if suite.sessionId !== undefined && window.size.width >= MIN_WINDOW_WIDTH && (hasPlan || hasFiles)}
  <View
    style={{
      width: WIDTH,
      shrink: 0,
      gap: p.space.l,
      padding: p.space.m,
      background: p.surface,
      borderLeftWidth: 1,
      borderColor: p.border,
    }}
  >
    {#if hasPlan}<Plan />{/if}
    {#if hasFiles}<Files />{/if}
  </View>
{/if}
