import { expect, test } from "bun:test";
import { execFileSync } from "node:child_process";
import { cp, mkdir, readFile, writeFile } from "node:fs/promises";
import { existsSync } from "node:fs";
import { join } from "node:path";
import { loadMcpServers } from "../src/mcp-config.ts";
import { loadPluginCommands, pluginPrompt } from "../src/plugin-commands.ts";
import {
  addMarketplace,
  installPlugin,
  parseSource,
  PluginError,
  readMarketplaces,
  removeMarketplace,
  resolveInstalled,
  searchPlugins,
  setPluginEnabled,
  uninstallPlugin,
} from "../src/plugin-install.ts";
import { enabledPlugins, readInstalled } from "../src/plugins.ts";
import { enabledLspServers } from "../src/plugin-lsp.ts";
import { loadSkills } from "../src/skills.ts";
import { tempDir } from "./helpers.ts";

/** The message a promise rejects with; fails the test when it resolves. */
async function failure(promise: Promise<unknown>): Promise<string> {
  try {
    await promise;
  } catch (error) {
    return error instanceof Error ? error.message : String(error);
  }
  throw new Error("expected a rejection");
}

const FIXTURE = join(import.meta.dir, "fixtures", "marketplace");

function git(cwd: string, ...args: string[]): string {
  return execFileSync("git", ["-c", "user.name=t", "-c", "user.email=t@t", ...args], {
    cwd,
    encoding: "utf8",
  }).trim();
}

/** A git repository with the fixture marketplace in it. */
async function marketplaceRepo(): Promise<string> {
  const repo = await tempDir();
  await cp(FIXTURE, repo, { recursive: true });
  git(repo, "init", "-q", "-b", "main");
  git(repo, "add", "-A");
  git(repo, "commit", "-q", "-m", "marketplace");
  return repo;
}

test("a marketplace is named as owner/repo, a git URL or a path", () => {
  expect(parseSource("anthropics/claude-plugins-official")).toEqual({
    source: "github",
    repo: "anthropics/claude-plugins-official",
  });
  expect(parseSource("https://example.com/x/y.git")).toEqual({
    source: "git",
    url: "https://example.com/x/y.git",
  });
  expect(parseSource("git@github.com:x/y.git")).toMatchObject({ source: "git" });
  expect(parseSource("./local/market", "/work")).toEqual({
    source: "path",
    path: "/work/local/market",
  });
  expect(parseSource("/abs/market")).toEqual({ source: "path", path: "/abs/market" });
  expect(() => parseSource("  ")).toThrow(PluginError);
});

test("a marketplace is copied from a path or cloned from a URL, and recorded", async () => {
  const dir = await tempDir();
  const added = await addMarketplace(dir, FIXTURE);
  expect(added.record.name).toBe("mini-official");
  expect(added.plugins).toBe(4);
  expect(
    existsSync(join(dir, "marketplaces", "mini-official", ".claude-plugin", "marketplace.json")),
  ).toBe(true);
  expect(readMarketplaces(dir).marketplaces["mini-official"]?.from).toEqual({
    source: "path",
    path: FIXTURE,
  });

  const repo = await marketplaceRepo();
  const other = await tempDir();
  const cloned = await addMarketplace(other, `file://${repo}`);
  expect(cloned.record.commit).toBe(git(repo, "rev-parse", "HEAD"));
  expect(JSON.parse(await readFile(join(other, "marketplaces.json"), "utf8")).version).toBe(1);

  expect(await failure(addMarketplace(dir, join(dir, "nothing-here")))).toContain("doesn't exist");
  await mkdir(join(dir, "empty"));
  expect(await failure(addMarketplace(dir, join(dir, "empty")))).toContain("marketplace.json");
  // The failed ones left nothing behind.
  expect(Object.keys(readMarketplaces(dir).marketplaces)).toEqual(["mini-official"]);
});

