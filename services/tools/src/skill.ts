// What the agent reads before it writes a tool of its own (the `writing-tools` skill of the user's
// instance): where a file goes, its shape, the rules it runs under, and how to follow long work.
// `{session}` is put in by the gateway's skills_load: the id of the session that loads it.
import type { SkillSpec } from "@neoworks-dev/neo-sdk";

export type Scope = "user" | "project" | "session";

export function writingTools(dir: string, sessions?: string): SkillSpec {
  let sessionLine = "";
  if (sessions !== undefined) {
    sessionLine = `\n- ${sessions}/{session}/ — this session's only, deleted with it; app tools-session-<first 8 characters of the id>. Use it for anything one-off, like following one deploy.`;
  }
  return {
    name: "writing-tools",
    description:
      "Writing a tool of your own: a file that becomes a tool, for a check or action you keep repeating, or for a doc that follows something (a deploy, a training run, a download)",
    text: `A tool file you write becomes a tool like any other: tools_search finds it, tools_call runs it, a neoui <Query hx-get="app.name"/> reads it.

Where it goes decides who sees it:
- ${dir}/ — yours, every session; app tools.
- <project>/.neo/tools/ — one project's, sessions inside it; app tools-<directory>-<4 hex>.${sessionLine}

Every file runs only once the user allowed it as it is. If they allowed your write or edit of it, that counts; otherwise writing or changing one asks them. Once allowed, the app is announced to you a moment later. A change to the file, or to a helper or package in its folder, asks again unless they allowed that change too.

A file default-exports one tool, or an array:

    export default {
      name: "deploy_status",                   // letters, digits and _
      description: "State of a GitHub Actions run and whether it is done. Poll it to follow a deploy.",
      inputSchema: {
        type: "object",
        properties: { repo: { type: "string" }, run: { type: "string" } },
        required: ["repo", "run"],
      },
      async handler({ repo, run }) {
        const r = await Bun.$\`gh run view \${run} -R \${repo} --json status,conclusion,jobs\`.json();
        const finished = r.jobs.filter((j) => j.status === "completed").length;
        return {
          content: [{ type: "text", text: \`\${r.status} \${r.conclusion ?? ""}\` }],
          structuredContent: {
            done: r.status === "completed",
            ok: r.conclusion === "success",
            percent: Math.round((finished / Math.max(r.jobs.length, 1)) * 100),
          },
        };
      },
    };

Rules:
- A file may import built-ins (node:*, bun), files of its own folder, and packages installed in that folder (run bun add in it; nothing from a node_modules above it). Put helpers in a subfolder such as lib/: every .ts, .js or .mjs file at the top is loaded as a tool.
- Each file runs in a worker thread of its own. Module variables, timers and child processes live between calls. A call that takes over a minute fails, but the worker goes on. A worker that crashes or stops answering (a busy loop) is restarted, with its state fresh.
- It is not a sandbox: it runs with the user's rights. Keep a tool to one job, with what it needs as arguments; its description says when to use it.
- Return structuredContent (an object) for what a doc reads, text for what you read. A thrown error is the tool's error.
- Check a new file with <app>.status; change a tool by changing its file, remove it by deleting the file.

Following long work (a deploy, a build, a training run, a download):
- Never wait inside a call. The tool says how the work is now and returns at once; a doc asks it again:

    <Query id="d" hx-get="tools.deploy_status" hx-vals={{ repo: "o/r", run: "4812" }}
           hx-trigger="load, every 10s" until={d.done} default={{ done: false, percent: 0 }}/>
    <Progress value={d.percent} label="Deploy"/>

  It asks only while the user has the session open, at most once a second, and stops when until holds.
- Read what the work already writes: a status command (gh run view, kubectl get), a log or metrics file (Keras CSVLogger, a JSON file the script writes), a file's size against the size expected (pass it in).
- If it writes nothing readable, start it from the tool file (spawn it at the top of the file or from a start tool), keep what its output says in module variables, and return them from a status tool.
- You are not told when the work ends; the doc shows it to the user. Call the tool when you need to know.`,
  };
}
