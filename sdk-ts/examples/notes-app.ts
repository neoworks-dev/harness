// A tiny connector: a notes app with two tools, registered with the suite until Ctrl-C.
//   bun run sdk-ts/examples/notes-app.ts
// Then ask the agent (e.g. `suite attach --new`) to use the notes app.
import { SuiteClient } from "@suite/sdk";
import { createToolServer } from "@suite/sdk/tools";

const notes: string[] = [];

const tools = createToolServer({
  name: "notes",
  instructions: "Notes are short lines of text, newest last.",
  tools: [
    {
      name: "add_note",
      description: "Add a note",
      inputSchema: {
        type: "object",
        properties: { text: { type: "string", description: "The note" } },
        required: ["text"],
      },
      handler: (args) => {
        notes.push(String(args.text));
        return Promise.resolve(`Added note #${notes.length}.`);
      },
    },
    {
      name: "list_notes",
      description: "List all notes",
      inputSchema: { type: "object", properties: {} },
      outputSchema: {
        type: "object",
        properties: { notes: { type: "array", items: { type: "string" } } },
        required: ["notes"],
      },
      handler: () =>
        Promise.resolve({
          content: [
            { type: "text", text: notes.map((n, i) => `${i + 1}. ${n}`).join("\n") || "(none)" },
          ],
          structuredContent: { notes },
        }),
    },
  ],
});

const suite = await SuiteClient.connect({
  client: "notes",
  waitForDaemon: true,
  presence: { app: "notes", kind: "app", title: "Notes", open: true, mcp: tools.endpoint },
});
process.stderr.write("notes app registered; Ctrl-C to quit\n");

process.on("SIGINT", () => {
  void suite
    .close()
    .then(() => tools.close())
    .then(() => process.exit(0));
});