test("installing copies the plugin into the cache and writes installed.json the way plugins.ts reads it", async () => {
  const repo = await marketplaceRepo();
  const dir = await tempDir();
  await addMarketplace(dir, `file://${repo}`);
  const commit = git(repo, "rev-parse", "HEAD").slice(0, 12);

  const result = await installPlugin(dir, "frontend-design@mini-official");
  expect(result).toMatchObject({ id: "frontend-design@mini-official", version: commit });
  expect(result.root).toBe(join(dir, "cache", "mini-official", "frontend-design", commit));
  expect(result.parts).toMatchObject({ skills: 1, commands: 0, mcpServers: 0, hooks: false });
  expect(existsSync(join(result.root, "skills", "frontend-design", "SKILL.md"))).toBe(true);

  const file = JSON.parse(await readFile(join(dir, "installed.json"), "utf8"));
  expect(file.version).toBe(1);
  expect(file.plugins["frontend-design@mini-official"]).toMatchObject({
    root: result.root,
    version: commit,
    enabled: true,
    entry: { name: "frontend-design", source: "./plugins/frontend-design" },
  });
  expect(typeof file.plugins["frontend-design@mini-official"].installedAt).toBe("string");
  expect(enabledPlugins(dir).map((p) => p.id)).toEqual(["frontend-design@mini-official"]);

  // The marketplace may be left out when only one has the plugin; renames are followed.
  const serena = await installPlugin(dir, "old-serena");
  expect(serena.id).toBe("serena@mini-official");
  expect(await failure(installPlugin(dir, "nope"))).toContain("no plugin nope");
  expect(await failure(installPlugin(dir, "x@unknown"))).toContain("no marketplace named unknown");
});

test("the skills, commands, MCP servers and language servers of installed plugins load", async () => {
  const dir = await tempDir();
  await addMarketplace(dir, FIXTURE);
  for (const name of ["frontend-design", "commit-commands", "serena", "clangd-lsp"]) {
    await installPlugin(dir, name);
  }

  // Skills: under the user's, with the folder they came from.
  const skills = loadSkills(await tempDir(), {
    bundled: join(dir, "none"),
    user: join(dir, "none"),
    plugins: dir,
  });
  const skill = skills.find((s) => s.name === "frontend-design");
  expect(skill?.description).toContain("visual design");
  expect(skill?.body).toContain("Base directory for this skill:");
  const user = join(dir, "user");
  await mkdir(join(user, "frontend-design"), { recursive: true });
  await writeFile(
    join(user, "frontend-design", "SKILL.md"),
    "---\nname: frontend-design\ndescription: mine\n---\nmine",
  );
  const overridden = loadSkills(await tempDir(), {
    bundled: join(dir, "none"),
    user,
    plugins: dir,
  });
  expect(overridden.find((s) => s.name === "frontend-design")?.description).toBe("mine");

  // Commands: /name, with the description and hint in the menu, and the arguments in the prompt.
  const commands = loadPluginCommands(dir);
  expect(commands.map((c) => c.name)).toEqual(["commit", "commit-push-pr"]);
  expect(commands[0]).toMatchObject({
    full: "commit-commands:commit",
    description: "Create a git commit",
    plugin: "commit-commands@mini-official",
  });
  const prompt = pluginPrompt(commands, "/commit fix the build");
  expect(prompt?.shown).toBe("/commit fix the build");
  expect(prompt?.text).toContain("create a single git commit");
  expect(prompt?.text.endsWith("ARGUMENTS: fix the build")).toBe(true);
  expect(pluginPrompt(commands, "/commit-commands:commit")?.text).not.toContain("ARGUMENTS");
  expect(pluginPrompt(commands, "/unknown x")).toBeUndefined();
  expect(pluginPrompt(commands, "no slash")).toBeUndefined();
  // A name the daemon uses itself is reached with the plugin's name in front.
  const reserved = loadPluginCommands(dir, new Set(["commit"]));
  expect(reserved.map((c) => c.name)).toEqual(["commit-commands:commit", "commit-push-pr"]);

  // MCP: serena's .mcp.json is the bare shape.
  const mcp = loadMcpServers({ plugins: dir, env: {} });
  expect(mcp.servers).toMatchObject([
    { app: "mcp-serena", config: { type: "stdio", command: "uvx" } },
  ]);

  // LSP: from the marketplace entry kept in installed.json.
  const lsp = enabledLspServers(dir);
  expect(lsp).toMatchObject([
    {
      plugin: "clangd-lsp@mini-official",
      name: "clangd",
      command: ["clangd", "--background-index"],
    },
  ]);
  expect(lsp[0]?.extensions[".cpp"]).toBe("cpp");

  // Disabled plugins load nothing.
  await setPluginEnabled(dir, "commit-commands@mini-official", false);
  expect(loadPluginCommands(dir)).toEqual([]);
});

