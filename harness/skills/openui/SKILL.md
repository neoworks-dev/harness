---
name: openui
description: Draw UI in the chat instead of prose. Load it whenever the user asks to show, visualize, chart, compare or get an overview or status of something, whenever an answer has several numbers, a breakdown, a trend or rows of data, and whenever you need several inputs or a choice from the user.
---

# OpenUI

A reply can contain UI: a program in OpenUI Lang, written in a fenced block with the language
`openui`. The app parses it while you write it and draws the result in the chat, with prose
before and after. The user never sees the program.

```openui
root = Card([usage, rows], "Disk usage · /dev/nvme0n1")
usage = Progress(0.72, "used")
rows = Table(["Path", "Size"], [["/home", "310 GB"], ["/var", "42 GB"]])
```

## When to use it

- Use it for what reads better as UI: several numbers (Metric), a trend (LineChart, BarChart,
  Sparkline), a breakdown (BarChart, PieChart, Table), steps with their status, or a form when
  you need several inputs from the user at once. "Show me", "visualize", "how much", "what's
  the status of" are asks for UI.
- Prefer prose for a simple answer, an explanation or a single question. Don't wrap every
  reply in a card.
- Keep a program small: what fits in one screen of a terminal. Several short blocks in one
  reply are fine; each is a program of its own and shares nothing with the others.
- Write the prose around it the way you would without the UI: say what the block shows, and
  don't explain the syntax. Close the fence.

## Layout

Lay a dashboard out the way a page is, not as one long column:

- Key numbers first, as Metrics side by side: `Stack([m1, m2, m3], "row")`. A Metric's value
  is the number, its label says what it is, its delta the change or context ("of 896 GB").
- Then the details as Cards in a `Grid`: a chart next to a table, two breakdowns next to
  each other. The app puts as many side by side as the screen fits.
- A Card's title is its heading. Don't add a Header that repeats it; use Header once, for the
  whole page, or not at all.
- Don't repeat what a component already shows: a Progress draws its percentage, so its label
  says what it measures ("used"), not "92% used".
- Several small cards beat one tall one. Cut long tables to the rows that matter (top 5–8,
  then "other").

## Picking components

- End a reply with `FollowUpBlock` when there are obvious next questions: two to four short
  ones. Choosing one sends its text to you as the user's next message.
- Use `EntityList` with `ListItem`s for things that have a name, a detail and a status or
  value (people, files, services, tickets). Prefer it to a Table with one real column.

## Forms and buttons

- A form answers you with `@ToAssistant("message", context?)`: when the user presses the
  button, you get a message that starts with `[ui action] The user pressed "Label" …`, with the
  form's values as JSON (`form name: {…}`) and the context. Check required fields with rules,
  e.g. `{ required: true, email: true }`; the app does not send the message until they pass.
- A button without an action sends its label to you.
- `@OpenUrl("https://…")` opens a link, `@Copy("text")` copies text; neither involves you.
- `$variables` are state in the app: bind one to an input as its last argument
  (`Select("env", options, null, null, $env)`) and other statements can read it.

## Data from apps

`Query("app.tool", args, defaults)` reads and `Mutation("app.tool", args)` writes through a
connector's tool, by the full name `tools_search` lists (`analytics.revenue`). A query runs when
the block is shown, so use it for data that should stay live, and write literal values for a
result you have already read. Give a query defaults; the block draws with them until the data
arrives.

# The language and the library (suite@1)

OpenUI Lang is a declarative language: one statement per line, `name = Component(args)`.

## Syntax Rules

1. Each statement is on its own line: `identifier = Expression`
2. `root` is the entry point — every program must define `root = Stack(...)`
3. Expressions are: strings ("..."), numbers, booleans (true/false), null, arrays ([...]), objects ({...}), or component calls TypeName(arg1, arg2, ...)
4. Use references for readability: define `name = ...` on one line, then use `name` later
5. EVERY variable (except root) MUST be referenced by at least one other variable. Unreferenced variables are silently dropped and will NOT render. Always include defined variables in their parent's children/items array.
6. Arguments are POSITIONAL (order matters, not names). Write `SomeComp([children], "row", "l")` NOT `SomeComp([children], direction: "row", gap: "l")` — colon syntax is NOT supported and silently breaks
7. Optional arguments can be omitted from the end
8. Declare mutable state with `$varName = defaultValue`. Components marked with `$binding` can read/write these. Undeclared $variables are auto-created with null default.
9. String concatenation: `"text" + $var + "more"`
10. Dot member access: `query.field` reads a field; on arrays it extracts that field from every element
11. Index access: `arr[0]`, `data[index]`
12. Arithmetic operators: +, -, *, /, % (work on numbers; + is string concat when either side is a string)
13. Comparison: ==, !=, >, <, >=, <=
14. Logical: &&, ||, ! (prefix)
15. Ternary: `condition ? valueIfTrue : valueIfFalse`
16. Parentheses for grouping: `(a + b) * c`
- Strings use double quotes with backslash escaping

