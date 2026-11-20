<script lang="ts">
  // The sessions sidebar (Ctrl-S): the pinned sessions, then every other one grouped by project
  // and worktree, newest first, then the settled ones folded away at the bottom. The search
  // field (Ctrl-Shift-F) replaces the list with what matches; the header's buttons scope it to a
  // project, add one, and start a session. Pressing a row shows that session, a right press
  // opens its menu. F6 gives the sidebar the keyboard: arrows move, [ and ] jump between groups,
  // Enter opens, Esc gives the keyboard back.
  import { untrack } from "svelte";
  import { clipboard, files } from "@neoworks/svelte-gpui";
  import { Button, ContextMenu, Icon, Text, View, List } from "@neoworks/svelte-gpui/ui";
  import type { KeyEvent, MenuItem, PointerEvent } from "@neoworks/svelte-gpui/ui";
  import { clock } from "../../widgets/clock.svelte.ts";
  import { useCtx } from "../../kernel/slots/context.ts";
  import checkIcon from "@phosphor-icons/core/regular/check.svg";
  import copyIcon from "@phosphor-icons/core/regular/copy.svg";
  import filterIcon from "@phosphor-icons/core/regular/funnel.svg";
  import folderPlus from "@phosphor-icons/core/regular/folder-plus.svg";
  import newSession from "@phosphor-icons/core/regular/note-pencil.svg";
  import pinIcon from "@phosphor-icons/core/regular/push-pin.svg";
  import sparkle from "@phosphor-icons/core/regular/sparkle.svg";
  import undoIcon from "@phosphor-icons/core/regular/arrow-counter-clockwise.svg";
  import { arrange, inProject, projectsOf, resultLines, type Line } from "./arrange.ts";
  import HeaderButton from "./HeaderButton.svelte";
  import { icons } from "./icons.svelte.ts";
  import { isPinned } from "./marks.ts";
  import ProjectIcon from "./ProjectIcon.svelte";
  import ProjectPicker from "./ProjectPicker.svelte";
  import SearchField from "./SearchField.svelte";
  import { searchSessions, termsOf } from "./search.ts";
  import SessionRow from "./SessionRow.svelte";
  import SettledHeader from "./SettledHeader.svelte";
  import SlimRow from "./SlimRow.svelte";
  import { jumpGroup, lastSegment, notableBranch, placeOf, statusOf, step } from "./sessions.ts";
  import GroupHeader from "./GroupHeader.svelte";
  import caretRight from "@phosphor-icons/core/bold/caret-right-bold.svg";
  import columnsIcon from "@phosphor-icons/core/regular/columns.svg";
  import { target } from "../../kernel/target.ts";
  import { sessions } from "./state.svelte.ts";

  /** Rows a PgUp / PgDn moves. */
  const PAGE = 5;
  const WIDTH = 284;

  const { daemon } = useCtx();

  const p = $derived(daemon.palette);
  const projects = $derived(projectsOf(daemon.sessions, sessions.places));
  /** The project the list is scoped to; none once it has no sessions left. */
  const scope = $derived(projects.find((project) => project.repo === sessions.project));
  const visible = $derived(inProject(daemon.sessions, sessions.places, scope?.repo));
  const searching = $derived(sessions.query.trim() !== "");
  const terms = $derived(termsOf(sessions.query));
  /** The clock to the minute: what folds as idle is worked out again once a minute, not every tick. */
  const minute = $derived(Math.floor(clock.now / 60_000) * 60_000);
  const arranged = $derived(
    searching
      ? resultLines(
          searchSessions(visible, sessions.query, (info) =>
            lastSegment(placeOf(info.cwd, sessions.places).repo),
          ),
        )
      : arrange(visible, sessions.places, sessions.marks, {
          open: sessions.settledOpen,
          all: sessions.settledAll,
          groups: sessions.groupsOpen,
          keep: (info) =>
            info.id === daemon.sessionId ||
            info.state === "running" ||
            info.unseen === true ||
            sessions.finished.has(info.id),
          nowMs: minute,
        }),
  );
  const lines = $derived([...arranged.lines]);

  $effect(() => {
    const repos = daemon.sessions.map((entry) => placeOf(entry.cwd, sessions.places).repo);
    untrack(() => icons.ask(repos));
  });
  let list: ReturnType<typeof List<Line>> | undefined = $state();
  let keys: ReturnType<typeof View> | undefined = $state();
  let search: ReturnType<typeof SearchField> | undefined = $state();
  let menu: ReturnType<typeof ContextMenu> | undefined = $state();
  /** The row the keys are on while the sidebar has the keyboard, or while searching. */
  let selected = $state<string>();
  /** The new-session button was pressed with shift: asking which project. */
  let asking = $state(false);
  let filtering = $state(false);
  /** The searches asked for so far that the field has answered. */
  let answered = 0;

  // Ages and elapsed times tick while the sidebar is drawn.
  $effect(() => {
    if (sessions.open) return clock.join();
  });

  // Ctrl-Shift-F gives the search field the keyboard.
  $effect(() => {
    const asked = sessions.searchRequest;
    if (asked === answered || !search) return;
    answered = asked;
    untrack(() => search?.focus());
  });

  // The keys start on the shown session.
  $effect(() => {
    if (!sessions.focused || !sessions.open) return;
    untrack(() => {
      selected = daemon.sessionId ?? arranged.order[0];
      keys?.focus();
    });
  });

  // A search starts on its best result, and keeps the one the keys are on while it still shows.
  $effect(() => {
    void sessions.query;
    untrack(() => (selected = arranged.order[0]));
  });
  $effect(() => {
    const order = arranged.order;
    if (!searching) return;
    untrack(() => {
      if (selected === undefined || !order.includes(selected)) selected = order[0];
    });
  });

  function indexOfSession(id: string): number {
    return lines.findIndex((line) => line.kind === "session" && line.key === id);
  }

  /** Brings a session into view. */
  function reveal(id: string | undefined): void {
    if (id === undefined) return;
    const index = indexOfSession(id);
    if (index < 0) return;
    list?.scrollToIndex(index, { align: "nearest" });
  }

  // The shown session stays in view.
  $effect(() => {
    const id = daemon.sessionId;
    if (id !== undefined && arranged.order.includes(id)) untrack(() => reveal(id));
  });

  function moveTo(id: string | undefined): void {
    if (id === undefined) return;
    selected = id;
    reveal(id);
  }

  function leave(): void {
    sessions.focused = false;
  }

  function open(id: string): void {
    if (id !== daemon.sessionId) void daemon.open(id);
  }

  /** Opens a session from the list; a search is over once one is picked. */
  function pick(id: string): void {
    open(id);
    if (searching) {
      sessions.query = "";
      search?.blur();
    }
  }

  function key(event: KeyEvent): void {
    if (event.ctrl || event.alt || event.meta) return;
    switch (event.key) {
      case "ArrowUp":
      case "k":
        moveTo(step(arranged.order, selected, -1));
        break;
      case "ArrowDown":
      case "j":
        moveTo(step(arranged.order, selected, 1));
        break;
      case "PageUp":
        moveTo(step(arranged.order, selected, -PAGE));
        break;
      case "PageDown":
        moveTo(step(arranged.order, selected, PAGE));
        break;
      case "[":
        moveTo(jumpGroup(arranged, selected, -1));
        break;
      case "]":
        moveTo(jumpGroup(arranged, selected, 1));
        break;
      case "Home":
      case "g":
        moveTo(arranged.order[0]);
        break;
      case "End":
      case "G":
        moveTo(arranged.order[arranged.order.length - 1]);
        break;
      case "Enter":
        if (selected !== undefined) open(selected);
        leave();
        break;
      case "Escape":
      case "q":
      case "h":
      case "ArrowLeft":
        leave();
        break;
      default:
        return;
    }
    event.preventDefault();
  }

  /** Keys in the search field: Up and Down walk the results, Enter opens one, Escape clears. */
  function searchKey(event: KeyEvent): void {
    if (event.ctrl || event.alt || event.meta) return;
    switch (event.key) {
      case "ArrowUp":
        moveTo(step(arranged.order, selected, -1));
        break;
      case "ArrowDown":
        moveTo(step(arranged.order, selected, 1));
        break;
      case "Enter":
        if (selected !== undefined && searching) pick(selected);
        break;
      case "Escape":
        // Clears first; with nothing to clear it gives the keyboard back.
        if (sessions.query !== "") sessions.query = "";
        else search?.blur();
        event.stopPropagation();
        break;
      default:
        return;
    }
    event.preventDefault();
  }

  /** Adds a project: a folder from the system's picker, with a new session in it. */
  async function addProject(): Promise<void> {
    try {
      const picked = await files.pickFiles({ directories: true, prompt: "Add project" });
      const folder = picked?.[0];
      if (folder === undefined) return;
      // The new session would be out of sight in a scoped list.
      sessions.project = undefined;
      await daemon.newSession(folder);
    } catch (error) {
      daemon.error = error instanceof Error ? error.message : String(error);
    }
  }

  /** The right-press menu of a session. */
  function menuOf(info: Extract<Line, { kind: "session" }>): MenuItem[] {
    const pinned = isPinned(sessions.marks, info.key);
    const settled = info.slim;
    const repo = placeOf(info.info.cwd, sessions.places).repo;
    const scoped = scope?.repo === repo;
    return [
      {
        label: pinned ? "Unpin" : "Pin",
        icon: pinIcon,
        onselect: () => sessions.togglePin(info.info, pinned),
      },
      {
        label: settled ? "Un-settle" : "Settle",
        icon: settled ? undoIcon : checkIcon,
        onselect: () => sessions.toggleSettle(info.info, settled),
      },
      { separator: true },
      {
        label: "Regenerate title",
        icon: sparkle,
        onselect: () => void daemon.sendTo(info.info.id, "/title"),
      },
      ...(target.openBeside === undefined
        ? []
        : [
            {
              label: "Open in a new column",
              icon: columnsIcon,
              onselect: () => target.openBeside?.(info.info.id),
            },
          ]),
      {
        label: "New session in this folder",
        icon: newSession,
        onselect: () => void daemon.newSession(info.info.cwd),
      },
      { label: "Copy path", icon: copyIcon, onselect: () => void clipboard.write(info.info.cwd) },
      { label: "Copy session id", icon: copyIcon, onselect: () => void clipboard.write(info.key) },
      { separator: true },
      {
        label: scoped ? "Show all projects" : "Filter by this project",
        icon: filterIcon,
        onselect: () => (sessions.project = scoped ? undefined : repo),
      },
    ];
  }

  function context(line: Extract<Line, { kind: "session" }>, event: PointerEvent): void {
    menu?.openAt(event.windowX, event.windowY, menuOf(line));
  }