test("$ARGUMENTS and $1… are filled in, and quotes group words", async () => {
  const dir = await tempDir();
  const root = join(dir, "cache", "m", "p", "1");
  await mkdir(join(root, "commands", "sub"), { recursive: true });
  await writeFile(
    join(root, "commands", "greet.md"),
    "---\ndescription: Greet\nargument-hint: <who> <how>\n---\nSay $2 to $1 ($ARGUMENTS) from ${CLAUDE_PLUGIN_ROOT}. $3.",
  );
  await writeFile(join(root, "commands", "sub", "deep.md"), "# Deep command\n\nbody");
  await writeFile(
    join(dir, "installed.json"),
    JSON.stringify({
      version: 1,
      plugins: { "p@m": { root, version: "1", enabled: true, installedAt: "" } },
    }),
  );
  const commands = loadPluginCommands(dir);
  expect(commands.map((c) => c.name)).toEqual(["greet", "sub:deep"]);
  expect(commands[0]?.hint).toBe("<who> <how>");
  expect(commands[1]?.description).toBe("Deep command");
  expect(pluginPrompt(commands, '/greet "big bad" wolf')?.text).toBe(
    `Say wolf to big bad ("big bad" wolf) from ${root}. .`,
  );
});

test("url and git-subdir sources are cloned at the pinned commit", async () => {
  const dir = await tempDir();
  const external = await tempDir();
  await mkdir(join(external, "pkg", "sub", "skills", "pinned"), { recursive: true });
  const skillFile = join(external, "pkg", "sub", "skills", "pinned", "SKILL.md");
  await writeFile(skillFile, "---\nname: pinned\ndescription: First version\n---\nv1");
  git(external, "init", "-q", "-b", "main");
  git(external, "add", "-A");
  git(external, "commit", "-q", "-m", "one");
  const pinned = git(external, "rev-parse", "HEAD");
  await writeFile(skillFile, "---\nname: pinned\ndescription: Second version\n---\nv2");
  git(external, "commit", "-q", "-am", "two");

  const market = await tempDir();
  await mkdir(join(market, ".claude-plugin"));
  await writeFile(
    join(market, ".claude-plugin", "marketplace.json"),
    JSON.stringify({
      name: "ext",
      plugins: [
        {
          name: "sub",
          source: {
            source: "git-subdir",
            url: `file://${external}`,
            path: "pkg/sub",
            ref: "main",
            sha: pinned,
          },
        },
        { name: "whole", source: { source: "url", url: `file://${external}` } },
        {
          name: "sneaky",
          source: { source: "git-subdir", url: `file://${external}`, path: "../..", sha: pinned },
        },
        { name: "packed", source: { source: "npm", package: "x" } },
        { name: "dashy", source: { source: "url", url: "--upload-pack=touch /tmp/pwned" } },
        { name: "badsha", source: { source: "url", url: `file://${external}`, sha: "--foo" } },
        { name: "badref", source: { source: "url", url: `file://${external}`, ref: "--foo" } },
      ],
    }),
  );
  await addMarketplace(dir, market);

  const sub = await installPlugin(dir, "sub@ext");
  expect(sub.version).toBe(pinned.slice(0, 12));
  const skills = loadSkills(await tempDir(), {
    bundled: join(dir, "x"),
    user: join(dir, "x"),
    plugins: dir,
  });
  expect(skills.map((s) => [s.name, s.description])).toEqual([["pinned", "First version"]]);
  expect(existsSync(join(sub.root, ".git"))).toBe(false);

  const whole = await installPlugin(dir, "whole@ext");
  expect(whole.version).toBe(git(external, "rev-parse", "HEAD").slice(0, 12));
  expect(await failure(installPlugin(dir, "sneaky@ext"))).toContain("isn't in");
  expect(await failure(installPlugin(dir, "packed@ext"))).toContain("aren't supported");
  expect(await failure(installPlugin(dir, "dashy@ext"))).toContain("isn't a repository URL");
  expect(await failure(installPlugin(dir, "badsha@ext"))).toContain("isn't a commit");
  expect(await failure(installPlugin(dir, "badref@ext"))).toContain("isn't a branch or tag");
  // Failed installs leave nothing in installed.json.
  expect(Object.keys(readInstalled(dir).plugins).sort()).toEqual(["sub@ext", "whole@ext"]);
});

