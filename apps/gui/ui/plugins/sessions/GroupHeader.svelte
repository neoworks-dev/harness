<script lang="ts">
  // A heading of the sidebar. A project's is its icon and name, a little apart from the group above,
  // with a button for a new session in it while the pointer is on it; a worktree's is its branch
  // mark and name under it, smaller; "Pinned" has the pin. A group whose older sessions are shown
  // folds them again from its heading.
  import { Icon, Text, View } from "@neoworks/svelte-gpui/ui";
  import branchIcon from "@phosphor-icons/core/regular/git-branch.svg";
  import caretDown from "@phosphor-icons/core/bold/caret-down-bold.svg";
  import plusIcon from "@phosphor-icons/core/regular/plus.svg";
  import pinIcon from "@phosphor-icons/core/fill/push-pin-fill.svg";
  import { useCtx } from "../../kernel/slots/context.ts";
  import ProjectIcon from "./ProjectIcon.svelte";
  import RowAction from "./RowAction.svelte";

  interface Props {
    text: string;
    /** A worktree's heading, under its project's. */
    sub?: boolean;
    /** A project's heading: its repository. */
    repo?: string;
    icon?: string | null;
    /** Its older sessions are shown: pressing the heading folds them. */
    open?: boolean;
    onfold?: () => void;
    /** A new session in the project. */
    onnew?: () => void;
  }

  let { text, sub = false, repo, icon, open = false, onfold, onnew }: Props = $props();
  const { daemon } = useCtx();
  const p = $derived(daemon.palette);
  let hovered = $state(false);
</script>

<View
  onpress={open ? onfold : undefined}
  onhover={(over) => (hovered = over)}
  style={{
    direction: "row",
    align: "center",
    gap: 8,
    height: sub ? 26 : 30,
    marginTop: sub ? 2 : 12,
    paddingLeft: sub ? 14 : 8,
    paddingRight: 6,
    color: sub ? p.muted : p.fg,
    cursor: open ? "pointer" : "default",
  }}
>
  {#if repo !== undefined}
    <ProjectIcon name={text} src={icon} />
  {:else if sub}
    <Icon svg={branchIcon} size={12} />
  {:else}
    <View style={{ color: p.muted }}><Icon svg={pinIcon} size={14} /></View>
  {/if}
  <Text
    style={{
      flex: 1,
      minWidth: 0,
      truncate: true,
      fontSize: sub ? 12 : 13,
      fontWeight: sub ? "normal" : "semibold",
      color: sub ? p.muted : "rgba(241, 243, 247, 0.92)",
    }}>{text}</Text
  >
  {#if open}
    <View style={{ color: p.muted }}><Icon svg={caretDown} size={10} /></View>
  {/if}
  {#if onnew && hovered}
    <RowAction svg={plusIcon} label="New session in {text}" onpress={onnew} />
  {/if}
</View>
