// A tiny MCP server over stdio, for the tests of the daemon's stdio servers: `echo` returns its
// text with the server's name (argv[2], default "fixture"), `pid` returns the process id (a test
// kills the process to see it start again) and `env` returns an environment variable.
import { Server } from "@modelcontextprotocol/sdk/server/index.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { CallToolRequestSchema, ListToolsRequestSchema } from "@modelcontextprotocol/sdk/types.js";

const name = process.argv[2] ?? "fixture";
const server = new Server(
  { name, version: "1.0.0" },
  { capabilities: { tools: {} }, instructions: `Test server ${name}.` },
);

server.setRequestHandler(ListToolsRequestSchema, () => ({
  tools: [
    {
      name: "echo",
      description: "Repeats the text back",
      inputSchema: {
        type: "object",
        properties: { text: { type: "string" } },
        required: ["text"],
      },
    },
    { name: "pid", description: "The process id of this server", inputSchema: { type: "object" } },
    {
      name: "env",
      description: "The value of an environment variable",
      inputSchema: {
        type: "object",
        properties: { name: { type: "string" } },
        required: ["name"],
      },
    },
  ],
}));

server.setRequestHandler(CallToolRequestSchema, (request) => {
  const args = request.params.arguments ?? {};
  let text = "no such tool";
  if (request.params.name === "echo") text = `${name}: ${String(args.text)}`;
  if (request.params.name === "pid") text = String(process.pid);
  if (request.params.name === "env") text = process.env[String(args.name)] ?? "(unset)";
  return { content: [{ type: "text", text }] };
});

await server.connect(new StdioServerTransport());
