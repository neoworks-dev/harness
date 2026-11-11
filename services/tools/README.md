# tools service

The tools agents write for themselves (`kind: service`). Each file at the top of a tools folder is a tool, or an array of them, loaded when it is saved and offered to every agent that may see the folder as `<app>.<name>`. The daemon runs one instance per folder (`NEO_SERVICES=none` to skip; each exits when the daemon does); by hand, `bun run services/tools/main.ts` runs `~/.config/neo/tools` with every file allowed.

| Folder                          | App                       | Seen by                        |
| ------------------------------- | ------------------------- | ------------------------------ |
| `~/.config/neo/tools/`          | `tools`                   | every session                  |
| `<project>/.neo/tools/`         | `tools-<name>-<4 hex>`    | sessions inside the project    |
| `<dataDir>/session-tools/<id>/` | `tools-session-<8 chars>` | that session and its subagents |

A session's folder is deleted with the session. `/settings project-tools off` stops the projects' folders.

- **Every file runs only once you allowed it, as it is.** A file is allowed by a hash of its content and of the other files of its folder. A new or changed file asks ("Run your tools?" / "this project's" / "this session's") after the tool call that wrote it, or at the next turn, unless you allowed that write with the whole file in its diff (neo's Write and Edit show it): then it was the question. The daemon writes the hashes you allowed to the `--approved` file; the service watches it and starts or stops a file within a moment.
- **It may import only built-ins, its folder and the packages installed in it.** Helpers go in a subfolder (`lib/`), packages in the folder's own `node_modules` (`bun add` there). Every module a worker loads from the folder is checked against the hash you allowed; packages are covered by `package.json` and the lockfile, which are files of the folder, so what runs is what you approved. A `node_modules` above the folder is refused. What a tool spawns or reads at call time is not covered.
- **A worker thread for each file.** A slow call fails after a minute and the worker goes on, keeping its module state (a watcher, a child process). A worker that stops answering pings for 10 s (a busy loop) or crashes is restarted with fresh state. It is not a sandbox.
- **Reloaded on save** (250 ms after the last change). A file that did not change keeps its worker; the tool list is replaced with a `tools/list_changed`.
- **One failure per file.** A file that does not load, has no default export or has an invalid tool gives no tools and an entry in `status`. A name already taken, or `status`, is left out. A schema that is not an object's is refused.

```ts
// a session's folder: follow one deploy
export default {
  name: "deploy_status",
  description: "State of a GitHub Actions run and whether it is done. Poll it to follow a deploy.",
  inputSchema: {
    type: "object",
    properties: { repo: { type: "string" }, run: { type: "string" } },
    required: ["repo", "run"],
  },
  async handler({ repo, run }) {
    const r = await Bun.$`gh run view ${run} -R ${repo} --json status,conclusion`.json();
    return {
      content: [{ type: "text", text: r.status }],
      structuredContent: { done: r.status === "completed" },
    };
  },
};
```

A neoui doc follows it, asking every few seconds while someone has the session open, until the work is done:

```html
<Query id="d" hx-get="tools-session-1a2b3c4d.deploy_status" hx-vals={{ repo: "o/r", run: "4812" }}
       hx-trigger="load, every 10s" until={d.done} default={{ done: false }}/>
```

The `writing-tools` skill of `tools` tells the agent all of this; `skills_load` fills in the session's own folder.
