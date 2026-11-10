//! Talking to GitHub through the `gh` CLI. [`Gh`] is what the view runs commands through (tests
//! feed recorded JSON instead); [`GhCli`] is the real one. `run` reads and refuses any command
//! [`is_read_only`] doesn't know; `write` runs only the few the user can ask for in an app
//! (a comment, a review comment on lines, a reaction, closing and reopening, edits:
//! [`is_allowed_write`]).

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

    /// Runs one of the few commands that write ([`is_allowed_write`]), as the user `gh` is
    /// logged in as, with `input` on its stdin. Only what the user asked for in the app, one
    /// action at a time. Refuses by default.
    fn write(&self, _cwd: &Path, _args: &[String], _input: Option<&str>) -> Result<String, GhError> {
        Err(GhError::Other("writing to GitHub is not available here".into()))
    }
}

/// The real `gh`.
pub struct GhCli;

impl GhCli {
    fn exec(cwd: &Path, args: &[String], input: Option<&str>) -> Result<String, GhError> {
        use std::io::Write;
        let mut child = Command::new("gh")
            .args(args)
            .current_dir(cwd)
            .stdin(if input.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("GH_PROMPT_DISABLED", "1")
            .env("GH_NO_UPDATE_NOTIFIER", "1")
            .env("GH_PAGER", "cat")
            .env("NO_COLOR", "1")
            .spawn()
            .map_err(|e| match e.kind() {
                std::io::ErrorKind::NotFound => GhError::Missing,
                _ => GhError::Other(format!("could not run gh: {e}")),
            })?;
        if let (Some(text), Some(mut stdin)) = (input, child.stdin.take()) {
            // A closed pipe shows in the exit status below.
            let _ = stdin.write_all(text.as_bytes());
        }
        let out = child
            .wait_with_output()
            .map_err(|e| GhError::Other(format!("could not run gh: {e}")))?;
        if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).into_owned())
        } else {
            Err(classify(&String::from_utf8_lossy(&out.stderr)))
        }
    }
}

impl Gh for GhCli {
    fn run(&self, cwd: &Path, args: &[String]) -> Result<String, GhError> {
        if !is_read_only(args) {
            return Err(GhError::Other(
                "refusing a gh command that may write".into(),
            ));
        }
        GhCli::exec(cwd, args, None)
    }

    fn write(&self, cwd: &Path, args: &[String], input: Option<&str>) -> Result<String, GhError> {
        if !is_allowed_write(args) {
            return Err(GhError::Other("refusing a gh command that is not one of the app's".into()));
        }
        GhCli::exec(cwd, args, input)
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
/// request's diff and checks, the log of a workflow job, the repository's name, and a GET of
/// one `repos/…` API path (no method, field or input flags).
pub fn is_read_only(args: &[String]) -> bool {
    let a: Vec<&str> = args.iter().map(String::as_str).collect();
    if a.iter().any(|s| matches!(*s, "--web" | "-w")) {
        return false;
    }
    match a.as_slice() {
        ["repo", "view", ..] => true,
        ["project", "list", ..] => true,
        ["issue", "list" | "view", ..] => true,
        ["pr", "list" | "view" | "diff" | "checks", ..] => true,
        // A job's log, and nothing else of `run` (no rerun, cancel, download).
        ["run", "view", "--job", job, "-R", _, "--log" | "--log-failed"] => {
            !job.is_empty() && job.bytes().all(|b| b.is_ascii_digit())
        }
        ["api", path] => {
            path.starts_with("repos/") && !path.starts_with('-')
                || *path == "user"
                || is_org_listing(path)
        }
        _ => false,
    }
}

/// Why an issue is closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseReason {
    Completed,
    NotPlanned,
    /// Of this issue.
    Duplicate(u64),
}

/// What a reaction goes on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReactOn {
    /// The issue or pull request itself.
    Item(u64),
    /// A comment, by its number (the one in its link).
    Comment(u64),
}

fn noun(kind: Kind) -> &'static str {
    match kind {
        Kind::Issue => "issue",
        Kind::Pr => "pr",
    }
}

