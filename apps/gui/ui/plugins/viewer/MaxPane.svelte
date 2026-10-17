<script lang="ts">
  // The panel in the conversation's place, inside its card: the input stays under it, so comments
  // can be sent from here.
  import { View } from "@neoworks/svelte-gpui/ui";
  import Panel from "./Panel.svelte";
</script>

<View style={{ flex: 1, minHeight: 0 }}>
  <Panel />
</View>
