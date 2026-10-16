# renderers/ratatui

Rust renderer (`suite-renderer-ratatui`) that draws the resolved view format as styled [ratatui](https://ratatui.rs) lines, for full-screen terminal apps. It has no terminal, input or scrolling code: the app owns the screen and scrolls over the lines. Conventions shared by all renderers are in `../README.md`.

```rust
use suite_renderer_ratatui::{buttons, render_item, Options, Palette};

let opts = Options { theme: Palette::from_theme(&theme), ..Options::default() };
let lines: Vec<Line<'static>> = render_item(&item, width, &opts);   // wrapped to `width` cells
let actions = buttons(&item);   // label, action, args, key; in drawing order, for arrow-key selection
// press: send ui.action {session, ref: item.ref, action, args}; `copy:` actions: actions[i].copy_text()
```

- `render_item(&ViewItem, width, &Options) -> Vec<Line<'static>>` is pure and does not touch the terminal, so the app can cache the lines per item until the item, the width or the options change. No line is wider than `width` cells (CJK and emoji count by display width, long words are cut, tables and kv are truncated with `…`). Items are not separated: the app puts a blank line between them (or not).
- `Options`: `expanded` (tool items: full result instead of the compact form), `show_thoughts`, `focused_button: Option<usize>` (index into `buttons(item)`; drawn reversed), `theme: Palette`.
- `Palette` is the daemon's `Theme` colors as `Color::Rgb` (`Palette::from_theme`, `Default` = `protocol/themes/default.json`). Fields are public: map them to 16/256 colors or `Color::Reset` for `NO_COLOR`. Plain text has no foreground color of its own, so the terminal's is used.
- Untrusted text is cleaned before it is drawn: control characters (escape sequences in tool output) and bidi overrides are dropped, tabs expand to 4-column stops.

## How items look

| kind      | Drawn as                                                                                                                                                             |
| --------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `user`    | `> text` (prompt in accent, text hangs under it); the app it came from dimmed after the text (`> make it smaller  · browser`)                                        |
| `agent`   | the text; a guest's label as a small accent header line                                                                                                              |
| `thought` | `∴ thinking` (muted); with `show_thoughts` the text in muted italic after `∴ `                                                                                       |
| `tool`    | `● label` (bullet: pending muted, running accent, done success, failed danger), then the result: compact under `⎿ `, or the whole component indented with `expanded` |
| `notice`  | the text in muted (danger when failed)                                                                                                                               |

This deliberately differs from the shared item conventions (`» label` header, `✓`/`✗` state glyphs, `… thinking`) in favour of a quieter, denser look; the structure and the buttons behave the same.

**Compact tool results** are flattened into leaves (card title, text, code, diff, ...) and share about three lines, every leaf getting one before any gets a second, so `Exit code 1` after a screenful of output stays visible. A cut leaf ends in `… +N lines`. Code: its first lines, cut to the width. Diff: `path  +3 −1`. Kv: `k: v · k: v`. Table: `3 rows · name, size`. Image: `[image alt]`. Card: its title, then the children. Text: wrapped lines. Everything short (file, link, progress) as it is. **Items that contain buttons are always drawn in full**: the buttons are what they are for (permission requests, choices).

**Components**: `card` is a `╭ title` line and a `│ ` gutter (tone colors both); `row` puts children side by side only when each is one line and they all fit, else stacks them; `stack` gap `l` is one blank line, `s`/`m` none; `text` blocks as in the Go TUI (headings bold, `•`/`1.`/`[x]`, `│ ` quotes, rules, inline code on the surface color, links accent+underline with the URL after them); `code` on the surface color with the band filling the width; `diff` colored `+`/`-` lines with muted hunk headers; `table` columns separated by `│` under a rule, widest columns cut first; `progress` `▓▓▓░░ 60% label`; `button` `[k Label]`; `image` `[image alt · W×H]` (terminals can't show it here); `file` `📄 path`. Unknown components and blocks (a newer protocol) draw a muted `[unsupported: type]` placeholder.

## Tests

`cargo test -p suite-renderer-ratatui`. Every `fixtures/<name>/resolved.json` is drawn and compared with `ratatui.txt` (80 columns), `ratatui-40.txt`, `ratatui-expanded.txt` (tool results in full, thoughts shown) and `ratatui-styled.txt` (every styled run as `⟦style|text⟧`, colors by theme name) next to it; `UPDATE_SNAPSHOTS=1` rewrites them. A sweep checks that no line of any fixture is wider than the width, from 1 to 200 columns. `tests/behavior.rs` covers wrapping, compact summaries, `buttons()`, focus, unknown components and narrow widths. Everything is headless: lines are inspected as data, nothing opens a terminal.
