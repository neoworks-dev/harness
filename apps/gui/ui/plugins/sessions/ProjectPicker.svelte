<script lang="ts">
  // A popover to pick a project from, under its trigger: the filter button (which scopes the
  // list) and the new-session button when shift is held (which starts a session in the one picked).
  import type { Snippet } from "svelte";
  import { Popover } from "@neoworks/svelte-gpui/ui";
  import type { PopoverApi } from "@neoworks/svelte-gpui/ui";
  import { useCtx } from "../../kernel/slots/context.ts";
  import type { Project } from "./arrange.ts";
  import ProjectList from "./ProjectList.svelte";

  interface Props {
    open: boolean;
    projects: readonly Project[];
    current: string | undefined;
    /** Offer "All projects". */
    all: boolean;
    onpick: (repo: string | undefined) => void;
    trigger: Snippet<[PopoverApi]>;
  }

  let { open = $bindable(), projects, current, all, onpick, trigger }: Props = $props();
  const { daemon } = useCtx();
  const p = $derived(daemon.palette);
</script>

<Popover
  bind:open
  {trigger}
  side="bottom"
  align="end"
  offset={6}
  focusPanel={false}
  anchorStyle={{ alignSelf: "center" }}
  style={{
    background: p.surface,
    radius: 14,
    borderWidth: 1,
    borderColor: p.border,
    overflow: "hidden",
    shadow: { y: 8, blur: 24, color: "rgba(0, 0, 0, 0.5)" },
  }}
>
  {#snippet children({ close })}
    <ProjectList
      {projects}
      {current}
      {all}
      onpick={(repo) => {
        close("select");
        onpick(repo);
      }}
    />
  {/snippet}
</Popover>
