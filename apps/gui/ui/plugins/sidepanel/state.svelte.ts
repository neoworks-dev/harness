import { Claims } from "../../kernel/claims.svelte.ts";

/** Which sections have something to show; the column is there while any does. */
export const sections = new Claims();

/** Which panes cover the column with something of their own (the viewer takes its place). */
export const covers = new Claims();

/** Whether the user hid the column with its button in the toolbar. */
export const panel = $state({ hidden: false });