/// A new comment; its text goes on stdin.
pub fn comment_args(kind: Kind, repo: &str, number: u64) -> Vec<String> {
    let mut args = strings(&[noun(kind), "comment"]);
    args.push(number.to_string());
    args.extend(strings(&["-R", repo, "--body-file", "-"]));
    args
}

/// Closing; an issue says why, a pull request is just closed.
pub fn close_args(kind: Kind, repo: &str, number: u64, reason: CloseReason) -> Vec<String> {
    let mut args = strings(&[noun(kind), "close"]);
    args.push(number.to_string());
    args.extend(strings(&["-R", repo]));
    if kind == Kind::Issue {
        match reason {
            CloseReason::Completed => args.extend(strings(&["--reason", "completed"])),
            CloseReason::NotPlanned => args.extend(strings(&["--reason", "not planned"])),
            CloseReason::Duplicate(of) => {
                args.extend(strings(&["--reason", "duplicate", "--duplicate-of"]));
                args.push(of.to_string());
            }
        }
    }
    args
}

pub fn reopen_args(kind: Kind, repo: &str, number: u64) -> Vec<String> {
    let mut args = strings(&[noun(kind), "reopen"]);
    args.push(number.to_string());
    args.extend(strings(&["-R", repo]));
    args
}

fn reactions_path(repo: &str, on: ReactOn) -> String {
    match on {
        ReactOn::Item(n) => format!("repos/{repo}/issues/{n}/reactions"),
        ReactOn::Comment(id) => format!("repos/{repo}/issues/comments/{id}/reactions"),
    }
}

/// Who gave which reactions on an item or a comment (REST), with the id each can be taken
/// back by.
pub fn reactions_args(repo: &str, on: ReactOn) -> Vec<String> {
    vec![
        "api".into(),
        format!("{}?per_page=100", reactions_path(repo, on)),
    ]
}

/// The user `gh` is logged in as (REST).
pub fn viewer_args() -> Vec<String> {
    vec!["api".into(), "user".into()]
}

/// Takes back the reaction with this id.
pub fn unreact_args(repo: &str, on: ReactOn, id: u64) -> Vec<String> {
    vec![
        "api".into(),
        "-X".into(),
        "DELETE".into(),
        format!("{}/{id}", reactions_path(repo, on)),
    ]
}

/// A reaction (`+1`, `heart`… in REST's words) on an item or one of its comments.
pub fn react_args(repo: &str, on: ReactOn, content: &str) -> Vec<String> {
    let path = reactions_path(repo, on);
    vec![
        "api".into(),
        "-X".into(),
        "POST".into(),
        path,
        "-f".into(),
        format!("content={content}"),
    ]
}

/// The few commands [`Gh::write`] runs: a comment, closing and reopening, and a reaction. The
/// shapes are the ones [`comment_args`] and the others build, nothing looser.
pub fn is_allowed_write(args: &[String]) -> bool {
    let a: Vec<&str> = args.iter().map(String::as_str).collect();
    let number = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    let repo = |s: &str| {
        let mut parts = s.split('/');
        let ok = |p: Option<&str>| {
            p.is_some_and(|p| {
                !p.is_empty()
                    && p.bytes()
                        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
            })
        };
        ok(parts.next()) && ok(parts.next()) && parts.next().is_none()
    };
    match a.as_slice() {
        ["issue" | "pr", "comment", num, "-R", r, "--body-file", "-"] => number(num) && repo(r),
        ["issue", "close", num, "-R", r, "--reason", why] => {
            number(num) && repo(r) && matches!(*why, "completed" | "not planned")
        }
        ["issue", "close", num, "-R", r, "--reason", "duplicate", "--duplicate-of", of] => {
            number(num) && repo(r) && number(of)
        }
        ["pr", "close", num, "-R", r] => number(num) && repo(r),
        ["issue" | "pr", "edit", num, "-R", r, "--body-file", "-"] => number(num) && repo(r),
        ["api", "-X", "PATCH", path, "--input", "-"] => {
            let parts: Vec<&str> = path.split('/').collect();
            matches!(parts.as_slice(), ["repos", o, n, "issues", "comments", id]
                if repo(&format!("{o}/{n}")) && number(id))
        }
        [k @ ("issue" | "pr"), "edit", num, "-R", r, flags @ ..] => {
            number(num) && repo(r) && edit_flags_ok(k, flags)
        }
        ["api", "-X", "POST", path, "--input", "-"] => {
            let parts: Vec<&str> = path.split('/').collect();
            match parts.as_slice() {
                ["repos", o, n, "issues", i, "issue-field-values"]
                | ["repos", o, n, "pulls", i, "comments"] => {
                    repo(&format!("{o}/{n}")) && number(i)
                }
                _ => false,
            }
        }
        ["issue" | "pr", "reopen", num, "-R", r] => number(num) && repo(r),
        ["api", "-X", "POST", path, "-f", content] => {
            let reaction = matches!(
                content.strip_prefix("content="),
                Some("+1" | "-1" | "laugh" | "confused" | "heart" | "hooray" | "rocket" | "eyes")
            );
            reaction && reactions_path_ok(path, false, &repo, &number)
        }
        ["api", "-X", "DELETE", path] => reactions_path_ok(path, true, &repo, &number),
        _ => false,
    }
}

