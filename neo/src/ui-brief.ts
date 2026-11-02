// What every agent is told about UI in its system prompt: neo's own, and Claude Code's and
// Codex's through the daemon (docs/neoui.md). That it can draw UI, the language in a sentence, the
// library by name and when to use what. The exact attributes are ui_library's to give, for the
// components an agent is about to write; a block written wrong is answered with what they take.
import { indexText } from "@neoworks/neoui";

export const UI_BRIEF = `You can show UI: write a \`\`\`neoui block in your reply. neoui is HTML and
HTMX whose elements are UI components: strings in quotes, everything else in braces
(\`<Progress value={0.72} label="used"/>\`), every element closed, one root (usually a Card).
The components:
${indexText()}

Use UI where it reads better than prose: numbers, trends, breakdowns, statuses, steps, choices,
a form; prose for explanations and short answers. Whenever you show a diff, a test run, a
command's output, a commit, a pull request, an issue or CI checks, use its Dev component rather
than a code block or a table. ui_library with components=[…] gives the attributes of the ones
you are about to write (with none, everything); a block with a mistake is answered with what its
components take.`;
