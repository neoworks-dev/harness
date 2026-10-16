// Git worktrees for delegate {worktree}: `.worktrees/<name>` in the repository of the session's
// directory, on a branch of the same name. Only git itself is asked (no shell).
import { existsSync, mkdirSync, readFileSync, statSync, appendFileSync } from "node:fs";
import { basename, dirname, join, resolve } from "node:path";

/** Where a delegation works, when that isn't the session's directory. */
export interface Target {
  /** The absolute directory. */
  dir: string;
  /** Set for a worktree: its name and whether it has to be created first. */
  worktree?: { name: string; root: string; create: boolean };
}

const NAME = /^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$/;

/** A worktree name is a plain branch-like word: no slashes, dots-only parts or `.lock`. */
export function validWorktreeName(name: string): boolean {
  return NAME.test(name) && !name.includes("..") && !name.endsWith(".lock") && !name.endsWith(".");
}

async function git(
  cwd: string,
  args: string[],
): Promise<{ ok: boolean; out: string; err: string }> {
  try {
    const proc = Bun.spawn(["git", "-C", cwd, ...args], {
      stdout: "pipe",
      stderr: "pipe",
      env: { ...process.env, GIT_TERMINAL_PROMPT: "0" },
    });
    const [out, err, code] = await Promise.all([
      new Response(proc.stdout).text(),
      new Response(proc.stderr).text(),
      proc.exited,
    ]);
    return { ok: code === 0, out: out.trim(), err: err.trim() };
  } catch (err) {
    return { ok: false, out: "", err: err instanceof Error ? err.message : String(err) };
  }
}

/** The main checkout of the repository `cwd` is in (also from inside one of its worktrees). */
export async function repoRoot(cwd: string): Promise<string | undefined> {
  const found = await git(cwd, ["rev-parse", "--path-format=absolute", "--git-common-dir"]);
  if (!found.ok || found.out === "") return undefined;
  // A bare repository has no checkout to put .worktrees in.
  return basename(found.out) === ".git" ? dirname(found.out) : undefined;
}

/**
 * Where `cwd` / `worktree` of a delegate call point, relative to the session's directory.
 * Returns a message for a directory that can't be used.
 */
export async function resolveTarget(
  sessionCwd: string,
  args: { cwd?: string; worktree?: string },
): Promise<Target | string | undefined> {
  const { cwd, worktree } = args;
  const hasCwd = cwd !== undefined && cwd.trim() !== "";
  const hasWorktree = worktree !== undefined && worktree.trim() !== "";
  if (hasCwd && hasWorktree) return "delegate: give cwd or worktree, not both.";
  if (hasWorktree) {
    if (!validWorktreeName(worktree)) {
      return `delegate: "${worktree}" is not a usable worktree name (letters, digits, . _ - ; no slashes).`;
    }
    const root = await repoRoot(sessionCwd);
    if (root === undefined) {
      return "delegate: worktree needs the session's directory to be in a git repository.";
    }
    const dir = join(root, ".worktrees", worktree);
    if (dir === resolve(sessionCwd)) return undefined;
    return { dir, worktree: { name: worktree, root, create: !isDirectory(dir) } };
  }
  if (!hasCwd) return undefined;
  const dir = resolve(sessionCwd, cwd);
  if (!isDirectory(dir)) return `delegate: ${dir} is not a directory.`;
  return dir === resolve(sessionCwd) ? undefined : { dir };
}

function isDirectory(path: string): boolean {
  try {
    return statSync(path).isDirectory();
  } catch {
    return false;
  }
}

/** `.worktrees/` is the user's scratch space for these: keep it out of `git status`. */
async function keepIgnored(root: string): Promise<void> {
  if ((await git(root, ["check-ignore", "-q", ".worktrees/x"])).ok) return;
  const path = await git(root, [
    "rev-parse",
    "--path-format=absolute",
    "--git-path",
    "info/exclude",
  ]);
  if (!path.ok) return;
  mkdirSync(dirname(path.out), { recursive: true });
  let line = ".worktrees/\n";
  if (existsSync(path.out)) {
    const current = readFileSync(path.out, "utf8");
    if (current !== "" && !current.endsWith("\n")) line = `\n${line}`;
  }
  appendFileSync(path.out, line);
}

/**
 * `git worktree add .worktrees/<name> -b <name>` in the repository; a branch of that name that
 * exists already is checked out instead. Returns git's complaint, or undefined when it worked.
 */
export async function createWorktree(target: Target): Promise<string | undefined> {
  const { worktree } = target;
  if (!worktree?.create) return undefined;
  const { root, name } = worktree;
  await keepIgnored(root);
  const path = join(".worktrees", name);
  const added = await git(root, ["worktree", "add", path, "-b", name]);
  if (added.ok) return undefined;
  // The branch exists: use it (git refuses when it is checked out elsewhere).
  const reused = await git(root, ["worktree", "add", path, name]);
  if (reused.ok) return undefined;
  return `git worktree add ${path} failed: ${reused.err || added.err}`;
}
