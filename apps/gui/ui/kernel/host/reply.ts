import type { CommandMap } from "@neoworks/svelte-gpui";

/** What the host command `N` answers with. */
export type Reply<N extends keyof CommandMap> = Promise<CommandMap[N]["result"]>;
