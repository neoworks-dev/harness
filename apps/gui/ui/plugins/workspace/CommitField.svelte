<script lang="ts">
  // The commit message, in the card under the commit button. Enter commits, Esc puts it away.
  import { Input, Text, View, type KeyEvent } from "@neoworks/svelte-gpui/ui";
  import { useCtx } from "../../kernel/slots/context.ts";
  import { workspace } from "./state.svelte.ts";

  const { daemon } = useCtx();
  const p = $derived(daemon.palette);
  let field: Input | undefined = $state();

  $effect(() => {
    field?.focus();
  });

  function key(event: KeyEvent): void {
    if (event.key !== "Escape") return;
    event.preventDefault();
    event.stopPropagation();
    workspace.cancelCommit();
  }
</script>

<View style={{ gap: 4, paddingTop: 4 }}>
  <Input
    bind:this={field}
    bind:value={workspace.message}
    placeholder="Commit message"
    onsubmit={() => void workspace.submitCommit()}
    onkey={key}
    style={{
      height: 32,
      paddingX: 10,
      radius: 8,
      fontSize: 13,
      background: p.bg,
      borderWidth: 1,
      borderColor: p.border,
    }}
    focus={{ borderColor: p.muted }}
  />
  <Text style={{ paddingX: 4, fontSize: 11, color: p.muted }}>
    Enter {workspace.committing?.push ? "commits and pushes" : "commits"}, Esc cancels
  </Text>
</View>
