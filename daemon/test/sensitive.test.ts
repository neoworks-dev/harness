import { expect, test } from "bun:test";
import { mkdir, symlink, writeFile } from "node:fs/promises";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { displayPath, sensitiveInCommand, sensitivePath } from "../src/sensitive.ts";

const home = "/home/me";
const cwd = "/work/app";
const sensitive = (path: string): boolean => sensitivePath(path, cwd, home) !== undefined;

test("sensitive files", () => {
  const yes = [
    ".env",
    ".env.local",
    ".env.production",
    "sub/dir/.env",
    "/work/app/.ENV",
    "../.env",
    "cert.pem",
    "server.key",
    "bundle.p12",
    "bundle.pfx",
    "id_rsa",
    "id_ed25519",
    "id_ecdsa",
    "/tmp/keys/id_rsa_work",
    "~/.ssh/config",
    "~/.ssh/known_hosts",
    "~/.ssh/id_rsa.pub",
    "~/.ssh",
    "/home/me/.ssh/id_ed25519",
    "~/.aws/credentials",
    "~/.gnupg/pubring.kbx",
    "~/.netrc",
    "~/.pi/agent/auth.json",
    "~/.config/gh/hosts.yml",
    "~/.docker/config.json",
    "~/.kube/config",
    "gcp-credentials.json",
    "credentials.json",
    ".npmrc",
    "pkg/.pypirc",
    "secrets.yaml",
    "secrets.json",
  ];
  for (const path of yes) expect([path, sensitive(path)]).toEqual([path, true]);

  const no = [
    ".env.example",
    ".env.sample",
    ".env.template",
    "id_rsa.pub",
    "id_ed25519.pub",
    "src/environment.ts",
    ".environment",
    "env",
    "README.md",
    "keys.ts",
    "monkey.keyboard",
    "~/.aws/config",
    "~/.config/gh/config.yml",
    "~/.docker/daemon.json",
    "credentials.ts",
    "/home/me/.sshx/config",
    "/home/me/projects/.ssh-notes.md",
    "secrets",
    "package.json",
  ];
  for (const path of no) expect([path, sensitive(path)]).toEqual([path, false]);
});

test("paths resolve against the working directory and ~, and links to sensitive files count", async () => {
  expect(sensitivePath(".env", cwd, home)).toBe("/work/app/.env");
  expect(sensitivePath("../app/.env", cwd, home)).toBe("/work/app/.env");
  expect(sensitivePath("~/.ssh/config", cwd, home)).toBe("/home/me/.ssh/config");

  const dir = mkdtempSync(join(tmpdir(), "suite-sensitive-"));
  await mkdir(join(dir, "home", ".ssh"), { recursive: true });
  await writeFile(join(dir, "home", ".ssh", "config"), "Host x\n");
  await writeFile(join(dir, "notes.txt"), "plain\n");
  await symlink(join(dir, "home", ".ssh", "config"), join(dir, "innocent.txt"));
  await symlink(join(dir, "notes.txt"), join(dir, "link.txt"));
  const real = join(dir, "home", ".ssh", "config");
  expect(sensitivePath("innocent.txt", dir, join(dir, "home"))).toBe(real);
  expect(sensitivePath("link.txt", dir, join(dir, "home"))).toBeUndefined();
});

test("commands: the words that name a sensitive file", () => {
  const found = (command: string): string[] => sensitiveInCommand(command, cwd, home);
  expect(found("cat .env")).toEqual(["/work/app/.env"]);
  expect(found("source .env && node app.js")).toEqual(["/work/app/.env"]);
  expect(found("grep token ~/.ssh/config | head")).toEqual(["/home/me/.ssh/config"]);
  expect(found("cat $HOME/.aws/credentials")).toEqual(["/home/me/.aws/credentials"]);
  expect(found('cat "${HOME}/.netrc"')).toEqual(["/home/me/.netrc"]);
  expect(found("FILE=.env.local; cat $FILE")).toEqual(["/work/app/.env.local"]);
  expect(found("cp key.pem /tmp/ && echo $(cat .env)")).toEqual([
    "/work/app/key.pem",
    "/work/app/.env",
  ]);
  expect(found("cat <.env")).toEqual(["/work/app/.env"]);
  expect(found("cat .env .env")).toEqual(["/work/app/.env"]);
  expect(found("ls -la")).toEqual([]);
  expect(found("cat .env.example id_rsa.pub")).toEqual([]);
  expect(found("echo environment")).toEqual([]);
});

test("paths are shown the way the user says them", () => {
  expect(displayPath("/work/app/.env", cwd, home)).toBe(".env");
  expect(displayPath("/work/app/sub/.env", cwd, home)).toBe("sub/.env");
  expect(displayPath("/home/me/.ssh/config", cwd, home)).toBe("~/.ssh/config");
  expect(displayPath("/etc/secrets.json", cwd, home)).toBe("/etc/secrets.json");
});
