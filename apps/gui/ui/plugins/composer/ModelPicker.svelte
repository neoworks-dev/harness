<script lang="ts">
  // The model picker, with the harness switcher in it: the harnesses as icons down the left, the
  // models of the one looked at on the right, with a search field over them. Picking a model of
  // another harness sets that harness's model and moves the session onto it.
  import { untrack } from "svelte";
  import { Button, Icon, Input, Popover, Scroll, Text, View } from "@neoworks/svelte-gpui/ui";
  import check from "@phosphor-icons/core/regular/check.svg";
  import search from "@phosphor-icons/core/regular/magnifying-glass.svg";
  import { useCtx } from "../../kernel/slots/context.ts";
  import { currentSettings } from "../../lib/status.ts";
  import HarnessIcon from "../../widgets/HarnessIcon.svelte";
  import Picker from "./Picker.svelte";
  import { optionOf, settings } from "./settings.svelte.ts";

  const { daemon } = useCtx();
  const p = $derived(daemon.palette);
  const status = $derived(daemon.status);

  let open = $state(false);
  let query = $state("");
  /** The harness whose models are listed. */
  let viewed = $state<string>();

  /** The session's harness: what the daemon lists, else what its events said last. */
  const running = $derived(daemon.session?.harness ?? status.harness);
  const label = $derived(currentSettings(status).model ?? running);
  const harness = $derived(
    settings.harnesses.find((h) => h.name === viewed) ?? settings.harnesses.find((h) => h.current),
  );
  const models = $derived(optionOf(harness, "model"));
  const shown = $derived(
    (models?.values ?? []).filter((value) =>
      `${value.name} ${value.value}`.toLowerCase().includes(query.trim().toLowerCase()),
    ),
  );

  // Opening asks what the harnesses offer and starts on the session's own.
  $effect(() => {
    if (!open) return;
    untrack(() => {
      query = "";
      viewed = running;
      void settings.load(daemon);
    });
  });

  async function pick(name: string, value?: string): Promise<void> {
    open = false;
    if (value !== undefined) await daemon.send(`/model ${name} ${value}`);
    if (name !== running) await daemon.send(`/harness ${name}`);
  }
</script>

<Popover
  bind:open
  side="top"
  align="start"
  offset={8}
  style={{
    direction: "row",
    width: 460,
    height: 340,
    background: p.surface,
    radius: 14,
    borderWidth: 1,
    borderColor: p.border,
    overflow: "hidden",
    shadow: { y: 8, blur: 24, color: "rgba(0, 0, 0, 0.5)" },
  }}
>
  {#snippet trigger({ toggle })}
    <Picker {label} onpress={toggle}>
      {#snippet icon()}<HarnessIcon harness={running} size={16} />{/snippet}
    </Picker>
  {/snippet}
  {#snippet children()}
    <View style={{ width: 56, align: "center", gap: 6, paddingY: 10, borderRightWidth: 1, borderColor: p.border }}>
      {#each settings.harnesses as entry (entry.name)}
        <Button
          onpress={() => (viewed = entry.name)}
          style={{
            size: 36,
            radius: p.radius.s,
            align: "center",
            justify: "center",
            color: p.muted,
            background: harness?.name === entry.name ? p.raised : "transparent",
          }}
          hover={{ background: p.hover, color: p.fg }}
        >
          <HarnessIcon harness={entry.name} size={20} />
        </Button>
      {/each}
    </View>
    <View style={{ flex: 1, minWidth: 0 }}>
      <View style={{ direction: "row", align: "center", gap: 8, paddingX: 14, height: 44, borderBottomWidth: 1, borderColor: p.border, color: p.muted }}>
        <Icon svg={search} size={16} />
        <Input bind:value={query} placeholder="Search models…" style={{ flex: 1, color: p.fg }} />
      </View>
      <Scroll style={{ flex: 1, minHeight: 0 }}>
        <View style={{ padding: 6, gap: 2 }}>
          {#if settings.loading && settings.harnesses.length === 0}
            <Text style={{ color: p.muted, padding: 10 }}>Asking the harnesses…</Text>
          {:else if settings.error}
            <Text style={{ color: p.danger, padding: 10 }}>{settings.error}</Text>
          {:else if harness?.error}
            <Text style={{ color: p.muted, padding: 10 }}>{harness.title}: {harness.error}</Text>
          {:else if harness && !models}
            <Button
              onpress={() => void pick(harness.name)}
              style={{ direction: "column", align: "stretch", gap: 2, paddingX: 10, paddingY: 8, radius: p.radius.s, color: p.fg }}
              hover={{ background: p.raised }}
            >
              <Text style={{ fontWeight: "medium" }}>Use {harness.title}</Text>
              <Text style={{ fontSize: 12, color: p.muted }}>It has no models to choose from.</Text>
            </Button>
          {:else if harness}
            {#each shown as value (value.value)}
              {@const current = harness.current && models?.current === value.value}
              <Button
                onpress={() => void pick(harness.name, value.value)}
                style={{
                  direction: "row",
                  align: "center",
                  gap: 10,
                  paddingX: 10,
                  paddingY: 8,
                  radius: p.radius.s,
                  color: p.fg,
                  background: current ? p.hover : "transparent",
                }}
                hover={{ background: p.raised }}
              >
                <View style={{ flex: 1, minWidth: 0, gap: 2 }}>
                  <Text style={{ fontWeight: "medium", truncate: true }}>{value.name}</Text>
                  <View style={{ direction: "row", align: "center", gap: 6, color: p.muted }}>
                    <HarnessIcon harness={harness.name} size={12} />
                    <Text style={{ fontSize: 12, truncate: true }}>{value.description ?? harness.title}</Text>
                  </View>
                </View>
                {#if current}<Icon svg={check} size={16} color={p.accent} />{/if}
              </Button>
            {:else}
              <Text style={{ color: p.muted, padding: 10 }}>No model matches.</Text>
            {/each}
          {/if}
        </View>
      </Scroll>
    </View>
  {/snippet}
</Popover>
