<script lang="ts">
  // The files a turn changed, on a card under its last reply: how many and how much, a button for
  // the diff of all of them in the viewer, and a row per file (its type's icon, its path, its changes) that
  // opens that file's diff. The files are listed flat, the directory muted in front of the name.
  import { Button, Icon, Text, View } from "@neoworks/svelte-gpui/ui";
  import gitDiff from "@phosphor-icons/core/regular/git-diff.svg";
  import { useCtx } from "../../kernel/slots/context.ts";
  import FileIcon from "../../widgets/FileIcon.svelte";
  import { count } from "./format.ts";
  import { codeColor, softBorder } from "./layout.ts";
  import type { FileDiff } from "../../lib/unified.ts";
  import type { FileChange } from "./work.ts";

  let { files }: { files: readonly FileChange[] } = $props();

  const { daemon, viewer } = useCtx();
  const p = $derived(daemon.palette);

  const added = $derived(files.reduce((sum, file) => sum + file.added, 0));
  const removed = $derived(files.reduce((sum, file) => sum + file.removed, 0));

  /** A path as its directory (with the slash) and its name. */
  function parts(path: string): { dir: string; name: string } {
    const at = path.lastIndexOf("/");
    return { dir: path.slice(0, at + 1), name: path.slice(at + 1) };
  }

  function open(file?: FileChange): void {
    const shown = file === undefined ? files : [file];
    const diffs: FileDiff[] = [];
    for (const f of shown) if (f.diff !== undefined) diffs.push({ path: f.path, lines: f.diff.lines });
    if (diffs.length > 0 && diffs.length === shown.length) viewer.showPatch(count(shown.length, "changed file"), diffs);
  }
</script>

<View style={{ marginTop: 16, radius: p.radius.m, background: p.surface, borderWidth: 1, borderColor: softBorder(p), overflow: "hidden" }}>
  <View style={{ direction: "row", align: "center", gap: 8, paddingX: 12, paddingY: 6, minHeight: 36 }}>
    <Text style={{ fontSize: 12, fontWeight: "medium", color: p.fg }}>{count(files.length, "changed file")}</Text>
    <Text style={{ fontFamily: "mono", fontSize: 12 }}><Text style={{ color: p.success }}>+{added}</Text>{" "}<Text style={{ color: p.danger }}>−{removed}</Text></Text>
    <View style={{ flex: 1 }} />
    <Button
      onpress={() => open()}
      style={{ direction: "row", align: "center", gap: 6, paddingX: 8, paddingY: 4, radius: 6, fontSize: 12, color: p.muted }}
      hover={{ background: p.hover, color: p.fg }}
    >
      <Icon svg={gitDiff} size={14} />
      <Text>Open diff</Text>
    </Button>
  </View>
  <View style={{ padding: 8, paddingTop: 0 }}>
    {#each files as file (file.path)}
      {@const where = parts(file.path)}
      <View
        onpress={() => open(file)}
        hover={{ background: p.hover }}
        style={{ direction: "row", align: "center", gap: 8, paddingX: 8, paddingY: 6, radius: p.radius.s, cursor: "pointer" }}
      >
        <FileIcon path={file.path} size={16} />
        <Text style={{ flex: 1, minWidth: 0, truncate: true, fontFamily: "mono", fontSize: 12, color: codeColor(p) }}><Text style={{ color: p.muted }}>{where.dir}</Text>{where.name}</Text>
        <Text style={{ fontFamily: "mono", fontSize: 11 }}><Text style={{ color: p.success }}>+{file.added}</Text>{" "}<Text style={{ color: p.danger }}>−{file.removed}</Text></Text>
      </View>
    {/each}
  </View>
</View>
