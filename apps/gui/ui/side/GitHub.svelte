<script lang="ts">
  // The GitHub view (Ctrl-R), in place of the chat: the issues and pull requests of the repository
  // `gh` finds for the session's directory, each with its conversation, and a way to hand one to
  // the model. Read-only. Keys: Tab or 1/2 switch lists, arrows move, Enter opens, / searches
  // (GitHub search syntax), o lists open or all, r reloads, s sends to the model, d shows a pull
  // request's diff, Esc goes back and closes.
  import { untrack } from "svelte";
  import { Button, Input, List, Scroll, Text, View } from "@neoworks/svelte-gpui/ui";
  import type { KeyEvent, ScrollPosition } from "@neoworks/svelte-gpui/ui";
  import type { Kind, Summary } from "../lib/github.ts";
  import { suite } from "../state/suite.svelte.ts";
  import { ui } from "../state/ui.svelte.ts";
  import { clock } from "./clock.svelte.ts";
  import GitHubDetail from "./GitHubDetail.svelte";
  import GitHubRow from "./GitHubRow.svelte";
  import { github } from "./github.svelte.ts";

  /** Rows a PgUp / PgDn moves; pixels an arrow scrolls a detail. */
  const PAGE = 8;
  const STEP = 48;
  const ROW_HEIGHT = 52;

  const p = $derived(suite.palette);
  const NO_ROWS: Summary[] = [];

  function listRows(loaded: typeof github.list): Summary[] {
    if (loaded?.status !== "ready") return NO_ROWS;
    return loaded.value;
  }

  let keys: ReturnType<typeof View> | undefined = $state();
  let search: ReturnType<typeof Input> | undefined = $state();
  let list: ReturnType<typeof List<Summary>> | undefined = $state();
  let scroller: ReturnType<typeof Scroll> | undefined = $state();
  let scrollY = $state(0);
  let scrollMax = $state(0);
  let typed = $state(github.query);

  const rows = $derived(listRows(github.list));
  const opened = $derived(github.open === undefined ? undefined : github.detail(github.open.kind, github.open.number));

  // The keys are the view's from the moment it opens.
  $effect(() => {
    if (ui.githubOpen) untrack(() => keys?.focus());
  });
  // Ages tick while the view is drawn.
  $effect(() => clock.join());

  function move(delta: number): void {
    if (rows.length === 0) return;
    github.selected = Math.min(rows.length - 1, Math.max(0, github.selected + delta));
    list?.scrollToIndex(github.selected, { align: "nearest" });
  }

  function openSelected(): void {
    const row = rows[github.selected];
    if (row) github.openItem(github.tab, row);
  }

  function close(): void {
    ui.githubOpen = false;
  }

  function switchTo(tab: Kind): void {
    github.tab = tab;
    github.selected = 0;
  }

  function applySearch(text: string): void {
    github.query = text.trim();
    github.selected = 0;
    keys?.focus();
  }

  function scrollBy(delta: number): void {
    scroller?.scrollTo(Math.min(scrollMax, Math.max(0, scrollY + delta)));
  }

  function track(position: ScrollPosition): void {
    scrollY = position.y;
    scrollMax = position.maxY;
  }

  function listKey(event: KeyEvent): boolean {
    switch (event.key) {
      case "ArrowUp":
      case "k":
        move(-1);
        return true;
      case "ArrowDown":
      case "j":
        move(1);
        return true;
      case "PageUp":
        move(-PAGE);
        return true;
      case "PageDown":
        move(PAGE);
        return true;
      case "Home":
      case "g":
        move(-rows.length);
        return true;
      case "End":
      case "G":
        move(rows.length);
        return true;
      case "Enter":
        openSelected();
        return true;
      case "Tab":
        switchTo(github.tab === "issue" ? "pr" : "issue");
        return true;
      case "1":
        switchTo("issue");
        return true;
      case "2":
        switchTo("pr");
        return true;
      case "/":
        search?.focus();
        return true;
      case "o":
        github.all = !github.all;
        github.selected = 0;
        return true;
      case "r":
        github.reload();
        return true;
      case "s": {
        const row = rows[github.selected];
        if (row) void github.send(github.tab, row.number);
        return true;
      }
      case "Escape":
      case "q":
        close();
        return true;
      default:
        return false;
    }
  }

  function detailKey(event: KeyEvent): boolean {
    const open = github.open;
    if (open === undefined) return false;
    switch (event.key) {
      case "ArrowUp":
      case "k":
        scrollBy(-STEP);
        return true;
      case "ArrowDown":
      case "j":
        scrollBy(STEP);
        return true;
      case "PageUp":
      case "b":
        scrollBy(-STEP * PAGE);
        return true;
      case "PageDown":
      case " ":
        scrollBy(STEP * PAGE);
        return true;
      case "Home":
      case "g":
        scrollBy(-scrollMax);
        return true;
      case "End":
      case "G":
        scrollBy(scrollMax);
        return true;
      case "r":
        github.reload();
        return true;
      case "s":
        void github.send(open.kind, open.number);
        return true;
      case "d":
        if (open.kind === "pr") void github.showDiff(open.number);
        return true;
      case "Escape":
      case "Backspace":
        github.back();
        return true;
      case "q":
        close();
        return true;
      default:
        return false;
    }
  }

  function key(event: KeyEvent): void {
    if (event.ctrl || event.alt || event.meta) return;
    const handled = github.open === undefined ? listKey(event) : detailKey(event);
    if (!handled) return;
    event.preventDefault();
    event.stopPropagation();
  }

  function searchKey(event: KeyEvent): void {
    if (event.key !== "Escape") return;
    event.preventDefault();
    event.stopPropagation();
    typed = github.query;
    keys?.focus();
  }
