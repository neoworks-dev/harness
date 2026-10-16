//! Talking to GitHub through the `gh` CLI, read-only. [`Gh`] is what the view runs commands
//! through (tests feed recorded JSON instead); [`GhCli`] is the real one, which refuses any
//! command [`is_read_only`] doesn't know, so nothing here can write to GitHub.

use std::path::Path;
use std::process::{Command, Stdio};

use crate::model::Kind;

/// Why a `gh` run failed, as far as the user can do something about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GhError {
    /// `gh` is not installed.
    Missing,
    /// Not inside a repository GitHub knows (no remote, or not a git repository).
    NotRepo,
    /// `gh` is not logged in.
    NotAuth,
    Other(String),
}

impl GhError {
    pub fn message(&self) -> String {
        match self {
            GhError::Missing => {
                "the GitHub CLI (gh) is not installed: see https://cli.github.com".into()
            }
            GhError::NotRepo => {
                "this directory is not a GitHub repository (no git remote that gh knows)".into()
            }
            GhError::NotAuth => "gh is not logged in: run `gh auth login` in a terminal".into(),
            GhError::Other(why) => why.clone(),
        }
    }
}

/// Runs `gh` with these arguments in `cwd` and returns its stdout.
pub trait Gh: Send + Sync {
    fn run(&self, cwd: &Path, args: &[String]) -> Result<String, GhError>;
}

/// The real `gh`.
pub struct GhCli;

impl Gh for GhCli {
    fn run(&self, cwd: &Path, args: &[String]) -> Result<String, GhError> {
        if !is_read_only(args) {
            return Err(GhError::Other(
                "refusing a gh command that may write".into(),
            ));
        }
        let out = Command::new("gh")
            .args(args)
            .current_dir(cwd)
            .stdin(Stdio::null())
            .env("GH_PROMPT_DISABLED", "1")
            .env("GH_NO_UPDATE_NOTIFIER", "1")
            .env("GH_PAGER", "cat")
            .env("NO_COLOR", "1")
            .output()
            .map_err(|e| match e.kind() {
                std::io::ErrorKind::NotFound => GhError::Missing,
                _ => GhError::Other(format!("could not run gh: {e}")),
            })?;
        if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).into_owned())
        } else {
            Err(classify(&String::from_utf8_lossy(&out.stderr)))
        }
    }
}

/// The error for gh's stderr.
pub fn classify(stderr: &str) -> GhError {
    let low = stderr.to_lowercase();
    let has = |s: &str| low.contains(s);
    if has("gh auth login")
        || has("not logged in")
        || has("authentication")
        || has("bad credentials")
    {
        GhError::NotAuth
    } else if has("no git remotes")
        || has("not a git repository")
        || has("none of the git remotes")
        || has("could not determine")
        || has("unable to determine")
    {
        GhError::NotRepo
    } else {
        let line = stderr
            .lines()
            .map(str::trim)
            .find(|l| !l.is_empty())
            .unwrap_or("gh failed");
        GhError::Other(crate::model::sane_line(line))
    }
}

/// Whether these `gh` arguments only read: viewing and listing issues and pull requests, a pull
/// request's diff and checks, the repository's name, and a GET of one `repos/…` API path (no
/// method, field or input flags).
pub fn is_read_only(args: &[String]) -> bool {
    let a: Vec<&str> = args.iter().map(String::as_str).collect();
    if a.iter().any(|s| matches!(*s, "--web" | "-w")) {
        return false;
    }
    match a.as_slice() {
        ["repo", "view", ..] => true,
        ["issue", "list" | "view", ..] => true,
        ["pr", "list" | "view" | "diff" | "checks", ..] => true,
        ["api", path] => path.starts_with("repos/") && !path.starts_with('-'),
        _ => false,
    }
}

fn strings(args: &[&str]) -> Vec<String> {
    args.iter().map(|s| (*s).to_string()).collect()
}

pub fn repo_args() -> Vec<String> {
    strings(&["repo", "view", "--json", "nameWithOwner"])
}

const ISSUE_LIST: &str = "number,title,state,author,labels,comments,updatedAt";
const PR_LIST: &str = "number,title,state,isDraft,author,labels,comments,updatedAt";
const ISSUE_VIEW: &str = "number,title,state,author,labels,body,comments,url,createdAt,updatedAt";
const PR_VIEW: &str = "number,title,state,isDraft,author,labels,body,comments,reviews,statusCheckRollup,files,additions,deletions,url,createdAt,updatedAt,baseRefName,headRefName,reviewDecision";
/// Rows asked for per list.
pub const LIST_LIMIT: &str = "50";

/// The list command; `query` goes to `--search` (GitHub search syntax), newest update first.
pub fn list_args(kind: Kind, repo: &str, all: bool, query: &str) -> Vec<String> {
    let (noun, fields) = match kind {
        Kind::Issue => ("issue", ISSUE_LIST),
        Kind::Pr => ("pr", PR_LIST),
    };
    let search = format!("{} sort:updated-desc", query.trim());
    let mut args = strings(&[
        noun, "list", "-R", repo, "--limit", LIST_LIMIT, "--json", fields,
    ]);
    args.extend(strings(&["--state", if all { "all" } else { "open" }]));
    args.extend(["--search".to_string(), search.trim().to_string()]);
    args
}

pub fn view_args(kind: Kind, repo: &str, number: u64) -> Vec<String> {
    let (noun, fields) = match kind {
        Kind::Issue => ("issue", ISSUE_VIEW),
        Kind::Pr => ("pr", PR_VIEW),
    };
    let mut args = strings(&[noun, "view"]);
    args.extend([number.to_string()]);
    args.extend(strings(&["-R", repo, "--json", fields]));
    args
}

/// Comments on lines of a pull request's diff (REST; the CLI's own view has none).
pub fn review_comments_args(repo: &str, number: u64) -> Vec<String> {
    vec![
        "api".into(),
        format!("repos/{repo}/pulls/{number}/comments?per_page=100"),
    ]
}

pub fn diff_args(repo: &str, number: u64) -> Vec<String> {
    let mut args = strings(&["pr", "diff"]);
    args.extend([number.to_string()]);
    args.extend(strings(&["-R", repo, "--color", "never"]));
    args
}
