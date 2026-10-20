# OpenUI in the suite

[OpenUI](https://github.com/thesysdev/openui) (spec 1.0, `@openuidev/lang-core`) is how the model and apps describe UI beyond the fixed component set: a compact, streamable program, one statement per line, with reactive state, forms and tool queries. The suite uses it as the **authoring** format. The daemon parses and evaluates every program with `lang-core`, and renderers draw the evaluated tree. A renderer never parses OpenUI Lang, evaluates an expression or holds `$state`; it draws `UiNode`s, keeps purely visual state (selected tab, open accordion, cursor, scroll) and sends what the user does back.

The terminal renderer (`renderers/ratatui`, used by `apps/term`) is the first to draw it, and the desktop window (`apps/gui`, in Svelte) draws it too. web, the Rust GPUI renderer and the Go TUI show a placeholder until they follow.

````
model (```openui fence) ─┐
app template (OpenUI Lang)┴─► daemon: lang-core parse → evaluate ($state store, queries) → UiDoc
                                         ▲                                                 │
                 ui.action openui.input / openui.press                       view.snapshot / view.patch
                                         └──────────────── renderer ◄──────────────────────┘
````

## Three layers

What the agent writes is made of three kinds of components, and only the first is a renderer's business:

1. **The library (`suite@1`)** is what renderers draw: the specific components (`Card`, `Table`, the charts, the inputs…) and the generic ones (`Box`, `Shape`) that the specific ones cannot cover. Every renderer draws all of them, natively (the tables below). There are no library versions and no runtime checks for what a renderer supports: a component is in the library when every renderer draws it, and the fixtures (`fixtures/openui-*`) use every component, so a renderer that misses one fails its snapshots.
2. **App components** are what an app adds to the library for the agent: a name, a description, typed parameters and a program over the library (`app.presence.components`, [protocol/README.md](../protocol/README.md#components)). `BrightnessCard("asset://a", "Sunset")` in the agent's program is a call lang-core checks against the params like any other.
3. **The daemon** sits between them. It parses programs with the library plus the components of the apps it knows, and replaces each use of an app component with what its program draws, evaluated with the call's arguments as `args`, before the tree goes to a renderer. A use is a program of its own, with its own `$state`, queries and keys, so two `BrightnessCard`s do not share `$level`. Renderers see library components only.

```
agent / app program ─► daemon: parse with library + app components
                               │  each BrightnessCard(…) ──► its program, run with args ──► library components
                               ▼
                         UiDoc of library components ──► renderers
```

## How the agent learns it

Every harness has the same channel, the gateway, so none has its own copy of the language:

- **The prompt.** neo's system prompt, `GUEST_PROMPT` and `MAIN_PROMPT` (`daemon/src/guests/prompt.ts`, for Claude Code and Codex) each say: _You can show UI by writing a ```openui block. Call ui_library before your first one._ A test keeps the three the same.
- **`ui_library()`**, a core tool of the gateway, returns the guidance (when UI beats prose, how to lay a page out, which component for what), lang-core's reference for the language and the `suite@1` library, and the components of the **connected** apps: under each app, `Name(arg: type, other?: type) — what it is for`. The text is generated when asked (`openui/src/skill.ts`), from the library and the registrations, so nothing on disk can go stale: a gateway test checks that every signature of the library is in it. The chat shows `N lines`, not the text.
- **The announcement.** `<app connected="photo-editor" kind="app" v="1" tools="…" components="BrightnessCard, PhotoCard" about="…"/>` names the components of an app when it first connects in a session; `<app updated … components="…"/>` lists them again when they changed (a change of components is a new version of the app), empty for none left. The agent calls `ui_library` again to read them. Apps connect while a session runs, so this is how an agent finds out, not the prompt.

## Where programs come from

- **The model** writes a fenced block with the language `openui` in a normal message. The daemon's markdown parser turns that fence into an `OpenUiBlock` (`{type: "openui", ui: UiDoc}`) among the message's other blocks, so it streams like text: every chunk re-parses the program and the block is replaced by the `blocks` op from the first changed block on. `ui.streaming` is true until the message is complete. Every harness learns the language and the library from the gateway's **`ui_library`** tool (see **How the agent learns it** below), which its prompt tells it to call before its first block.
- **Apps** may register a template that is OpenUI Lang instead of component JSON: `templates: {"<tool>": {"openui": "<program>"}}` in `app.presence`. The tool's `structuredContent` is appended to the program as a last statement `data = <value>` (a later statement wins, so a template can't shadow it), and the result is an `OpenUiComponent` (`{type: "openui", ui: UiDoc}`). A template that fails to produce a root falls back to the generic card, as JSON templates do.
- **App components** are programs too: `components: {"<Name>": {description, params, openui}}` in `app.presence`. The agent writes `Name(args…)` in its own program (or an app in a template), and the daemon runs the component's program in its place (see **App components** below).

## The wire format

```jsonc
// UiDoc: one evaluated program
{ "id": "0",              // unique within the item: the n-th OpenUI doc in the item, in document order
  "root": UiNode,         // absent until there is something to draw
  "streaming": true,      // still arriving: draw it, but no field or button can be used yet
  "errors": [ { "code": "unknown-component", "message": "…", "statementId": "chart" } ] }

// UiNode: one evaluated component
{ "component": "Input",   // library name (the tables below)
  "key": "email",         // only on interactive nodes: stable within the doc
  "partial": true,        // this node is still streaming in
  "props": { … } }        // concrete JSON by prop name; an object with "component" is a child node
```

Props are what the library declares (tables below), already evaluated: expressions, `@Each`, ternaries and references are resolved, and markdown is parsed. The daemon adds to the props:

| Node                                                                                                                                              | Added props                                                                                                                                                                                                                                   |
| ------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `Text`, `Callout`                                                                                                                                 | `blocks`: the text as parsed markdown (`Block[]` of the view format, never another `openui` block). Draw these, not `text`.                                                                                                                   |
| inputs (`Input`, `TextArea`, `Select`, `RadioGroup`, `OptionCards`, `CheckBox`, `Switch`, `CheckBoxGroup`, `SwitchGroup`, `Slider`, `DatePicker`) | `value`: the current value (string, bool or number, see below; absent = empty). `error`: the validation message to show, set after a submit attempt fails or a field is left invalid. `required`: true when the rules say so (draw a marker). |
| `Button`                                                                                                                                          | `url` when its only action is `@OpenUrl` (the client opens or shows it), `copy` when its only action is `@Copy` (the client copies). Otherwise pressing sends `openui.press`. `disabled` when it can't be pressed now.                        |

The `key` of an interactive node is its statement id, or the statement id plus a path when the node is inline or produced by `@Each` (`row.2.done`). It stays the same while the program re-evaluates.

## What the user does

Both go over the existing `ui.action {session, ref, action, args}`:

- `openui.input {doc, key, value}` is a field's new value: a string for `Input`, `TextArea`, `DatePicker`, `Select`, `RadioGroup`, `OptionCards`; a bool for `CheckBox`, `Switch`; a list of item names for `CheckBoxGroup`; an object of item name to bool for `SwitchGroup` (a partial one changes only the items it names); a number for `Slider`. The daemon writes it to the bound `$variable` or form field, re-evaluates and patches the item (`set`). Clients send it as the value changes, debounced (about 150 ms) for typing, and always right before an `openui.press`. While a field has focus the client keeps its own text and cursor and ignores the `value` coming back for that field.
- `openui.press {doc, key}` presses a button. The daemon runs its action plan: `@Set`, `@Reset`, `@Run` (queries and mutations call app tools through the gateway, as `app.tool` actions do), and `@ToAssistant(message, context?)`, which sends the agent an input like a button press does today, with the form's values and the context attached. A button inside a `Form` first validates the form's fields: if any fails, the daemon sets their `error` props and runs nothing.

Nothing is sent while `streaming` is true. State lives in the daemon, per session, item and doc, in memory: a restart keeps the program but forgets typed values.

## The library (`suite@1`)

Arguments are positional in the order listed. Optional ones are marked `?`, and `Node` means a component call. **The order is part of the format**: stored programs carry no prop names, so props are only ever added at the end (spec §3). `Tone` = `accent | muted | success | danger`.

### Layout

| Component       | Props                                                                                                                                                                                                                                                                          |
| --------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `Stack`         | `children: Node[]`, `direction?: "column" \| "row"` (column), `gap?: "none" \| "s" \| "m" \| "l"` (m), `wrap?: bool`, `align?: "start" \| "center" \| "end" \| "stretch"`, `justify?: "start" \| "center" \| "end" \| "between"`                                               |
| `Grid`          | `children: Node[]`, `columns?: number` (a maximum; default: as many as fit)                                                                                                                                                                                                    |
| `Carousel`      | `children: Node[]`, `visible?: number` (a maximum; default: as many as fit)                                                                                                                                                                                                    |
| `Card`          | `children: Node[]`, `title?: string`, `tone?: Tone`, `variant?: "card" \| "sunk" \| "clear"`                                                                                                                                                                                   |
| `Header`        | `title: string`, `subtitle?: string`                                                                                                                                                                                                                                           |
| `Separator`     | (none)                                                                                                                                                                                                                                                                         |
| `Tabs`          | `tabs: Tab[]`                                                                                                                                                                                                                                                                  |
| `Tab`           | `label: string`, `children: Node[]`                                                                                                                                                                                                                                            |
| `Accordion`     | `items: AccordionItem[]`                                                                                                                                                                                                                                                       |
| `AccordionItem` | `title: string`, `children: Node[]`, `open?: bool`                                                                                                                                                                                                                             |
| `Steps`         | `steps: Step[]`                                                                                                                                                                                                                                                                |
| `Step`          | `title: string`, `detail?: string`, `status?: "done" \| "current" \| "pending" \| "failed"`                                                                                                                                                                                    |
| `Box`           | `children: Node[]`, `dir?: "column" \| "row"` (column), `gap?: "none" \| "s" \| "m" \| "l"` (s), `pad?: "none" \| "s" \| "m" \| "l"` (none), `align?`, `justify?`, `wrap?` (as `Stack`), `width?: "fill" \| "fit" \| number`, `fill?: Tone`, `border?: Tone`, `rounded?: bool` |

### Content

| Component         | Props                                                                                                                                           |
| ----------------- | ----------------------------------------------------------------------------------------------------------------------------------------------- |
| `Text`            | `text: string` (markdown), `tone?: Tone`, `role?: "title" \| "body" \| "code"`                                                                  |
| `Code`            | `code: string`, `language?: string`                                                                                                             |
| `Table`           | `columns: string[]`, `rows: (string \| number \| bool \| Node)[][]` (cells may be `Tag`, `Link` or `Text`)                                      |
| `List`            | `items: (string \| Node)[]`, `ordered?: bool`                                                                                                   |
| `KeyValue`        | `pairs: KV[]`                                                                                                                                   |
| `KV`              | `key: string`, `value: string \| number`, `tone?: Tone`                                                                                         |
| `Tag`             | `label: string`, `tone?: Tone`                                                                                                                  |
| `EntityList`      | `items: ListItem[]`                                                                                                                             |
| `ListItem`        | `title: string`, `subtitle?: string`, `trailing?: string \| number \| Tag`                                                                      |
| `Callout`         | `text: string` (markdown), `tone?: Tone`, `title?: string`                                                                                      |
| `Metric`          | `label: string`, `value: string \| number`, `delta?: string`, `tone?: Tone`                                                                     |
| `MetricIndicator` | `value: string \| number`, `delta?: string`, `direction?: "up" \| "down"`, `tone?: Tone` (default: up success, down danger), `subtext?: string` |
| `Progress`        | `value: number` (0..1), `label?: string`                                                                                                        |
| `Image`           | `src: string`, `alt?: string`                                                                                                                   |
| `ImageGallery`    | `images: Image[]` (the first is drawn large)                                                                                                    |
| `Link`            | `text: string`, `url: string`                                                                                                                   |
| `Shape`           | `kind: "rect" \| "line" \| "path" \| "arc"`, `points: number[]`, `tone?: Tone`, `fill?: bool`, `aspect?: number`                                |

### Charts

| Component   | Props                                                                         |
| ----------- | ----------------------------------------------------------------------------- |
| `BarChart`  | `labels: string[]`, `series: Series[]`, `horizontal?: bool`, `stacked?: bool` |
| `LineChart` | `labels: string[]`, `series: Series[]`                                        |
| `AreaChart` | `labels: string[]`, `series: Series[]`                                        |
| `PieChart`  | `slices: Slice[]`, `donut?: bool`                                             |
| `Sparkline` | `values: number[]`, `label?: string`                                          |
| `Series`    | `name: string`, `values: number[]`                                            |
| `Slice`     | `label: string`, `value: number`                                              |

### Forms

| Component       | Props                                                                                                                                  |
| --------------- | -------------------------------------------------------------------------------------------------------------------------------------- |
| `Form`          | `name: string`, `children: Node[]`, `buttons?: Buttons`                                                                                |
| `FormControl`   | `label: string`, `input: Node`, `hint?: string`                                                                                        |
| `Input`         | `name: string`, `placeholder?: string`, `type?: "text" \| "email" \| "url" \| "number" \| "password"`, `rules?: Rules`, `value?: $var` |
| `TextArea`      | `name: string`, `placeholder?: string`, `rules?: Rules`, `value?: $var`                                                                |
| `Select`        | `name: string`, `options: Option[]`, `placeholder?: string`, `rules?: Rules`, `value?: $var`                                           |
| `Option`        | `value: string`, `label?: string`                                                                                                      |
| `RadioGroup`    | `name: string`, `options: Option[]`, `rules?: Rules`, `value?: $var`                                                                   |
| `OptionCards`   | `name: string`, `options: OptionCard[]`, `rules?: Rules`, `value?: $var`                                                               |
| `OptionCard`    | `value: string`, `title: string`, `description?: string`                                                                               |
| `CheckBox`      | `name: string`, `label: string`, `rules?: Rules`, `value?: $var`                                                                       |
| `Switch`        | `name: string`, `label: string`, `value?: $var`                                                                                        |
| `CheckBoxGroup` | `name: string`, `items: CheckBoxItem[]`, `rules?: Rules`, `value?: $var` (the list of checked names)                                   |
| `CheckBoxItem`  | `name: string`, `label: string`, `description?: string`, `checked?: bool` (starts checked)                                             |
| `SwitchGroup`   | `name: string`, `items: SwitchItem[]`, `value?: $var` (an object, name to on/off)                                                      |
| `SwitchItem`    | `name: string`, `label: string`, `description?: string`, `checked?: bool` (starts on)                                                  |
| `Slider`        | `name: string`, `min: number`, `max: number`, `step?: number`, `value?: $var`                                                          |
| `DatePicker`    | `name: string`, `rules?: Rules`, `value?: $var` (`YYYY-MM-DD`)                                                                         |
| `Button`        | `label: string`, `action?: Action`, `variant?: "primary" \| "secondary" \| "danger"`                                                   |
| `Buttons`       | `buttons: Button[]`                                                                                                                    |
| `FollowUpBlock` | `items: FollowUpItem[]` (suggested next questions)                                                                                     |
| `FollowUpItem`  | `text: string` (choosing it sends the text to the assistant)                                                                           |

`Box` and `Shape` are the generic components: what no specific one above is (a swatch, a timeline, a bar of parts, a gauge) is built from them, within what a terminal can draw. **Sizes are shares and colours are tones**: never pixels, cells, hex colours or fonts.

- **`Box`** is a `Stack` with a look. `width` is how much of its parent's width it takes: `fill` (what the siblings leave; the default in a column), `fit` (its content's; the default in a row) or a number from 0 to 1, a share of the parent's width (in a row, of what the gaps leave, so `0.5, 0.5` fills it exactly). `fill` is a wash of a tone behind the children, `border` a frame in a tone, `rounded` rounds the frame's corners, `pad` the room between frame and children. A box with none of them is just its children.
- **`Shape`** draws one `rect`, `line`, `path` or `arc` in a box of 0 to 1 on both sides, the top left corner `0,0`, which the parent sizes to its width and `aspect` (width over height, default 2) to a height. `points` are flat numbers, all 0 to 1 (outside is held to the edge): `rect` is `x, y, w, h`; `line` is `x1, y1, x2, y2` and any further points to connect; `path` is the corners of a closed shape; `arc` is `cx, cy, r, from, to`, `r` a share of the box's shorter side so a circle stays round, `from` and `to` turns clockwise from the top (`0.25` is 3 o'clock; the way round that passes the top counts, a whole turn is a circle). `fill` fills a rect, a path or, for an arc, the wedge to its center; the stroke is in `tone` (the text color without one). **Shapes that follow each other as siblings share one box and are drawn over each other in order**: a timeline row is its bars as Shapes, side by side in one Box.

