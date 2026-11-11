// A browser tab's entry (the native window's is main.ts): the same window, which knows it is in a
// tab (for its defaults), with the one thing a File object needs, a way to reach the daemon's
// disk. Built by `bun apps/gui/build.mjs --web`.
import "@neoworks/svelte-gpui";
import "./web/target.ts";
import "./web/attachments.ts";
import "./main.ts";
