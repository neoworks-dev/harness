//! What the workspace card asks of git, and the little it changes: where the session's directory
//! stands (branch, upstream, what changed), the local branches, switching branch and committing
//! all changes, with a push. Unlike `git.rs` this writes, so every failure carries git's own
//! words, shortened, for the card to show.

use std::io::Read;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use svelte_gpui::{CommandError, Commands};

use crate::git;

/// A push waits this long for the remote; one that asks for a password nobody can type ends here.
const PUSH_LIMIT: Duration = Duration::from_secs(60);
/// Untracked files are counted as added lines up to these limits (a build folder that was not
/// ignored must not stall the card).
const UNTRACKED_FILES: usize = 300;
const UNTRACKED_BYTES: u64 = 2_000_000;
/// The most untracked files shown in the diff of everything.
const DIFF_UNTRACKED_FILES: usize = 50;

pub fn register(commands: Commands) -> Commands {
    commands
        .command("git_workspace", |args| {
            Ok::<_, CommandError>(overview(args["cwd"].as_str().unwrap_or("")))
        })
        .command("git_branches", |args| {
            branches(args["cwd"].as_str().unwrap_or(""))
        })
        .command("git_checkout", |args| {
            let cwd = args["cwd"].as_str().unwrap_or("");
            checkout(cwd, args["branch"].as_str().unwrap_or("")).map(|()| Value::Null)
        })
        .command("git_commit", |args| {
            let cwd = args["cwd"].as_str().unwrap_or("");
            let message = args["message"].as_str().unwrap_or("");
            commit(cwd, message, args["push"].as_bool().unwrap_or(false))
        })
        .command("git_diff_all", |args| {
            diff_all(args["cwd"].as_str().unwrap_or("")).map(Value::from)
        })
}

/// A `git` command for `cwd` that never waits for the terminal.
fn git_in(cwd: &str) -> Command {
    let mut command = Command::new("git");
    command
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_PAGER", "cat")
        .arg("-C")
        .arg(cwd)
        .stdin(Stdio::null());
    command
}

/// Git's complaint in a line: the first `fatal:` or `error:` (without the label), else the first
/// line there is, cut to a length the card can show.
fn brief(stderr: &str) -> String {
    let lines = || {
        stderr
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
    };
    let line = lines()
        .find_map(|line| {
            line.strip_prefix("fatal: ")
                .or_else(|| line.strip_prefix("error: "))
        })
        .or_else(|| lines().next())
        .unwrap_or("git failed");
    let mut short: String = line.chars().take(200).collect();
    if short.len() < line.len() {
        short.push('…');
    }
    short
}

/// Runs `git args` in `cwd` and answers its output, or git's complaint.
fn run(cwd: &str, args: &[&str]) -> Result<String, String> {
    let out = git_in(cwd)
        .args(args)
        .output()
        .map_err(|error| format!("git: {error}"))?;
    finish(out)
}

fn finish(out: Output) -> Result<String, String> {
    if out.status.success() {
        return Ok(String::from_utf8_lossy(&out.stdout).into_owned());
    }
    Err(brief(&String::from_utf8_lossy(&out.stderr)))
}

/// Like `run`, but gives up after `limit`.
fn run_limited(cwd: &str, args: &[&str], limit: Duration) -> Result<String, String> {
    let mut child = git_in(cwd)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("git: {error}"))?;
    let drain = |pipe: Option<Box<dyn Read + Send>>| {
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            if let Some(mut pipe) = pipe {
                let _ = pipe.read_to_end(&mut bytes);
            }
            bytes
        })
    };
    let stdout = drain(child.stdout.take().map(|p| Box::new(p) as _));
    let stderr = drain(child.stderr.take().map(|p| Box::new(p) as _));
    let deadline = Instant::now() + limit;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(30)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("git did not answer in time (is it waiting for a password?)".into());
            }
            Err(error) => return Err(format!("git: {error}")),
        }
    };
    let out = Output {
        status,
        stdout: stdout.join().unwrap_or_default(),
        stderr: stderr.join().unwrap_or_default(),
    };
    finish(out)
}

/// Where `HEAD` stands, from `git status --porcelain=v2 --branch`.
#[derive(Debug, PartialEq, Eq)]
struct Head {
    /// The branch checked out; none when detached.
    branch: Option<String>,
    upstream: Option<String>,
    ahead: u32,
    /// Anything changed, untracked files included.
    dirty: bool,
}