## Component Signatures

Arguments marked with ? are optional. Sub-components can be inline or referenced; prefer references for better streaming.
Props typed `ActionExpression` accept one @step, or Action([@steps...]) for several. See the Action section for available steps (@Run, @ToAssistant, @OpenUrl, @Set, @Reset, @Copy).
Props marked `$binding<type>` accept a `$variable` reference for two-way binding.

### Layout
Stack(children: Component[], direction?: "column" | "row", gap?: "none" | "s" | "m" | "l", wrap?: boolean, align?: "start" | "center" | "end" | "stretch", justify?: "start" | "center" | "end" | "between") — Lays children out top to bottom, or side by side with direction "row"
Grid(children: Component[], columns?: number) — Lays children out in columns, left to right then down; columns is a maximum, a narrow screen uses fewer (default: as many as fit)
Card(children: Component[], title?: string, tone?: "accent" | "muted" | "success" | "danger", variant?: "card" | "sunk" | "clear") — A framed group with an optional title; tone colors the frame, variant sunk fills it, clear has no frame
Header(title: string, subtitle?: string) — A heading with an optional subtitle
Separator() — A horizontal rule
Tabs(tabs: Tab[]) — Switchable panels; only the selected tab is shown
Tab(label: string, children: Component[]) — One tab of Tabs
Accordion(items: AccordionItem[]) — Collapsible sections
AccordionItem(title: string, children: Component[], open?: boolean) — One collapsible section of Accordion; open starts it expanded
Steps(steps: Step[]) — A sequence of steps with their progress
Step(title: string, detail?: string, status?: "done" | "current" | "pending" | "failed") — One step of Steps

### Content
Text(text: string, tone?: "accent" | "muted" | "success" | "danger", role?: "title" | "body" | "code") — Markdown text; role title is a heading, code is monospace
Code(code: string, language?: string) — A block of code
Table(columns: string[], rows: string | number | boolean | Component[][]) — A table; cells are text, numbers or Tag, Link, Text
List(items: string | number | boolean | Component[], ordered?: boolean) — A bulleted or numbered list of text or components
KeyValue(pairs: KV[]) — Aligned key and value lines
KV(key: string, value: string | number, tone?: "accent" | "muted" | "success" | "danger") — One pair of KeyValue
Tag(label: string, tone?: "accent" | "muted" | "success" | "danger") — A small label chip
EntityList(items: ListItem[]) — Rows of things with a title, a muted subtitle and a trailing value or Tag: people, files, services, tickets
ListItem(title: string, subtitle?: string, trailing?: string | number | Tag) — One row of EntityList; trailing is a value or a Tag
Callout(text: string, tone?: "accent" | "muted" | "success" | "danger", title?: string) — A highlighted note; text is markdown
Metric(label: string, value: string | number, delta?: string, tone?: "accent" | "muted" | "success" | "danger") — A headline number with an optional change
Progress(value: number, label?: string) — A progress bar; value is 0 to 1
Image(src: string, alt?: string) — An image from a URL or asset handle
Link(text: string, url: string) — A link

### Charts
BarChart(labels: string[], series: Series[], horizontal?: boolean, stacked?: boolean) — Bars per label; one Series per bar color
LineChart(labels: string[], series: Series[]) — Lines over labels; one Series per line
AreaChart(labels: string[], series: Series[]) — Filled areas over labels; one Series per area
PieChart(slices: Slice[], donut?: boolean) — Shares of a whole; donut hollows it out
Sparkline(values: number[], label?: string) — A tiny inline trend
Series(name: string, values: number[]) — One data series: a name and a value per label
Slice(label: string, value: number) — One slice of PieChart

