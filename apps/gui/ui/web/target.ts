/// <reference lib="dom" />
// Imported first by main.web.ts: the window is in a browser tab. The address may name the session
// to open (`?session=…`) and say the tab is a column beside another session's (`?column`), as the
// columns (web/columns) open them.
import { target } from "../kernel/target.ts";

target.web = true;
const params = new URLSearchParams(location.search);
target.session = params.get("session") ?? undefined;
target.column = params.has("column");
