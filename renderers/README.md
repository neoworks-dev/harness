# renderers

Four renderers draw the same resolved view (`protocol/schemas/view.schema.json`): `web/` (TS, DOM), `gpui/` (Rust), `tui/` (Go) and `ratatui/` (Rust, styled lines for full-screen terminal apps; compact tool results, see its README). They never parse markdown, diff text or fill templates: the daemon did that. Their input is a list of `ViewItem`s plus a `Theme` (`protocol/themes/default.json` is the default the daemon serves), kept current by `view.snapshot` + `view.patch`.

## Conventions every renderer follows

So the chat behaves the same everywhere (consistent structure and behavior, not identical pixels):

**Items**

| kind      | Drawn as                                                                                                                     |
| --------- | ---------------------------------------------------------------------------------------------------------------------------- |
| `user`    | Header `» <label>` (label = the app it came from; bare `»` without one), then the component. Web/GPUI: right-leaning bubble. |
| `agent`   | The component, no header.                                                                                                    |
| `thought` | Collapsed to one muted line `… thinking` (expandable where the UI can; TUI: collapsed).                                      |
| `tool`    | Header `<state> <label>`, then the component indented / inside the card.                                                     |
| `notice`  | One muted line (danger tone for errors).                                                                                     |

Tool states: `pending` ○ (muted), `running` ◐ (accent; animated where possible), `done` ✓ (success), `failed` ✗ (danger).

A message between agents is a `done` `tool` item labelled `message: main → claude d_1a2b3c4d` (`reply: …` for a background subagent's final reply) whose component is the text, so it is as compact as any tool result; a per-agent transcript shows what was written to that agent as a `user` item labelled `from main`. The turn an idle agent starts for a message has no item of its own.

**Components** (TUI column per the design §8)

- `card`: box with optional title; `tone` colors the border/title. TUI: box-drawn frame (`╭─ title ─╮`), or indentation when narrower than 20 columns.
- `row`: side by side, wrapping to a stack when it doesn't fit. TUI: always stacked unless every child is a button (then on one line).
- `stack`: top to bottom; `gap` s/m/l = theme spacing (TUI: 0/0/1 blank lines); no `gap` means `s` (also for `row`).
- `text`: blocks in order. Headings bold (`title` role style for level ≤ 2); lists with `•` / `1.` and `[x]`/`[ ]` for tasks, nested items indented 2; `code_block` like a `code` component; quote with a `│ ` bar, muted; rule `───`; `table_block` (a markdown table: header cells, rows of cells with spans, optional `align` per column) as a grid with the header bold, cells wrapping inside their column (TUI: the `table` component's box, a rule between rows once one takes more than one line; web: a `<table>` with `data-align`). Spans: bold, italic, strike, inline code (code style / TUI: reverse or dim backtick-free), links underlined (TUI: `text (url)` unless text == url). `role` picks the theme text style; `tone` colors the text.
- `image`: web/GPUI draw `data:image/` and http(s) sources (`asset://` handles resolve through the host: web's `resolveImage` maps them to the daemon's `GET /assets/<id>` URL); anything else shows the fallback every renderer uses: `[image W×H · src]` (`[image · src]` when the size is unknown), the caption on the next line. Never fetch other schemes. The ratatui renderer can also reserve rows for a picture the app draws itself (its README).
- `file`: chip `📄 name` (name = `name` or the path's base name); web/GPUI tooltip/full path, TUI `📄 path`.
- `code`: monospace on the surface color, no wrapping in web/GPUI (scroll), TUI hard-wraps at the width.
- `diff`: TUI colored unified diff (`+` success, `-` danger, hunk headers muted, `path` as a header line); web/GPUI may show side by side.
- `table`: grid with a header row. TUI: box-drawn, columns cut to fit (cut cells end in `…`), widest columns shrink first.
- `kv`: aligned `key  value` lines, keys muted. An item's `tone` highlights its value (ratatui: the tone's color, bold; web: the tone's color; the others may ignore it).
- `progress`: bar + percent (`▓▓▓░░ 60%` in TUI, 20 cells) + label.
- `button`: web/GPUI button; TUI `[k] Label` (k = `key`, else its 1-based index among the item's buttons). Pressing sends `ui.action {session, ref, action, args}` — except `copy:<text>`, which copies `<text>` locally. A button is triggered by its key while its item (or anything inside it) has keyboard focus; focus moves through items with buttons in order (up/down). If two buttons share a key (an explicit `"2"` and the second keyless one), the first in order wins.
- `link`: underlined text, URL in TUI as `text (url)`.
- `openui` (a component, or a block of a `text` component): a program the model or an app wrote in neoui, evaluated by the daemon into a tree of `neo@1` components (`docs/neoui.md`). ratatui draws it, and so does the desktop window (`apps/gui`, which draws the view format itself in Svelte). The others show the one muted line `[unsupported component: openui]` in its place (TUI too, which otherwise skips what it doesn't know) until they follow.

**Patches**: `view.subscribe` answers with a snapshot before any patch for that subscription, and every patch carries the `seq` of the session event it came from (one patch per event, events without a visible change send none). Drop patches whose `seq` is below the snapshot's. `blocks` replaces the text component's blocks from `from` on (a `from` past the end appends); an op whose ref or path doesn't fit is skipped, and the next snapshot repairs it.

**Theme**: colors are meanings (`accent`, `muted`, `success`, `danger` plus `fg`/`bg`/`surface`/`border`); web maps them to CSS variables, GPUI to its colors, the TUI to the nearest color the terminal supports (true color, 256, 16, or none when `NO_COLOR` is set or output isn't a terminal).

## Snapshot tests

Every fixture in `fixtures/<name>/resolved.json` is drawn by every renderer and compared with a snapshot next to it: `tui.txt` (80 columns, no color), `tui-40.txt` (40 columns), `web.html`, `gpui.txt` (the GPUI renderer's layout model), `ratatui.txt` / `ratatui-40.txt` / `ratatui-expanded.txt` / `ratatui-styled.txt` (the ratatui renderer's lines as text). `UPDATE_SNAPSHOTS=1` rewrites them; review the diff like code. Pixel screenshots for web and GPUI are not set up yet (see the design's open questions).