fn parse_head(out: &str) -> Head {
    let mut head = Head {
        branch: None,
        upstream: None,
        ahead: 0,
        dirty: false,
    };
    for line in out.lines() {
        let Some(rest) = line.strip_prefix("# ") else {
            head.dirty = true;
            continue;
        };
        if let Some(name) = rest.strip_prefix("branch.head ") {
            head.branch = (name != "(detached)").then(|| name.to_string());
        } else if let Some(name) = rest.strip_prefix("branch.upstream ") {
            head.upstream = Some(name.to_string());
        } else if let Some(counts) = rest.strip_prefix("branch.ab ") {
            let plus = counts.split(' ').find_map(|part| part.strip_prefix('+'));
            head.ahead = plus.and_then(|n| n.parse().ok()).unwrap_or(0);
        }
    }
    head
}

fn head_of(cwd: &str) -> Result<Head, String> {
    let out = run(cwd, &["status", "--porcelain=v2", "--branch"])?;
    Ok(parse_head(&out))
}

/// The top of the repository `cwd` is in.
fn toplevel(cwd: &str) -> Option<String> {
    let out = run(cwd, &["rev-parse", "--show-toplevel"]).ok()?;
    let top = out.trim();
    (!top.is_empty()).then(|| top.to_string())
}

/// Lines in a text file, or none for a binary one, a big one or one that cannot be read.
fn count_lines(path: &Path) -> Option<u32> {
    let meta = path.metadata().ok()?;
    if !meta.is_file() || meta.len() > UNTRACKED_BYTES {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    if bytes.iter().take(8000).any(|&byte| byte == 0) {
        return None;
    }
    let newlines = bytes.iter().filter(|&&byte| byte == b'\n').count();
    let unterminated = bytes.last().is_some_and(|&last| last != b'\n');
    Some((newlines + usize::from(unterminated)) as u32)
}

/// Where the directory stands, for the card: the branch, its upstream, whether anything changed
/// and what: `git diff --numstat` against HEAD (text, summed by the UI) and the line counts of
/// the untracked text files. Null outside a repository.
pub fn overview(cwd: &str) -> Value {
    if cwd.is_empty() {
        return Value::Null;
    }
    let Some(top) = toplevel(cwd) else {
        return Value::Null;
    };
    let Ok(head) = head_of(&top) else {
        return Value::Null;
    };
    let has_head = run(&top, &["rev-parse", "--verify", "-q", "HEAD"]).is_ok();
    // Before the first commit there is no HEAD: what is staged is the change.
    let against = if has_head { "HEAD" } else { "--cached" };
    let numstat = run(
        &top,
        &[
            "diff",
            "--numstat",
            "--no-renames",
            "--no-ext-diff",
            against,
        ],
    )
    .unwrap_or_default();
    let untracked: Vec<u32> = untracked_files(&top)
        .iter()
        .take(UNTRACKED_FILES)
        .map(|file| count_lines(&Path::new(&top).join(file)).unwrap_or(0))
        .collect();
    json!({
        "branch": head.branch,
        "upstream": head.upstream,
        "ahead": head.ahead,
        "dirty": head.dirty,
        "numstat": numstat,
        "untracked": untracked,
    })
}

/// The untracked files that are not ignored, relative to `top`.
fn untracked_files(top: &str) -> Vec<String> {
    let out = run(top, &["ls-files", "--others", "--exclude-standard", "-z"]).unwrap_or_default();
    out.split('\0')
        .filter(|path| !path.is_empty())
        .map(str::to_string)
        .collect()
}

/// The local branches, the latest commit first, and the one checked out.
pub fn branches(cwd: &str) -> Result<Value, CommandError> {
    let out = run(
        cwd,
        &[
            "for-each-ref",
            "--sort=-committerdate",
            "--format=%(refname:short)",
            "refs/heads",
        ],
    )?;
    let names: Vec<&str> = out.lines().filter(|name| !name.is_empty()).collect();
    let current = head_of(cwd).ok().and_then(|head| head.branch);
    Ok(json!({ "branches": names, "current": current }))
}

/// Switches to a local branch. A tree with changes refuses: switching could take them along or
/// fail halfway, and neither is for a button to decide.
pub fn checkout(cwd: &str, branch: &str) -> Result<(), CommandError> {
    if branch.is_empty() || branch.starts_with('-') {
        return Err("not a branch name".into());
    }
    if head_of(cwd)?.dirty {
        return Err("Uncommitted changes: commit or stash them before switching branch".into());
    }
    run(cwd, &["switch", "--no-guess", branch])?;
    Ok(())
}

/// Commits all changes (new files included) with `message`, then pushes when asked and the
/// branch has an upstream; the answer says why not otherwise. Nothing changed but commits are
/// ahead: only the push. The commit stands when the push fails; `failed` says so.
pub fn commit(cwd: &str, message: &str, push: bool) -> Result<Value, CommandError> {
    let message = message.trim();
    let head = head_of(cwd)?;
    let mut commit = None;
    if head.dirty {
        if message.is_empty() {
            return Err("Write a commit message".into());
        }
        run(cwd, &["add", "-A"])?;
        run(cwd, &["commit", "-q", "-m", message])?;
        let sha = run(cwd, &["rev-parse", "--short", "HEAD"])?;
        commit = Some(sha.trim().to_string());
    } else if !push || head.ahead == 0 {
        return Err("Nothing to commit".into());
    }
    let mut pushed = false;
    let mut note = None;
    let mut failed = false;
    if push {
        match (&head.branch, &head.upstream) {
            (None, _) => note = Some("Not pushed: HEAD is detached".to_string()),
            (Some(branch), None) => {
                note = Some(format!("Not pushed: {branch} has no upstream branch"));
            }
            (Some(_), Some(_)) => match run_limited(cwd, &["push"], PUSH_LIMIT) {
                Ok(_) => pushed = true,
                Err(error) => {
                    note = Some(format!("Push failed: {error}"));
                    failed = true;
                }
            },
        }
    }
    Ok(json!({ "commit": commit, "pushed": pushed, "note": note, "failed": failed }))
}

/// The unified diff of everything changed against HEAD, untracked files as added.
pub fn diff_all(cwd: &str) -> Result<String, CommandError> {
    let top = toplevel(cwd).ok_or("not a git repository")?;
    let has_head = run(&top, &["rev-parse", "--verify", "-q", "HEAD"]).is_ok();
    let against = if has_head { "HEAD" } else { "--cached" };
    let mut text = run(
        &top,
        &["diff", "--no-color", "--no-ext-diff", "-U3", against],
    )?;
    for file in untracked_files(&top).iter().take(DIFF_UNTRACKED_FILES) {
        if let Ok(added) = git::diff(&top, file, true, false) {
            text.push_str(&added);
        }
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("neo-gui-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A repository with one commit on `main`, committing as `t`.
    fn repo(name: &str) -> (PathBuf, String) {
        let dir = temp(name);
        let path = dir.to_string_lossy().to_string();
        run(&path, &["init", "-q", "-b", "main"]).unwrap();
        for (key, value) in [
            ("user.name", "t"),
            ("user.email", "t@t"),
            ("commit.gpgsign", "false"),
        ] {
            run(&path, &["config", key, value]).unwrap();
        }
        std::fs::write(dir.join("a.txt"), "one\ntwo\n").unwrap();
        run(&path, &["add", "a.txt"]).unwrap();
        run(&path, &["commit", "-q", "-m", "first"]).unwrap();
        (dir, path)
    }

    #[test]
    fn git_complaints_are_shortened_to_the_point() {
        let stderr = "hint: waiting\nerror: failed to push some refs to 'x'\nhint: more\n";
        assert_eq!(brief(stderr), "failed to push some refs to 'x'");
        assert_eq!(
            brief("fatal: not a git repository\n"),
            "not a git repository"
        );
        assert_eq!(brief("just words\nmore\n"), "just words");
        assert_eq!(brief(""), "git failed");
        assert!(brief(&"x".repeat(500)).chars().count() <= 201);
    }

    #[test]
    fn the_head_is_read_from_status() {
        let out = "# branch.oid abc\n# branch.head main\n# branch.upstream origin/main\n# branch.ab +2 -1\n1 .M N... 100644 100644 100644 a b a.txt\n";
        assert_eq!(
            parse_head(out),
            Head {
                branch: Some("main".into()),
                upstream: Some("origin/main".into()),
                ahead: 2,
                dirty: true,
            }
        );
        let clean = parse_head("# branch.oid abc\n# branch.head (detached)\n");
        assert_eq!(clean.branch, None);
        assert!(!clean.dirty);
        assert!(parse_head("? new.txt\n").dirty);
    }

    #[test]
    fn lines_are_counted_in_text_files_only() {
        let dir = temp("count");
        std::fs::write(dir.join("two"), "a\nb\n").unwrap();
        std::fs::write(dir.join("open"), "a\nb").unwrap();
        std::fs::write(dir.join("binary"), [1u8, 0, 2]).unwrap();
        assert_eq!(count_lines(&dir.join("two")), Some(2));
        assert_eq!(count_lines(&dir.join("open")), Some(2));
        assert_eq!(count_lines(&dir.join("binary")), None);
        assert_eq!(count_lines(&dir), None);
        assert_eq!(count_lines(&dir.join("missing")), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_overview_counts_what_changed() {
        let (dir, path) = repo("overview");
        assert_eq!(overview("/nonexistent-dir"), Value::Null);
        assert_eq!(overview(""), Value::Null);
        let clean = overview(&path);
        assert_eq!(clean["branch"], "main");
        assert_eq!(clean["dirty"], false);
        assert_eq!(clean["upstream"], Value::Null);

        std::fs::write(dir.join("a.txt"), "one\nthree\nfour\n").unwrap();
        std::fs::write(dir.join("new.txt"), "x\ny\nz\n").unwrap();
        std::fs::write(dir.join("blob.bin"), [0u8, 1]).unwrap();
        let dirty = overview(&path);
        assert_eq!(dirty["dirty"], true);
        assert_eq!(dirty["numstat"], "2\t1\ta.txt\n");
        let mut counts: Vec<u64> = dirty["untracked"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n.as_u64().unwrap())
            .collect();
        counts.sort_unstable();
        assert_eq!(counts, [0, 3]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn switching_branch_refuses_a_dirty_tree() {
        let (dir, path) = repo("checkout");
        run(&path, &["branch", "side"]).unwrap();
        let listed = branches(&path).unwrap();
        assert_eq!(listed["current"], "main");
        let names: Vec<&str> = listed["branches"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect();
        assert!(names.contains(&"main") && names.contains(&"side"));

        std::fs::write(dir.join("a.txt"), "changed\n").unwrap();
        let refused = checkout(&path, "side").unwrap_err();
        assert!(refused.message.contains("Uncommitted changes"));
        assert_eq!(head_of(&path).unwrap().branch.as_deref(), Some("main"));

        run(&path, &["checkout", "--", "a.txt"]).unwrap();
        checkout(&path, "side").unwrap();
        assert_eq!(head_of(&path).unwrap().branch.as_deref(), Some("side"));
        assert!(checkout(&path, "--detach").is_err());
        assert!(checkout(&path, "nowhere").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn committing_takes_everything_and_pushes_only_with_an_upstream() {
        let (dir, path) = repo("commit");
        let bare = temp("commit-remote");
        let remote = bare.to_string_lossy().to_string();
        run(&remote, &["init", "-q", "--bare", "-b", "main"]).unwrap();

        assert_eq!(
            commit(&path, "x", false).unwrap_err().message,
            "Nothing to commit"
        );

        std::fs::write(dir.join("a.txt"), "one\nthree\n").unwrap();
        std::fs::write(dir.join("new.txt"), "fresh\n").unwrap();
        assert_eq!(
            commit(&path, "  ", false).unwrap_err().message,
            "Write a commit message"
        );
        // No upstream: committed, and the answer says why nothing was pushed.
        let first = commit(&path, "second", true).unwrap();
        assert!(first["commit"].is_string());
        assert_eq!(first["pushed"], false);
        assert_eq!(first["note"], "Not pushed: main has no upstream branch");
        assert_eq!(first["failed"], false);
        assert!(!head_of(&path).unwrap().dirty);
        let tracked = run(&path, &["ls-files"]).unwrap();
        assert!(tracked.contains("new.txt"));

        run(&path, &["remote", "add", "origin", &remote]).unwrap();
        run(&path, &["push", "-q", "-u", "origin", "main"]).unwrap();
        std::fs::write(dir.join("a.txt"), "again\n").unwrap();
        let second = commit(&path, "third", true).unwrap();
        assert_eq!(second["pushed"], true);
        assert_eq!(second["note"], Value::Null);
        assert_eq!(
            run(&remote, &["log", "--format=%s", "-1"]).unwrap().trim(),
            "third"
        );

        // Committed but not pushed yet: pushing alone needs no message.
        std::fs::write(dir.join("a.txt"), "more\n").unwrap();
        let local = commit(&path, "fourth", false).unwrap();
        assert_eq!(local["pushed"], false);
        assert_eq!(head_of(&path).unwrap().ahead, 1);
        let only_push = commit(&path, "", true).unwrap();
        assert_eq!(only_push["commit"], Value::Null);
        assert_eq!(only_push["pushed"], true);
        assert_eq!(
            commit(&path, "", true).unwrap_err().message,
            "Nothing to commit"
        );

        // A push the remote refuses keeps the commit and says so.
        run(
            &path,
            &["config", "remote.origin.url", "/nonexistent-remote"],
        )
        .unwrap();
        std::fs::write(dir.join("a.txt"), "last\n").unwrap();
        let broken = commit(&path, "fifth", true).unwrap();
        assert_eq!(broken["failed"], true);
        assert!(broken["note"].as_str().unwrap().starts_with("Push failed"));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&bare);
    }

    #[test]
    fn the_diff_of_everything_includes_new_files() {
        let (dir, path) = repo("diff-all");
        std::fs::write(dir.join("a.txt"), "one\nthree\n").unwrap();
        std::fs::write(dir.join("new.txt"), "fresh\n").unwrap();
        let text = diff_all(&path).unwrap();
        assert!(text.contains("+three") && text.contains("-two"));
        assert!(text.contains("+fresh"));
        assert!(diff_all("/nonexistent-dir").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
