// A tiny connector: a notes app with two tools, registered with neo until Ctrl-C.
//   bun run sdk-ts/examples/notes-app.ts
// Then ask the agent (e.g. `neo attach --new`) to use the notes app. Its results are drawn with
// the templates below (buttons included) in every pane.
import { connect } from "@neoworks-dev/neo-sdk";
import { daemonJsonAddress } from "@neoworks-dev/neo-sdk";
import { createToolServer } from "@neoworks-dev/neo-sdk";

const notes: string[] = [];

const tools = await createToolServer({
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
        const text = String(args.text);
        notes.push(text);
        return Promise.resolve({
          content: [{ type: "text", text: `Added note #${notes.length}.` }],
          structuredContent: { text, total: notes.length },
        });
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
    {
      name: "clear_notes",
      description: "Delete all notes",
      inputSchema: { type: "object", properties: {} },
      handler: () => {
        const removed = notes.splice(0).length;
        return Promise.resolve({
          content: [{ type: "text", text: `Removed ${removed} notes.` }],
          structuredContent: { removed },
        });
      },
    },
  ],
});

const buttons = {
  type: "row",
  children: [
    { type: "button", label: "Show all", action: "notes.list_notes", key: "l" },
    { type: "button", label: "Clear", action: "notes.clear_notes", key: "x" },
  ],
};

const templates = {
  add_note: {
    type: "card",
    title: "Note #{{data.total}}",
    children: [
      { type: "text", markdown: "{{data.text}}" },
      {
        type: "row",
        children: [
          ...buttons.children,
          { type: "button", label: "Copy", action: "copy:{{data.text}}", key: "c" },
        ],
      },
    ],
  },
  list_notes: {
    type: "card",
    title: "Notes",
    children: [{ type: "text", markdown: "- {{item}}", each: "data.notes", as: "item" }, buttons],
  },
  clear_notes: { type: "text", markdown: "Removed **{{data.removed}}** notes.", tone: "muted" },
};

const neo = await connect({
  client: "notes",
  address: daemonJsonAddress,
  waitForDaemon: true,
  presence: {
    app: "notes",
    kind: "app",
    title: "Notes",
    description: "Notes: short lines of text the user keeps.",
    open: true,
    mcp: tools.endpoint,
    templates,
  },
});
process.stderr.write("notes app registered; Ctrl-C to quit\n");

process.on("SIGINT", () => {
  void neo
    .close()
    .then(() => tools.close())
    .then(() => process.exit(0));
});
