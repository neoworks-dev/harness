//! The GitHub view's data: issues and pull requests of the session's repository, read through
//! `neo-github`, whose `gh` runner refuses anything that could write.

use std::path::Path;

use neo_github::fetch;
use neo_github::format;
use neo_github::gh::{Gh, GhCli, GhError};
use neo_github::model::Kind;
use serde::Serialize;
use serde_json::{Value, json};
use svelte_gpui::{CommandError, Commands};

/// A `gh` failure for the UI: the code says what the user can do about it.
fn failure(error: GhError) -> CommandError {
    let code = match error {
        GhError::Missing => "GhMissing",
        GhError::NotRepo => "NotRepo",
        GhError::NotAuth => "NotAuth",
        GhError::Other(_) => "GhFailed",
    };
    CommandError::with_code(code, error.message())
}

fn text<'a>(args: &'a Value, key: &str) -> Result<&'a str, CommandError> {
    args[key]
        .as_str()
        .ok_or_else(|| CommandError::new(format!("missing argument {key}")))
}

fn number(args: &Value) -> Result<u64, CommandError> {
    args["number"]
        .as_u64()
        .ok_or_else(|| CommandError::new("missing argument number"))
}

fn json_of<T: Serialize>(value: &T) -> Result<Value, CommandError> {
    serde_json::to_value(value).map_err(|error| CommandError::new(error.to_string()))
}

/// `owner/name` of the repository `gh` finds for `cwd`.
pub fn repo(gh: &dyn Gh, args: &Value) -> Result<Value, CommandError> {
    let cwd = text(args, "cwd")?;
    let name = fetch::repo(gh, Path::new(cwd)).map_err(failure)?;
    Ok(json!(name))
}

/// The rows of the issues or pull requests list. `all` includes closed ones; `query` is GitHub
/// search syntax.
pub fn list(gh: &dyn Gh, kind: Kind, args: &Value) -> Result<Value, CommandError> {
    let cwd = text(args, "cwd")?;
    let repo = text(args, "repo")?;
    let all = args["all"].as_bool().unwrap_or(false);
    let query = args["query"].as_str().unwrap_or("");
    let rows = fetch::list(gh, Path::new(cwd), kind, repo, all, query).map_err(failure)?;
    json_of(&rows)
}

/// One issue or pull request with its conversation.
pub fn detail(gh: &dyn Gh, kind: Kind, args: &Value) -> Result<Value, CommandError> {
    let cwd = text(args, "cwd")?;
    let repo = text(args, "repo")?;
    let detail = fetch::detail(gh, Path::new(cwd), kind, repo, number(args)?).map_err(failure)?;
    json_of(&detail)
}

/// A pull request's unified diff.
pub fn pr_diff(gh: &dyn Gh, args: &Value) -> Result<Value, CommandError> {
    let cwd = text(args, "cwd")?;
    let repo = text(args, "repo")?;
    let diff = fetch::diff(gh, Path::new(cwd), repo, number(args)?).map_err(failure)?;
    Ok(json!(diff))
}

/// What "send to the model" puts into the session: a line of instruction, then the issue or
/// pull request with its conversation, trimmed to a bounded size.
pub fn send_text(gh: &dyn Gh, kind: Kind, args: &Value) -> Result<Value, CommandError> {
    let cwd = text(args, "cwd")?;
    let repo = text(args, "repo")?;
    let detail = fetch::detail(gh, Path::new(cwd), kind, repo, number(args)?).map_err(failure)?;
    Ok(json!(format::send_text(repo, &detail)))
}

/// The commands, on the real `gh`.
pub fn register(commands: Commands) -> Commands {
    commands
        .command("gh_repo", |args| repo(&GhCli, &args))
        .command("gh_issues", |args| list(&GhCli, Kind::Issue, &args))
        .command("gh_prs", |args| list(&GhCli, Kind::Pr, &args))
        .command("gh_issue", |args| detail(&GhCli, Kind::Issue, &args))
        .command("gh_pr", |args| detail(&GhCli, Kind::Pr, &args))
        .command("gh_pr_diff", |args| pr_diff(&GhCli, &args))
        .command("gh_issue_text", |args| {
            send_text(&GhCli, Kind::Issue, &args)
        })
        .command("gh_pr_text", |args| send_text(&GhCli, Kind::Pr, &args))
}

#[cfg(test)]
mod tests {
    use super::*;
    use neo_github::fixtures::*;

    /// Answers like the recorded acme/site, and fails the test on a command that may write.
    struct Recorded;

    impl Gh for Recorded {
        fn run(&self, _cwd: &Path, args: &[String]) -> Result<String, GhError> {
            assert!(neo_github::gh::is_read_only(args), "{args:?}");
            let line = args.join(" ");
            let reply = [
                ("repo view", REPO),
                ("issue list", ISSUES),
                ("pr list", PRS),
                ("issue view 12", ISSUE_12),
                ("pr view 31", PR_31),
                ("pulls/31/comments", PR_31_COMMENTS),
                ("pr diff 31", DIFF_31),
            ]
            .into_iter()
            .find(|(key, _)| line.contains(key));
            reply
                .map(|(_, text)| text.to_string())
                .ok_or(GhError::NotRepo)
        }
    }

    #[test]
    fn lists_and_details_come_as_camel_case_json() {
        let args = json!({ "cwd": "/w", "repo": "acme/site", "all": false, "query": "" });
        assert_eq!(repo(&Recorded, &args).unwrap(), json!("acme/site"));
        let issues = list(&Recorded, Kind::Issue, &args).unwrap();
        assert_eq!(issues[0]["number"], 12);
        assert_eq!(issues[0]["state"], "open");
        assert!(issues[0]["updatedMs"].as_i64().unwrap() > 0);
        let prs = list(&Recorded, Kind::Pr, &args).unwrap();
        assert_eq!(prs[1]["state"], "draft");

        let args = json!({ "cwd": "/w", "repo": "acme/site", "number": 31 });
        let pr = detail(&Recorded, Kind::Pr, &args).unwrap();
        assert_eq!(pr["head"], "fix-header");
        assert_eq!(pr["reviewComments"].as_array().unwrap().len(), 2);
        assert_eq!(pr_diff(&Recorded, &args).unwrap(), json!(DIFF_31));
        let sent = send_text(&Recorded, Kind::Pr, &args).unwrap();
        assert!(
            sent.as_str()
                .unwrap()
                .contains("<github-pr repo=\"acme/site\"")
        );
    }

    #[test]
    fn a_failure_carries_a_code_the_view_can_explain() {
        let args = json!({ "cwd": "/w" });
        let error = repo(&FailsWith(GhError::NotAuth), &args).unwrap_err();
        assert_eq!(error.code.as_deref(), Some("NotAuth"));
        let error = repo(&FailsWith(GhError::Missing), &args).unwrap_err();
        assert_eq!(error.code.as_deref(), Some("GhMissing"));
        let error = repo(&Recorded, &json!({})).unwrap_err();
        assert!(error.message.contains("cwd"));
    }

    struct FailsWith(GhError);

    impl Gh for FailsWith {
        fn run(&self, _cwd: &Path, _args: &[String]) -> Result<String, GhError> {
            Err(self.0.clone())
        }
    }

    #[test]
    fn the_real_runner_cannot_write() {
        // The commands never build a write, and the runner refuses one that someone adds.
        let args = ["issue", "comment", "1", "--body", "x"].map(String::from);
        assert!(GhCli.run(Path::new("."), &args).is_err());
    }
}
