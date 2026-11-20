<script lang="ts">
  // An edit the agent asks to make, whole and in the chat's place until it is answered. The keys
  // are vim's: j/k (a count before them), gg/G, Ctrl-D/U/F/B, n/N or ]c/[c between changes, V for
  // a visual run; c comments on the lines under the cursor. Enter accepts, x declines, both with
  // the comments as the answer's note; Esc goes back to the chat, where the request waits. The
  // mouse does the same: a press puts the cursor, shift stretches, a double press comments.
  import { untrack } from "svelte";
  import { Button, Input, Text, View } from "@neoworks/svelte-gpui/ui";
  import type { KeyEvent } from "@neoworks/svelte-gpui/ui";
  import { useCtx } from "../../kernel/slots/context.ts";
  import { diffStats } from "../../lib/diff.ts";
  import FileIcon from "../../widgets/FileIcon.svelte";
  import { forDiff } from "./highlights.ts";
  import { partners } from "./inline.ts";
  import { answers } from "./permit.ts";
  import { permit } from "./permit.svelte.ts";
  import PermitLine from "./PermitLine.svelte";
  import { HEADER_HEIGHT, LINE_HEIGHT } from "./rows.ts";
  import VirtualList from "./VirtualList.svelte";

  /** Lines Ctrl-D / Ctrl-U move; Ctrl-F / Ctrl-B twice that. */
  const HALF = 20;
  const COMMENT_HEIGHT = 40;
  const EDITOR_HEIGHT = 112;

  type Row =
    | { kind: "file"; key: string; path: string; added: number; removed: number }
    | { kind: "line"; key: string; at: number; file: number; index: number }
    | { kind: "comment"; key: string; at: number }
    | { kind: "editor"; key: string };

  const { daemon } = useCtx();
  const p = $derived(daemon.palette);
  const edit = $derived(permit.current);
  const options = $derived(edit === undefined ? {} : answers(edit.options));

  /** An Enter this soon after a request shows was meant for the input it took the keys from. */
  const SETTLE_MS = 500;
  let shownAt = 0;

  // A new request: the cursor on its first change.
  $effect(() => {
    void edit?.id;
    shownAt = Date.now();
    untrack(() => permit.sync());
  });

  const rows = $derived.by((): Row[] => {
    if (edit === undefined) return [];
    const out: Row[] = [];
    let at = 0;
    edit.files.forEach((file, f) => {
      const { added, removed } = diffStats(file.lines);
      out.push({ kind: "file", key: `f${f}`, path: file.path, added, removed });
      file.lines.forEach((_, index) => {
        out.push({ kind: "line", key: `l${f}:${index}`, at, file: f, index });
        at += 1;
        permit.comments.forEach((comment, c) => {
          if (comment.file === f && comment.to === index) out.push({ kind: "comment", key: `c${c}`, at: c });
        });
        const draft = permit.draft;
        if (draft !== undefined && draft.file === f && draft.to === index) out.push({ kind: "editor", key: "editor" });
      });
    });
    return out;
  });
  const lights = $derived(edit?.files.map((file) => forDiff(file.path, file.lines)) ?? []);
  const pairs = $derived(edit?.files.map((file) => partners(file.lines)) ?? []);
  const digits = $derived(
    Math.max(1, ...(edit?.files ?? []).map((file) => String(file.lines.length).length)),
  );

  function commented(file: number, index: number): boolean {
    return permit.comments.some((comment) => comment.file === file && index >= comment.from && index <= comment.to);
  }

  let list: ReturnType<typeof VirtualList<Row>> | undefined = $state();
  let keys: ReturnType<typeof View> | undefined = $state();
  let field: ReturnType<typeof Input> | undefined = $state();

  // The cursor stays in view.
  $effect(() => {
    const { at } = permit.jump;
    untrack(() => {
      const row = rows.findIndex((r) => r.kind === "line" && r.at === at);
      if (row >= 0) setTimeout(() => list?.scrollToIndex(row, { align: "nearest" }), 0);
    });
  });

  // The keys are the pane's, or the comment's while one is written (a tick later, so the key that
  // opened it is not typed into it).
  $effect(() => {
    const writing = permit.draft !== undefined;
    void edit?.id;
    const timer = setTimeout(() => (writing ? field?.focus() : keys?.focus()), 0);
    return () => clearTimeout(timer);
  });

  /** A count typed before a motion (5j), and the first key of gg, ]c, [c. */
  let count = "";
  let prefix = "";

  function key(event: KeyEvent): void {
    if (event.alt || event.meta) return;
    const n = Math.max(1, Number.parseInt(count || "1", 10));
    const take = (): void => {
      event.preventDefault();
      event.stopPropagation();
    };
    if (event.ctrl) {
      const by = { d: HALF, u: -HALF, f: 2 * HALF, b: -2 * HALF }[event.key];
      if (by === undefined) return;
      permit.move(by * n);
      count = "";
      take();
      return;
    }
    const first = prefix;
    prefix = "";
    if (first === "g" && event.key === "g") {
      permit.moveTo(count === "" ? 0 : n - 1, false);
    } else if ((first === "]" || first === "[") && event.key === "c") {
      for (let i = 0; i < n; i += 1) permit.nextChange(first === "]" ? 1 : -1);
    } else if (/^[0-9]$/.test(event.key) && !(event.key === "0" && count === "")) {
      count += event.key;
      take();
      return;
    } else {
      switch (event.key) {
        case "j":
        case "ArrowDown":
          permit.move(n, event.shift);
          break;
        case "J":
          permit.move(n, true);
          break;
        case "k":
        case "ArrowUp":
          permit.move(-n, event.shift);
          break;
        case "K":
          permit.move(-n, true);
          break;
        case "PageDown":
          permit.move(2 * HALF, event.shift);
          break;
        case "PageUp":
          permit.move(-2 * HALF, event.shift);
          break;
        case "g":
        case "]":
        case "[":
          prefix = event.key;
          take();
          return;
        case "G":
        case "End":
          permit.moveTo(count === "" ? permit.spots.length - 1 : n - 1, false);
          break;
        case "Home":
          permit.moveTo(0, false);
          break;
        case "n":
          for (let i = 0; i < n; i += 1) permit.nextChange(1);
          break;
        case "N":
          for (let i = 0; i < n; i += 1) permit.nextChange(-1);
          break;
        case "V":
        case "v":
          permit.toggleVisual();
          break;
        case "c":
        case "i":
        case "a":
          permit.startComment();
          break;
        case "Enter":
          if (Date.now() - shownAt >= SETTLE_MS) void permit.answer(true);
          break;
        case "x":
          void permit.answer(false);
          break;
        case "Escape":
          if (permit.visual) permit.visual = false;
          else permit.putAside();
          break;
        default:
          count = "";
          return;
      }
    }
    count = "";
    take();
  }

  function editorKey(event: KeyEvent): void {
    if (event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      permit.cancelComment();
    } else if (event.key === "Enter" && !event.shift && !event.alt) {
      event.preventDefault();
      event.stopPropagation();
      permit.saveComment();
    }
  }

  const button = { paddingX: 12, height: 28, radius: 6, align: "center", justify: "center" } as const;