/// Whether `path` is `repos/o/r/issues/N/reactions` or `repos/o/r/issues/comments/ID/reactions`,
/// with `/ID` of the reaction after it when `with_id`.
fn reactions_path_ok(
    path: &str,
    with_id: bool,
    repo: &dyn Fn(&str) -> bool,
    number: &dyn Fn(&str) -> bool,
) -> bool {
    let parts: Vec<&str> = path.split('/').collect();
    let (head, tail) = match parts.as_slice() {
        ["repos", o, n, "issues", "comments", id, "reactions", rest @ ..] => {
            ((format!("{o}/{n}"), *id), rest)
        }
        ["repos", o, n, "issues", id, "reactions", rest @ ..] => ((format!("{o}/{n}"), *id), rest),
        _ => return false,
    };
    let tail_ok = match (with_id, tail) {
        (false, []) => true,
        (true, [reaction]) => number(reaction),
        _ => false,
    };
    tail_ok && repo(&head.0) && number(head.1)
}

/// `orgs/<org>/issue-fields` or `orgs/<org>/issue-types`: what an organization offers.
fn is_org_listing(path: &str) -> bool {
    let parts: Vec<&str> = path.split('/').collect();
    matches!(parts.as_slice(), ["orgs", org, "issue-fields" | "issue-types"]
        if !org.is_empty() && org.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.')))
}

/// What to change about an issue or pull request in one go (`gh issue edit`). `Some(None)` takes
/// the milestone, type or parent away.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Changes {
    pub add_assignees: Vec<String>,
    pub remove_assignees: Vec<String>,
    pub add_labels: Vec<String>,
    pub remove_labels: Vec<String>,
    pub milestone: Option<Option<String>>,
    pub issue_type: Option<Option<String>>,
    pub add_projects: Vec<String>,
    pub remove_projects: Vec<String>,
    pub parent: Option<Option<u64>>,
    pub add_sub_issues: Vec<u64>,
    pub remove_sub_issues: Vec<u64>,
    pub add_blocked_by: Vec<u64>,
    pub remove_blocked_by: Vec<u64>,
    pub add_blocking: Vec<u64>,
    pub remove_blocking: Vec<u64>,
    pub title: Option<String>,
}

impl Changes {
    pub fn is_empty(&self) -> bool {
        *self == Changes::default()
    }
}

