<script lang="ts">
  // A file's icon by its type, in its own colours.
  import { Image } from "@neoworks/svelte-gpui/ui";
  import { fileIcon } from "../lib/file-icons.ts";

  let { path, size = 16 }: { path: string; size?: number } = $props();
</script>

<Image src={fileIcon(path)} fit="contain" style={{ size }} />
