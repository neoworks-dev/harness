# neoui in neo

[neoui](../../neoui/DESIGN.md) (`@neoworks/neoui`, its own repository next to this one) is how the model and apps describe UI beyond the fixed component set: HTML-style markup whose elements are library components, with named form fields, tool queries (`hx-get`) and buttons that act (`hx-post`, `ui-ask`). Neo uses it as the **authoring** format. The daemon parses and evaluates every program with neoui, and renderers draw the evaluated tree. A renderer never parses neoui, evaluates an expression or holds a variable; it draws `UiNode`s, keeps purely visual state (selected tab, open accordion, cursor, scroll) and sends what the user does back. The language itself (syntax, fields and rules, queries, actions, expressions, streaming, the whole library) is in neoui's [DESIGN.md](../../neoui/DESIGN.md) and not repeated here; this page is what neo does with it.

The terminal renderer (`renderers/ratatui`, used by `apps/term`) is the first to draw it, and the desktop window (`apps/gui`, in Svelte) draws it too. web, the Rust GPUI renderer and the Go TUI show a placeholder until they follow.

````
model (```neoui fence) ─┐
app template (neoui) ───┴─► daemon: neoui parse → evaluate (fields, query results) → UiDoc
                                         ▲                                               │
                 ui.action neoui.input / neoui.press                     view.snapshot / view.patch
                                         └──────────────── renderer ◄─────────────────────┘
````

On the wire the block and component type is `neoui`, the actions are `neoui.input` and `neoui.press`.

## Three layers

What the agent writes is made of three kinds of components, and only the first is a renderer's business:

1. **The library (`neo@1`)** is what renderers draw: the specific components (`Card`, `Table`, the charts, the inputs…) and the generic ones (`Box`, `Shape`) that the specific ones cannot cover. Every renderer draws all of them, natively (the tables below). There are no library versions and no runtime checks for what a renderer supports: a component is in the library when every renderer draws it, and the fixtures (`fixtures/neoui-*`) use every component, so a renderer that misses one fails its snapshots.
2. **App components** are what an app adds to the library for the agent: a name, a description, typed parameters and a neoui body over the library (`app.presence.components`, [protocol/README.md](../protocol/README.md#components)). They are written with their app, `<photo-editor.BrightnessCard asset="asset://a" name="Sunset"/>` in the agent's program: the app id, a dot and the name, so nothing clashes with the library or with another app.
3. **The daemon** sits between them. It parses programs with the library plus the components of the apps it knows, and replaces each use of an app component with what its body draws, evaluated with the use's attributes as its params, before the tree goes to a renderer. A use is a program of its own, with its own fields, queries and keys, so two `photo-editor.BrightnessCard`s do not share `level`. Renderers see library components only.

```
agent / app program ─► daemon: parse with library + app components
                               │  each <photo-editor.BrightnessCard …/> ──► its body, run with its params ──► library components
                               ▼
                         UiDoc of library components ──► renderers
```

## How the agent learns it

Every harness has the same channel, the gateway, so none has its own copy of the language:

- **The brief.** neo's system prompt, `GUEST_PROMPT` and `MAIN_PROMPT` (`daemon/src/guests/prompt.ts`, for Claude Code and Codex) each carry `UI_BRIEF` (`neo/src/ui-brief.ts`, about 400 tokens): that the agent can write a ```neoui block, the language in a sentence, every component by name, a line per group (neoui's `indexText`), when UI beats prose, and to use the Dev components for diffs, test runs, command output, commits, PRs, issues and checks. So an agent knows what there is without a tool call; a test keeps the three the same.
- **`ui_library({components?})`**, a core tool of the gateway. With `components` (names like `TestRun`, groups like `Dev`, an app's `photo-editor.BrightnessCard`) it returns their signatures, the components their slots take and their groups' notes (neoui's `lookupText`): what an agent asks for before writing components it doesn't know the attributes of. With none it returns everything: the guidance (`daemon/src/gateway/ui-library.ts`, the harness's own, about 400 tokens), neoui's reference for the language (as what it adds to HTML and htmx) and the `neo@1` library, and the components of the **connected** apps, under each app `<app.Name arg="type" other?="type"/> — what it is for`. Both are generated when asked from the library and the registrations, so nothing on disk can go stale. The chat shows `N lines`, not the text.
- **Mistakes are answered.** When a turn ends, the main agent's neoui blocks are evaluated (`daemon/src/ui-lint.ts`); an unknown attribute, a missing required one, a child in the wrong place or an unknown component is told to the agent with its next prompt (a `user_note`), with the signature of the component it is about or the nearest name. An agent that writes from the brief alone is corrected rather than left guessing. Text that runs long is noted the same way.
- **The announcement.** `<app connected="photo-editor" kind="app" v="1" tools="…" components="photo-editor.BrightnessCard, photo-editor.PhotoCard" about="…"/>` names the components of an app, as they are written, when it first connects in a session; `<app updated … components="…"/>` lists them again when they changed (a change of components is a new version of the app), empty for none left. The agent calls `ui_library` again to read them. Apps connect while a session runs, so this is how an agent finds out, not the prompt.

## Where programs come from

- **The model** writes a fenced block with the language `neoui` in a normal message. The daemon's markdown parser turns that fence into a `NeoUiBlock` (`{type: "neoui", ui: UiDoc}`) among the message's other blocks, so it streams like text: every chunk re-parses the program and the block is replaced by the `blocks` op from the first changed block on. `ui.streaming` is true until the message is complete. Every harness learns the language and the library from the gateway's **`ui_library`** tool (see **How the agent learns it** above), which its prompt tells it to call before its first block. A ```openui fence (OpenUI, the language before neoui) is code like any other: old programs are not read.
- **Apps** may register a template that is neoui instead of component JSON: `templates: {"<tool>": {"neoui": "<markup>"}}` in `app.presence`. The tool's `structuredContent` is given to the program as the value `data` (`program.provide({ data })`), so the template reads `{data.total}` and `<For each={data.rows} …>`; it is not text appended to the program. The result is a `NeoUiComponent` (`{type: "neoui", ui: UiDoc}`). A template that fails to produce a root falls back to the generic card, as JSON templates do.
- **App components** are programs too: `components: {"<Name>": {description, params, neoui}}` in `app.presence`. The agent writes `<app.Name …/>` in its own program (or an app in a template), and the daemon runs the component's body in its place (see **App components** below).

## The wire format

```jsonc
// UiDoc: one evaluated program
{ "id": "0",              // unique within the item: the n-th neoui doc in the item, in document order
  "root": UiNode,         // absent until there is something to draw
  "streaming": true,      // still arriving: draw it, but no field or button can be used yet
  "errors": [ { "code": "unknown-component", "message": "…", "statementId": "0.2" } ] }

// UiNode: one evaluated component
{ "component": "Input",   // library name (the tables below)
  "key": "email",         // only on interactive nodes: stable within the doc
  "partial": true,        // this node is still streaming in
  "props": { … } }        // concrete JSON by prop name; an object with "component" is a child node
```

Props are what the library declares (tables below), already evaluated: expressions, `<For>`, `<If>` and `{…}` in text are resolved, and markdown is parsed. The daemon adds to the props:

| Node                                                                                                                                              | Added props                                                                                                                                                                                                                                   |
| ------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `Text`, `Callout`                                                                                                                                 | `blocks`: the text as parsed markdown (`Block[]` of the view format, never another `neoui` block). Draw these, not `text`.                                                                                                                    |
| inputs (`Input`, `TextArea`, `Select`, `RadioGroup`, `OptionCards`, `CheckBox`, `Switch`, `CheckBoxGroup`, `SwitchGroup`, `Slider`, `DatePicker`) | `value`: the current value (string, bool or number, see below; absent = empty). `error`: the validation message to show, set after a submit attempt fails or a field is left invalid. `required`: true when the rules say so (draw a marker). |
| `Button`                                                                                                                                          | `url` when its only behaviour is `href` (the client opens or shows it), `copy` when it is `ui-copy` (the client copies). Otherwise pressing sends `neoui.press`. `disabled` when it can't be pressed now.                                     |

The `key` of an interactive node (the eleven inputs, `Button`, `FollowUpItem`) is its element's `id` when it has one, else for an input its field `name`, else its path (the child indices from the root, `0.3.1`; `2.1.0` is the first child of the second iteration of a `For` at `2`). A repeated key gets `#2`. Inside an app component it is `<where the use sits>/<key inside it>` (`a/level`, `0.1/level`). It stays the same while the program text before the element does not change.

## What the user does

Both go over the existing `ui.action {session, ref, action, args}`:

- `neoui.input {doc, key, value}` is a field's new value: a string for `Input`, `TextArea`, `DatePicker`, `Select`, `RadioGroup`, `OptionCards`; a bool for `CheckBox`, `Switch`; a list of item names for `CheckBoxGroup`; an object of item name to bool for `SwitchGroup` (a partial one changes only the items it names); a number for `Slider`. The daemon holds it as the field's variable (`$name`), re-evaluates and patches the item (`set`). Clients send it as the value changes, debounced (about 150 ms) for typing, and always right before a `neoui.press`. While a field has focus the client keeps its own text and cursor and ignores the `value` coming back for that field.
- `neoui.press {doc, key}` presses a button. The daemon runs its steps in neoui's fixed order: `ui-set`, then the request (`hx-post` and `hx-get` call app tools through the gateway, as `app.tool` actions do), `ui-reset`, and `ui-ask="message"` (with `ui-context`), which sends the agent an input like a button press does today, with the form's values and the context attached. A button inside a `Form` first validates the form's fields: if any fails, the daemon sets their `error` props and runs nothing. A failing request halts the rest.

Nothing is sent while `streaming` is true. State lives in the daemon, per session, item and doc, in memory: a restart keeps the program but forgets typed values.

## The library (`neo@1`)

The props of the evaluated nodes, by name; in a neoui program they are the attributes of the element (neoui's `src/library.ts` is the definition and its `ui_library` text the signatures a model reads; children go where the element is nested, and `Table`/`List` text may be written between the tags). Optional ones are marked `?`, and `Node` means a component. `Tone` = `accent | muted | success | danger`.

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

### Dev

The things of a codebase and its work, so a model shows them the way a developer reads them instead of as a `Table` or a `Code` block. The props are what the model knows (from `git`, `gh`, a test run); nothing is fetched.

| Component       | Props                                                                                                                                                                                                                                                                                 |
| --------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `Diff`          | `diff: string` (a unified diff, the body), `path?: string`                                                                                                                                                                                                                            |
| `FileTree`      | `files: File[]`                                                                                                                                                                                                                                                                       |
| `File`          | `path: string`, `status?: "modified" \| "added" \| "deleted" \| "renamed" \| "untracked" \| "unchanged"`, `added?: number`, `removed?: number`, `note?: string`                                                                                                                       |
| `CodeRef`       | `path: string`, `line?: number`, `text?: string`                                                                                                                                                                                                                                      |
| `Commit`        | `sha: string`, `title: string`, `author?: string`, `date?: string`, `added?: number`, `removed?: number`, `files?: number`, `url?: string`                                                                                                                                            |
| `CommandOutput` | `command: string`, `output: string` (the body), `exit?: number`, `duration?: string`, `cwd?: string`                                                                                                                                                                                  |
| `TestRun`       | `tests: Test[]`, `name?: string`, `passed?: number`, `failed?: number`, `skipped?: number`, `duration?: string`                                                                                                                                                                       |
| `Test`          | `name: string`, `status?: "passed" \| "failed" \| "skipped"`, `duration?: string`, `message?: string` (the body)                                                                                                                                                                      |
| `StackTrace`    | `message: string`, `kind?: string`, `frames: Frame[]` (innermost first)                                                                                                                                                                                                               |
| `Frame`         | `path: string`, `line?: number`, `name?: string`                                                                                                                                                                                                                                      |
| `Issue`         | `number: number`, `title: string`, `state?: "open" \| "closed"`, `repo?: string`, `author?: string`, `labels?: string[]`, `comments?: number`, `url?: string`                                                                                                                         |
| `PullRequest`   | `number: number`, `title: string`, `state?: "open" \| "draft" \| "merged" \| "closed"`, `repo?`, `branch?`, `base?`, `author?: string`, `checks?: "passing" \| "failing" \| "pending"`, `review?: "approved" \| "changes" \| "pending"`, `added?`, `removed?: number`, `url?: string` |
| `Checks`        | `checks: Check[]`                                                                                                                                                                                                                                                                     |
| `Check`         | `name: string`, `status?: "passed" \| "failed" \| "pending" \| "skipped"`, `duration?: string`, `url?: string`                                                                                                                                                                        |
| `Timeline`      | `items: TimelineItem[]`                                                                                                                                                                                                                                                               |
| `TimelineItem`  | `title: string`, `time?: string`, `detail?: string`, `tone?: Tone`                                                                                                                                                                                                                    |

`Diff`, `CommandOutput`, `Test` and `Code` take their body as written: the indentation is kept and `{ }` are literal. An item written outside its list (a `File`, a `Test`, a `Check`…) is drawn as a list of one.

### Forms

| Component       | Props                                                                                                                            |
| --------------- | -------------------------------------------------------------------------------------------------------------------------------- |
| `Form`          | `name: string`, `children: Node[]`, `buttons?: Buttons`                                                                          |
| `FormControl`   | `label: string`, `input: Node`, `hint?: string`                                                                                  |
| `Input`         | `name: string`, `placeholder?: string`, `type?: "text" \| "email" \| "url" \| "number" \| "password"`, `rules?: Rules`, `value?` |
| `TextArea`      | `name: string`, `placeholder?: string`, `rules?: Rules`, `value?`                                                                |
| `Select`        | `name: string`, `options: Option[]`, `placeholder?: string`, `rules?: Rules`, `value?`                                           |
| `Option`        | `value: string`, `label?: string`                                                                                                |
| `RadioGroup`    | `name: string`, `options: Option[]`, `rules?: Rules`, `value?`                                                                   |
| `OptionCards`   | `name: string`, `options: OptionCard[]`, `rules?: Rules`, `value?`                                                               |
| `OptionCard`    | `value: string`, `title: string`, `description?: string`                                                                         |
| `CheckBox`      | `name: string`, `label: string`, `rules?: Rules`, `value?`                                                                       |
| `Switch`        | `name: string`, `label: string`, `value?`                                                                                        |
| `CheckBoxGroup` | `name: string`, `items: CheckBoxItem[]`, `rules?: Rules`, `value?` (the list of checked names)                                   |
| `CheckBoxItem`  | `name: string`, `label: string`, `description?: string`, `checked?: bool` (starts checked)                                       |
| `SwitchGroup`   | `name: string`, `items: SwitchItem[]`, `value?` (an object, name to on/off)                                                      |
| `SwitchItem`    | `name: string`, `label: string`, `description?: string`, `checked?: bool` (starts on)                                            |
| `Slider`        | `name: string`, `min: number`, `max: number`, `step?: number`, `value?`                                                          |
| `DatePicker`    | `name: string`, `rules?: Rules`, `value?` (`YYYY-MM-DD`)                                                                         |
| `Button`        | `label: string`, `variant?: "primary" \| "secondary" \| "danger"`                                                                |
| `Buttons`       | `buttons: Button[]`                                                                                                              |
| `FollowUpBlock` | `items: FollowUpItem[]` (suggested next questions)                                                                               |
| `FollowUpItem`  | `text: string` (choosing it sends the text to the assistant)                                                                     |

`Box` and `Shape` are the generic components: what no specific one above is (a swatch, a timeline, a bar of parts, a gauge) is built from them, within what a terminal can draw. **Sizes are shares and colours are tones**: never pixels, cells, hex colours or fonts.

- **`Box`** is a `Stack` with a look. `width` is how much of its parent's width it takes: `fill` (what the siblings leave; the default in a column), `fit` (its content's; the default in a row) or a number from 0 to 1, a share of the parent's width (in a row, of what the gaps leave, so `0.5, 0.5` fills it exactly). `fill` is a wash of a tone behind the children, `border` a frame in a tone, `rounded` rounds the frame's corners, `pad` the room between frame and children. A box with none of them is just its children.
- **`Shape`** draws one `rect`, `line`, `path` or `arc` in a box of 0 to 1 on both sides, the top left corner `0,0`, which the parent sizes to its width and `aspect` (width over height, default 2) to a height. `points` are flat numbers, all 0 to 1 (outside is held to the edge): `rect` is `x, y, w, h`; `line` is `x1, y1, x2, y2` and any further points to connect; `path` is the corners of a closed shape; `arc` is `cx, cy, r, from, to`, `r` a share of the box's shorter side so a circle stays round, `from` and `to` turns clockwise from the top (`0.25` is 3 o'clock; the way round that passes the top counts, a whole turn is a circle). `fill` fills a rect, a path or, for an arc, the wedge to its center; the stroke is in `tone` (the text color without one). **Shapes that follow each other as siblings share one box and are drawn over each other in order**: a timeline row is its bars as Shapes, side by side in one Box.

The inputs' `rules` are the HTML-named rules neoui reads off the attributes (`required`, `type="email"`, `minLength`, `pattern`…; [DESIGN.md](../../neoui/DESIGN.md#rules)); the node keeps them as an object, with `required: true` and the `error` to show.

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
- **Dev components** look like the terminal's own: a `Diff` like an edit's diff, a `FileTree` like the files panel (`M`, `A`, `D`, `?` marks, folders holding one folder joined, `+12 −3` at the edge), `$ command  ✗ exit 101 · 8.2s` over the last 12 lines of a `CommandOutput` (error lines of a failed one in the danger color), a `TestRun`'s `✓ 434 passed  ✗ 1 failed` over its tests, each failed one with the first 6 lines of its message, `Issue` and `PullRequest` as a state pill, `#41` and the title over a muted line of the rest. Ctrl-O (expanded) shows outputs and messages whole.
- **Charts** are drawn by meaning, not by pixels: vertical bars with eighth blocks (`▁▂▃▄▅▆▇█`), horizontal ones half-height (`▄▄▄▖`) so neighbouring bars never touch, value at the end, horizontal whatever `horizontal` says once the labels don't fit under vertical bars; line and area charts on a braille canvas with a legend; pie and donut as one stacked bar of the slices with a `label  42%` legend; sparklines with `▁▂▃▄▅▆▇█`.
- **Option cards.** `OptionCards` draws its options as small framed cards, as many side by side as fit (22 to 34 cells each, a further row when they do not), each `(•) Title` over its muted description; the chosen card has the accent frame. The group is one focus target that works like a `RadioGroup` (Left/Right choose).
- **Groups.** `CheckBoxGroup` and `SwitchGroup` draw one `[x] Label` / `● on  Label` line per item with its muted description wrapped under, and are one focus target: Up/Down move a cursor between the items (past the first or last they go on to the neighbouring field), Space toggles the item under it, and the cursor item is highlighted.
- **Follow-ups.** A `FollowUpBlock` is its items as pills, `( Why is it slow? )  ( Show the queries )`, side by side and wrapping, the focused one reversed. They are focusable (Left/Right between them, Enter or Space sends the text) but are not a form: a doc with only follow-ups does not take the editor's place, it is reached with Ctrl-P / Ctrl-N.
- **Forms.** Every input is a focusable field: `Label *` over `[ value or placeholder▏ ]`, the error under it in the danger tone. `Select` shows the chosen option with `▾` and opens a list on Enter; `RadioGroup` `(•)`/`( )`; `CheckBox` `[x]`/`[ ]`; `Switch` `● on`/`○ off`; `Slider` a bar with the number; `DatePicker` a text field checked as `YYYY-MM-DD`.
- **Focus.** A doc with keyed nodes is a form: in `apps/term` it takes the editor's place like a prompt, and Tab / Shift-Tab, typing, Space, Left/Right, Enter and Esc work it (the full key list is under **Forms** in `apps/term/README.md`). Docs with only tabs, accordions, carousels or follow-ups are reached with Ctrl-P / Ctrl-N. While `streaming` the doc draws but takes no focus.
- **Errors.** `errors` are not drawn in the chat; a doc with no root but errors shows one muted line `[ui: <first error message>]`.

## Drawing it in the GUI

The desktop window (`apps/gui`, `ui/plugins/transcript/neoui/`) draws the same tree as real cards, charts and controls instead of character art, and keeps the terminal's meaning: what a prop does, how layout adapts to the width, what is sent when. The README there has what the user sees and the keys.

- **Where.** A ` ```neoui ` block is drawn in its place among the agent's prose, a UI result of a tool (an app's template) under its row, which stays open while the doc has a field or a button. Layout works from the width the column gives (never measured after the fact), so a doc is laid out in the first frame.
- **Layout.** Pixels where the terminal has cells: a `Grid` or a run of sibling Cards has as many columns as fit at 240 px (at most `columns`), every card in a row as tall as the tallest; a run of two or more Metrics, and a row of only Metrics, is one band divided by thin rules; a row Stack stacks when its children do not fit at their narrowest and wraps with `wrap`; a Carousel shows as many slides as fit (at most `visible`) with `‹ 2–3/5 ›` under them. `EntityList` puts each subtitle under its title when the row does not fit.
- **Content.** Cards are rounded on the surface colour (`tone` colours the outline and title, `sunk` is the fill alone, `clear` neither), `Tag` is a pill on a wash of its tone, `Callout` has a bar in its tone, tables right-align number columns, `Code` is the chat's code block. A `MetricIndicator` is one inline line with a caret for the direction.
- **Box and Shape.** A `Box` is a view with the share of the parent's width it asks for in pixels (`pad` `s`, `m`, `l` is 8, 14 or 20 px), its tone fill as a wash of the surface and its border a 1 px frame; `rounded` rounds it. A `Shape` is an SVG per shape over one box, `aspect` times wider than tall, tinted by its tone; strokes are 2 px and an outline sits inside the box. Shapes that follow each other in a column are one box.
- **Charts** are drawn, not typed: bars are views with a value axis and a tooltip on hover (they lie down when the labels do not fit under them or with `horizontal`), lines, areas and slices are SVG paths tinted with the series' colour.
- **Fields** are real controls: text fields (the field keeps its own text while it has the keyboard and ignores the `value` coming back for it), a dropdown for `Select`, rings for `RadioGroup`, cards for `OptionCards`, boxes and switches, a slider to drag, and a calendar for `DatePicker`. The label has a `*` when `required`, the `error` shows under the field in the danger colour. Typing goes to the daemon about 150 ms after the last key (a date once it is `YYYY-MM-DD` or empty), everything else at once; a press sends what is waiting first. A value chosen here shows until the daemon echoes it.
- **Keys.** Tab and Shift-Tab go through the fields and buttons of the doc in drawing order (what a closed accordion item or another tab hides is skipped) once one has the keyboard, Esc gives it back. Enter in a field presses the form's first button. A button with a `url` opens in the system's browser (`open_url` in the host: `http`, `https` and `mailto` only), one with `copy` copies.
- **Streaming.** A doc that is still `streaming` draws but its controls are dimmed and send nothing.
- **Images** come from the daemon: an `asset://` handle, a path, or an `http(s)` URL, which the window has the daemon download with `asset.fetch` into its asset store (`fetchAsset` in the TypeScript SDK) and then reads from there. Each picture has a box of fixed size, and says why when it cannot be loaded.

## How the daemon does it

The language is `@neoworks/neoui` (a sibling repository: a fresh clone runs `cd ../neoui && bun link` once, see the README) and the daemon's part is `daemon/src/view/` (`neoui.ts`: a running `Program` for each doc, `markdown.ts`, `session-view.ts`) and `daemon/src/gateway/` (`registry.ts` for the apps' components, `ui-library.ts` for the text `ui_library` returns).

- **Fences.** A top-level code block whose language is `neoui` becomes a `neoui` block; in lists and quotes, in thoughts and in messages between agents it stays code, and so does ```neoui. A fence still open is `streaming` until it closes; a message that ends (`turn_ended`) with a fence left open finishes it, with the usual errors. A half-written closing fence (`` ` `` or ` `` `) is not part of the program.
- **While streaming** only the element being written is `partial`, and `errors` stay out: what looks broken may be unfinished. Queries do not start, and `neoui.input` and `neoui.press` do nothing.
- **Keys.** See **The wire format**: the element's `id`, else the field's name, else its path.
- **Fields.** A field's `name` is its variable; two fields of one name are one variable. Select, RadioGroup and OptionCards only take one of their options, a CheckBoxGroup a list of its items' names (kept in the items' order) and a SwitchGroup an object of them (an item not mentioned keeps its state; an item's `checked` is where it starts), a Slider is held to `min`..`max`, a CheckBox and a Switch take a bool, and the rest a string; anything else is ignored. A required CheckBox or Switch has to be on. A DatePicker has to be `YYYY-MM-DD`. A field that failed is checked again whenever it changes. Messages are lang-core's carried over ("Please enter a valid email").
- **Buttons.** A button with no behaviour sends its label to the agent; a `FollowUpItem` is such a button whose label is its text. `url` is only set for `http:`, `https:` and `mailto:` addresses. `href` and `ui-copy` on a button that does more do nothing. `disabled` is set while a request the button made is in flight.
- **What the agent hears** is one input per `ui-ask`: `[ui action] The user pressed "Deploy" in a neoui doc: Deploy with these settings`, then `form deploy: {…}` (the form's values; outside a form `state: {…}` with the variables, by name), then `context: {…}`.
- **Tools.** A `<Query>` runs when its doc is finished and draws, and again when what it reads changes, through the gateway (`app.tool`), and is not recorded. An `hx-post` request runs on press and is recorded in the session like a button that runs a tool (so its result shows in the chat with the app's template); `hx-get` on a button is a read and is not recorded. A failing tool is an entry in `errors`; the doc keeps the query's default.
- **App components.** The library the daemon parses with is neo's plus the components of every connector it knows (also closed ones, which are kept like templates, so an old reply still draws), built when a component changes and shared by all sessions; a program asks for it whenever it is set, so an app that connects while a reply streams is used by the next chunk. Every component is in it under `app.Name`, so apps never compete for a name and the order of registration does not matter. Registration (`app.presence`) checks each component on its own (`checkComponents`) and leaves out, with a warning in the log, one whose name is not a PascalCase identifier, whose description is empty, whose params are not typed or put an optional before a required one, which has a `neoui` body instead of `neoui`, or whose body has repairs, does not draw or uses what it does not define; the app connects with the rest. A plain `<BrightnessCard>` that only an app has is an `unknown-component` that says `did you mean photo-editor.BrightnessCard?`; it is never guessed. A use is expanded where it sits: the element is replaced by the root of the body drawn in a program of its own, with the use's attributes as the params, made once per place in the doc (its `id`, or its path) and kept for as long as the place holds a use of that component, with its own fields, queries and keys. Its keys are the use's place and the key inside it (`a/level`), which is what `neoui.input` and `neoui.press` carry; they are routed to that program. A query or request in it goes through the gateway like any other; a query answering patches the item. The body is made of the library alone (an app component cannot call another), which keeps expansion finite. A use whose component is gone, or whose body draws nothing, is a `Callout` saying so, with the error on the doc.