/// `gh issue edit` / `gh pr edit` with the changes. A pull request has no type, parent,
/// sub-issues or blocking: those are left out for it.
pub fn edit_args(kind: Kind, repo: &str, number: u64, c: &Changes) -> Vec<String> {
    let mut args = strings(&[noun(kind), "edit"]);
    args.push(number.to_string());
    args.extend(strings(&["-R", repo]));
    let mut each = |flag: &str, values: &[String]| {
        for v in values {
            args.push(flag.to_string());
            args.push(v.clone());
        }
    };
    each("--add-assignee", &c.add_assignees);
    each("--remove-assignee", &c.remove_assignees);
    each("--add-label", &c.add_labels);
    each("--remove-label", &c.remove_labels);
    each("--add-project", &c.add_projects);
    each("--remove-project", &c.remove_projects);
    if let Some(title) = &c.title {
        args.extend(["--title".to_string(), title.clone()]);
    }
    match &c.milestone {
        Some(Some(m)) => args.extend(["--milestone".to_string(), m.clone()]),
        Some(None) => args.push("--remove-milestone".into()),
        None => {}
    }
    if kind == Kind::Issue {
        match &c.issue_type {
            Some(Some(t)) => args.extend(["--type".to_string(), t.clone()]),
            Some(None) => args.push("--remove-type".into()),
            None => {}
        }
        match c.parent {
            Some(Some(p)) => args.extend(["--parent".to_string(), p.to_string()]),
            Some(None) => args.push("--remove-parent".into()),
            None => {}
        }
        for (flag, numbers) in [
            ("--add-sub-issue", &c.add_sub_issues),
            ("--remove-sub-issue", &c.remove_sub_issues),
            ("--add-blocked-by", &c.add_blocked_by),
            ("--remove-blocked-by", &c.remove_blocked_by),
            ("--add-blocking", &c.add_blocking),
            ("--remove-blocking", &c.remove_blocking),
        ] {
            for n in numbers {
                args.extend([flag.to_string(), n.to_string()]);
            }
        }
    }
    args
}

/// A new description for an issue or pull request; the text goes on stdin.
pub fn edit_body_args(kind: Kind, repo: &str, number: u64) -> Vec<String> {
    let mut args = strings(&[noun(kind), "edit"]);
    args.push(number.to_string());
    args.extend(strings(&["-R", repo, "--body-file", "-"]));
    args
}

/// A new text for a comment (REST; `{"body": …}` on stdin).
/// A review comment on lines of a pull request's diff (REST, the comment as JSON on stdin).
pub fn review_comment_args(repo: &str, number: u64) -> Vec<String> {
    vec![
        "api".into(),
        "-X".into(),
        "POST".into(),
        format!("repos/{repo}/pulls/{number}/comments"),
        "--input".into(),
        "-".into(),
    ]
}

pub fn edit_comment_args(repo: &str, id: u64) -> Vec<String> {
    vec![
        "api".into(),
        "-X".into(),
        "PATCH".into(),
        format!("repos/{repo}/issues/comments/{id}"),
        "--input".into(),
        "-".into(),
    ]
}

/// Setting issue fields: the body goes on stdin (`{"issue_field_values":[…]}`).
pub fn field_values_write_args(repo: &str, number: u64) -> Vec<String> {
    vec![
        "api".into(),
        "-X".into(),
        "POST".into(),
        format!("repos/{repo}/issues/{number}/issue-field-values"),
        "--input".into(),
        "-".into(),
    ]
}

/// The issue fields of an organization (REST).
pub fn org_fields_args(owner: &str) -> Vec<String> {
    vec!["api".into(), format!("orgs/{owner}/issue-fields")]
}

/// The issue types of an organization (REST).
pub fn issue_types_args(owner: &str) -> Vec<String> {
    vec!["api".into(), format!("orgs/{owner}/issue-types")]
}

/// The open milestones of the repository (REST).
pub fn milestones_args(repo: &str) -> Vec<String> {
    vec!["api".into(), format!("repos/{repo}/milestones?per_page=100")]
}

/// What the issue fields of an issue are set to (REST).
pub fn field_values_args(repo: &str, number: u64) -> Vec<String> {
    vec!["api".into(), format!("repos/{repo}/issues/{number}/issue-field-values")]
}

/// The projects of an owner (needs gh's `project` scope).
pub fn projects_args(owner: &str) -> Vec<String> {
    strings(&["project", "list", "--owner", owner, "--format", "json", "--limit", "100"])
}

