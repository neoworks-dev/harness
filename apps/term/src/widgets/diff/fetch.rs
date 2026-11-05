use super::*;

/// Starts `git diff` on a thread.
pub(super) fn spawn(
    cwd: PathBuf,
    req: DiffRequest,
    full: bool,
) -> Receiver<Result<String, String>> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let loaded = load(&cwd, &req, full);
        // The whole file asked for, and git sees no change in it: the file as it is.
        let loaded = match loaded {
            Ok(text) if full && text.trim().is_empty() => file_as_diff(&cwd, &req.rel),
            other => other,
        };
        let _ = tx.send(loaded);
    });
    rx
}

/// Files larger than this aren't shown whole.
pub(super) const FILE_LIMIT: u64 = 2 << 20;

/// A file without changes, as a diff of only context lines, so the view shows it like any other.
pub(super) fn file_as_diff(cwd: &Path, rel: &str) -> Result<String, String> {
    let path = cwd.join(rel);
    let meta = std::fs::metadata(&path).map_err(|e| format!("{rel}: {e}"))?;
    if meta.len() > FILE_LIMIT {
        return Err(format!("{rel} is larger than 2 MB"));
    }
    let bytes = std::fs::read(&path).map_err(|e| format!("{rel}: {e}"))?;
    if bytes.contains(&0) {
        return Err(format!("{rel} is a binary file"));
    }
    let text = String::from_utf8_lossy(&bytes);
    let lines: Vec<&str> = text.lines().collect();
    let mut out = format!(
        "--- a/{rel}\n+++ b/{rel}\n@@ -1,{n} +1,{n} @@\n",
        n = lines.len()
    );
    for line in lines {
        out.push(' ');
        out.push_str(line);
        out.push('\n');
    }
    Ok(out)
}

pub(super) fn run(cmd: &mut std::process::Command, no_index: bool) -> Result<String, String> {
    let out = cmd
        .stderr(std::process::Stdio::piped())
        .output()
        .map_err(|e| format!("git: {e}"))?;
    // --no-index exits 1 when the files differ.
    let ok = out.status.success() || (no_index && out.status.code() == Some(1));
    if ok {
        return Ok(String::from_utf8_lossy(&out.stdout).into_owned());
    }
    // What git said, on one line: the view shows it in place of the diff.
    let said = String::from_utf8_lossy(&out.stderr);
    let said = said.lines().map(str::trim).find(|l| !l.is_empty());
    Err(said.map_or_else(|| "git diff failed".into(), clean_line))
}

/// The diff text of one row: against HEAD (staged and unstaged together), or the whole file
/// for untracked ones. `full` keeps the whole file as context.
/// What commit `sha` changed, a merge against its first parent.
pub(super) fn git_show(cwd: &Path, sha: &str) -> Result<String, String> {
    let context = format!("-U{CONTEXT}");
    let show = [
        "show",
        "--first-parent",
        "--format=",
        "--no-color",
        "--no-ext-diff",
    ];
    let out = run(git(cwd).args(show).arg(&context).arg(sha).arg("--"), false)?;
    if out.trim().is_empty() {
        Err(format!("{sha} changed no file"))
    } else {
        Ok(out)
    }
}

pub(super) fn load(cwd: &Path, req: &DiffRequest, full: bool) -> Result<String, String> {
    if req.rel.ends_with('/') {
        return Err("untracked directory".into());
    }
    let context = format!("-U{}", if full { FULL_CONTEXT } else { CONTEXT });
    let base = ["diff", "--no-color", "--no-ext-diff", &context];
    if req.mark == Mark::Untracked || (!req.in_repo && req.mark == Mark::Added) {
        return run(
            git(cwd)
                .args(base)
                .args(["--no-index", "--", "/dev/null"])
                .arg(&req.rel),
            true,
        );
    }
    if !req.in_repo {
        return Err("no diff outside a git repository".into());
    }
    if req.mark == Mark::Committed {
        // What the last commit that touched it changed. Asked where the file is: one outside the
        // session's repository (../other/a.rs) is in a repository of its own.
        let log = [
            "log",
            "-1",
            "--format=",
            "-p",
            "--no-color",
            "--no-ext-diff",
        ];
        let path = cwd.join(&req.rel);
        let (dir, name) = match (path.parent(), path.file_name()) {
            (Some(dir), Some(name)) if dir.is_dir() => (dir.to_path_buf(), PathBuf::from(name)),
            _ => (cwd.to_path_buf(), PathBuf::from(&req.rel)),
        };
        let out = run(
            git(&dir).args(log).arg(&context).arg("--").arg(&name),
            false,
        )?;
        return if out.trim().is_empty() {
            Err("no change left: it was changed back".into())
        } else {
            Ok(out)
        };
    }
    let mut paths = vec![req.rel.clone()];
    paths.extend(req.orig.clone());
    let has_head = git(cwd)
        .args(["rev-parse", "--verify", "-q", "HEAD"])
        .output()
        .is_ok_and(|o| o.status.success());
    if has_head {
        return run(git(cwd).args(base).args(["HEAD", "--"]).args(&paths), false);
    }
    // No commit yet: what is staged, then what isn't.
    let staged = run(
        git(cwd).args(base).args(["--cached", "--"]).args(&paths),
        false,
    )?;
    let unstaged = run(git(cwd).args(base).arg("--").args(&paths), false)?;
    Ok(staged + &unstaged)
}
