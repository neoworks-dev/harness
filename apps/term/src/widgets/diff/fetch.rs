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

/// A permission request's diff over the whole file, as a diff of one hunk: the file on disk is
/// the old version (an edit asked about) or the new one (an edit review mode already wrote),
/// whichever the diff's lines agree with; the rest of it is context. A file that agrees with
/// neither (changed since) shows the request's diff as it is.
pub(super) fn proposed_diff(
    cwd: &Path,
    rel: &str,
    lines: &[neo_protocol::DiffLine],
) -> Result<String, String> {
    let path = cwd.join(rel);
    let disk = match std::fs::metadata(&path) {
        Ok(meta) if meta.len() > FILE_LIMIT => None,
        Ok(_) => std::fs::read(&path)
            .ok()
            .filter(|b| !b.contains(&0))
            .map(|b| String::from_utf8_lossy(&b).into_owned()),
        // A file the edit creates.
        Err(_) => Some(String::new()),
    };
    let disk: Vec<&str> = disk
        .as_deref()
        .map_or_else(Vec::new, |d| d.lines().collect());
    // Numbered from the file's top, or (Claude Code's edits: the replaced text and its
    // replacement) from the snippet's, found in the file.
    let body = [true, false]
        .into_iter()
        .find_map(|old| {
            splice(&disk, lines, old).or_else(|| {
                let by = locate(&disk, lines, old)?;
                splice(&disk, &shifted(lines, by), old)
            })
        })
        .map(|rows| {
            let old = rows.iter().filter(|r| !r.starts_with('+')).count();
            let new = rows.iter().filter(|r| !r.starts_with('-')).count();
            format!("@@ -1,{old} +1,{new} @@\n{}", rows.join("\n"))
        })
        .unwrap_or_else(|| as_is(lines));
    if body.trim().is_empty() {
        return Err("the edit changes nothing".into());
    }
    Ok(format!("--- a/{rel}\n+++ b/{rel}\n{body}\n"))
}

/// How far the diff's lines of the base (the old file's, or the new one's) are from where they
/// first occur together in `disk`, in lines; None when they don't or are where they say.
fn locate(disk: &[&str], lines: &[neo_protocol::DiffLine], old: bool) -> Option<i64> {
    use neo_protocol::DiffLineKind as K;
    let base: Vec<&neo_protocol::DiffLine> = lines
        .iter()
        .filter(|l| {
            matches!(
                (l.kind, old),
                (K::Context, _) | (K::Remove, true) | (K::Add, false)
            )
        })
        .collect();
    let first = base.first().and_then(|l| if old { l.old } else { l.new })?;
    let at = disk.windows(base.len()).position(|w| {
        w.iter()
            .zip(&base)
            .all(|(d, l)| d.trim_end_matches('\r') == l.text.trim_end_matches(['\r', '\n']))
    })?;
    let by = i64::try_from(at).ok()? + 1 - first;
    (by != 0).then_some(by)
}

/// The diff's lines, their numbers moved by `by`.
fn shifted(lines: &[neo_protocol::DiffLine], by: i64) -> Vec<neo_protocol::DiffLine> {
    lines
        .iter()
        .map(|l| neo_protocol::DiffLine {
            old: l.old.map(|n| n + by),
            new: l.new.map(|n| n + by),
            ..l.clone()
        })
        .collect()
}

/// The diff's lines placed in `disk`, the old file (`old`) or the new one, as `+`/`-`/` ` rows;
/// None when a line the base has doesn't match it.
fn splice(disk: &[&str], lines: &[neo_protocol::DiffLine], old: bool) -> Option<Vec<String>> {
    use neo_protocol::DiffLineKind as K;
    let no = |n: Option<i64>| n.and_then(|n| usize::try_from(n).ok());
    let same = |at: usize, text: &str| {
        disk.get(at.checked_sub(1)?)
            .is_some_and(|d| d.trim_end_matches('\r') == text.trim_end_matches(['\r', '\n']))
            .then_some(())
    };
    let mut rows = Vec::new();
    // The next line of the old and the new file.
    let (mut o, mut n) = (1usize, 1usize);
    for l in lines {
        if l.kind == K::Hunk {
            continue;
        }
        // Where the line is in the base: its own number there, or worked out from the other's.
        let at = if old {
            no(l.old).or_else(|| (no(l.new)? + o).checked_sub(n))?
        } else {
            no(l.new).or_else(|| (no(l.old)? + n).checked_sub(o))?
        };
        loop {
            let b = if old { o } else { n };
            if b >= at {
                break;
            }
            rows.push(format!(" {}", disk.get(b - 1)?));
            (o, n) = (o + 1, n + 1);
        }
        // A line of the base out of order: not this base.
        let in_base = matches!(
            (l.kind, old),
            (K::Context, _) | (K::Remove, true) | (K::Add, false)
        );
        if in_base && (if old { o } else { n }) != at {
            return None;
        }
        let text = l.text.trim_end_matches(['\r', '\n']);
        match l.kind {
            K::Context => {
                same(at, text)?;
                rows.push(format!(" {text}"));
                (o, n) = (o + 1, n + 1);
            }
            K::Remove => {
                if old {
                    same(at, text)?;
                }
                rows.push(format!("-{text}"));
                o += 1;
            }
            K::Add => {
                if !old {
                    same(at, text)?;
                }
                rows.push(format!("+{text}"));
                n += 1;
            }
            K::Hunk => {}
        }
    }
    let b = if old { o } else { n };
    rows.extend(disk.iter().skip(b - 1).map(|d| format!(" {d}")));
    Some(rows)
}

/// The request's diff by itself, a hunk header before each run.
fn as_is(lines: &[neo_protocol::DiffLine]) -> String {
    use neo_protocol::DiffLineKind as K;
    let mut out = Vec::new();
    let mut open = false;
    for l in lines {
        let text = l.text.trim_end_matches(['\r', '\n']);
        let sign = match l.kind {
            K::Hunk => {
                open = false;
                continue;
            }
            K::Context => ' ',
            K::Add => '+',
            K::Remove => '-',
        };
        if !open {
            let (o, n) = (l.old.unwrap_or(1), l.new.unwrap_or(1));
            out.push(format!("@@ -{o} +{n} @@"));
            open = true;
        }
        out.push(format!("{sign}{text}"));
    }
    out.join("\n")
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
