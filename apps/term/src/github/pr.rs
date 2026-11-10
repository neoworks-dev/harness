//! Reviewing a pull request: its diff a file at a time with the review comments on their lines
//! and the whole file on `f`, and the logs of its checks. Pure functions over what `gh` gave;
//! `mod.rs` fetches and opens the views.

use super::model::{Check, CheckState, ReviewComment, short_age};
use crate::widgets::Remark;

/// The pull request's diff cut into its files: each file's path (the new one, the old one
/// for a deleted file) and its part of the diff, headers and all.
pub(super) fn split_diff(text: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for line in text.split_inclusive('\n') {
        if line.starts_with("diff --git ") || out.is_empty() {
            out.push((String::new(), String::new()));
        }
        let (path, section) = out.last_mut().expect("pushed above");
        section.push_str(line);
        let l = line.trim_end_matches('\n');
        if let Some(p) = l.strip_prefix("+++ b/") {
            *path = p.to_string();
        } else if let Some(p) = l.strip_prefix("--- a/")
            && path.is_empty()
        {
            *path = p.to_string();
        } else if path.is_empty()
            && let Some(rest) = l.strip_prefix("diff --git a/")
            && let Some((_, b)) = rest.split_once(" b/")
        {
            // Until `+++` says otherwise (a rename, or a file with no lines changed).
            *path = b.to_string();
        }
    }
    out.retain(|(path, _)| !path.is_empty());
    out
}

/// A hunk header's numbers: `@@ -a,b +c,d @@` (a missing count is 1).
fn hunk_numbers(line: &str) -> Option<(usize, usize, usize, usize)> {
    let rest = line.strip_prefix("@@ -")?;
    let (old, rest) = rest.split_once(" +")?;
    let (new, _) = rest.split_once(" @@")?;
    let pair = |s: &str| -> Option<(usize, usize)> {
        match s.split_once(',') {
            Some((a, b)) => Some((a.parse().ok()?, b.parse().ok()?)),
            None => Some((s.parse().ok()?, 1)),
        }
    };
    let (a, b) = pair(old)?;
    let (c, d) = pair(new)?;
    Some((a, b, c, d))
}

/// One file's diff with the whole new file around its changes: the lines between the hunks
/// (the same in both versions) come from `new_file`, and it all becomes one hunk.
pub(super) fn expand(section: &str, new_file: &str) -> String {
    let file: Vec<&str> = new_file.lines().collect();
    let mut head = String::new();
    let mut body = String::new();
    let (mut adds, mut dels) = (0usize, 0usize);
    // The next line of the new file not written yet, from 1.
    let mut next = 1usize;
    let mut in_hunks = false;
    let context = |body: &mut String, from: usize, to: usize| {
        for no in from..to {
            if let Some(text) = file.get(no - 1) {
                body.push(' ');
                body.push_str(text);
                body.push('\n');
            }
        }
    };
    for line in section.lines() {
        if let Some((_, _, c, d)) = hunk_numbers(line) {
            in_hunks = true;
            // A hunk that only removes (`+c,0`) comes after line c.
            let start = if d == 0 { c + 1 } else { c };
            context(&mut body, next, start.max(next));
            next = start.max(next);
            continue;
        }
        if !in_hunks {
            head.push_str(line);
            head.push('\n');
            continue;
        }
        match line.chars().next() {
            Some('+') => {
                adds += 1;
                next += 1;
            }
            Some('-') => dels += 1,
            Some('\\') => {}
            _ => next += 1,
        }
        body.push_str(line);
        body.push('\n');
    }
    if !in_hunks {
        return section.to_string();
    }
    context(&mut body, next, file.len() + 1);
    let new_len = file.len();
    let old_len = (new_len + dels).saturating_sub(adds);
    format!("{head}@@ -1,{old_len} +1,{new_len} @@\n{body}")
}