</script>

{#snippet tab(kind: Kind, label: string)}
  <Button
    onpress={() => (switchTo(kind), keys?.focus())}
    style={{
      paddingX: p.space.m,
      paddingY: p.space.s,
      radius: 4,
      color: github.tab === kind ? p.accent : p.muted,
      fontWeight: github.tab === kind ? "bold" : "normal",
      background: github.tab === kind ? p.surface : "transparent",
    }}
    hover={{ background: p.surface }}
  >
    <Text>{label}</Text>
  </Button>
{/snippet}

{#snippet message(text: string, color: string)}
  <View style={{ flex: 1, align: "center", justify: "center", padding: p.space.l }}>
    <Text style={{ color, textAlign: "center" }}>{text}</Text>
  </View>
{/snippet}

<View bind:this={keys} focusable onkey={key} style={{ flex: 1, minHeight: 0 }}>
  <View
    style={{
      direction: "row",
      align: "center",
      gap: p.space.s,
      paddingX: p.space.l,
      paddingY: p.space.s,
      background: p.surface,
      borderBottomWidth: 1,
      borderColor: p.border,
    }}
  >
    {#if github.open}
      <Button
        onpress={() => (github.back(), keys?.focus())}
        style={{ paddingX: p.space.m, paddingY: p.space.s, radius: 4, color: p.accent }}
        hover={{ background: p.band }}
      >
        <Text>◂ back</Text>
      </Button>
    {:else}
      {@render tab("issue", "Issues")}
      {@render tab("pr", "Pull requests")}
    {/if}
    <Text style={{ shrink: 1, minWidth: 0, truncate: true, color: p.muted }}>
      {github.repo?.status === "ready" ? github.repo.value : "GitHub"}
    </Text>
    <View style={{ flex: 1 }} />
    {#if !github.open}
      <Button
        onpress={() => ((github.all = !github.all), (github.selected = 0), keys?.focus())}
        style={{ paddingX: p.space.m, paddingY: p.space.s, radius: 4, color: p.muted }}
        hover={{ background: p.band }}
      >
        <Text>{github.all ? "all" : "open"}</Text>
      </Button>
      <Input
        bind:this={search}
        bind:value={typed}
        placeholder="Search (label:bug …)"
        onsubmit={applySearch}
        onkey={searchKey}
        style={{ width: 200, shrink: 1, minWidth: 80, paddingX: p.space.m, paddingY: p.space.s, radius: 4, background: p.bg, color: p.fg }}
      />
    {/if}
    <Button
      onpress={() => (github.reload(), keys?.focus())}
      style={{ paddingX: p.space.m, paddingY: p.space.s, radius: 4, color: p.muted }}
      hover={{ background: p.band }}
    >
      <Text>reload</Text>
    </Button>
    <Button
      onpress={close}
      style={{ paddingX: p.space.m, paddingY: p.space.s, radius: 4, color: p.muted }}
      hover={{ background: p.band }}
    >
      <Text>✕</Text>
    </Button>
  </View>

  {#if github.repo === undefined || github.repo.status === "loading"}
    {@render message("loading…", p.muted)}
  {:else if github.repo.status === "failed"}
    {@render message(github.repo.message, p.danger)}
  {:else if github.open}
    {#if opened === undefined || opened.status === "loading"}
      {@render message("loading…", p.muted)}
    {:else if opened.status === "failed"}
      {@render message(opened.message, p.danger)}
    {:else}
      <GitHubDetail detail={opened.value} nowMs={clock.now} onscroll={track} bind:scroller />
    {/if}
  {:else if github.list === undefined || github.list.status === "loading"}
    {@render message("loading…", p.muted)}
  {:else if github.list.status === "failed"}
    {@render message(github.list.message, p.danger)}
  {:else if rows.length === 0}
    {@render message(github.query === "" ? "Nothing here." : "Nothing matches.", p.muted)}
  {:else}
    <List
      bind:this={list}
      items={rows}
      rowHeight={ROW_HEIGHT}
      key={(row: Summary) => row.number}
      style={{ flex: 1, minHeight: 0 }}
    >
      {#snippet row(item: Summary, index: number)}
        <GitHubRow
          row={item}
          selected={index === github.selected}
          nowMs={clock.now}
          onpress={() => ((github.selected = index), github.openItem(github.tab, item))}
        />
      {/snippet}
    </List>
  {/if}

  <View style={{ paddingX: p.space.l, paddingY: p.space.s, borderTopWidth: 1, borderColor: p.border }}>
    <Text style={{ color: p.muted, truncate: true }}>
      {github.open
        ? `esc back · s send to the model${github.open.kind === "pr" ? " · d diff" : ""} · r reload · q close`
        : "tab switch · ↑/↓ enter open · / search · o open/all · s send to the model · r reload · esc close"}
    </Text>
  </View>
</View>
