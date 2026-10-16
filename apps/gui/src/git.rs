//! What the side panels ask git: where a session works (repository and worktree), the status of
//! its directory and the diff of one file. Reads only, with no optional locks, since this runs
//! behind the user's back.

use std::path::Path;
use std::process::{Command, Stdio};

use serde_json::{Value, json};
use svelte_gpui::{CommandError, Commands};

/// Lines of context around a change, as in `git diff -U`.
const CONTEXT: u32 = 3;
/// Enough to show a whole file.
const FULL_CONTEXT: u32 = 100_000;

pub fn register(commands: Commands) -> Commands {
    commands
        .command("places", |args| {
            let cwds = args["cwds"].as_array().cloned().unwrap_or_default();
            let places = cwds.iter().filter_map(Value::as_str).map(|cwd| {
                let (repo, worktree) = place_of(cwd);
                // No `worktree` key at all for a main checkout, as the UI's type says.
                let mut place = json!({ "repo": repo });
                if let Some(worktree) = worktree {
                    place["worktree"] = json!(worktree);
                }
                (cwd.to_string(), place)
            });
            Ok::<_, CommandError>(Value::Object(places.collect()))
        })
        .command("git_status", |args| {
            Ok::<_, CommandError>(status(args["cwd"].as_str().unwrap_or("")))
        })
        .command("git_diff", |args| {
            let cwd = args["cwd"].as_str().unwrap_or("");
            let path = args["path"].as_str().unwrap_or("");
            let untracked = args["untracked"].as_bool().unwrap_or(false);
            let full = args["full"].as_bool().unwrap_or(false);
            diff(cwd, path, untracked, full).map(Value::from)
        })
}

/// A `git` command for `cwd`: no optional locks, no pager, literal pathspecs.
fn git(cwd: &str) -> Command {
    let mut command = Command::new("git");
    command
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_PAGER", "cat")
        .arg("--literal-pathspecs")
        .arg("-C")
        .arg(cwd)
        .stdin(Stdio::null())
        .stderr(Stdio::null());
    command
}

fn base(path: &str) -> &str {
    path.trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(path)
}

/// Where a working directory belongs: the repository (or, outside one, the directory itself)
/// and the worktree of it, if it is not the main checkout. The `.worktrees/<name>` convention
/// first, which needs no disk access, then the nearest `.git` above: a directory is a main
/// checkout, a file naming `<repo>/.git/worktrees/<id>` a linked worktree, named by its branch.
pub fn place_of(cwd: &str) -> (String, Option<String>) {
    let cwd = cwd.trim_end_matches('/');
    if let Some(at) = cwd.find("/.worktrees/") {
        let name = cwd[at + "/.worktrees/".len()..]
            .split('/')
            .next()
            .unwrap_or("");
        if !name.is_empty() {
            return (cwd[..at].to_string(), Some(name.to_string()));
        }
    }
    for dir in Path::new(cwd).ancestors() {
        let dot_git = dir.join(".git");
        if dot_git.is_dir() {
            return (dir.to_string_lossy().into_owned(), None);
        }
        if !dot_git.is_file() {
            continue;
        }
        let text = std::fs::read_to_string(&dot_git).unwrap_or_default();
        let gitdir = text.trim().strip_prefix("gitdir:").map(str::trim);
        if let Some(gitdir) = gitdir
            && let Some(at) = gitdir.find("/.git/worktrees/")
        {
            let branch = std::fs::read_to_string(Path::new(gitdir).join("HEAD"))
                .ok()
                .and_then(|head| {
                    head.trim()
                        .strip_prefix("ref: refs/heads/")
                        .map(str::to_string)
                });
            let name = branch.unwrap_or_else(|| base(&dir.to_string_lossy()).to_string());
            return (gitdir[..at].to_string(), Some(name));
        }
        // A submodule or a gitdir elsewhere: its own checkout.
        return (dir.to_string_lossy().into_owned(), None);
    }
    (cwd.to_string(), None)
}

/// `git status --porcelain=v2 --branch -z` for `cwd`, and where `cwd` sits inside the repository
/// (`src/a/`, empty at the root), for the UI to read. Null outside a repository.
pub fn status(cwd: &str) -> Value {
    if cwd.is_empty() {
        return Value::Null;
    }
    let out = git(cwd)
        .args(["status", "--porcelain=v2", "--branch", "-z"])
        .output();
    let Some(out) = out.ok().filter(|out| out.status.success()) else {
        return Value::Null;
    };
    let prefix = git(cwd)
        .args(["rev-parse", "--show-prefix"])
        .output()
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .unwrap_or_default();
    json!({ "out": String::from_utf8_lossy(&out.stdout), "prefix": prefix })
}