### Forms
Form(name: string, children: Component[], buttons?: Buttons) — Inputs submitted together; a button inside checks their rules first and a message carries their values
FormControl(label: string, input: Input | TextArea | Select | RadioGroup | CheckBox | Switch | Slider | DatePicker, hint?: string) — A field of a form: a label over an input, with an optional hint
Input(name: string, placeholder?: string, type?: "text" | "email" | "url" | "number" | "password", rules?: {required?: boolean, email?: boolean, url?: boolean, numeric?: boolean, min?: number, max?: number, minLength?: number, maxLength?: number, pattern?: string}, value?: $binding<string>) — A single-line text field
TextArea(name: string, placeholder?: string, rules?: {required?: boolean, email?: boolean, url?: boolean, numeric?: boolean, min?: number, max?: number, minLength?: number, maxLength?: number, pattern?: string}, value?: $binding<string>) — A multi-line text field
Select(name: string, options: Option[], placeholder?: string, rules?: {required?: boolean, email?: boolean, url?: boolean, numeric?: boolean, min?: number, max?: number, minLength?: number, maxLength?: number, pattern?: string}, value?: $binding<string>) — A dropdown of options
Option(value: string, label?: string) — A choice of Select or RadioGroup; label defaults to the value
RadioGroup(name: string, options: Option[], rules?: {required?: boolean, email?: boolean, url?: boolean, numeric?: boolean, min?: number, max?: number, minLength?: number, maxLength?: number, pattern?: string}, value?: $binding<string>) — One choice out of a few options
CheckBox(name: string, label: string, rules?: {required?: boolean, email?: boolean, url?: boolean, numeric?: boolean, min?: number, max?: number, minLength?: number, maxLength?: number, pattern?: string}, value?: $binding<boolean>) — A checkbox with a label
Switch(name: string, label: string, value?: $binding<boolean>) — An on/off switch with a label
Slider(name: string, min: number, max: number, step?: number, value?: $binding<number>) — A number between min and max
DatePicker(name: string, rules?: {required?: boolean, email?: boolean, url?: boolean, numeric?: boolean, min?: number, max?: number, minLength?: number, maxLength?: number, pattern?: string}, value?: $binding<string>) — A date, YYYY-MM-DD
Button(label: string, action?: ActionExpression, variant?: "primary" | "secondary" | "danger") — A button. Without an action it sends its label to the assistant
Buttons(buttons: Button[]) — A row of buttons
FollowUpBlock(items: FollowUpItem[]) — Suggested next questions as pills at the end of a reply; choosing one sends its text to the assistant
FollowUpItem(text: string) — One suggestion of FollowUpBlock

## Built-in Functions

Data functions prefixed with `@` to distinguish from components. These are the ONLY functions available — do NOT invent new ones.
Use @-prefixed built-in functions (@Count, @Sum, @Avg, @Min, @Max, @Round) on Query results. Do NOT hardcode computed values.

@Count(array: any[]) → number — Returns array length
@First(array: any[]) → any — Returns first element of array
@Last(array: any[]) → any — Returns last element of array
@Sum(numbers: number[]) → number — Sum of numeric array
@Avg(numbers: number[]) → number — Average of numeric array
@Min(numbers: number[]) → number — Minimum value in array
@Max(numbers: number[]) → number — Maximum value in array
@Sort(array: any[], field: string, direction?: "asc" | "desc") → any[] — Sort array by field. Direction: "asc" (default) or "desc"
@Filter(array: any[], field: string, operator: "==" | "!=" | ">" | "<" | ">=" | "<=" | "contains", value: any) → any[] — Filter array by field value
@Round(number: number, decimals?: number) → number — Round to N decimal places (default 0)
@Abs(number: number) → number — Absolute value
@Floor(number: number) → number — Round down to nearest integer
@Ceil(number: number) → number — Round up to nearest integer
@Each(array: any[], varName: string, template: any) — Evaluate template for each element. varName is the loop variable — use it ONLY inside the template expression (inline). Do NOT create a separate statement for the template.

Builtins compose — output of one is input to the next:
`@Count(@Filter(data.rows, "field", "==", "val"))` for KPIs/chart values, `@Round(@Avg(data.rows.score), 1)`, `@Each(data.rows, "item", Comp(item.field))` for per-item rendering.
Array pluck: `data.rows.field` extracts a field from every row → use with @Sum, @Avg, charts, tables.

IMPORTANT @Each rule: The loop variable (e.g. "item") is ONLY available inside the @Each template expression. Always inline the template — do NOT extract it to a separate statement.
CORRECT: `list = @Each(rows, "t", Comp(t.name))`
WRONG: `item = Comp(t.name)` then `list = @Each(rows, "t", item)`, because t is undefined in item.

## Query — Live Data Fetching

Fetch data from available tools. Returns defaults instantly, swaps in real data when it arrives.

```
metrics = Query("tool_name", {arg1: value, arg2: $binding}, {defaultField: 0, defaultData: []}, refreshInterval?)
```

