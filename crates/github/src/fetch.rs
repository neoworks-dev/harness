//! The asks both apps make, end to end: run the `gh` command, parse its answer.

use std::path::Path;

use crate::gh::{self, Gh, GhError};
use crate::model::{self, Detail, Kind, Summary};

/// `owner/name` of the repository `gh` finds for `cwd`.
pub fn repo(gh: &dyn Gh, cwd: &Path) -> Result<String, GhError> {
    let out = gh.run(cwd, &gh::repo_args())?;
    model::parse_repo(&out).map_err(GhError::Other)
}

/// The issues or pull requests of `repo`, newest update first. `query` is GitHub search syntax.
pub fn list(
    gh: &dyn Gh,
    cwd: &Path,
    kind: Kind,
    repo: &str,
    all: bool,
    query: &str,
) -> Result<Vec<Summary>, GhError> {
    let out = gh.run(cwd, &gh::list_args(kind, repo, all, query))?;
    model::parse_list(kind, &out).map_err(GhError::Other)
}

/// One issue or pull request with its conversation. A pull request also gets the comments on
/// lines of its diff, which is best effort: it is useful without them.
pub fn detail(
    gh: &dyn Gh,
    cwd: &Path,
    kind: Kind,
    repo: &str,
    number: u64,
) -> Result<Detail, GhError> {
    let out = gh.run(cwd, &gh::view_args(kind, repo, number))?;
    let mut detail = model::parse_detail(kind, &out).map_err(GhError::Other)?;
    if kind == Kind::Pr
        && let Ok(out) = gh.run(cwd, &gh::review_comments_args(repo, number))
    {
        detail.review_comments = model::parse_review_comments(&out);
    }
    Ok(detail)
}

/// The repository's labels.
pub fn labels(gh: &dyn Gh, cwd: &Path, repo: &str) -> Result<Vec<model::Label>, GhError> {
    Ok(model::parse_labels(&gh.run(cwd, &gh::labels_args(repo))?))
}

/// Who can be assigned in the repository.
pub fn people(gh: &dyn Gh, cwd: &Path, repo: &str) -> Result<Vec<model::Person>, GhError> {
    Ok(model::parse_people(&gh.run(cwd, &gh::people_args(repo))?))
}

// Writes to GitHub: each one thing the user asked for in the app, done as the user.

/// Adds a comment to an issue or pull request.
pub fn comment(
    gh: &dyn Gh,
    cwd: &Path,
    kind: Kind,
    repo: &str,
    number: u64,
    body: &str,
) -> Result<(), GhError> {
    gh.write(cwd, &gh::comment_args(kind, repo, number), Some(body))
        .map(drop)
}

/// Closes an issue (with the reason) or a pull request.
pub fn close(
    gh: &dyn Gh,
    cwd: &Path,
    kind: Kind,
    repo: &str,
    number: u64,
    reason: gh::CloseReason,
) -> Result<(), GhError> {
    gh.write(cwd, &gh::close_args(kind, repo, number, reason), None)
        .map(drop)
}

pub fn reopen(
    gh: &dyn Gh,
    cwd: &Path,
    kind: Kind,
    repo: &str,
    number: u64,
) -> Result<(), GhError> {
    gh.write(cwd, &gh::reopen_args(kind, repo, number), None)
        .map(drop)
}

/// Reacts to an item or one of its comments (`content` is REST's word, `heart`), or takes the
/// reaction back if the user already gave that one: true when it was given, false when taken
/// back.
pub fn toggle_reaction(
    gh: &dyn Gh,
    cwd: &Path,
    repo: &str,
    on: gh::ReactOn,
    content: &str,
) -> Result<bool, GhError> {
    let me = model::parse_login(&gh.run(cwd, &gh::viewer_args())?)
        .ok_or_else(|| GhError::Other("could not tell who you are on GitHub".into()))?;
    let given = model::parse_reaction_list(&gh.run(cwd, &gh::reactions_args(repo, on))?);
    let mine = given
        .iter()
        .find(|(_, login, kind)| login.eq_ignore_ascii_case(&me) && kind == content);
    match mine {
        Some((id, _, _)) => {
            gh.write(cwd, &gh::unreact_args(repo, on, *id), None)?;
            Ok(false)
        }
        None => {
            gh.write(cwd, &gh::react_args(repo, on, content), None)?;
            Ok(true)
        }
    }
}

/// Assignees, labels, milestone, type, projects and relationships of an issue or pull request.
pub fn edit(
    gh: &dyn Gh,
    cwd: &Path,
    kind: Kind,
    repo: &str,
    number: u64,
    changes: &gh::Changes,
) -> Result<(), GhError> {
    gh.write(cwd, &gh::edit_args(kind, repo, number, changes), None)
        .map(drop)
}

/// Replaces the description of an issue or pull request.
pub fn edit_body(
    gh: &dyn Gh,
    cwd: &Path,
    kind: Kind,
    repo: &str,
    number: u64,
    body: &str,
) -> Result<(), GhError> {
    gh.write(cwd, &gh::edit_body_args(kind, repo, number), Some(body))
        .map(drop)
}

/// Replaces the text of a comment (its number is the one in its link).
pub fn edit_comment(
    gh: &dyn Gh,
    cwd: &Path,
    repo: &str,
    id: u64,
    body: &str,
) -> Result<(), GhError> {
    let json = serde_json::json!({ "body": body }).to_string();
    gh.write(cwd, &gh::edit_comment_args(repo, id), Some(&json))
        .map(drop)
}

