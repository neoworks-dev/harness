// The openui skill: what neo loads to learn OpenUI Lang and the suite@1 library. Generated from
// the library, so it never goes stale: `bun run --cwd openui skill` writes it, and a test fails
// when the file on disk is not what this produces.
import { generateSystemPrompt } from "@openuidev/lang-core";
import { library } from "./library.ts";

export const SKILL_PATH = "harness/skills/openui/SKILL.md";

const DESCRIPTION =
  "Draw UI in the chat instead of prose. Load it whenever the user asks to show, visualize, " +
  "chart, compare or get an overview or status of something, whenever an answer has several " +
  "numbers, a breakdown, a trend or rows of data, and whenever you need several inputs or a " +
  "choice from the user.";

const GUIDANCE = `# OpenUI

A reply can contain UI: a program in OpenUI Lang, written in a fenced block with the language
\`openui\`. The app parses it while you write it and draws the result in the chat, with prose
before and after. The user never sees the program.

\`\`\`openui
root = Card([usage, rows], "Disk usage · /dev/nvme0n1")
usage = Progress(0.72, "used")
rows = Table(["Path", "Size"], [["/home", "310 GB"], ["/var", "42 GB"]])
\`\`\`

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

- Key numbers first, as Metrics side by side: \`Stack([m1, m2, m3], "row")\`. A Metric's value
  is the number, its label says what it is, its delta the change or context ("of 896 GB").
- Then the details as Cards in a \`Grid\`: a chart next to a table, two breakdowns next to
  each other. The app puts as many side by side as the screen fits.
- A Card's title is its heading. Don't add a Header that repeats it; use Header once, for the
  whole page, or not at all.
- Don't repeat what a component already shows: a Progress draws its percentage, so its label
  says what it measures ("used"), not "92% used".
- Several small cards beat one tall one. Cut long tables to the rows that matter (top 5–8,
  then "other").

## Picking components

- End a reply with \`FollowUpBlock\` when there are obvious next questions: two to four short
  ones. Choosing one sends its text to you as the user's next message.

## Forms and buttons

- A form answers you with \`@ToAssistant("message", context?)\`: when the user presses the
  button, you get a message that starts with \`[ui action] The user pressed "Label" …\`, with the
  form's values as JSON (\`form name: {…}\`) and the context. Check required fields with rules,
  e.g. \`{ required: true, email: true }\`; the app does not send the message until they pass.
- A button without an action sends its label to you.
- \`@OpenUrl("https://…")\` opens a link, \`@Copy("text")\` copies text; neither involves you.
- \`$variables\` are state in the app: bind one to an input as its last argument
  (\`Select("env", options, null, null, $env)\`) and other statements can read it.

## Data from apps

\`Query("app.tool", args, defaults)\` reads and \`Mutation("app.tool", args)\` writes through a
connector's tool, by the full name \`tools_search\` lists (\`analytics.revenue\`). A query runs when
the block is shown, so use it for data that should stay live, and write literal values for a
result you have already read. Give a query defaults; the block draws with them until the data
arrives.

# The language and the library (suite@1)

`;

const EXAMPLES = [
  `A dashboard from numbers you already have: key numbers in a row, details in a grid of cards:

\`\`\`openui
root = Stack([kpis, details])
kpis = Stack([used, free, largest], "row")
used = Metric("Used", "820 GB", "of 896 GB", "danger")
free = Metric("Free", "72 GB", "8%", "success")
largest = Metric("Largest", "Documents", "355 GB")
details = Grid([where, docs])
where = Card([BarChart(["Documents", ".cache", "Games", "Other"], [Series("GB", [355, 65, 48, 103])], true)], "Where the space goes")
docs = Card([Table(["Path", "Size"], [["neoworks", "210 GB"], ["clients", "45 GB"], ["other", "47 GB"]])], "Documents")
\`\`\``,
  `A form that asks for several things, and answers you when it is sent:

\`\`\`openui
root = Form("deploy", [service, email], Buttons([send]))
service = FormControl("Service", Select("service", [Option("api", "API"), Option("web", "Web app")], "Pick one", { required: true }))
email = FormControl("Notify", Input("email", "you@company.com", "email", { required: true, email: true }))
send = Button("Deploy", @ToAssistant("Deploy with these settings"), "primary")
\`\`\``,
];

/** The text of SKILL.md. */
export function skillMarkdown(): string {
  const reference = generateSystemPrompt({
    library: library.toSpec(),
    promptOptions: {
      preamble:
        "OpenUI Lang is a declarative language: one statement per line, `name = Component(args)`.",
      toolCalls: true,
      bindings: true,
      examples: EXAMPLES,
    },
  });
  return `---\nname: openui\ndescription: ${DESCRIPTION}\n---\n\n${GUIDANCE}${reference}\n`;
}