test("disable, enable, uninstall and remove keep installed.json and the cache in step", async () => {
  const dir = await tempDir();
  await addMarketplace(dir, FIXTURE);
  const first = await installPlugin(dir, "frontend-design@mini-official");
  await installPlugin(dir, "serena@mini-official");

  expect(resolveInstalled(dir, "serena")).toBe("serena@mini-official");
  expect(() => resolveInstalled(dir, "missing")).toThrow("isn't installed");

  await setPluginEnabled(dir, "frontend-design@mini-official", false);
  expect(readInstalled(dir).plugins["frontend-design@mini-official"]?.enabled).toBe(false);
  // Installing again updates it and leaves it off.
  await installPlugin(dir, "frontend-design@mini-official");
  expect(readInstalled(dir).plugins["frontend-design@mini-official"]?.enabled).toBe(false);
  await setPluginEnabled(dir, "frontend-design@mini-official", true);
  expect(enabledPlugins(dir).map((p) => p.name)).toEqual(["frontend-design", "serena"]);

  // Fields the installer doesn't know are kept when it writes.
  const file = JSON.parse(await readFile(join(dir, "installed.json"), "utf8"));
  file.plugins["serena@mini-official"].hooksApproved = true;
  await writeFile(join(dir, "installed.json"), JSON.stringify(file));
  await setPluginEnabled(dir, "serena@mini-official", true);
  expect(
    JSON.parse(await readFile(join(dir, "installed.json"), "utf8")).plugins["serena@mini-official"]
      .hooksApproved,
  ).toBe(true);

  await uninstallPlugin(dir, "frontend-design@mini-official");
  expect(existsSync(first.root)).toBe(false);
  expect(Object.keys(readInstalled(dir).plugins)).toEqual(["serena@mini-official"]);
  expect(await failure(uninstallPlugin(dir, "frontend-design@mini-official"))).toContain(
    "isn't installed",
  );

  expect(await searchPlugins(dir, "lsp")).toMatchObject([{ id: "clangd-lsp@mini-official" }]);
  expect((await searchPlugins(dir, "")).length).toBe(4);

  expect(await removeMarketplace(dir, "mini-official")).toEqual(["serena@mini-official"]);
  expect(readInstalled(dir).plugins).toEqual({});
  expect(existsSync(join(dir, "marketplaces", "mini-official"))).toBe(false);
  expect(readMarketplaces(dir).marketplaces).toEqual({});
  expect(await failure(removeMarketplace(dir, "mini-official"))).toContain("no marketplace");
});