/// The unified diff of one file against HEAD (staged and unstaged together), or the whole file
/// as added when it is untracked. `full` keeps the whole file as context.
pub fn diff(cwd: &str, path: &str, untracked: bool, full: bool) -> Result<String, CommandError> {
    if path.ends_with('/') {
        return Err("untracked directory".into());
    }
    let context = format!("-U{}", if full { FULL_CONTEXT } else { CONTEXT });
    let mut command = git(cwd);
    command.args(["diff", "--no-color", "--no-ext-diff", &context]);
    if untracked {
        command.args(["--no-index", "--", "/dev/null"]).arg(path);
    } else {
        let has_head = git(cwd)
            .args(["rev-parse", "--verify", "-q", "HEAD"])
            .output()
            .is_ok_and(|out| out.status.success());
        // Before the first commit there is no HEAD: what is staged is the change.
        command.arg(if has_head { "HEAD" } else { "--cached" });
        command.args(["--", path]);
    }
    let out = command.output().map_err(|error| format!("git: {error}"))?;
    // `--no-index` exits 1 when the files differ.
    let fine = out.status.success() || (untracked && out.status.code() == Some(1));
    if !fine {
        return Err("git diff failed".into());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("suite-gui-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn places_come_from_the_worktrees_convention_and_from_git_files() {
        let root = temp("places");
        let repo = root.join("app");
        let linked = root.join("app-hotfix");
        let admin = repo.join(".git/worktrees/app-hotfix");
        std::fs::create_dir_all(repo.join("src")).unwrap();
        std::fs::create_dir_all(&admin).unwrap();
        std::fs::create_dir_all(&linked).unwrap();
        std::fs::write(admin.join("HEAD"), "ref: refs/heads/hotfix-1\n").unwrap();
        std::fs::write(
            linked.join(".git"),
            format!("gitdir: {}\n", admin.display()),
        )
        .unwrap();
        let (repo_s, linked_s) = (repo.to_string_lossy(), linked.to_string_lossy());

        // A subdirectory belongs to the checkout around it.
        assert_eq!(
            place_of(&format!("{repo_s}/src")),
            (repo_s.to_string(), None)
        );
        // A linked worktree elsewhere groups under its repository, named by its branch.
        assert_eq!(
            place_of(&linked_s),
            (repo_s.to_string(), Some("hotfix-1".to_string()))
        );
        // The convention needs no disk access.
        assert_eq!(
            place_of("/nowhere/proj/.worktrees/fix-login/crates/a"),
            ("/nowhere/proj".to_string(), Some("fix-login".to_string()))
        );
        // Outside any repository: the directory itself.
        assert_eq!(
            place_of("/nowhere/notes/"),
            ("/nowhere/notes".to_string(), None)
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    fn run(dir: &Path, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args([
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .stdout(Stdio::null())
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    }

    #[test]
    fn status_and_diff_of_a_real_repository() {
        let dir = temp("status");
        let path = dir.to_string_lossy().to_string();
        assert_eq!(status("/nonexistent-dir"), Value::Null);
        run(&dir, &["init", "-q"]);
        std::fs::write(dir.join("a.txt"), "one\ntwo\n").unwrap();
        run(&dir, &["add", "a.txt"]);
        run(&dir, &["commit", "-q", "-m", "first"]);
        std::fs::write(dir.join("a.txt"), "one\nthree\n").unwrap();
        std::fs::write(dir.join("new.txt"), "fresh\n").unwrap();

        let status = status(&path);
        let out = status["out"].as_str().unwrap();
        assert!(out.contains("# branch.head"));
        assert!(out.contains("a.txt") && out.contains("new.txt"));
        assert_eq!(status["prefix"], "");

        let changed = diff(&path, "a.txt", false, false).unwrap();
        assert!(changed.contains("-two") && changed.contains("+three"));
        let added = diff(&path, "new.txt", true, false).unwrap();
        assert!(added.contains("+fresh"));
        assert!(diff(&path, "dir/", true, false).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
