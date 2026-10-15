# renderers/tui

Go renderer that draws resolved view items in the terminal: the TUI column of `renderers/README.md`. It is a pure function from items, theme and options to text, so the CLI, tests and snapshots all use the same code.

```go
import "neoworks.dev/suite/renderers/tui"

out := tui.Render(items, theme, tui.Options{Width: 80, Color: tui.TrueColor})
one := tui.RenderItem(item, theme, tui.Options{Width: 80})   // no trailing newline
for _, b := range tui.Buttons(item) { /* b.Key, b.Label, b.Action, b.Args */ }
```

`Options`:

- `Width` columns (0 means 80; clamped to at least 10). Every output line fits it, wide characters included.
- `Color`: `None` (no escape codes at all), `ANSI16`, `ANSI256`, `TrueColor`. Theme colors are mapped to the nearest the terminal has; text without a role keeps the terminal's own colors. Code blocks and inline code use the theme's surface color as background.
- `Frame` picks the glyph of the spinner of running tools (`◐◓◑◒`), so a caller redrawing a live item can animate it.
- `Thoughts` draws thought items in full instead of one `… thinking` line.

`tui.DefaultTheme()` is `protocol/themes/default.json`, for when no theme has arrived yet. `tui.Width(s)` is the display width the renderer uses.

## Snapshots

`TestSnapshots` draws every `fixtures/*/resolved.json` at 80 and 40 columns without color and compares with `tui.txt` / `tui-40.txt` next to it; `TestSnapshotANSI256` does the same with escapes for `chat-turn` (`tui-256.ans`, view it with `cat`). `UPDATE_SNAPSHOTS=1 go test ./...` rewrites them; review the diff like code.

## Layout decisions the README leaves open

- Items are separated by one blank line; `stack`/`row` gaps are 0/0/1 blank lines (s/m/l), whatever the theme's pixel spacing says.
- A `rule` spans the full width.
- Nested list items hang under the text of their parent (2 columns after `• `, 3 after `1. `); numbers are right-aligned when the list reaches 10.
- `file` shows the path (and `name (path)` when a distinct name is given); `image` ignores `alt`.
- Diffs are unified (`+`, `-`, space) without line numbers; long lines are cut at the width and continued under the same column.
