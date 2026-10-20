<script lang="ts">
  // The daemon's question for a secret (an API key, a login code): a masked field in a band above
  // the input. Enter sends, Esc cancels. The text is shown as dots and kept only in `secretField`.
  import { Input, Text, View } from "@neoworks/svelte-gpui/ui";
  import type { KeyEvent } from "@neoworks/svelte-gpui/ui";
  import { useCtx } from "../../kernel/slots/context.ts";
  import { secretField } from "./state.svelte.ts";

  const { daemon } = useCtx();
  const p = $derived(daemon.palette);
  const ask = $derived(secretField.ask);

  let field: ReturnType<typeof Input> | undefined = $state();

  // The field has the keyboard as soon as the question opens (and for a replacing question).
  $effect(() => {
    if (ask !== undefined) field?.focus();
  });

  function key(event: KeyEvent): void {
    if (event.ctrl || event.alt || event.meta) return;
    if (event.key === "Enter") secretField.submit();
    else if (event.key === "Escape") secretField.cancel();
    else return;
    event.preventDefault();
    event.stopPropagation();
  }
</script>

{#if ask}
  <View
    style={{
      background: p.band,
      paddingX: p.space.l,
      paddingY: p.space.m,
      gap: p.space.s,
      borderColor: p.accent,
      borderWidth: 1,
    }}
  >
    <Text style={{ fontWeight: 600, truncate: true }}>{ask.title}</Text>
    {#key ask.id}
      <View style={{ direction: "row", gap: p.space.m, align: "center" }}>
        <Text style={{ color: p.accent, fontWeight: 600 }}>›</Text>
        <Input
          bind:this={field}
          bind:value={secretField.value}
          secure
          placeholder={ask.placeholder ?? ""}
          style={{ flex: 1, minWidth: 0, color: p.fg }}
          onkey={key}
        />
      </View>
    {/key}
    <Text style={{ color: p.muted, fontSize: p.text.body - 1 }}>
      Enter sends · Esc cancels · hidden, never shown or logged
    </Text>
  </View>
{/if}
