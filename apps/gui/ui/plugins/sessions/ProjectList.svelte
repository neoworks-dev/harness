<script lang="ts">
  // What the project popover holds: a search field, "All projects" when the list is to be
  // filtered, then each project with its icon and how many sessions it has. Up and Down move,
  // Enter picks; it takes the keyboard when it opens.
  import { untrack } from "svelte";
  import { Button, Icon, Input, List, Text, View } from "@neoworks/svelte-gpui/ui";
  import type { KeyEvent } from "@neoworks/svelte-gpui/ui";
  import check from "@phosphor-icons/core/regular/check.svg";
  import search from "@phosphor-icons/core/regular/magnifying-glass.svg";
  import { useCtx } from "../../kernel/slots/context.ts";
  import { matchProjects, type Project } from "./arrange.ts";
  import { icons } from "./icons.svelte.ts";
  import ProjectIcon from "./ProjectIcon.svelte";

  interface Props {
    projects: readonly Project[];
    /** The project the list is scoped to, if any: marked with a check. */
    current: string | undefined;
    /** Offer "All projects" first. */
    all: boolean;
    /** The repository picked; undefined for "All projects". */
    onpick: (repo: string | undefined) => void;
  }

  /** What a row is: "All projects" has no repository. */
  interface Choice {
    key: string;
    repo: string | undefined;
    name: string;
    count: number;
  }

  const ROW = 34;
  const SHOWN = 7;

  let { projects, current, all, onpick }: Props = $props();
  const { daemon } = useCtx();
  const p = $derived(daemon.palette);
  let query = $state("");
  let at = $state(0);
  let field: ReturnType<typeof Input> | undefined = $state();
  let list: ReturnType<typeof List<Choice>> | undefined = $state();

  const choices = $derived.by(() => {
    const out: Choice[] = [];
    if (all && "all projects".includes(query.trim().toLowerCase())) {
      out.push({ key: "all", repo: undefined, name: "All projects", count: projects.reduce((n, x) => n + x.count, 0) });
    }
    for (const project of matchProjects(projects, query)) {
      out.push({ key: `repo ${project.repo}`, repo: project.repo, name: project.name, count: project.count });
    }
    return out;
  });

  $effect(() => {
    const repos = projects.map((project) => project.repo);
    untrack(() => icons.ask(repos));
  });
  $effect(() => {
    untrack(() => field?.focus());
  });
  // A new search starts on its first result.
  $effect(() => {
    void query;
    untrack(() => (at = 0));
  });

  function move(delta: number): void {
    at = Math.min(choices.length - 1, Math.max(0, at + delta));
    list?.scrollToIndex(at, { align: "nearest" });
  }

  function key(event: KeyEvent): void {
    if (event.ctrl || event.alt || event.meta) return;
    if (event.key === "ArrowDown") move(1);
    else if (event.key === "ArrowUp") move(-1);
    else if (event.key === "Enter") {
      const choice = choices[at];
      if (choice) onpick(choice.repo);
    } else return;
    event.preventDefault();
  }
</script>

<View style={{ width: 240 }}>
  <View style={{ direction: "row", align: "center", gap: 8, paddingX: 12, height: 40, borderBottomWidth: 1, borderColor: p.border, color: p.muted }}>
    <Icon svg={search} size={16} />
    <Input
      bind:this={field}
      bind:value={query}
      placeholder="Search projects…"
      onkey={key}
      style={{ flex: 1, minWidth: 0, color: p.fg }}
    />
  </View>
  {#if choices.length === 0}
    <Text style={{ padding: 14, color: p.muted }}>No projects found</Text>
  {:else}
    <List
      bind:this={list}
      items={choices}
      rowHeight={ROW}
      key={(choice: Choice) => choice.key}
      style={{ height: Math.min(choices.length, SHOWN) * ROW + 12, paddingX: 6, paddingY: 6 }}
    >
      {#snippet row(choice: Choice, index: number)}
        <Button
          onpress={() => onpick(choice.repo)}
          style={{
            direction: "row",
            align: "center",
            gap: 8,
            height: ROW,
            paddingX: 8,
            radius: p.radius.s,
            color: p.fg,
            background: index === at ? p.raised : "transparent",
          }}
          hover={{ background: p.raised }}
        >
          {#if choice.repo !== undefined}
            <ProjectIcon name={choice.name} src={icons.byRepo[choice.repo]} />
          {/if}
          <Text style={{ flex: 1, minWidth: 0, truncate: true, fontWeight: "medium" }}>{choice.name}</Text>
          {#if choice.repo === current}
            <Icon svg={check} size={14} color={p.accent} />
          {:else}
            <Text style={{ fontSize: 12, color: p.muted }}>{choice.count}</Text>
          {/if}
        </Button>
      {/snippet}
    </List>
  {/if}
</View>