/// Whether the diff is all of the file already: a file added or deleted.
pub(super) fn is_whole(section: &str) -> bool {
    section
        .lines()
        .take_while(|l| !l.starts_with("@@"))
        .any(|l| l.starts_with("new file mode") || l.starts_with("deleted file mode"))
}

/// The review comments on `path` as remarks on their lines, oldest first; a reply says so.
pub(super) fn remarks(comments: &[ReviewComment], path: &str, now_ms: i64) -> Vec<Remark> {
    comments
        .iter()
        .filter(|c| c.path == path)
        .filter_map(|c| {
            let line = usize::try_from(c.line?).ok()?;
            let mut head = format!("{}{}", if c.reply { "↳ " } else { "" }, c.author);
            let age = short_age(c.at_ms, now_ms);
            if !age.is_empty() {
                head.push_str(&format!(" · {age} ago"));
            }
            if c.outdated {
                head.push_str(" · outdated");
            }
            Some(Remark {
                line,
                old: c.old,
                head,
                text: c.body.clone(),
            })
        })
        .collect()
}

/// How many review comments are on `path`.
pub(super) fn remark_count(comments: &[ReviewComment], path: &str) -> usize {
    comments.iter().filter(|c| c.path == path && c.line.is_some()).count()
}

/// The checks in the order the detail lists them: failed, pending, passed, skipped; by name
/// within each.
pub(super) fn check_order(checks: &[Check]) -> Vec<usize> {
    let rank = |s: CheckState| match s {
        CheckState::Fail => 0,
        CheckState::Pending => 1,
        CheckState::Pass => 2,
        CheckState::Skipped => 3,
    };
    let mut order: Vec<usize> = (0..checks.len()).collect();
    order.sort_by(|&a, &b| {
        (rank(checks[a].state), &checks[a].name).cmp(&(rank(checks[b].state), &checks[b].name))
    });
    order
}

/// How long a check ran: `3m 20s`, `45s`, `1h 2m`; empty when not known.
pub(super) fn took(c: &Check) -> String {
    if c.started_ms <= 0 || c.finished_ms < c.started_ms {
        return String::new();
    }
    let s = (c.finished_ms - c.started_ms) / 1000;
    match s {
        0..60 => format!("{s}s"),
        60..3600 => format!("{}m {}s", s / 60, s % 60),
        _ => format!("{}h {}m", s / 3600, s % 3600 / 60),
    }
}

