# tools service

The tools the agent wrote for itself (`kind: service`). Each file in `~/.config/neo/tools/` (`NEO_TOOLS_DIR` to move it) is a tool, or an array of them, loaded when the file is saved and offered to every agent as `tools.<name>`. The daemon starts it for every kind of work (`NEO_SERVICES=none` to skip; it exits when the daemon does); by hand: `bun run services/tools/main.ts`.

**Two scopes.** The files above are yours, for every project. A project can bring its own in `<project>/.neo/tools/`: the daemon runs another instance for it (`--dir <folder> --app tools-<name>-<hash> --approved <file>`), only for sessions inside that project, and each file only as you allowed it.

- **By content.** A file is allowed by a hash of its content and of what it may import: everything under the folder that is not a tool file itself (put helpers in a `lib/` subfolder; `node_modules` is not covered). A new file, or one that changed, asks "Run this project's tools?" with the files listed, a changed one marked; an allowed file that did not change is not asked about again, and a refused one is not asked about again until it changes.
- **When.** At the start of a session's turn in the project, and after every completed tool call, so a file the agent writes asks at once.
- **At once.** The daemon writes the hashes of the allowed files to the `--approved` file; the service watches it and the folder, and runs a file only while its hash is there. A file allowed mid-session starts within a moment, one that changed stops (`status` says "changed since the user approved it"), with no restart.
- **Off.** `/settings project-tools off` (on by default): nothing asks and every project instance stops.

```ts
// ~/.config/neo/tools/progress.ts
export default {
  name: "progress",
  description: "How far a download is, in percent of its size on disk",
  inputSchema: { type: "object", properties: { file: { type: "string" } }, required: ["file"] },
  async handler({ file }) {
    const percent = Math.min(100, Math.round((Bun.file(String(file)).size / 1_000_000) * 100));
    return { content: [{ type: "text", text: `${percent}%` }], structuredContent: { percent } };
  },
};
```

A neoui doc reads it and follows it, asking every few seconds until the work is done:

```html
<Query id="dl" hx-get="tools.progress" hx-vals={{ file: "/tmp/big.iso" }}
       hx-trigger="load, every 5s" until={dl.percent >= 100} default={{ percent: 0 }}/>
<Progress value={dl.percent} label="Downloading"/>
```

| Tool     | Does                                                           |
| -------- | -------------------------------------------------------------- |
| `status` | Lists the files, the tools each gave, and why a file gave none |
| the rest | Whatever the files define                                      |

The `writing-tools` skill tells the agent the format; it loads it with `skills_load` before it writes a file.

How it behaves:

- **One file, one failure.** A file that does not load, has no default export, or has an invalid tool (a name that is not letters, digits and `_`, no description, no schema, no handler) gives no tools and an entry in `status`; the others load. A name already taken by another file, or by `status`, is left out.
- **A worker thread for each file.** A tool that spins, or throws where nobody catches it, costs its file and nothing else. A call that takes longer than a minute fails and its worker is stopped (`terminate`, which ends a busy loop too); one that crashes its worker fails that call. The next call starts a new worker, so a file's variables last between calls and start over after a stop.
- **Reloaded on save.** Files are watched (250 ms after the last change). A changed or new file gets a new worker, a file that did not change keeps its worker and its state, a removed one loses it, and the tool list is replaced with a `tools/list_changed` to the gateway.
- **Approval is the write.** The agent writes the file with its Write tool, which asks the user. A tool then runs with the service's rights, for every agent that can call it, so the file is code the user approved, not something a doc or a subagent can create.

Workers keep a bad tool from hanging or crashing the rest; they are not a sandbox: a worker has the service's filesystem and network. There is no `delete` tool, a file is removed with Bash or by the user.
