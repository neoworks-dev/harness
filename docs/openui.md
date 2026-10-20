# OpenUI in the suite

[OpenUI](https://github.com/thesysdev/openui) (spec 1.0, `@openuidev/lang-core`) is how the model and apps describe UI beyond the fixed component set: a compact, streamable program, one statement per line, with reactive state, forms and tool queries. The suite uses it as the **authoring** format. The daemon parses and evaluates every program with `lang-core`, and renderers draw the evaluated tree. A renderer never parses OpenUI Lang, evaluates an expression or holds `$state`; it draws `UiNode`s, keeps purely visual state (selected tab, open accordion, cursor, scroll) and sends what the user does back.

The terminal renderer (`renderers/ratatui`, used by `apps/term`) is the first to draw it. web, GPUI and the Go TUI show a placeholder until they follow.

````
model (```openui fence) ─┐
app template (OpenUI Lang)┴─► daemon: lang-core parse → evaluate ($state store, queries) → UiDoc
                                         ▲                                                 │
                 ui.action openui.input / openui.press                       view.snapshot / view.patch
                                         └──────────────── renderer ◄──────────────────────┘
````

## Where programs come from

- **The model** writes a fenced block with the language `openui` in a normal message. The daemon's markdown parser turns that fence into an `OpenUiBlock` (`{type: "openui", ui: UiDoc}`) among the message's other blocks, so it streams like text: every chunk re-parses the program and the block is replaced by the `blocks` op from the first changed block on. `ui.streaming` is true until the message is complete. The model learns the language and the library from the **`openui` skill** (`harness/skills/openui/SKILL.md`, generated from the library by `bun run --cwd openui skill`; a test fails when it's stale).
- **Apps** may register a template that is OpenUI Lang instead of component JSON: `templates: {"<tool>": {"openui": "<program>"}}` in `app.presence`. The tool's `structuredContent` is appended to the program as a last statement `data = <value>` (a later statement wins, so a template can't shadow it), and the result is an `OpenUiComponent` (`{type: "openui", ui: UiDoc}`). A template that fails to produce a root falls back to the generic card, as JSON templates do.

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

| Node                                                                                                              | Added props                                                                                                                                                                                                                                   |
| ----------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `Text`, `Callout`                                                                                                 | `blocks`: the text as parsed markdown (`Block[]` of the view format, never another `openui` block). Draw these, not `text`.                                                                                                                   |
| inputs (`Input`, `TextArea`, `Select`, `RadioGroup`, `OptionCards`, `CheckBox`, `Switch`, `Slider`, `DatePicker`) | `value`: the current value (string, bool or number, see below; absent = empty). `error`: the validation message to show, set after a submit attempt fails or a field is left invalid. `required`: true when the rules say so (draw a marker). |
| `Button`                                                                                                          | `url` when its only action is `@OpenUrl` (the client opens or shows it), `copy` when its only action is `@Copy` (the client copies). Otherwise pressing sends `openui.press`. `disabled` when it can't be pressed now.                        |

The `key` of an interactive node is its statement id, or the statement id plus a path when the node is inline or produced by `@Each` (`row.2.done`). It stays the same while the program re-evaluates.

## What the user does

Both go over the existing `ui.action {session, ref, action, args}`:

- `openui.input {doc, key, value}` is a field's new value: a string for `Input`, `TextArea`, `DatePicker`, `Select`, `RadioGroup`, `OptionCards`; a bool for `CheckBox`, `Switch`; a number for `Slider`. The daemon writes it to the bound `$variable` or form field, re-evaluates and patches the item (`set`). Clients send it as the value changes, debounced (about 150 ms) for typing, and always right before an `openui.press`. While a field has focus the client keeps its own text and cursor and ignores the `value` coming back for that field.
- `openui.press {doc, key}` presses a button. The daemon runs its action plan: `@Set`, `@Reset`, `@Run` (queries and mutations call app tools through the gateway, as `app.tool` actions do), and `@ToAssistant(message, context?)`, which sends the agent an input like a button press does today, with the form's values and the context attached. A button inside a `Form` first validates the form's fields: if any fails, the daemon sets their `error` props and runs nothing.

Nothing is sent while `streaming` is true. State lives in the daemon, per session, item and doc, in memory: a restart keeps the program but forgets typed values.

## The library (`suite@1`)

Arguments are positional in the order listed. Optional ones are marked `?`, and `Node` means a component call. **The order is part of the format**: stored programs carry no prop names, so props are only ever added at the end (spec §3). `Tone` = `accent | muted | success | danger`.

### Layout

| Component       | Props                                                                                                                                                                                                                            |
| --------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `Stack`         | `children: Node[]`, `direction?: "column" \| "row"` (column), `gap?: "none" \| "s" \| "m" \| "l"` (m), `wrap?: bool`, `align?: "start" \| "center" \| "end" \| "stretch"`, `justify?: "start" \| "center" \| "end" \| "between"` |
| `Grid`          | `children: Node[]`, `columns?: number` (a maximum; default: as many as fit)                                                                                                                                                      |
| `Card`          | `children: Node[]`, `title?: string`, `tone?: Tone`, `variant?: "card" \| "sunk" \| "clear"`                                                                                                                                     |
| `Header`        | `title: string`, `subtitle?: string`                                                                                                                                                                                             |
| `Separator`     | (none)                                                                                                                                                                                                                           |
| `Tabs`          | `tabs: Tab[]`                                                                                                                                                                                                                    |
| `Tab`           | `label: string`, `children: Node[]`                                                                                                                                                                                              |
| `Accordion`     | `items: AccordionItem[]`                                                                                                                                                                                                         |
| `AccordionItem` | `title: string`, `children: Node[]`, `open?: bool`                                                                                                                                                                               |
| `Steps`         | `steps: Step[]`                                                                                                                                                                                                                  |
| `Step`          | `title: string`, `detail?: string`, `status?: "done" \| "current" \| "pending" \| "failed"`                                                                                                                                      |

### Content

| Component    | Props                                                                                                      |
| ------------ | ---------------------------------------------------------------------------------------------------------- |
| `Text`       | `text: string` (markdown), `tone?: Tone`, `role?: "title" \| "body" \| "code"`                             |
| `Code`       | `code: string`, `language?: string`                                                                        |
| `Table`      | `columns: string[]`, `rows: (string \| number \| bool \| Node)[][]` (cells may be `Tag`, `Link` or `Text`) |
| `List`       | `items: (string \| Node)[]`, `ordered?: bool`                                                              |
| `KeyValue`   | `pairs: KV[]`                                                                                              |
| `KV`         | `key: string`, `value: string \| number`, `tone?: Tone`                                                    |
| `Tag`        | `label: string`, `tone?: Tone`                                                                             |
| `EntityList` | `items: ListItem[]`                                                                                        |
| `ListItem`   | `title: string`, `subtitle?: string`, `trailing?: string \| number \| Tag`                                 |
| `Callout`    | `text: string` (markdown), `tone?: Tone`, `title?: string`                                                 |
| `Metric`     | `label: string`, `value: string \| number`, `delta?: string`, `tone?: Tone`                                |
| `Progress`   | `value: number` (0..1), `label?: string`                                                                   |
| `Image`      | `src: string`, `alt?: string`                                                                              |
| `Link`       | `text: string`, `url: string`                                                                              |

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
| `Slider`        | `name: string`, `min: number`, `max: number`, `step?: number`, `value?: $var`                                                          |
| `DatePicker`    | `name: string`, `rules?: Rules`, `value?: $var` (`YYYY-MM-DD`)                                                                         |
| `Button`        | `label: string`, `action?: Action`, `variant?: "primary" \| "secondary" \| "danger"`                                                   |
| `Buttons`       | `buttons: Button[]`                                                                                                                    |
| `FollowUpBlock` | `items: FollowUpItem[]` (suggested next questions)                                                                                     |
| `FollowUpItem`  | `text: string` (choosing it sends the text to the assistant)                                                                           |

`Rules` is the spec's validation object (`required`, `email`, `url`, `numeric`, `minLength`, `maxLength`, `min`, `max`, `pattern`). Actions are the spec's built-ins (`@ToAssistant`, `@OpenUrl`, `@Set`, `@Reset`, `@Run`) plus the library's `@Copy(text)`.

## Drawing it in a terminal

The ratatui renderer's rules (the README there is the full reference):

- **Layout.** A `row` Stack measures each child's minimum and preferred width and splits the width among them, wrapping to further lines with `wrap`, and stacking when the children don't fit at their minimum. A `Grid` puts its children in as many columns as fit at a minimum of about 30 cells each (never more than `columns`), left to right then down, every cell of a row as tall as the row's tallest.
- **Auto-layout.** The terminal doesn't draw a page as one long column just because the program says so: in a column Stack (or a Card, Tab, AccordionItem), a run of two or more sibling `Card`s is drawn as a `Grid`, and a run of two or more sibling `Metric`s as a metric strip. A model's layout gets the same treatment as one written with `Grid`.
- **Cards** float: a full rounded outline (`╭─ title ─╮ │ │ ╰─╯`, `tone` colors it) around a fill in the theme's surface color, one cell of padding inside. `sunk` is the fill without the outline, `clear` neither.
- **Entity lists.** An `EntityList` is one row per `ListItem` in columns: the title bold, the subtitle muted, the trailing value or `Tag` right-aligned (numbers line up), `Ada Lovelace  analyst  [admin]`. A subtitle that does not fit is cut down to a minimum, and a list that still does not fit puts each subtitle muted under its title.
- **Metrics.** A metric strip puts each `Metric` in its own column, divided by `│`: the value bold, the `delta` after it in its tone, the label muted under them. A single Metric is the same column on its own.
- **Disclosure** is local: `Tabs` is a tab strip (`│ Overview │ Billing │`, the selected one reversed) over the selected tab's children; `AccordionItem` is `▸ title` / `▾ title`. Both are focusable and toggle with Enter, Left/Right switch tabs.
- **Charts** are drawn by meaning, not by pixels: vertical bars with eighth blocks (`▁▂▃▄▅▆▇█`), horizontal ones half-height (`▄▄▄▖`) so neighbouring bars never touch, value at the end, horizontal whatever `horizontal` says once the labels don't fit under vertical bars; line and area charts on a braille canvas with a legend; pie and donut as one stacked bar of the slices with a `label  42%` legend; sparklines with `▁▂▃▄▅▆▇█`.
- **Option cards.** `OptionCards` draws its options as small framed cards, as many side by side as fit (22 to 34 cells each, a further row when they do not), each `(•) Title` over its muted description; the chosen card has the accent frame. The group is one focus target that works like a `RadioGroup` (Left/Right choose).
- **Follow-ups.** A `FollowUpBlock` is its items as pills, `( Why is it slow? )  ( Show the queries )`, side by side and wrapping, the focused one reversed. They are focusable (Left/Right between them, Enter or Space sends the text) but are not a form: a doc with only follow-ups does not take the editor's place, it is reached with Ctrl-P / Ctrl-N.
- **Forms.** Every input is a focusable field: `Label *` over `[ value or placeholder▏ ]`, the error under it in the danger tone. `Select` shows the chosen option with `▾` and opens a list on Enter; `RadioGroup` `(•)`/`( )`; `CheckBox` `[x]`/`[ ]`; `Switch` `● on`/`○ off`; `Slider` a bar with the number; `DatePicker` a text field checked as `YYYY-MM-DD`.
- **Focus.** A doc with keyed nodes is a form: in `apps/term` it takes the editor's place like a prompt, and Tab / Shift-Tab, typing, Space, Left/Right, Enter and Esc work it (the full key list is under **Forms** in `apps/term/README.md`). Docs with only tabs, accordions or follow-ups are reached with Ctrl-P / Ctrl-N. While `streaming` the doc draws but takes no focus.
- **Errors.** `errors` are not drawn in the chat; a doc with no root but errors shows one muted line `[ui: <first error message>]`.

## How the daemon does it

The code is `openui/` (the `suite@1` library, `Program`: lang-core parse, evaluate and state; `bun run --cwd openui skill` writes the skill) and `daemon/src/view/` (`openui.ts`, `markdown.ts`, `session-view.ts`).

- **Fences.** A top-level code block whose language is `openui` (or `openui-lang`) becomes an `openui` block; in lists and quotes, in thoughts and in messages between agents it stays code. A fence still open is `streaming` until it closes; a message that ends (`turn_ended`) with a fence left open finishes it, with the usual errors. A half-written closing fence (`` ` `` or ` `` `) is not part of the program.
- **While streaming** only the statement being written is `partial`, and `errors` stay out: what looks broken may be unfinished. Queries do not start, and `openui.input` and `openui.press` do nothing.
- **Keys.** An interactive node's `key` is the id of the statement it is in, when that statement has only that one (`service = FormControl("Service", Select("service", …))`: `service`). Otherwise the statement id and the path to the node: `c.buttons.1`, `root.children.2` (a node made by `@Each`).
- **Fields.** An input bound to a `$variable` writes it; any other keeps its value in the doc, per form and name. Select, RadioGroup and OptionCards only take one of their options, a Slider is held to `min`..`max`, a CheckBox and a Switch take a bool, and the rest a string; anything else is ignored. A required CheckBox or Switch has to be on. A DatePicker has to be `YYYY-MM-DD`. A field that failed is checked again whenever it changes. Messages are lang-core's ("Please enter a valid email").
- **Buttons.** A button with no action sends its label to the agent; a `FollowUpItem` is such a button whose label is its text. `url` is only set for `http:`, `https:` and `mailto:` addresses. `@OpenUrl` and `@Copy` inside a plan of several steps do nothing. `disabled` is set while a mutation the button runs is in flight.
- **What the agent hears** is one input per `@ToAssistant`: `[ui action] The user pressed "Deploy" in an OpenUI doc: Deploy with these settings`, then `form deploy: {…}` (the form's values; outside a form `state: {…}` with the `$variables`), then `context: {…}`.
- **Tools.** A `Query` runs when its doc is finished and draws, and after a `@Run` of it, through the gateway (`app.tool`), and is not recorded. A `Mutation` runs on press and is recorded in the session like a button that runs a tool (so its result shows in the chat with the app's template). A failing tool is an entry in `errors`; the doc keeps the query's defaults.
- **State** is kept per view, item and doc, in memory, and a doc is told apart by its item `ref` (`m-<messageId>` for a message) and `id`.