/// A job's log as `gh` prints it (`job\tstep\ttime text` a line) made readable: the step
/// as a heading when it changes, the time and the job dropped, colors and control characters
/// gone, GitHub's `##[group]` markers as plain words.
pub(super) fn tidy_log(raw: &str) -> String {
    let mut out = String::new();
    let mut step = None::<String>;
    for line in raw.lines() {
        let mut parts = line.splitn(3, '\t');
        let (text, this_step) = match (parts.next(), parts.next(), parts.next()) {
            (Some(_job), Some(s), Some(rest)) => {
                // `2026-10-16T14:00:01.1234567Z text`: the time goes.
                let text = match rest.split_once(' ') {
                    Some((time, text)) if time.ends_with('Z') && time.contains('T') => text,
                    _ if rest.ends_with('Z') && rest.contains('T') => "",
                    _ => rest,
                };
                (text, Some(s))
            }
            _ => (line, None),
        };
        if let Some(s) = this_step
            && step.as_deref() != Some(s)
        {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&format!("── {s}\n"));
            step = Some(s.to_string());
        }
        let mut sgr = crate::widgets::Sgr::default();
        let plain = crate::widgets::plain(&crate::widgets::parse_line(text, &mut sgr));
        let plain = plain.trim_start_matches('\u{feff}');
        let plain = plain
            .strip_prefix("##[group]")
            .or_else(|| plain.strip_prefix("##[endgroup]"))
            .unwrap_or(plain);
        let plain = match plain.strip_prefix("##[error]") {
            Some(rest) => format!("error: {rest}"),
            None => plain.to_string(),
        };
        out.push_str(&plain);
        out.push('\n');
    }
    out.trim_end_matches('\n').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use neo_github::fixtures::{DIFF_31, PR_31, PR_31_COMMENTS};
    use neo_github::model::{Kind, parse_detail, parse_review_comments};

    #[test]
    fn the_diff_is_cut_into_its_files() {
        let files = split_diff(DIFF_31);
        let paths: Vec<&str> = files.iter().map(|(p, _)| p.as_str()).collect();
        assert_eq!(paths, ["src/header.css", "src/layout.css"]);
        assert!(files[0].1.starts_with("diff --git a/src/header.css"));
        assert!(files[0].1.contains("scrollbar-gutter") && !files[0].1.contains("margin"));
        assert!(files[1].1.contains("+  margin: 0;"));
        // A deleted file is known by its old name.
        let gone = "diff --git a/old.txt b/old.txt\ndeleted file mode 100644\n--- a/old.txt\n+++ /dev/null\n@@ -1 +0,0 @@\n-bye\n";
        assert_eq!(split_diff(gone)[0].0, "old.txt");
        assert!(is_whole(gone) && !is_whole(DIFF_31));
    }

    #[test]
    fn a_file_is_expanded_to_all_its_lines_around_the_changes() {
        let section = "--- a/f\n+++ b/f\n@@ -2,3 +2,3 @@\n two\n-three\n+THREE\n four\n@@ -7,1 +7,2 @@\n seven\n+eight\n";
        let file = "one\ntwo\nTHREE\nfour\nfive\nsix\nseven\neight\nnine\n";
        let full = expand(section, file);
        assert_eq!(
            full,
            "--- a/f\n+++ b/f\n@@ -1,8 +1,9 @@\n one\n two\n-three\n+THREE\n four\n five\n six\n seven\n+eight\n nine\n"
        );
        // Only removing: the hunk comes after its line.
        let section = "--- a/f\n+++ b/f\n@@ -2,1 +1,0 @@\n-gone\n";
        assert_eq!(expand(section, "kept\nlast\n"), "--- a/f\n+++ b/f\n@@ -1,3 +1,2 @@\n kept\n-gone\n last\n");
    }

    #[test]
    fn review_comments_become_remarks_on_their_files_lines() {
        let comments = parse_review_comments(PR_31_COMMENTS);
        let r = remarks(&comments, "src/header.css", 0);
        assert_eq!(r.len(), 3);
        assert_eq!((r[0].line, r[0].old), (3, false));
        assert!(r[0].head.starts_with("alice") && r[1].head.starts_with("↳ bob"));
        assert!(r[2].old);
        assert!(remarks(&comments, "src/layout.css", 0).is_empty());
        assert_eq!(remark_count(&comments, "src/header.css"), 3);
    }

    #[test]
    fn failed_checks_come_first_and_say_how_long_they_took() {
        let d = parse_detail(Kind::Pr, PR_31).unwrap();
        let order = check_order(&d.checks);
        let names: Vec<&str> = order.iter().map(|&i| d.checks[i].name.as_str()).collect();
        assert_eq!(names, ["test (ubuntu)", "test (macos)", "ci/deploy", "lint", "docs"]);
        assert_eq!(took(&d.checks[order[0]]), "3m 20s");
        assert_eq!(took(&d.checks[order[1]]), "");
    }

    #[test]
    fn a_log_loses_its_times_and_gains_its_steps() {
        let raw = "build\tRun tests\t2026-10-16T14:00:01.1234567Z \u{1b}[31mFAIL\u{1b}[0m header\nbuild\tRun tests\t2026-10-16T14:00:02.0000000Z ##[error]Process completed with exit code 1.\nbuild\tPost\t2026-10-16T14:00:03.0000000Z ##[group]Cleaning up";
        assert_eq!(
            tidy_log(raw),
            "── Run tests\nFAIL header\nerror: Process completed with exit code 1.\n\n── Post\nCleaning up"
        );
    }
}
