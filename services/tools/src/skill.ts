// What the agent reads before it writes a tool of its own (the `writing-tools` skill of this
// service): where the file goes, its shape, and how a neoui doc reads the tool.
import type { SkillSpec } from "@neoworks-dev/neo-sdk";

export function writingTools(dir: string, app: string, project = false): SkillSpec {
  const projectRule = `A project's tool files run only once the user allowed them, each as it is: when you write or change one the user is asked at once ("Run this project's tools?"), and it starts within a second or two of the answer (until then ${app}.status says "not allowed yet"). A change to a file, or to a helper it imports, asks again. Put helpers in a subfolder (lib/), not next to the tool files: every .ts, .js or .mjs file in the folder itself is loaded as a tool.`;
  const where = project
    ? `A tool of this project is a file in ${dir}: only sessions in this project see it. A tool for every project goes in ~/.config/neo/tools instead. ${projectRule}`
    : `A tool of yours is a file in ${dir}: every session sees it. A tool for one project only goes in .neo/tools of that project, and is called as tools-<directory name>-<4 hex digits>.<name> (find the app with tools_search once it runs). ${projectRule.replace(`${app}.status`, "its status tool")}`;
  return {
    name: "writing-tools",
    description:
      "Writing a tool of your own: a file the agent adds to give itself a check or an action it keeps using, or that a neoui doc reads (a progress bar that updates itself)",
    text: `${where} It is loaded when the file is saved, without a restart, and it is called as ${app}.<name>, like any tool: tools_search finds it, tools_call runs it, and a neoui <Query hx-get="${app}.<name>"/> reads it.

Write the file with the Write tool (the user is asked before it is written, and for a project's tools once more to allow it: that is the approval, and it covers every agent that can call the tool afterwards, so write only what you would explain). Then call ${app}.status to see whether it loaded: it lists each file with its tools, or why it gave none.

A file default-exports one tool, or an array of them:

    export default {
      name: "progress",                       // letters, digits and _
      description: "How far a download is: percent of its size on disk. Use it for a progress bar.",
      inputSchema: {
        type: "object",
        properties: { file: { type: "string", description: "Absolute path of the file being downloaded" } },
        required: ["file"],
      },
      async handler({ file }) {
        const have = Bun.file(String(file)).size;
        const want = 1_000_000;
        const percent = Math.min(100, Math.round((have / want) * 100));
        return { content: [{ type: "text", text: \`\${percent}%\` }], structuredContent: { percent } };
      },
    };

- Each file runs in a worker thread of its own, with Bun's and Node's built-ins and fetch; it may import packages that are installed. A call that takes longer than a minute fails and the worker is stopped; so is one that crashes it. The file's variables last between calls, and start over after that or when the file changes. This keeps a bad tool from hanging the others: it is not a sandbox, so a tool must not read or write what it is not for.
- Return structuredContent for what a doc reads (an object: {id.field} in a doc is that object's field), and text for what you read. A thrown error is the tool's error.
- Keep a tool small and narrow: it does one thing and takes what it needs as arguments. The description is how you, and the agents after you, find it, so say when to use it.
- To change a tool, change its file. To remove it, delete the file.

A doc that follows it asks again every few seconds until the work is done:

    <Query id="dl" hx-get="${app}.progress" hx-vals={{ file: "/tmp/big.iso" }}
           hx-trigger="load, every 5s" until={dl.percent >= 100} default={{ percent: 0 }}/>
    <Progress value={dl.percent} label="Downloading"/>

It asks only while the user has the session open, never more often than once a second, and stops when until holds.`,
  };
}