/// Sets one issue field to a value (the name of an option for a single select).
pub fn set_field(
    gh: &dyn Gh,
    cwd: &Path,
    repo: &str,
    number: u64,
    field: &model::OrgField,
    value: &str,
) -> Result<(), GhError> {
    let value = if field.data_type == "number" {
        value.parse::<f64>().map_or(serde_json::json!(value), |n| serde_json::json!(n))
    } else {
        serde_json::json!(value)
    };
    let body = serde_json::json!({"issue_field_values": [{"field_id": field.id, "value": value}]});
    gh.write(
        cwd,
        &gh::field_values_write_args(repo, number),
        Some(&body.to_string()),
    )
    .map(drop)
}

/// The issue fields the organization offers.
pub fn org_fields(gh: &dyn Gh, cwd: &Path, owner: &str) -> Result<Vec<model::OrgField>, GhError> {
    Ok(model::parse_org_fields(&gh.run(cwd, &gh::org_fields_args(owner))?))
}

/// The issue types the organization offers.
pub fn issue_types(gh: &dyn Gh, cwd: &Path, owner: &str) -> Result<Vec<String>, GhError> {
    Ok(model::parse_names(&gh.run(cwd, &gh::issue_types_args(owner))?, "name"))
}

/// The open milestones of the repository.
pub fn milestones(gh: &dyn Gh, cwd: &Path, repo: &str) -> Result<Vec<String>, GhError> {
    Ok(model::parse_names(&gh.run(cwd, &gh::milestones_args(repo))?, "title"))
}

/// The projects of the owner.
pub fn projects(gh: &dyn Gh, cwd: &Path, owner: &str) -> Result<Vec<String>, GhError> {
    Ok(model::parse_projects(&gh.run(cwd, &gh::projects_args(owner))?))
}

/// What the issue fields of an issue are set to.
pub fn field_values(
    gh: &dyn Gh,
    cwd: &Path,
    repo: &str,
    number: u64,
) -> Result<Vec<(String, String)>, GhError> {
    Ok(model::parse_field_values(
        &gh.run(cwd, &gh::field_values_args(repo, number))?,
    ))
}

/// For the item (`None`) or a comment (`Some(index)`): GitHub's words for the reactions the
/// user gave there.
pub type MyReactions = Vec<(Option<usize>, Vec<String>)>;

/// Which of the reactions on an item and its comments are the user's: for each that has
/// reactions (`None` is the item, `Some(i)` the i-th comment), GitHub's words for the ones the
/// user gave (`HEART`). gh's own view does not say, so this asks who gave each; best effort for
/// the caller.
pub fn my_reactions(
    gh: &dyn Gh,
    cwd: &Path,
    repo: &str,
    detail: &Detail,
) -> Result<MyReactions, GhError> {
    let mut targets: Vec<(Option<usize>, gh::ReactOn)> = Vec::new();
    if !detail.reactions.is_empty() {
        targets.push((None, gh::ReactOn::Item(detail.number)));
    }
    for (i, c) in detail.comments.iter().enumerate() {
        if !c.reactions.is_empty() && c.id != 0 {
            targets.push((Some(i), gh::ReactOn::Comment(c.id)));
        }
    }
    if targets.is_empty() {
        return Ok(Vec::new());
    }
    let me = model::parse_login(&gh.run(cwd, &gh::viewer_args())?)
        .ok_or_else(|| GhError::Other("could not tell who you are on GitHub".into()))?;
    let mut out = Vec::new();
    for (who, on) in targets {
        // One that can't be read is left unmarked, not a reason to mark none.
        let Ok(text) = gh.run(cwd, &gh::reactions_args(repo, on)) else {
            continue;
        };
        let given = model::parse_reaction_list(&text);
        let words: Vec<String> = given
            .iter()
            .filter(|(_, login, _)| login.eq_ignore_ascii_case(&me))
            .filter_map(|(_, _, content)| {
                model::REACTIONS
                    .iter()
                    .find(|r| r.1 == content)
                    .map(|r| r.0.to_string())
            })
            .collect();
        out.push((who, words));
    }
    Ok(out)
}

/// A pull request's unified diff.
pub fn diff(gh: &dyn Gh, cwd: &Path, repo: &str, number: u64) -> Result<String, GhError> {
    gh.run(cwd, &gh::diff_args(repo, number))
}

/// The log of a GitHub Actions job, as `gh` prints it (`job\tstep\ttime line` per line):
/// only the failed steps with `failed`.
pub fn job_log(
    gh: &dyn Gh,
    cwd: &Path,
    repo: &str,
    job: u64,
    failed: bool,
) -> Result<String, GhError> {
    gh.run(cwd, &gh::job_log_args(repo, job, failed))
}

/// A file's text at a commit. A binary file, or one too big for the API (over 1 MB), is an
/// error that says so.
pub fn file_at(
    gh: &dyn Gh,
    cwd: &Path,
    repo: &str,
    path: &str,
    sha: &str,
) -> Result<String, GhError> {
    let out = gh.run(cwd, &gh::file_args(repo, path, sha))?;
    let v: serde_json::Value = serde_json::from_str(&out)
        .map_err(|_| GhError::Other("the file came back unreadable".into()))?;
    if v["encoding"] != "base64" {
        return Err(GhError::Other("the file is too big to show whole".into()));
    }
    let bytes = model::base64(v["content"].as_str().unwrap_or(""))
        .ok_or_else(|| GhError::Other("the file came back unreadable".into()))?;
    String::from_utf8(bytes).map_err(|_| GhError::Other("a binary file has no text".into()))
}
