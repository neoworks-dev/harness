<script lang="ts">
  // One OpenUI doc (docs/openui.md): the daemon's evaluated tree, drawn as cards, charts, fields and
  // buttons. It holds no state of its own beyond what is purely visual (state.svelte.ts); what the
  // user types or presses goes back to the daemon, which evaluates again and sends the item anew.
  import { clipboard, invoke } from "@neoworks/svelte-gpui";
  import { Text, View } from "@neoworks/svelte-gpui/ui";
  import type { KeyEvent } from "@neoworks/svelte-gpui/ui";
  import type { UiDoc } from "@suite/protocol/types";
  import { useCtx } from "../../../kernel/slots/context.ts";
  import { useColumn } from "../../../widgets/column.ts";
  import { provideDoc } from "./context.ts";
  import Node from "./Node.svelte";
  import { DocSession, viewOf } from "./state.svelte.ts";

  interface Props {
    ui: UiDoc;
    /** The chat item the doc is in: what the daemon addresses its actions to. */
    ref: string;
    /** Pixels the doc's surroundings take from the column, such as an indent. */
    inset?: number;
  }

  let { ui, ref, inset = 0 }: Props = $props();
  const { daemon } = useCtx();
  const p = $derived(daemon.palette);
  const column = useColumn();
  const width = $derived(Math.max(160, column.width - inset - 8));
  const view = $derived(viewOf(daemon.sessionId, ref, ui.id));

  async function open(url: string): Promise<void> {
    try {
      await invoke("open_url", { url });
    } catch {
      // A host without the command, or no browser: leave the address to paste.
      await clipboard.write(url);
      daemon.error = `Could not open ${url}; it is on the clipboard.`;
    }
  }

  const session = new DocSession(() => ({
    ref,
    ui,
    view,
    send: (item, action, args) => void daemon.action(item, action, args),
    open: (url) => void open(url),
  }));
  provideDoc(session);

  function key(event: KeyEvent): void {
    if (event.key === "Tab" && !event.ctrl && !event.alt && !event.meta) {
      if (view.move(ui.root, event.shift ? -1 : 1)) {
        event.preventDefault();
        event.stopPropagation();
      }
    }
  }
</script>

{#if ui.root !== undefined}
  <View style={{ marginY: 4 }} onkey={key}>
    <Node node={ui.root} path="" {width} />
  </View>
{:else if (ui.errors?.length ?? 0) > 0}
  <Text style={{ color: p.muted }}>[ui: {ui.errors?.[0]?.message}]</Text>
{/if}