`Rules` is the spec's validation object (`required`, `email`, `url`, `numeric`, `minLength`, `maxLength`, `min`, `max`, `pattern`). Actions are the spec's built-ins (`@ToAssistant`, `@OpenUrl`, `@Set`, `@Reset`, `@Run`) plus the library's `@Copy(text)`.

## Drawing it in a terminal

The ratatui renderer's rules (the README there is the full reference):

- **Layout.** A `row` Stack measures each child's minimum and preferred width and splits the width among them, wrapping to further lines with `wrap`, and stacking when the children don't fit at their minimum. A `Grid` puts its children in as many columns as fit at a minimum of about 30 cells each (never more than `columns`), left to right then down, every cell of a row as tall as the row's tallest.
- **Auto-layout.** The terminal doesn't draw a page as one long column just because the program says so: in a column Stack (or a Card, Tab, AccordionItem), a run of two or more sibling `Card`s is drawn as a `Grid`, and a run of two or more sibling `Metric`s as a metric strip. A model's layout gets the same treatment as one written with `Grid`.
- **Carousels.** A `Carousel` lays its children out like a `Grid` row (as many as fit at about 30 cells each, never more than `visible`) starting from the first slide in view, with `‹ 2/5 ›` centered under them (`‹ 2–3/5 ›` when several are shown; each arrow is lit when there is more on that side). It is focusable: Left/Right scroll one slide, Home/End jump to the ends, Enter or Space go on round. The last slide can be the first one in view, so scrolling never stalls whatever the width. A carousel of one child has no strip.
- **Cards** float: a full rounded outline (`╭─ title ─╮ │ │ ╰─╯`, `tone` colors it) around a fill in the theme's surface color, one cell of padding inside. `sunk` is the fill without the outline, `clear` neither.
- **Entity lists.** An `EntityList` is one row per `ListItem` in columns: the title bold, the subtitle muted, the trailing value or `Tag` right-aligned (numbers line up), `Ada Lovelace  analyst  [admin]`. A subtitle that does not fit is cut down to a minimum, and a list that still does not fit puts each subtitle muted under its title.
- **Metrics.** A metric strip puts each `Metric` in its own column, divided by `│`: the value bold, the `delta` after it in its tone, the label muted under them. A single Metric is the same column on its own.
- **Metric indicators.** A `MetricIndicator` is one inline line, `1284 ▲ 12% vs last week`: the value bold, the change with a `▲` / `▼` for `direction` in the tone's color (up is success, down danger unless `tone` says otherwise), the `subtext` muted. It sits in rows, cards, lists and table cells like text.
- **Image galleries.** An `ImageGallery` reserves room for its first image large (up to 3/5 of the width and the app's picture height) with up to three more stacked small beside it, each at most half as tall as the large one, and `name · W×H  +2 more` under them when there are further images. The pictures themselves are drawn by the app over that room, as for `Image`; images that are not loaded (or failed) keep their `[image name] …` line under the gallery, and a gallery with fewer than two images ready, or where pictures cannot be drawn, is the images one after the other. Below 36 cells only the large picture is shown.
- **Disclosure** is local: `Tabs` is a tab strip (`│ Overview │ Billing │`, the selected one reversed) over the selected tab's children; `AccordionItem` is `▸ title` / `▾ title`. Both are focusable and toggle with Enter, Left/Right switch tabs.
- **Box and Shape.** A `Box` is the column or row of a `Stack` (its own `gap` default is `s`) inside its padding (`pad` `s`, `m`, `l` is 1, 2 or 3 cells across, and `l` one line above and below), a frame of `┌─┐` (`╭─╮` with `rounded`) in the `border` tone and a wash of the `fill` tone (the tint a `Tag` has) behind everything, frame included. Its `width` is a share of the cells of the parent: of a column's, of a row's after the gaps; `fit` is its widest line and a row's boxes that `fill` split what the others leave. A `Shape` is drawn on a braille canvas, two dots across and four down a cell, which is as wide as it is tall, so the box is `aspect` times wider than tall in dots (at most 16 lines, at least 1); a cell a filled shape covers all over is a block (`█ ▀ ▄ ▌ ▐`), the rest braille, in the shape's tone.
- **Charts** are drawn by meaning, not by pixels: vertical bars with eighth blocks (`▁▂▃▄▅▆▇█`), horizontal ones half-height (`▄▄▄▖`) so neighbouring bars never touch, value at the end, horizontal whatever `horizontal` says once the labels don't fit under vertical bars; line and area charts on a braille canvas with a legend; pie and donut as one stacked bar of the slices with a `label  42%` legend; sparklines with `▁▂▃▄▅▆▇█`.
- **Option cards.** `OptionCards` draws its options as small framed cards, as many side by side as fit (22 to 34 cells each, a further row when they do not), each `(•) Title` over its muted description; the chosen card has the accent frame. The group is one focus target that works like a `RadioGroup` (Left/Right choose).
- **Groups.** `CheckBoxGroup` and `SwitchGroup` draw one `[x] Label` / `● on  Label` line per item with its muted description wrapped under, and are one focus target: Up/Down move a cursor between the items (past the first or last they go on to the neighbouring field), Space toggles the item under it, and the cursor item is highlighted.
- **Follow-ups.** A `FollowUpBlock` is its items as pills, `( Why is it slow? )  ( Show the queries )`, side by side and wrapping, the focused one reversed. They are focusable (Left/Right between them, Enter or Space sends the text) but are not a form: a doc with only follow-ups does not take the editor's place, it is reached with Ctrl-P / Ctrl-N.
- **Forms.** Every input is a focusable field: `Label *` over `[ value or placeholder▏ ]`, the error under it in the danger tone. `Select` shows the chosen option with `▾` and opens a list on Enter; `RadioGroup` `(•)`/`( )`; `CheckBox` `[x]`/`[ ]`; `Switch` `● on`/`○ off`; `Slider` a bar with the number; `DatePicker` a text field checked as `YYYY-MM-DD`.
- **Focus.** A doc with keyed nodes is a form: in `apps/term` it takes the editor's place like a prompt, and Tab / Shift-Tab, typing, Space, Left/Right, Enter and Esc work it (the full key list is under **Forms** in `apps/term/README.md`). Docs with only tabs, accordions, carousels or follow-ups are reached with Ctrl-P / Ctrl-N. While `streaming` the doc draws but takes no focus.
- **Errors.** `errors` are not drawn in the chat; a doc with no root but errors shows one muted line `[ui: <first error message>]`.

## Drawing it in the GUI

The desktop window (`apps/gui`, `ui/plugins/transcript/openui/`) draws the same tree as real cards, charts and controls instead of character art, and keeps the terminal's meaning: what a prop does, how layout adapts to the width, what is sent when. The README there has what the user sees and the keys.

- **Where.** A ` ```openui ` block is drawn in its place among the agent's prose, a UI result of a tool (an app's template) under its row, which stays open while the doc has a field or a button. Layout works from the width the column gives (never measured after the fact), so a doc is laid out in the first frame.
- **Layout.** Pixels where the terminal has cells: a `Grid` or a run of sibling Cards has as many columns as fit at 240 px (at most `columns`), every card in a row as tall as the tallest; a run of two or more Metrics, and a row of only Metrics, is one band divided by thin rules; a row Stack stacks when its children do not fit at their narrowest and wraps with `wrap`; a Carousel shows as many slides as fit (at most `visible`) with `‹ 2–3/5 ›` under them. `EntityList` puts each subtitle under its title when the row does not fit.
- **Content.** Cards are rounded on the surface colour (`tone` colours the outline and title, `sunk` is the fill alone, `clear` neither), `Tag` is a pill on a wash of its tone, `Callout` has a bar in its tone, tables right-align number columns, `Code` is the chat's code block. A `MetricIndicator` is one inline line with a caret for the direction.
- **Box and Shape.** A `Box` is a view with the share of the parent's width it asks for in pixels (`pad` `s`, `m`, `l` is 8, 14 or 20 px), its tone fill as a wash of the surface and its border a 1 px frame; `rounded` rounds it. A `Shape` is an SVG per shape over one box, `aspect` times wider than tall, tinted by its tone; strokes are 2 px and an outline sits inside the box. Shapes that follow each other in a column are one box.
- **Charts** are drawn, not typed: bars are views with a value axis and a tooltip on hover (they lie down when the labels do not fit under them or with `horizontal`), lines, areas and slices are SVG paths tinted with the series' colour.
- **Fields** are real controls: text fields (the field keeps its own text while it has the keyboard and ignores the `value` coming back for it), a dropdown for `Select`, rings for `RadioGroup`, cards for `OptionCards`, boxes and switches, a slider to drag, and a calendar for `DatePicker`. The label has a `*` when `required`, the `error` shows under the field in the danger colour. Typing goes to the daemon about 150 ms after the last key (a date once it is `YYYY-MM-DD` or empty), everything else at once; a press sends what is waiting first. A value chosen here shows until the daemon echoes it.
- **Keys.** Tab and Shift-Tab go through the fields and buttons of the doc in drawing order (what a closed accordion item or another tab hides is skipped) once one has the keyboard, Esc gives it back. Enter in a field presses the form's first button. A button with a `url` opens in the system's browser (`open_url` in the host: `http`, `https` and `mailto` only), one with `copy` copies.
- **Streaming.** A doc that is still `streaming` draws but its controls are dimmed and send nothing.
- **Images** come from the daemon: an `asset://` handle, a path, or an `http(s)` URL, which the window has the daemon download with `asset.fetch` into its asset store (`fetchAsset` in the TypeScript SDK) and then reads from there. Each picture has a box of fixed size, and says why when it cannot be loaded.

## How the daemon does it

The code is `openui/` (the `suite@1` library, `components.ts` for the apps' components, `Program`: lang-core parse, evaluate and state, `skill.ts` for the text `ui_library` returns) and `daemon/src/view/` (`openui.ts`, `markdown.ts`, `session-view.ts`).

- **Fences.** A top-level code block whose language is `openui` (or `openui-lang`) becomes an `openui` block; in lists and quotes, in thoughts and in messages between agents it stays code. A fence still open is `streaming` until it closes; a message that ends (`turn_ended`) with a fence left open finishes it, with the usual errors. A half-written closing fence (`` ` `` or ` `` `) is not part of the program.
- **While streaming** only the statement being written is `partial`, and `errors` stay out: what looks broken may be unfinished. Queries do not start, and `openui.input` and `openui.press` do nothing.
- **Keys.** An interactive node's `key` is the id of the statement it is in, when that statement has only that one (`service = FormControl("Service", Select("service", …))`: `service`). Otherwise the statement id and the path to the node: `c.buttons.1`, `root.children.2` (a node made by `@Each`).
- **Fields.** An input bound to a `$variable` writes it; any other keeps its value in the doc, per form and name. Select, RadioGroup and OptionCards only take one of their options, a CheckBoxGroup a list of its items' names (kept in the items' order) and a SwitchGroup an object of them (an item not mentioned keeps its state; an item's `checked` is where it starts, unless a `$variable` already holds a value), a Slider is held to `min`..`max`, a CheckBox and a Switch take a bool, and the rest a string; anything else is ignored. A required CheckBox or Switch has to be on. A DatePicker has to be `YYYY-MM-DD`. A field that failed is checked again whenever it changes. Messages are lang-core's ("Please enter a valid email").
- **Buttons.** A button with no action sends its label to the agent; a `FollowUpItem` is such a button whose label is its text. `url` is only set for `http:`, `https:` and `mailto:` addresses. `@OpenUrl` and `@Copy` inside a plan of several steps do nothing. `disabled` is set while a mutation the button runs is in flight.
- **What the agent hears** is one input per `@ToAssistant`: `[ui action] The user pressed "Deploy" in an OpenUI doc: Deploy with these settings`, then `form deploy: {…}` (the form's values; outside a form `state: {…}` with the `$variables`), then `context: {…}`.
- **Tools.** A `Query` runs when its doc is finished and draws, and after a `@Run` of it, through the gateway (`app.tool`), and is not recorded. A `Mutation` runs on press and is recorded in the session like a button that runs a tool (so its result shows in the chat with the app's template). A failing tool is an entry in `errors`; the doc keeps the query's defaults.
- **App components.** The library the daemon parses with is the suite's plus the components of every connector it knows (also closed ones, which are kept like templates, so an old reply still draws), built when a component changes and shared by all sessions; a program asks for it whenever it is set, so an app that connects while a reply streams is used by the next chunk. Registration (`app.presence`) rejects a name that is not a PascalCase identifier or that the library, the language or another app has, params that are not typed or put an optional before a required one, and a program that does not parse, has no `root` or uses what it does not define; the app does not connect. A name is plain because lang-core reads `photo-editor.X` as a subtraction and a member access. A use is expanded where it sits: the node a call evaluates to is replaced by the root of a program of its own, `<component's program>` then `args = <the call's arguments as JSON>`, made once per place in the doc (`root.children.0`, `a`) and kept for as long as the place holds a use of that component, with its own `$state`, queries and keys. Its keys are the use's place and the key inside it (`a/slider`), which is what `openui.input` and `openui.press` carry; they are routed to that program. A query or mutation in it goes through the gateway like any other; a query answering patches the item. The component's program is made of the library alone (an app component cannot call another), which keeps expansion finite. A use whose component is gone, or whose program draws nothing, is a `Callout` saying so, with the error on the doc.
- **Plain statements in a query.** A `Query` or `Mutation` argument that names a plain statement (`args.asset`, `limit`) reads its value, as it does in a prop; lang-core leaves those as references and only resolves queries.
- **State** is kept per view, item and doc, in memory, and a doc is told apart by its item `ref` (`m-<messageId>` for a message) and `id`.
