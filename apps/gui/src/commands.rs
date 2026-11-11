//! The commands the UI calls with `invoke`. Each answers with JSON; none touches the UI.

use std::path::Path;
use std::process::Command;

use serde_json::{Value, json};
use svelte_gpui::{CommandError, Commands};

use crate::{clipboard, desktop, git, github, workspace};

pub fn all() -> Commands {
    let commands = Commands::new()
        .command("startup", |_| Ok::<_, CommandError>(startup()))
        .command("daemon_address", |_| daemon_address(&daemon_info_path()))
        .command("git_branch", |args| {
            Ok::<_, CommandError>(json!(git_branch(args["cwd"].as_str().unwrap_or(""))))
        })
        .command("mentions", |args| {
            let cwd = args["cwd"].as_str().unwrap_or("");
            let query = args["query"].as_str().unwrap_or("");
            let folders = args["folders"].as_bool().unwrap_or(false);
            Ok::<_, CommandError>(json!(mentions(cwd, query, folders)))
        })
        // A background command's output file (neo_jobs): read on from an offset, or its command
        // stopped. Only `….output` files, never any other path the page names.
        .command("job_read", |args| {
            let path = job_path(&args)?;
            let offset = args["offset"].as_u64().unwrap_or(0);
            Ok::<_, CommandError>(json!(neo_jobs::read_from(&path, offset)))
        })
        .command("job_stop", |args| {
            let path = job_path(&args)?;
            neo_jobs::stop(&path)
                .map(|groups| json!(groups))
                .map_err(CommandError::from)
        });
    clipboard::register(desktop::register(workspace::register(github::register(
        git::register(commands),
    ))))
}

/// Where the window was started and which session it should open: `neo gui` passes
/// `--session` and `--new` on as `NEO_GUI_SESSION` and `NEO_GUI_NEW=1`, since the host's
/// command line belongs to svelte-gpui.
fn startup() -> Value {
    let cwd = std::env::current_dir()
        .map(|dir| dir.display().to_string())
        .unwrap_or_default();
    let session = std::env::var("NEO_GUI_SESSION")
        .ok()
        .filter(|id| !id.is_empty());
    let new = std::env::var("NEO_GUI_NEW").is_ok_and(|value| value == "1");
    json!({ "cwd": cwd, "session": session, "new": new })
}

/// `daemon.json` sits next to the daemon's socket.
fn daemon_info_path() -> std::path::PathBuf {
    let socket = neo_sdk::default_socket_path();
    socket.with_file_name("daemon.json")
}

/// The daemon's loopback URL and token, read fresh each time: both change when it restarts.
/// Fails with the code `NotRunning` when there is no daemon.
pub fn daemon_address(path: &Path) -> Result<Value, CommandError> {
    let raw = std::fs::read_to_string(path).map_err(|_| {
        CommandError::with_code(
            "NotRunning",
            format!("neo daemon is not running (no {})", path.display()),
        )
    })?;
    let info: Value = serde_json::from_str(&raw)
        .map_err(|_| format!("{} is not a daemon info file", path.display()))?;
    match (info["url"].as_str(), info["token"].as_str()) {
        (Some(url), Some(token)) => Ok(json!({ "url": url, "token": token })),
        _ => Err(format!("{} is not a daemon info file", path.display()).into()),
    }
}

/// `args.path` when it is a background command's output file; refused otherwise.
fn job_path(args: &Value) -> Result<std::path::PathBuf, CommandError> {
    let path = std::path::PathBuf::from(args["path"].as_str().unwrap_or(""));
    if neo_jobs::is_output_file(&path) {
        Ok(path)
    } else {
        Err(format!("{} is not a background command's output", path.display()).into())
    }
}

/// The files (or with `folders` the folders) an `@query` could be in a session working in `cwd`:
/// the directory listed, or everything under it matched loosely (neo_mention). Nothing without a
/// directory.
pub fn mentions(cwd: &str, query: &str, folders: bool) -> Vec<neo_mention::Candidate> {
    static FINDER: std::sync::OnceLock<neo_mention::Finder> = std::sync::OnceLock::new();
    if cwd.is_empty() {
        return Vec::new();
    }
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    FINDER.get_or_init(neo_mention::Finder::default).candidates(
        Path::new(cwd),
        home.as_deref(),
        query,
        folders,
    )
}

/// The checked-out branch in `cwd`, or null outside a repository or on a detached head.
pub fn git_branch(cwd: &str) -> Option<String> {
    if cwd.is_empty() {
        return None;
    }
    let output = Command::new("git")
        .args(["-C", cwd, "branch", "--show-current"])
        .output()
        .ok()?;
    let branch = String::from_utf8(output.stdout).ok()?.trim().to_string();
    (output.status.success() && !branch.is_empty()).then_some(branch)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_address_from_daemon_json() {
        let dir = std::env::temp_dir().join(format!("neo-gui-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("daemon.json");
        std::fs::write(
            &path,
            r#"{"url":"http://127.0.0.1:4000","token":"t","pid":1}"#,
        )
        .unwrap();
        assert_eq!(
            daemon_address(&path).unwrap(),
            json!({ "url": "http://127.0.0.1:4000", "token": "t" })
        );
        std::fs::write(&path, r#"{"url":1}"#).unwrap();
        assert!(daemon_address(&path).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_missing_daemon_json_means_not_running() {
        let error = daemon_address(Path::new("/nonexistent/daemon.json")).unwrap_err();
        assert_eq!(error.code.as_deref(), Some("NotRunning"));
    }

    #[test]
    fn no_branch_outside_a_repository() {
        assert_eq!(git_branch(""), None);
        assert_eq!(git_branch("/"), None);
    }
}