</script>

{#if edit !== undefined}
  <View style={{ flex: 1, minHeight: 0 }}>
    <View style={{ direction: "row", align: "center", gap: 8, paddingX: 12, height: 48, shrink: 0, borderBottomWidth: 1, borderColor: p.border }}>
      <View style={{ flex: 1, minWidth: 0, gap: 2 }}>
        <Text style={{ truncate: true, fontWeight: "medium" }}>{edit.title}</Text>
        <Text style={{ truncate: true, fontSize: 12, color: p.muted }}>
          {permit.waiting > 1 ? `${permit.waiting} edits wait · ` : ""}{permit.comments.length > 0 ? `${permit.comments.length} comment${permit.comments.length === 1 ? "" : "s"} · ` : ""}j k move · V run · n next change · c comment · Esc back to the chat
        </Text>
      </View>
      <Button
        onpress={() => permit.startComment()}
        style={{ ...button, color: p.muted }}
        hover={{ background: p.hover, color: p.fg }}
      >
        <Text style={{ fontSize: 13 }}>Comment <Text style={{ color: p.muted }}>c</Text></Text>
      </Button>
      {#if options.decline}
        <Button
          onpress={() => void permit.answer(false)}
          disabled={permit.busy}
          style={{ ...button, borderWidth: 1, borderColor: p.border, color: p.fg }}
          hover={{ background: p.hover }}
        >
          <Text style={{ fontSize: 13 }}>{options.decline.name} <Text style={{ color: p.muted }}>x</Text></Text>
        </Button>
      {/if}
      {#if options.accept}
        <Button
          onpress={() => void permit.answer(true)}
          disabled={permit.busy}
          style={{ ...button, background: p.accent, color: "#ffffff" }}
          hover={{ opacity: 0.9 }}
        >
          <Text style={{ fontSize: 13 }}>{options.accept.name} <Text style={{ opacity: 0.7 }}>⏎</Text></Text>
        </Button>
      {/if}
    </View>

    <View bind:this={keys} focusable onkey={key} style={{ flex: 1, minHeight: 0 }}>
      <VirtualList
        bind:this={list}
        items={rows}
        key={(row) => row.key}
        height={(row) =>
          row.kind === "file" ? HEADER_HEIGHT : row.kind === "comment" ? COMMENT_HEIGHT : row.kind === "editor" ? EDITOR_HEIGHT : LINE_HEIGHT}
        style={{ flex: 1, minHeight: 0 }}
      >
        {#snippet row(row: Row)}
          {#if row.kind === "file"}
            <View style={{ direction: "row", align: "center", gap: 8, height: HEADER_HEIGHT, paddingX: 12, background: p.surface, borderBottomWidth: 1, borderColor: p.border }}>
              <FileIcon path={row.path} size={15} />
              <Text style={{ flex: 1, minWidth: 0, truncate: true, fontSize: 12 }}>{row.path}</Text>
              <Text style={{ fontSize: 12, fontFamily: "mono" }}><Text style={{ color: p.success }}>+{row.added}</Text>{" "}<Text style={{ color: p.danger }}>−{row.removed}</Text></Text>
            </View>
          {:else if row.kind === "line"}
            {@const file = edit.files[row.file]}
            {#if file}
              <PermitLine
                at={row.at}
                line={file.lines[row.index]!}
                tokens={lights[row.file]?.tokens(row.index)}
                partner={pairs[row.file]?.get(row.index)}
                {digits}
                commented={commented(row.file, row.index)}
              />
            {/if}
          {:else if row.kind === "comment"}
            {@const comment = permit.comments[row.at]}
            {#if comment}
              <View style={{ height: COMMENT_HEIGHT, paddingX: 12, paddingY: 4 }}>
                <View style={{ flex: 1, direction: "row", align: "center", gap: 8, paddingX: 10, radius: 6, background: p.surface, borderWidth: 1, borderColor: p.accent }}>
                  <Text style={{ flex: 1, minWidth: 0, truncate: true, fontSize: 13 }}>{comment.text}</Text>
                  <Button onpress={() => permit.removeComment(row.at)} style={{ paddingX: 6, height: 20, radius: 4, color: p.muted }} hover={{ background: p.hover, color: p.fg }}>
                    <Text style={{ fontSize: 11 }}>Remove</Text>
                  </Button>
                </View>
              </View>
            {/if}
          {:else}
            <View style={{ height: EDITOR_HEIGHT, paddingX: 12, paddingY: 6 }}>
              <View style={{ flex: 1, minHeight: 0, gap: 6, padding: 8, radius: 8, background: p.surface, borderWidth: 1, borderColor: p.border }}>
                <Input
                  bind:this={field}
                  bind:value={permit.text}
                  multiline
                  rows={2}
                  placeholder="What should change here? Enter keeps it, Esc drops it"
                  style={{ fontSize: 13, color: p.fg, flex: 1, minHeight: 0 }}
                  onkey={editorKey}
                />
                <View style={{ direction: "row", justify: "end", gap: 6 }}>
                  <Button onpress={() => permit.cancelComment()} style={{ paddingX: 10, height: 24, radius: 6, align: "center", color: p.muted }} hover={{ background: p.hover, color: p.fg }}>
                    <Text style={{ fontSize: 12 }}>Cancel</Text>
                  </Button>
                  <Button onpress={() => permit.saveComment()} style={{ paddingX: 10, height: 24, radius: 6, align: "center", background: p.accent, color: "#ffffff" }} hover={{ opacity: 0.9 }}>
                    <Text style={{ fontSize: 12 }}>Comment</Text>
                  </Button>
                </View>
              </View>
            </View>
          {/if}
        {/snippet}
      </VirtualList>
    </View>
  </View>
{/if}
