// The openui skill: what neo loads to learn OpenUI Lang and the suite@1 library. Generated from
// the library, so it never goes stale: `bun run --cwd openui skill` writes it, and a test fails
// when the file on disk is not what this produces.
import { generateSystemPrompt } from "@openuidev/lang-core";
import { library } from "./library.ts";

export const SKILL_PATH = "harness/skills/openui/SKILL.md";

const DESCRIPTION =
  "Show a result as UI rather than prose, such as a dashboard, charts, a table, a step list, " +
  "metrics, or a form that asks the user for several inputs at once. Load it before writing " +
  "an openui block in a reply.";

const GUIDANCE = `# OpenUI

A reply can contain UI: a program in OpenUI Lang, written in a fenced block with the language
\`openui\`. The app parses it while you write it and draws the result in the chat, with prose
before and after. The user never sees the program.

\`\`\`openui
root = Card([header, rows], "Disk usage")
header = Header("Disk usage", "/dev/nvme0n1")
rows = Table(["Path", "Size"], [["/home", "310 GB"], ["/var", "42 GB"]])
\`\`\`

## When to use it

- Use it for what reads better as UI: several numbers (Metric), a trend (LineChart, BarChart,
  Sparkline), a table, steps with their status, or a form when you need several inputs from
  the user at once.
- Prefer prose for a simple answer, an explanation or a single question. Don't wrap every
  reply in a card.
- Keep a program small: what fits in one screen of a terminal. Several short blocks in one
  reply are fine; each is a program of its own and shares nothing with the others.
- Write the prose around it the way you would without the UI: say what the block shows, and
  don't explain the syntax. Close the fence.

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
  `A dashboard from numbers you already have:

\`\`\`openui
root = Stack([kpis, trend])
kpis = Stack([Metric("Revenue", "$25k", "+19%", "success"), Metric("Churn", "2.1%", "+0.4pt", "danger")], "row")
trend = BarChart(["Jan", "Feb", "Mar"], [Series("Revenue", [12, 15, 14]), Series("Costs", [9, 9, 10])])
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
