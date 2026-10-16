import { Claims } from "../../kernel/claims.svelte.ts";

/** Which sections have something to show; the column is there while any does. */
export const sections = new Claims();

/** Whether the user hid the column with its button in the toolbar. */
export const panel = $state({ hidden: false });