/// Whether the flags after `issue edit N -R repo` are ones [`edit_args`] makes.
fn edit_flags_ok(kind: &str, flags: &[&str]) -> bool {
    let text = |s: &str| {
        !s.is_empty() && s.chars().count() <= 200 && !s.chars().any(char::is_control)
    };
    let number = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    let mut i = 0;
    while i < flags.len() {
        let flag = flags[i];
        let issue = kind == "issue";
        let bare = matches!(flag, "--remove-milestone")
            || (issue && matches!(flag, "--remove-type" | "--remove-parent"));
        if bare {
            i += 1;
            continue;
        }
        let Some(value) = flags.get(i + 1) else {
            return false;
        };
        let ok = match flag {
            "--add-assignee" | "--remove-assignee" | "--add-label" | "--remove-label"
            | "--milestone" | "--add-project" | "--remove-project" | "--title" => text(value),
            "--type" if issue => text(value),
            "--parent" | "--add-sub-issue" | "--remove-sub-issue" | "--add-blocked-by"
            | "--remove-blocked-by" | "--add-blocking" | "--remove-blocking"
                if issue =>
            {
                number(value)
            }
            _ => false,
        };
        if !ok {
            return false;
        }
        i += 2;
    }
    !flags.is_empty()
}

fn strings(args: &[&str]) -> Vec<String> {
    args.iter().map(|s| (*s).to_string()).collect()
}

pub fn repo_args() -> Vec<String> {
    strings(&["repo", "view", "--json", "nameWithOwner"])
}

const ISSUE_LIST: &str = "number,title,state,author,labels,comments,updatedAt";
const PR_LIST: &str = "number,title,state,isDraft,author,labels,comments,updatedAt";
const ISSUE_VIEW: &str = "number,title,state,author,labels,body,comments,url,createdAt,updatedAt,reactionGroups,stateReason,assignees,milestone,issueType,projectItems,parent,blockedBy,blocking";
const PR_VIEW: &str = "number,title,state,isDraft,author,labels,body,comments,reviews,statusCheckRollup,files,additions,deletions,url,createdAt,updatedAt,baseRefName,headRefName,reviewDecision,reactionGroups,assignees,milestone,projectItems,reviewRequests,headRefOid";
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

/// The repository's labels (REST).
pub fn labels_args(repo: &str) -> Vec<String> {
    vec!["api".into(), format!("repos/{repo}/labels?per_page=100")]
}

/// Who can be assigned in the repository (REST).
pub fn people_args(repo: &str) -> Vec<String> {
    vec!["api".into(), format!("repos/{repo}/assignees?per_page=100")]
}

/// The log of a GitHub Actions job: only its failed steps with `failed`, else all of it.
pub fn job_log_args(repo: &str, job: u64, failed: bool) -> Vec<String> {
    let mut args = strings(&["run", "view", "--job"]);
    args.push(job.to_string());
    args.extend(strings(&["-R", repo, if failed { "--log-failed" } else { "--log" }]));
    args
}

/// A file as it is at a commit (REST contents, base64 in JSON). Each part of the path is
/// percent-encoded, so nothing in it can change what is asked for.
pub fn file_args(repo: &str, path: &str, sha: &str) -> Vec<String> {
    let encode = |part: &str| {
        // `.` and `..` would walk the path: their dots are encoded too.
        let dots = matches!(part, "." | "..");
        part.bytes()
            .map(|b| match b {
                b'.' if dots => "%2E".to_string(),
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                    (b as char).to_string()
                }
                _ => format!("%{b:02X}"),
            })
            .collect::<String>()
    };
    let path: Vec<String> = path.split('/').map(encode).collect();
    vec![
        "api".into(),
        format!("repos/{repo}/contents/{}?ref={}", path.join("/"), encode(sha)),
    ]
}

pub fn diff_args(repo: &str, number: u64) -> Vec<String> {
    let mut args = strings(&["pr", "diff"]);
    args.extend([number.to_string()]);
    args.extend(strings(&["-R", repo, "--color", "never"]));
    args
}