</script>

{#snippet scopeFace()}
  {#if scope}<ProjectIcon name={scope.name} src={icons.byRepo[scope.repo]} />{/if}
{/snippet}

{#if sessions.open}
  <View
    style={{
      width: WIDTH,
      shrink: 0,
      background: p.side.bg,
    }}
  >
    <View
      style={{
        direction: "row",
        align: "center",
        gap: 2,
        paddingLeft: 20,
        paddingRight: 12,
        height: 52,
      }}
    >
      <Text
        style={{
          flex: 1,
          minWidth: 0,
          truncate: true,
          fontWeight: "medium",
          color: sessions.focused ? p.accent : p.fg,
        }}
      >
        Sessions{#if sessions.finished.size > 0}<Text style={{ color: p.success, fontWeight: "normal" }}>{"  "}{sessions.finished.size} done</Text>{/if}
      </Text>
      <ProjectPicker
        bind:open={filtering}
        {projects}
        current={scope?.repo}
        all
        onpick={(repo) => (sessions.project = repo)}
      >
        {#snippet trigger({ toggle })}
          <HeaderButton
            label={scope ? `Project: ${scope.name}` : "Filter by project"}
            svg={filterIcon}
            face={scope ? scopeFace : undefined}
            active={scope !== undefined}
            onpress={toggle}
          />
        {/snippet}
      </ProjectPicker>
      <HeaderButton label="Add project" svg={folderPlus} onpress={() => void addProject()} />
      <ProjectPicker
        bind:open={asking}
        {projects}
        current={undefined}
        all={false}
        onpick={(repo) => repo !== undefined && void daemon.newSession(repo)}
      >
        {#snippet trigger()}
          <HeaderButton
            label="New session (shift: choose the project)"
            svg={newSession}
            onpress={(event) => (event.shift ? (asking = true) : void daemon.newSession())}
          />
        {/snippet}
      </ProjectPicker>
    </View>

    <View style={{ direction: "row", paddingX: 12, paddingBottom: 8 }}>
      <SearchField bind:this={search} bind:value={sessions.query} onkey={searchKey} />
    </View>

    {#if searching && lines.length === 0}
      <Text style={{ padding: 20, color: p.muted, textAlign: "center" }}>No sessions found</Text>
    {/if}
    <View bind:this={keys} focusable onkey={key} onblur={leave} style={{ flex: 1, minHeight: 0 }}>
      <List
        bind:this={list}
        items={lines}
        key={(line: Line) => line.key}
        style={{ flex: 1, minHeight: 0, paddingX: 8 }}
      >
        {#snippet row(line: Line, index: number)}
          {#if line.kind === "label"}
            {@const repo = line.repo}
            <GroupHeader
              text={line.text}
              sub={line.sub}
              first={index === 0}
              {repo}
              icon={repo === undefined ? undefined : icons.byRepo[repo]}
              open={sessions.groupsOpen.has(line.key)}
              onfold={() => sessions.toggleGroup(line.key)}
              onnew={repo === undefined ? undefined : () => void daemon.newSession(repo)}
            />
          {:else if line.kind === "settled"}
            <SettledHeader
              count={line.count}
              open={line.open}
              onpress={() => (sessions.settledOpen = !sessions.settledOpen)}
            />
          {:else if line.kind === "more"}
            <Button
              onpress={() => {
                if (line.group === "settled") sessions.settledAll = true;
                else sessions.toggleGroup(line.group);
              }}
              style={{ direction: "row", gap: 6, height: 26, paddingX: 12, align: "center", radius: p.radius.s, color: p.muted }}
              hover={{ background: p.side.hover, color: p.fg }}
            >
              {#if line.group !== "settled"}<Icon svg={caretRight} size={10} />{/if}
              <Text style={{ fontSize: 12 }}
                >{line.group === "settled" ? `Show ${line.count} more` : `${line.count} older ${line.count === 1 ? "session" : "sessions"}`}</Text
              >
            </Button>
          {:else}
            {@const place = placeOf(line.info.cwd, sessions.places)}
            {#if line.slim}
              <SlimRow
                info={line.info}
                project={lastSegment(place.repo)}
                icon={icons.byRepo[place.repo]}
                current={line.key === daemon.sessionId}
                selected={sessions.focused && selected === line.key}
                nowMs={clock.now}
                onpress={() => pick(line.key)}
                onunsettle={() => sessions.toggleSettle(line.info, true)}
                oncontext={(event) => event.button === 2 && context(line, event)}
              />
            {:else}
              <SessionRow
                info={line.info}
                project={lastSegment(place.repo)}
                icon={icons.byRepo[place.repo]}
                showProject={line.mixed}
                branch={line.mixed
                  ? notableBranch(sessions.branches[line.info.cwd] ?? place.worktree, { repo: place.repo })
                  : notableBranch(sessions.branches[line.info.cwd], place)}
                child={line.child}
                current={line.key === daemon.sessionId}
                status={statusOf(line.info, sessions.finished.has(line.key), sessions.runningSince)}
                pin={isPinned(sessions.marks, line.key)}
                selected={(sessions.focused || searching) && selected === line.key}
                nowMs={clock.now}
                {terms}
                field={line.field}
                onpress={() => pick(line.key)}
                onsettle={() => sessions.toggleSettle(line.info, false)}
                oncontext={(event) => event.button === 2 && context(line, event)}
              />
            {/if}
          {/if}
        {/snippet}
      </List>
    </View>
    <ContextMenu bind:this={menu} menuStyle={{ width: 248 }} />
  </View>
{/if}