- First arg: tool name (string)
- Second arg: arguments object (may reference $bindings — re-fetches automatically on change)
- Third arg: default data (rendered immediately before fetch resolves)
- Fourth arg (optional): refresh interval in seconds (e.g. 30 for auto-refresh every 30s)
- Use dot access on results: metrics.totalEvents, metrics.data.day (array pluck)
- Query results must use regular identifiers: `metrics = Query(...)`, NOT `$metrics = Query(...)`
- Manual refresh: `Button("Refresh", Action([@Run(query1), @Run(query2)]), "secondary")` — re-fetches the listed queries
- Refresh all queries: create Action with @Run for each query

## Mutation — Write Operations

Execute state-changing tool calls (create, update, delete). Unlike Query (auto-fetches on render), Mutation fires only on button click via Action.

```
result = Mutation("tool_name", {arg1: $binding, arg2: "value"})
```

- First arg: tool name (string)
- Second arg: arguments object (evaluated with current $binding values at click time)
- result.status: "idle" | "loading" | "success" | "error"
- result.data: tool response on success
- result.error: error message on failure
- Mutation results use regular identifiers: `result = Mutation(...)`, NOT `$result`
- Show loading state: `result.status == "loading" ? TextContent("Saving...") : null`

## Action — Button Behavior

An action wires a button click to operations. Pass one @-prefixed step bare, e.g. Button("Ask", @ToAssistant("Tell me more")). Put several steps in Action([...]), e.g. Button("Read", Action([@OpenUrl("https://example.com"), @ToAssistant("Summarize this page")])). Steps execute in order.
Buttons without an action automatically send their label to the assistant (equivalent to @ToAssistant(label)).

Available steps:
- @Run(ref: Query | Mutation) — Execute a Mutation or re-fetch a Query (ref must be a declared Query/Mutation)
- @ToAssistant(message: string, context?: any) — Send a message to the assistant (for conversational buttons like "Tell me more", "Explain this"). The optional context is any value (object, array, string, number) passed to the assistant as hidden data, e.g. @ToAssistant("Show details", {orderId: 42})
- @OpenUrl(url: string) — Navigate to a URL
- @Set(variable: $variable, value: any) — Set a $variable to a specific value
- @Reset(variable: $variable) — Reset $variables to their declared defaults; pass several to reset each (e.g. @Reset($title, $priority) restores $title="" and $priority="medium")
- @Copy(text: string) — Copy text to the clipboard

Example — mutation + refresh + reset (PREFERRED pattern):
```
$binding = "default"
result = Mutation("tool_name", {field: $binding})
data = Query("tool_name", {}, {rows: []})
onSubmit = Action([@Run(result), @Run(data), @Reset($binding)])
```

Example — simple nav:
```
viewBtn = Button("View", @OpenUrl("https://example.com"))
```

- An action can be assigned to a variable or inlined: Button("Go", onSubmit) and Button("Go", Action([...])) both work
- If a @Run(mutation) step fails, remaining steps are skipped (halt on failure)
- @Run(queryRef) re-fetches the query (fire-and-forget, cannot fail)

## Interactive Filters

To let the user filter data with a dropdown:
1. Declare a $variable with a default: `$dateRange = "14"`
2. Create a Select with name, items, and binding: `Select("dateRange", [SelectItem("7", "Last 7 days"), ...], null, null, $dateRange)`
3. Wrap in FormControl for a label: `FormControl("Date Range", Select(...))`
4. Pass $dateRange in Query args: `Query("tool", {dateRange: $dateRange}, {defaults})`
5. When the user changes the Select, $dateRange updates and the Query automatically re-fetches

FILTER WIRING RULE: If a $binding filter is visible in the UI, EVERY relevant Query MUST reference that $binding in its args. Never show a filter dropdown while hardcoding the query args.

Rules for $variables:
- $variables hold simple values (strings or numbers), NOT arrays or objects
- $variables must be bound to a Select/Input component via the value argument (last positional arg) to be interactive
- Queries must use regular identifiers (NOT $variables): `metrics = Query(...)` not `$metrics = Query(...)`
- **Auto-declare**: You do NOT need to explicitly declare $variables. If you use `$foo` without declaring it, the parser auto-creates `$foo = null`. You can still declare explicitly to set a default: `$days = "14"`

## Forms

Simple form — no $bindings needed. Field values are managed internally by the Form via the name prop:
```
contactForm = Form("contact", submitBtn, [nameField, emailField])
nameField = FormControl("Name", Input("name", "Your name", "text", {required: true}))
emailField = FormControl("Email", Input("email", "your@email.com", "email", {required: true, email: true}))
submitBtn = Button("Submit")
```

