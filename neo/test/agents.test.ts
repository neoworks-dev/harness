import { expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { mkdir, writeFile } from "node:fs/promises";
import { join } from "node:path";
import {
  agentInfo,
  agentModel,
  agentPrompt,
  agentsPrompt,
  findAgent,
  listAgents,
  loadAgents,
  parseAgent,
  shellIsForLooking,
  toolFilter,
  type AgentDefinition,
} from "../src/agents.ts";
import { createNeoAgent } from "../src/neo-agent.ts";
import { fakeModel, reply, TEST_CONFIG } from "./fake-model.ts";
import { errorMessage, tempDir } from "./helpers.ts";

const FIXTURES = join(import.meta.dir, "fixtures", "agents");
const fixture = (name: string): string => readFileSync(join(FIXTURES, name), "utf8");

const SIMPLE = `---
name: reviewer
description: Reviews a diff for correctness. Use after a change is made.
tools: Read, Grep, Bash
model: sonnet
---
You review diffs.
`;

function definition(extra: Partial<AgentDefinition> = {}): AgentDefinition {
  return {
    name: "x",
    description: "d",
    prompt: "p",
    source: "project",
    path: "/x.md",
    ...extra,
  };
}

test("the issue's own example parses", () => {
  expect(parseAgent(SIMPLE, "file")).toEqual({
    name: "reviewer",
    description: "Reviews a diff for correctness. Use after a change is made.",
    prompt: "You review diffs.",
    tools: ["Read", "Grep", "Bash"],
    model: "sonnet",
  });
});

test("real agents from the official marketplace parse", () => {
  const reviewer = parseAgent(fixture("code-reviewer.md"), "x");
  expect(reviewer).toMatchObject({
    name: "code-reviewer",
    model: "sonnet",
    tools: [
      "Glob",
      "Grep",
      "LS",
      "Read",
      "NotebookRead",
      "WebFetch",
      "TodoWrite",
      "WebSearch",
      "KillShell",
      "BashOutput",
    ],
  });
  expect(reviewer?.description).toStartWith("Reviews code for bugs");
  expect(reviewer?.prompt).toStartWith("You are an expert code reviewer");

  // Tools as a JSON list, a block description with examples.
  const creator = parseAgent(fixture("agent-creator.md"), "x");
  expect(creator?.name).toBe("agent-creator");
  expect(creator?.tools).toEqual(["Write", "Read"]);
  expect(creator?.description).toStartWith("Use this agent when the user asks to");
  expect(creator?.description).not.toContain("<example>");
  expect(creator?.model).toBe("sonnet");

  // A tools list with a subagent permission in it.
  const explore = parseAgent(fixture("explore.md"), "x");
  expect(explore?.tools).toEqual(["Read", "Glob", "Grep", "Bash"]);
  expect(parseAgent(fixture("scan-loader.md"), "x")?.tools).toEqual(["Read"]);
  expect(parseAgent(fixture("math-proof-worker.md"), "x")).toMatchObject({
    tools: ["Read", "Write", "Edit"],
    model: "inherit",
  });
});

test("frontmatter forms: YAML lists, quotes, folded text, colons in a description", () => {
  const text = `---
name: "quoted"
description: >
  First line
  of the text.

  Second paragraph.
tools:
  - Read
  - "Bash"
disallowedTools: ["Write"]
model: 'openrouter/deepseek/x'
---
Body`;
  expect(parseAgent(text, "f")).toEqual({
    name: "quoted",
    description: "First line of the text. Second paragraph.",
    prompt: "Body",
    tools: ["Read", "Bash"],
    disallowedTools: ["Write"],
    model: "openrouter/deepseek/x",
  });
  const colon = "---\ndescription: Use when: the user asks, e.g. this\n---\nBody";
  expect(parseAgent(colon, "file-name")).toMatchObject({
    name: "file-name",
    description: "Use when: the user asks, e.g. this",
  });
  const escaped = '---\ndescription: "One.\\n\\nTwo."\n---\nBody';
  expect(parseAgent(escaped, "f")?.description).toBe("One. Two.");
  const parens = "---\ndescription: d\ntools: Read, Agent(a:b, c:d), Bash\n---\nBody";
  expect(parseAgent(parens, "f")?.tools).toEqual(["Read", "Agent(a:b, c:d)", "Bash"]);
});

test("a file without frontmatter, description or body is not an agent", () => {
  expect(parseAgent("Just text", "f")).toBeUndefined();
  expect(parseAgent("---\nname: a\n---\nBody", "f")).toBeUndefined();
  expect(parseAgent("---\ndescription: d\n---\n", "f")).toBeUndefined();
});

async function setup(): Promise<{
  cwd: string;
  user: string;
  plugins: string;
  put: (dir: string, file: string, name: string, description: string) => Promise<void>;
}> {
  const base = await tempDir();
  const cwd = join(base, "project");
  const user = join(base, "user-agents");
  const plugins = join(base, "plugins");
  await mkdir(plugins, { recursive: true });
  const put = async (
    dir: string,
    file: string,
    name: string,
    description: string,
  ): Promise<void> => {
    await mkdir(dir, { recursive: true });
    await writeFile(
      join(dir, file),
      `---\nname: ${name}\ndescription: ${description}\n---\nBody of ${description}`,
    );
  };
  return { cwd, user, plugins, put };
}

async function install(plugins: string, id: string): Promise<string> {
  const root = join(plugins, "cache", id);
  const file = join(plugins, "installed.json");
  let installed: { version: 1; plugins: Record<string, unknown> } = { version: 1, plugins: {} };
  try {
    installed = JSON.parse(readFileSync(file, "utf8"));
  } catch {
    // the first plugin
  }
  installed.plugins[id] = { root, version: "1", installedAt: "t" };
  await writeFile(file, JSON.stringify(installed));
  return root;
}

test("the project's agent wins over the user's, which wins over a plugin's", async () => {
  const { cwd, user, plugins, put } = await setup();
  const root = await install(plugins, "tools@market");
  await put(join(root, "agents"), "a.md", "reviewer", "from the plugin");
  await put(join(root, "agents"), "b.md", "only-plugin", "plugin only");
  await put(user, "a.md", "reviewer", "from the user");
  await put(user, "b.md", "only-user", "user only");
  await put(join(cwd, ".neo", "agents"), "a.md", "reviewer", "from the project");

  const all = loadAgents(cwd, { user, plugins });
  expect(findAgent(all, "reviewer")?.description).toBe("from the project");
  expect(findAgent(all, "Reviewer")?.source).toBe("project");
  expect(listAgents(all).map((a) => [a.name, a.source])).toEqual([
    ["reviewer", "project"],
    ["only-user", "user"],
    ["only-plugin", "plugin"],
  ]);

  // Without the project's, the user's wins; without both, the plugin's. The qualified name
  // reaches the plugin's whatever the others are.
  const noProject = loadAgents(join(cwd, "elsewhere"), { user, plugins });
  expect(findAgent(noProject, "reviewer")?.description).toBe("from the user");
  expect(findAgent(all, "tools:reviewer")?.description).toBe("from the plugin");
  const pluginOnly = loadAgents(join(cwd, "elsewhere"), { plugins });
  expect(findAgent(pluginOnly, "reviewer")).toMatchObject({
    description: "from the plugin",
    plugin: "tools",
  });
  expect(findAgent(all, "nobody")).toBeUndefined();
  expect(findAgent(all, "other:reviewer")).toBeUndefined();
});

test("a disabled plugin's agents are not there", async () => {
  const { cwd, plugins, put } = await setup();
  const root = await install(plugins, "off@market");
  await put(join(root, "agents"), "a.md", "ghost", "never");
  await writeFile(
    join(plugins, "installed.json"),
    JSON.stringify({
      version: 1,
      plugins: { "off@market": { root, version: "1", installedAt: "t", enabled: false } },
    }),
  );
  expect(loadAgents(cwd, { plugins })).toEqual([]);
});

test("model: Claude Code's names are the session's model, anything else is a neo setting", () => {
  for (const model of ["sonnet", "Opus", "haiku", "inherit", "claude-sonnet-4-5", "default"]) {
    expect(agentModel(definition({ model }))).toBeUndefined();
  }
  expect(agentModel(definition())).toBeUndefined();
  expect(agentModel(definition({ model: "openrouter/deepseek/deepseek-v4-flash" }))).toBe(
    "openrouter/deepseek/deepseek-v4-flash",
  );
  expect(agentInfo(definition({ model: "deepseek/deepseek-flash" }))).toEqual({
    name: "x",
    description: "d",
    source: "project",
    model: "deepseek/deepseek-flash",
  });
});

test("tools: Claude Code's names allow neo's tools; none listed means all", () => {
  const all = toolFilter(definition());
  expect(["bash", "read", "write", "edit", "delegate", "skill"].every(all)).toBe(true);

  const reviewer = toolFilter(definition({ tools: ["Read", "Grep", "Bash"] }));
  expect(["read", "bash"].every(reviewer)).toBe(true);
  expect(["write", "edit", "web_fetch"].some(reviewer)).toBe(false);

  const editor = toolFilter(definition({ tools: ["Edit"] }));
  expect(editor("edit") && editor("read")).toBe(true);
  expect(editor("bash")).toBe(false);

  const denied = toolFilter(definition({ disallowedTools: ["Write", "Edit", "Grep"] }));
  expect(denied("write")).toBe(false);
  expect(denied("edit")).toBe(false);
  expect(denied("read")).toBe(true);
  // Grep is bash only for looking; denying it doesn't take bash away.
  expect(denied("bash")).toBe(true);
});

test("an agent whose shell is for searching is told not to change anything with it", () => {
  const reader = definition({ tools: ["Read", "Grep", "Glob"] });
  expect(shellIsForLooking(reader)).toBe(true);
  expect(shellIsForLooking(definition({ tools: ["Read", "Bash"] }))).toBe(false);
  expect(shellIsForLooking(definition({ tools: ["Read"] }))).toBe(false);
  expect(shellIsForLooking(definition())).toBe(false);
  expect(agentPrompt("BASE", reader)).toContain("Do not use it to change anything");
  expect(agentPrompt("BASE", definition({ prompt: "Be brief." }))).toBe(
    'BASE\n\nYou are the "x" agent, working on a task another agent delegated to you. Your instructions:\n\nBe brief.',
  );
});

test("the main agent's prompt lists the agents, and nothing without any", () => {
  expect(agentsPrompt([])).toBe("");
  const text = agentsPrompt([
    definition({ name: "reviewer", description: "Reviews a diff." }),
    definition({ name: "reviewer", description: "Hidden by the first.", source: "user" }),
    definition({ name: "tester", description: "Writes tests." }),
  ]);
  expect(text).toContain("- reviewer: Reviews a diff.\n- tester: Writes tests.");
  expect(text).not.toContain("Hidden");
});

test("a neo session lists the agents in its prompt; a subagent doesn't, and runs as its agent", async () => {
  const cwd = await tempDir();
  const dir = join(cwd, ".neo", "agents");
  await mkdir(dir, { recursive: true });
  await writeFile(
    join(dir, "reviewer.md"),
    "---\nname: reviewer\ndescription: Reviews a diff.\ntools: Read, Grep\n---\nYou review diffs.",
  );
  const model = fakeModel([reply("main"), reply("sub")]);
  const agent = createNeoAgent({ config: TEST_CONFIG, streamFn: model.streamFn });
  expect(agent.agents?.(cwd).map((a) => a.name)).toEqual(["reviewer"]);

  const main = await agent.newSession({ sessionId: "m", cwd, mcpServers: [] });
  await main.prompt({ text: "hi", app: "cli", pendingEvents: [], onUpdate: () => {} });
  expect(model.systems[0]).toContain("- reviewer: Reviews a diff.");
  expect(model.tools[0]).toContain("write");

  const sub = await agent.newSession({
    sessionId: "m-neo-reviewer",
    cwd,
    mcpServers: [],
    subagent: { definition: "reviewer" },
  });
  await sub.prompt({ text: "look", app: "delegate", pendingEvents: [], onUpdate: () => {} });
  expect(model.systems[1]).toContain('You are the "reviewer" agent');
  expect(model.systems[1]).toContain("You review diffs.");
  expect(model.systems[1]).not.toContain("- reviewer: Reviews a diff.");
  expect(model.systems[1]).toContain("Do not use it to change anything");
  expect(model.tools[1]).toEqual(["bash", "read"]);

  const refused = await errorMessage(
    agent.newSession({ sessionId: "x", cwd, mcpServers: [], subagent: { definition: "nobody" } }),
  );
  expect(refused).toBe('There is no agent named "nobody".');
});
