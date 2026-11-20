<script lang="ts">
  // The workspace card, floating at the conversation's top right: where the session works (the
  // machine and the folder), the editor to open it in, the branch, a commit button and what has
  // changed. Modelled on t3code's thread details panel. Every button asks the host to do the work
  // (state.svelte.ts); a failure shows as a line at the bottom.
  import { window } from "@neoworks/svelte-gpui";
  import { Icon, Text, View, type MenuItem } from "@neoworks/svelte-gpui/ui";
  import caretDown from "@phosphor-icons/core/regular/caret-down.svg";
  import cloudArrowUp from "@phosphor-icons/core/regular/cloud-arrow-up.svg";
  import code from "@phosphor-icons/core/regular/code.svg";
  import copy from "@phosphor-icons/core/regular/copy.svg";
  import folderOpen from "@phosphor-icons/core/regular/folder-open.svg";
  import folderSimple from "@phosphor-icons/core/regular/folder-simple.svg";
  import gitBranch from "@phosphor-icons/core/regular/git-branch.svg";
  import gitCommit from "@phosphor-icons/core/regular/git-commit.svg";
  import gitDiff from "@phosphor-icons/core/regular/git-diff.svg";
  import monitor from "@phosphor-icons/core/regular/monitor.svg";
  import sparkle from "@phosphor-icons/core/regular/sparkle.svg";
  import { useCtx } from "../../kernel/slots/context.ts";
  import CommitField from "./CommitField.svelte";
  import Counts from "./Counts.svelte";
  import Dropdown from "./Dropdown.svelte";
  import { WIDTH, textOf } from "./look.ts";
  import Row from "./Row.svelte";
  import Split from "./Split.svelte";
  import { github } from "../github/state.svelte.ts";
  import { workspace } from "./state.svelte.ts";
  import { folderName, primaryAction } from "./workspace.ts";

  /** Under this the card would cover most of the conversation. */
  const MIN_WINDOW_WIDTH = 760;
  /** A row's menu opens under it, as wide as the card allows. */
  const STRETCH = { alignSelf: "stretch" } as const;

  const { daemon, viewer } = useCtx();

  const p = $derived(daemon.palette);
  const cwd = $derived(daemon.cwd);
  const overview = $derived(workspace.overview);
  const editor = $derived(workspace.editor);
  const dirty = $derived(overview?.dirty ?? false);
  const primary = $derived(overview ? primaryAction(overview) : undefined);
  const counts = $derived(workspace.counts);
  // A file opened in the chat's place, or the GitHub view, has the whole card: this one would sit
  // on its code or its buttons.
  const shown = $derived(
    !workspace.hidden && !viewer.inPlace && !github.shown && window.size.width >= MIN_WINDOW_WIDTH,
  );

  const placeItems = $derived<MenuItem[]>([
    { label: "Copy path", icon: copy, onselect: () => void workspace.copy(cwd, "the path") },
    { label: "Show in file manager", icon: folderOpen, onselect: () => void workspace.openFolder() },
  ]);
  const editorItems = $derived<MenuItem[]>(
    workspace.editors.map((e) => ({
      label: e.name,
      checked: e.id === editor?.id,
      onselect: () => void workspace.openEditor(e.id),
    })),
  );
  const branchItems = $derived.by<MenuItem[]>(() => {
    const items: MenuItem[] = workspace.branches.map((name) => ({
      label: name,
      checked: name === overview?.branch,
      onselect: () => name !== overview?.branch && void workspace.switchTo(name),
    }));
    if (items.length > 0) items.push({ separator: true });
    items.push({
      label: "Copy branch name",
      icon: copy,
      disabled: !overview?.branch,
      onselect: () => void workspace.copy(overview?.branch ?? "", "the branch name"),
    });
    return items;
  });
  const commitItems = $derived<MenuItem[]>([
    { label: "Commit", icon: gitCommit, disabled: !dirty, onselect: () => workspace.beginCommit(false) },
    { label: "Commit & push", icon: cloudArrowUp, disabled: !dirty, onselect: () => workspace.beginCommit(true) },
    { label: "Ask the agent to commit", icon: sparkle, disabled: !dirty, onselect: () => void workspace.askAgent() },
  ]);
</script>

{#if shown}
  <View
    style={{
      width: WIDTH,
      background: p.surface,
      radius: 24,
      borderWidth: 1,
      borderColor: p.border,
      overflow: "hidden",
      color: textOf(p),
      shadow: { y: 8, blur: 32, color: "rgba(0, 0, 0, 0.45)" },
    }}
  >
    <View style={{ padding: 8, paddingBottom: 10, gap: 2 }}>
      <Dropdown items={placeItems} anchorStyle={STRETCH}>
        {#snippet trigger({ toggle })}
          <Row onpress={toggle}>
            <Icon svg={monitor} size={16} color={p.muted} />
            <Text style={{ maxWidth: 104, truncate: true }}>{workspace.hostname || "This machine"}</Text>
            <Icon svg={folderSimple} size={16} color={p.muted} />
            <Text style={{ flex: 1, minWidth: 0, truncate: true }}>{folderName(cwd)}</Text>
            <Icon svg={caretDown} size={14} color={p.muted} />
          </Row>
        {/snippet}
      </Dropdown>
      <Split
        icon={code}
        label={editor ? `Open in ${editor.name}` : "No editor found on PATH"}
        disabled={editor === undefined}
        items={editorItems}
        onpress={() => void workspace.openEditor()}
      />
    </View>

    {#if overview}
      <View style={{ height: 1, background: p.border }} />
      <View style={{ padding: 8, paddingBottom: 10, gap: 2 }}>
        <Dropdown
          items={branchItems}
          anchorStyle={STRETCH}
          maxHeight={280}
          onopen={() => void workspace.loadBranches()}
        >
          {#snippet trigger({ toggle })}
            <Row onpress={toggle}>
              <Icon svg={gitBranch} size={16} color={p.muted} />
              <Text style={{ flex: 1, minWidth: 0, truncate: true }}>{overview.branch ?? "Detached HEAD"}</Text>
              <Icon svg={caretDown} size={14} color={p.muted} />
            </Row>
          {/snippet}
        </Dropdown>
        <Split
          icon={cloudArrowUp}
          label={primary?.label ?? "Commit & push"}
          disabled={primary?.kind === "none" || workspace.busy !== undefined}
          items={commitItems}
          onpress={() => workspace.primary()}
        />
        {#if workspace.committing}
          <CommitField />
        {/if}
      </View>

      <View style={{ height: 1, background: p.border }} />
      <View style={{ padding: 8, paddingBottom: 10 }}>
        <Row onpress={() => workspace.showChanges()} disabled={counts.files === 0}>
          <Icon svg={gitDiff} size={16} color={p.muted} />
          <Text style={{ flex: 1, minWidth: 0 }}>Changes</Text>
          {#if counts.files === 0}
            <Text style={{ fontSize: 12, color: p.muted }}>None</Text>
          {:else}
            <Counts added={counts.added} deleted={counts.deleted} />
          {/if}
        </Row>
      </View>
    {/if}

    {#if workspace.busy !== undefined || workspace.notice}
      <View style={{ height: 1, background: p.border }} />
      <Text
        style={{
          paddingX: 18,
          paddingY: 10,
          fontSize: 12,
          maxLines: 3,
          color: workspace.busy === undefined && workspace.notice?.error ? p.danger : p.muted,
        }}>{workspace.busy ?? workspace.notice?.text}</Text
      >
    {/if}
  </View>
{/if}
