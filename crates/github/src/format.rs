//! The text block that goes to the model: one issue or pull request with its conversation,
//! compact and bounded. The newest comments are kept and the oldest dropped, with a note.

use crate::model::{CheckState, Detail, Kind, date};
use crate::strip_comments;

/// The whole block stays under about this many characters.
pub const MAX_CHARS: usize = 12_000;
/// The description, and each comment, is cut to this.
const MAX_BODY: usize = 4_000;
const MAX_ITEM: usize = 1_500;
/// Files and failing checks named.
const MAX_FILES: usize = 40;
const MAX_NAMED_CHECKS: usize = 8;

/// `<github-issue …>…</github-issue>` (`github-pr` for a pull request).
pub fn block(repo: &str, d: &Detail) -> String {
    let tag = match d.kind {
        Kind::Issue => "github-issue",
        Kind::Pr => "github-pr",
    };
    let mut head = format!(
        "<{tag} repo=\"{}\" number=\"{}\" url=\"{}\" state=\"{}\">\nTitle: {}\n",
        attr(repo),
        d.number,
        attr(&d.url),
        d.state.word(),
        guard(&d.title),
    );
    let mut meta = vec![format!("Author: {}", d.author)];
    if !d.labels.is_empty() {
        meta.push(format!("Labels: {}", d.labels.join(", ")));
    }
    if d.created_ms > 0 {
        meta.push(format!("Opened: {}", date(d.created_ms)));
    }
    if d.updated_ms > 0 {
        meta.push(format!("Updated: {}", date(d.updated_ms)));
    }
    head.push_str(&guard(&meta.join(" · ")));
    head.push('\n');
    if d.kind == Kind::Pr {
        head.push_str(&pr_facts(d));
    }
    head.push_str("Description:\n");
    let body = strip_comments(&d.body);
    let body = body.trim();
    if body.is_empty() {
        head.push_str("(none)\n");
    } else {
        head.push_str(&guard(&cut(body, MAX_BODY)));
        head.push('\n');
    }

    let tail = format!("</{tag}>");
    let entries = timeline(d);
    let budget = MAX_CHARS.saturating_sub(head.chars().count() + tail.chars().count() + 80);
    let mut kept: Vec<&str> = Vec::new();
    let mut used = 0;
    for e in entries.iter().rev() {
        let n = e.chars().count() + 1;
        if used + n > budget && !kept.is_empty() {
            break;
        }
        kept.push(e);
        used += n;
    }
    kept.reverse();
    let dropped = entries.len() - kept.len();
    if !entries.is_empty() {
        let note = if dropped > 0 {
            format!(
                "Conversation ({} of {}, {dropped} oldest omitted):\n",
                kept.len(),
                entries.len()
            )
        } else {
            format!("Conversation ({}):\n", entries.len())
        };
        head.push_str(&note);
        for e in kept {
            head.push_str(e);
            head.push('\n');
        }
    }
    head.push_str(&tail);
    head
}

/// The text of "send now": a line of instruction, then the block.
pub fn send_text(repo: &str, d: &Detail) -> String {
    format!(
        "Here is {} #{} from GitHub.\n\n{}",
        d.kind.noun(),
        d.number,
        block(repo, d)
    )
}

/// Branch, size, review decision, checks and files of a pull request.
fn pr_facts(d: &Detail) -> String {
    let mut out = String::new();
    if !d.head.is_empty() || !d.base.is_empty() {
        out.push_str(&format!(
            "Branch: {} -> {} · +{} -{} in {} files",
            guard(&d.head),
            guard(&d.base),
            d.additions,
            d.deletions,
            d.files.len()
        ));
        if !d.decision.is_empty() {
            out.push_str(&format!(" · Review: {}", d.decision));
        }
        out.push('\n');
    }
    if !d.checks.is_empty() {
        let c = d.check_counts();
        out.push_str(&format!(
            "Checks: {} passed, {} failed, {} pending",
            c.pass, c.fail, c.pending
        ));
        let named: Vec<&str> = d
            .checks
            .iter()
            .filter(|c| c.state == CheckState::Fail)
            .map(|c| c.name.as_str())
            .take(MAX_NAMED_CHECKS)
            .collect();
        if !named.is_empty() {
            out.push_str(&format!(" (failing: {})", guard(&named.join(", "))));
        }
        out.push('\n');
    }
    if !d.files.is_empty() {
        out.push_str("Files:\n");
        for f in d.files.iter().take(MAX_FILES) {
            out.push_str(&format!(
                "  {} (+{} -{})\n",
                guard(&f.path),
                f.additions,
                f.deletions
            ));
        }
        if d.files.len() > MAX_FILES {
            out.push_str(&format!("  … and {} more\n", d.files.len() - MAX_FILES));
        }
    }
    out
}

/// Comments, reviews and review comments in time order, one text each.
fn timeline(d: &Detail) -> Vec<String> {
    let mut items: Vec<(i64, String)> = Vec::new();
    for c in &d.comments {
        items.push((c.at_ms, entry(&c.author, c.at_ms, "", &c.body)));
    }
    for r in &d.reviews {
        let commented = r.state.eq_ignore_ascii_case("COMMENTED");
        if commented && r.body.trim().is_empty() {
            continue;
        }
        let what = format!("review {}", r.state.to_lowercase().replace('_', " "));
        items.push((r.at_ms, entry(&r.author, r.at_ms, &what, &r.body)));
    }
    for c in &d.review_comments {
        let place = match c.line {
            Some(l) => format!("on {}:{l}", c.path),
            None => format!("on {}", c.path),
        };
        items.push((c.at_ms, entry(&c.author, c.at_ms, &place, &c.body)));
    }
    items.sort_by_key(|(at, _)| *at);
    items.into_iter().map(|(_, text)| text).collect()
}

/// `alice (2026-10-16, review approved): text`.
fn entry(author: &str, at_ms: i64, what: &str, body: &str) -> String {
    let mut who = author.to_string();
    let mut parts: Vec<String> = Vec::new();
    if at_ms > 0 {
        parts.push(date(at_ms));
    }
    if !what.is_empty() {
        parts.push(what.to_string());
    }
    if !parts.is_empty() {
        who.push_str(&format!(" ({})", parts.join(", ")));
    }
    let body = strip_comments(body);
    let body = body.trim();
    if body.is_empty() {
        return guard(&format!("{who}:"));
    }
    guard(&format!("{who}: {}", cut(body, MAX_ITEM)))
}

/// `s` cut to `max` characters with a note when something went.
fn cut(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((i, _)) => format!("{} … [{} more characters]", &s[..i], s[i..].chars().count()),
        None => s.to_string(),
    }
}

/// Text from GitHub, without HTML comments (templates are full of them), and unable to close
/// the block it is in.
fn guard(s: &str) -> String {
    strip_comments(s).replace("</github-", "<\\/github-")
}

fn attr(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('\n', " ")
}
