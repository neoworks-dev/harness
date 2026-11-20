//! The GitHub view's data: issues and pull requests of the session's repository, read through
//! `neo-github`, and the few writes a button in the view asks for (a comment, closing and
//! reopening, a reaction), which go through its allowlist as the terminal app's do.

use std::path::Path;

use neo_github::fetch;
use neo_github::format;
use neo_github::gh::{CloseReason, Gh, GhCli, GhError, ReactOn};
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

/// The repository's labels with their colors, so the view draws a label as GitHub does.
pub fn labels(gh: &dyn Gh, args: &Value) -> Result<Value, CommandError> {
    let cwd = text(args, "cwd")?;
    let repo = text(args, "repo")?;
    json_of(&fetch::labels(gh, Path::new(cwd), repo).map_err(failure)?)
}

fn kind_of(args: &Value) -> Result<Kind, CommandError> {
    match text(args, "kind")? {
        "issue" => Ok(Kind::Issue),
        "pr" => Ok(Kind::Pr),
        other => Err(CommandError::new(format!("unknown kind {other}"))),
    }
}

// Writes: each one thing the user pressed a button for, done as the user `gh` is logged in as,
// through the crate's allowlist (`neo_github::gh::is_allowed_write`), as the terminal app does.

/// Adds the user's comment to an issue or pull request.
pub fn comment(gh: &dyn Gh, args: &Value) -> Result<Value, CommandError> {
    let body = text(args, "body")?;
    if body.trim().is_empty() {
        return Err(CommandError::new("the comment is empty"));
    }
    fetch::comment(
        gh,
        Path::new(text(args, "cwd")?),
        kind_of(args)?,
        text(args, "repo")?,
        number(args)?,
        body,
    )
    .map_err(failure)?;
    Ok(Value::Null)
}

/// Closes an issue (`reason`: `completed` or `not planned`) or a pull request.
pub fn close(gh: &dyn Gh, args: &Value) -> Result<Value, CommandError> {
    let reason = match args["reason"].as_str().unwrap_or("completed") {
        "not planned" => CloseReason::NotPlanned,
        _ => CloseReason::Completed,
    };
    fetch::close(
        gh,
        Path::new(text(args, "cwd")?),
        kind_of(args)?,
        text(args, "repo")?,
        number(args)?,
        reason,
    )
    .map_err(failure)?;
    Ok(Value::Null)
}

pub fn reopen(gh: &dyn Gh, args: &Value) -> Result<Value, CommandError> {
    fetch::reopen(
        gh,
        Path::new(text(args, "cwd")?),
        kind_of(args)?,
        text(args, "repo")?,
        number(args)?,
    )
    .map_err(failure)?;
    Ok(Value::Null)
}

/// Gives a reaction (`content`: `+1`, `heart`…) to the item, or with `comment` to one of its
/// comments, or takes it back: true when given.
pub fn react(gh: &dyn Gh, args: &Value) -> Result<Value, CommandError> {
    let on = match args["comment"].as_u64() {
        Some(id) => ReactOn::Comment(id),
        None => ReactOn::Item(number(args)?),
    };
    let given = fetch::toggle_reaction(
        gh,
        Path::new(text(args, "cwd")?),
        text(args, "repo")?,
        on,
        text(args, "content")?,
    )
    .map_err(failure)?;
    Ok(json!(given))
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
        .command("gh_labels", |args| labels(&GhCli, &args))
        .command("gh_comment", |args| comment(&GhCli, &args))
        .command("gh_close", |args| close(&GhCli, &args))
        .command("gh_reopen", |args| reopen(&GhCli, &args))
        .command("gh_react", |args| react(&GhCli, &args))
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

    /// Takes the writes it is given (each must be one the allowlist knows) and their input.
    #[derive(Default)]
    struct Writes(std::sync::Mutex<Vec<(String, Option<String>)>>);

    impl Gh for Writes {
        fn run(&self, _cwd: &Path, _args: &[String]) -> Result<String, GhError> {
            Err(GhError::NotRepo)
        }
        fn write(
            &self,
            _cwd: &Path,
            args: &[String],
            input: Option<&str>,
        ) -> Result<String, GhError> {
            assert!(neo_github::gh::is_allowed_write(args), "{args:?}");
            self.0
                .lock()
                .unwrap()
                .push((args.join(" "), input.map(String::from)));
            Ok(String::new())
        }
    }

    #[test]
    fn the_view_s_buttons_write_through_the_allowlist() {
        let gh = Writes::default();
        let at = json!({ "cwd": "/w", "repo": "acme/site", "kind": "issue", "number": 12 });
        comment(&gh, &json!({ "cwd": "/w", "repo": "acme/site", "kind": "issue", "number": 12, "body": "Looks good" })).unwrap();
        close(&gh, &json!({ "cwd": "/w", "repo": "acme/site", "kind": "issue", "number": 12, "reason": "not planned" })).unwrap();
        reopen(&gh, &at).unwrap();
        let writes = gh.0.lock().unwrap();
        assert_eq!(writes[0].1.as_deref(), Some("Looks good"));
        assert!(writes[1].0.contains("close 12") && writes[1].0.contains("not planned"));
        assert!(writes[2].0.contains("reopen 12"));
        // An empty comment is never sent.
        drop(writes);
        let empty =
            json!({ "cwd": "/w", "repo": "acme/site", "kind": "pr", "number": 31, "body": "  " });
        assert!(comment(&gh, &empty).is_err());
        assert_eq!(gh.0.lock().unwrap().len(), 3);
    }

    #[test]
    fn the_real_runner_cannot_write() {
        // The commands never build a write, and the runner refuses one that someone adds.
        let args = ["issue", "comment", "1", "--body", "x"].map(String::from);
        assert!(GhCli.run(Path::new("."), &args).is_err());
    }
}