Use $bindings when you need to read field values elsewhere (in Action context, Query args, or conditionals). They are auto-declared:
```
$role = "engineer"
contactForm = Form("contact", submitBtn, [nameField, emailField, roleField])
nameField = FormControl("Name", Input("name", "Enter your name", "text", {required: true}, $name))
emailField = FormControl("Email", Input("email", "Enter your email", "email", {required: true, email: true}, $email))
roleField = FormControl("Role", Select("role", [SelectItem("engineer", "Engineer"), SelectItem("designer", "Designer"), SelectItem("pm", "PM")], null, {required: true}, $role))
submitBtn = Button("Submit")
```

For form + mutation patterns (create, refresh, reset), see the Action section example above.

IMPORTANT: Always add validation rules to form fields used with Mutations. Use OBJECT syntax: {required: true, email: true, minLength: 8}. The renderer shows error messages automatically and blocks submit when validation fails.

## Data Workflow

When tools are available, follow this workflow:
1. FIRST: Call the most relevant tool to inspect the real data shape before generating code
2. Use Query() for READ operations (data that should stay live) — NEVER hardcode tool results as literal arrays or objects
3. Use Mutation() for WRITE operations (create, update, delete) — triggered by button clicks via @Run(mutationRef)
4. Use the real data from step 1 as condensed Query defaults (3-5 rows) so the UI renders immediately
5. Use @-prefixed builtins (@Count, @Filter, @Sort, @Sum) on Query results for KPIs and aggregations — the runtime evaluates these live on every refresh
6. Hardcoded arrays are ONLY for static display data (labels, options) where no tool exists

WRONG — you called a tool and got data back, but you inlined the results:
```
openCount = 2
item1 = SomeComp("first item title")
item2 = SomeComp("second item title")
list = SomeList([item1, item2])
chart = SomeChart(["A", "B"], [12, 8])
```
This is static — it shows stale data and won't update. Creating item1, item2, item3... manually is ALWAYS wrong when a tool exists.

RIGHT — use Query() for live data, Mutation() for writes, @builtins to derive values:
```
data = Query("tool_name", {}, {rows: []})
openCount = @Count(@Filter(data.rows, "field", "==", "value"))
list = @Each(data.rows, "item", SomeComp(item.title, item.field))
createResult = Mutation("create_tool", {title: $title})
submitBtn = Button("Create", Action([@Run(createResult), @Run(data), @Reset($title)]))
```
Everything derives from the Query — when data refreshes, the entire dashboard updates automatically.

## Hoisting & Streaming (CRITICAL)

openui-lang supports hoisting: a reference can be used BEFORE it is defined. The parser resolves all references after the full input is parsed.

During streaming, the output is re-parsed on every chunk. Undefined references are temporarily unresolved and appear once their definitions stream in. This creates a progressive top-down reveal — structure first, then data fills in.

**Recommended statement order for optimal streaming:**
1. `root = Stack(...)` — UI shell appears immediately
2. $variable declarations — state ready for bindings
3. Query statements — defaults resolve immediately so components render with data
4. Component definitions — fill in with data already available
5. Data values — leaf content last

Always write the root = Stack(...) statement first so the UI shell appears immediately, even before child data has streamed in.

## Examples

A dashboard from numbers you already have: key numbers in a row, details in a grid of cards:

```openui
root = Stack([kpis, details])
kpis = Stack([used, free, largest], "row")
used = Metric("Used", "820 GB", "of 896 GB", "danger")
free = Metric("Free", "72 GB", "8%", "success")
largest = Metric("Largest", "Documents", "355 GB")
details = Grid([where, docs])
where = Card([BarChart(["Documents", ".cache", "Games", "Other"], [Series("GB", [355, 65, 48, 103])], true)], "Where the space goes")
docs = Card([Table(["Path", "Size"], [["neoworks", "210 GB"], ["clients", "45 GB"], ["other", "47 GB"]])], "Documents")
```

A form that asks for several things, and answers you when it is sent:

```openui
root = Form("deploy", [service, email], Buttons([send]))
service = FormControl("Service", Select("service", [Option("api", "API"), Option("web", "Web app")], "Pick one", { required: true }))
email = FormControl("Notify", Input("email", "you@company.com", "email", { required: true, email: true }))
send = Button("Deploy", @ToAssistant("Deploy with these settings"), "primary")
```

## Important Rules
- Choose components that best represent the content (tables for comparisons, charts for trends, forms for input, etc.)

## Final Verification
Before finishing, walk your output and verify:
1. root = Stack(...) is the FIRST line (for optimal streaming).
2. Every referenced name is defined. Every defined name (other than root) is reachable from root.
3. Every Query result is referenced by at least one component.
4. Every $binding appears in at least one component or expression.
5. Every visible filter $binding appears in at least one Query args object.
