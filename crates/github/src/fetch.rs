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

/// A pull request's unified diff.
pub fn diff(gh: &dyn Gh, cwd: &Path, repo: &str, number: u64) -> Result<String, GhError> {
    gh.run(cwd, &gh::diff_args(repo, number))
}
