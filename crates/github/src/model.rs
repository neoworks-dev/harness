//! What `gh --json` says about issues and pull requests, parsed leniently: a missing or odd
//! field is an empty value, never an error. Only JSON that isn't the expected shape at all is
//! one. Everything that ends up on screen or in the prompt is cleaned of control characters.

use serde::Serialize;
use serde_json::Value;

/// The two kinds of thing the view lists (and its two tabs).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Issue,
    Pr,
}

impl Kind {
    pub fn other(self) -> Kind {
        match self {
            Kind::Issue => Kind::Pr,
            Kind::Pr => Kind::Issue,
        }
    }

    /// "issue", "pull request".
    pub fn noun(self) -> &'static str {
        match self {
            Kind::Issue => "issue",
            Kind::Pr => "pull request",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    Open,
    Closed,
    Merged,
    Draft,
}

impl State {
    pub fn word(self) -> &'static str {
        match self {
            State::Open => "open",
            State::Closed => "closed",
            State::Merged => "merged",
            State::Draft => "draft",
        }
    }
}

/// A row of the list.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub number: u64,
    pub title: String,
    pub state: State,
    pub author: String,
    pub labels: Vec<String>,
    pub comments: usize,
    pub updated_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Comment {
    pub author: String,
    pub at_ms: i64,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Review {
    pub author: String,
    pub at_ms: i64,
    /// APPROVED, CHANGES_REQUESTED, COMMENTED, DISMISSED, PENDING.
    pub state: String,
    pub body: String,
}

/// A comment on a line of the diff.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewComment {
    pub author: String,
    pub at_ms: i64,
    pub path: String,
    pub line: Option<u64>,
    pub body: String,
    pub reply: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CheckState {
    Pass,
    Fail,
    Pending,
    Skipped,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Check {
    pub name: String,
    pub state: CheckState,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileChange {
    pub path: String,
    pub additions: u64,
    pub deletions: u64,
}

/// One issue or pull request with everything the detail shows.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Detail {
    pub kind: Kind,
    pub number: u64,
    pub title: String,
    pub state: State,
    pub author: String,
    pub labels: Vec<String>,
    pub body: String,
    pub url: String,
    pub created_ms: i64,
    pub updated_ms: i64,
    pub comments: Vec<Comment>,
    // Pull requests only from here on.
    pub reviews: Vec<Review>,
    pub review_comments: Vec<ReviewComment>,
    pub checks: Vec<Check>,
    pub files: Vec<FileChange>,
    pub base: String,
    pub head: String,
    pub decision: String,
    pub additions: u64,
    pub deletions: u64,
}

/// Counts of the checks by outcome.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CheckCounts {
    pub pass: usize,
    pub fail: usize,
    pub pending: usize,
    pub skipped: usize,
}

impl Detail {
    pub fn check_counts(&self) -> CheckCounts {
        let mut c = CheckCounts::default();
        for check in &self.checks {
            match check.state {
                CheckState::Pass => c.pass += 1,
                CheckState::Fail => c.fail += 1,
                CheckState::Pending => c.pending += 1,
                CheckState::Skipped => c.skipped += 1,
            }
        }
        c
    }
}

// ---- parsing ----

fn json(text: &str) -> Result<Value, String> {
    serde_json::from_str(text).map_err(|e| format!("unexpected output from gh: {e}"))
}

/// `gh repo view --json nameWithOwner`.
pub fn parse_repo(text: &str) -> Result<String, String> {
    let v = json(text)?;
    v["nameWithOwner"]
        .as_str()
        .filter(|s| s.contains('/'))
        .map(sane_line)
        .ok_or_else(|| "gh did not name the repository".to_string())
}

/// `gh issue list` / `gh pr list` with `--json number,title,state,…`.
pub fn parse_list(kind: Kind, text: &str) -> Result<Vec<Summary>, String> {
    let v = json(text)?;
    let items = v
        .as_array()
        .ok_or("unexpected output from gh: not a list")?;
    Ok(items
        .iter()
        .filter_map(|i| {
            Some(Summary {
                number: i["number"].as_u64()?,
                title: sane_line(str_of(&i["title"])),
                state: state_of(kind, i),
                author: author_of(&i["author"]),
                labels: labels_of(&i["labels"]),
                comments: i["comments"].as_array().map_or_else(
                    // `totalCount` shape of other tools.
                    || i["comments"]["totalCount"].as_u64().unwrap_or(0) as usize,
                    Vec::len,
                ),
                updated_ms: time_of(&i["updatedAt"]),
            })
        })
        .collect())
}

/// `gh issue view N --json …` / `gh pr view N --json …`.
pub fn parse_detail(kind: Kind, text: &str) -> Result<Detail, String> {
    let v = json(text)?;
    let number = v["number"]
        .as_u64()
        .ok_or("unexpected output from gh: no number")?;
    Ok(Detail {
        kind,
        number,
        title: sane_line(str_of(&v["title"])),
        state: state_of(kind, &v),
        author: author_of(&v["author"]),
        labels: labels_of(&v["labels"]),
        body: sane(str_of(&v["body"])),
        url: sane_line(str_of(&v["url"])),
        created_ms: time_of(&v["createdAt"]),
        updated_ms: time_of(&v["updatedAt"]),
        comments: array(&v["comments"])
            .iter()
            .map(|c| Comment {
                author: author_of(&c["author"]),
                at_ms: time_of(&c["createdAt"]),
                body: sane(str_of(&c["body"])),
            })
            .collect(),
        reviews: array(&v["reviews"])
            .iter()
            .map(|r| Review {
                author: author_of(&r["author"]),
                at_ms: time_of(&r["submittedAt"]),
                state: sane_line(str_of(&r["state"])),
                body: sane(str_of(&r["body"])),
            })
            .collect(),
        review_comments: Vec::new(),
        checks: array(&v["statusCheckRollup"])
            .iter()
            .filter_map(check_of)
            .collect(),
        files: array(&v["files"])
            .iter()
            .map(|f| FileChange {
                path: sane_line(str_of(&f["path"])),
                additions: f["additions"].as_u64().unwrap_or(0),
                deletions: f["deletions"].as_u64().unwrap_or(0),
            })
            .collect(),
        base: sane_line(str_of(&v["baseRefName"])),
        head: sane_line(str_of(&v["headRefName"])),
        decision: sane_line(str_of(&v["reviewDecision"])),
        additions: v["additions"].as_u64().unwrap_or(0),
        deletions: v["deletions"].as_u64().unwrap_or(0),
    })
}

/// `gh api repos/o/r/pulls/N/comments`: the REST shape (`user.login`, `created_at`…). Unreadable
/// output is no comments: they are a bonus on top of the pull request itself.
pub fn parse_review_comments(text: &str) -> Vec<ReviewComment> {
    let Ok(v) = json(text) else {
        return Vec::new();
    };
    let mut out: Vec<ReviewComment> = array(&v)
        .iter()
        .map(|c| ReviewComment {
            author: sane_line(str_of(&c["user"]["login"])),
            at_ms: time_of(&c["created_at"]),
            path: sane_line(str_of(&c["path"])),
            line: c["line"].as_u64().or_else(|| c["original_line"].as_u64()),
            body: sane(str_of(&c["body"])),
            reply: !c["in_reply_to_id"].is_null(),
        })
        .collect();
    out.sort_by_key(|c| c.at_ms);
    out
}

fn str_of(v: &Value) -> &str {
    v.as_str().unwrap_or("")
}

fn array(v: &Value) -> &[Value] {
    v.as_array().map_or(&[], Vec::as_slice)
}

fn state_of(kind: Kind, v: &Value) -> State {
    match (kind, str_of(&v["state"]).to_ascii_uppercase().as_str()) {
        (_, "MERGED") => State::Merged,
        (_, "CLOSED") => State::Closed,
        (Kind::Pr, _) if v["isDraft"].as_bool() == Some(true) => State::Draft,
        _ => State::Open,
    }
}

/// The login; bots come as `app/name`.
fn author_of(v: &Value) -> String {
    let login = str_of(&v["login"]);
    sane_line(login.strip_prefix("app/").unwrap_or(login))
}

fn labels_of(v: &Value) -> Vec<String> {
    array(v)
        .iter()
        .map(|l| sane_line(str_of(&l["name"])))
        .filter(|l| !l.is_empty())
        .collect()
}

fn time_of(v: &Value) -> i64 {
    parse_iso(str_of(v)).unwrap_or(0)
}

/// A check run (`conclusion`, `status`) or a commit status (`state`) of `statusCheckRollup`.
fn check_of(c: &Value) -> Option<Check> {
    let name = if c["__typename"] == "StatusContext" {
        str_of(&c["context"])
    } else {
        str_of(&c["name"])
    };
    if name.is_empty() {
        return None;
    }
    let state = if c["__typename"] == "StatusContext" {
        match str_of(&c["state"]) {
            "SUCCESS" => CheckState::Pass,
            "FAILURE" | "ERROR" => CheckState::Fail,
            _ => CheckState::Pending,
        }
    } else if str_of(&c["status"]) != "COMPLETED" {
        CheckState::Pending
    } else {
        match str_of(&c["conclusion"]) {
            "SUCCESS" => CheckState::Pass,
            "SKIPPED" | "NEUTRAL" | "STALE" => CheckState::Skipped,
            "" => CheckState::Pending,
            _ => CheckState::Fail,
        }
    };
    Some(Check {
        name: sane_line(name),
        state,
    })
}

/// Text from the network made safe to show and to put in the prompt: `\r` and control
/// characters (escape sequences) and bidi overrides are dropped, tabs become spaces.
pub fn sane(s: &str) -> String {
    s.chars()
        .filter_map(|c| match c {
            '\n' => Some('\n'),
            '\t' => Some(' '),
            c if c.is_control()
                || matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{200e}' | '\u{200f}') =>
            {
                None
            }
            c => Some(c),
        })
        .collect()
}

/// [`sane`] on one line: line breaks become spaces.
pub fn sane_line(s: &str) -> String {
    sane(&s.replace(['\n', '\r'], " ")).trim().to_string()
}

/// `2026-10-16T13:51:39Z` as unix milliseconds; None for anything else.
pub fn parse_iso(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() < 19 || b[4] != b'-' || b[7] != b'-' || !matches!(b[10], b'T' | b' ') {
        return None;
    }
    let num = |a: usize, z: usize| s.get(a..z)?.parse::<i64>().ok();
    let (y, m, d) = (num(0, 4)?, num(5, 7)?, num(8, 10)?);
    let (h, mi, sec) = (num(11, 13)?, num(14, 16)?, num(17, 19)?);
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    // Days since 1970-01-01 (civil calendar, proleptic Gregorian).
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(((days * 24 + h) * 60 + mi) * 60_000 + sec * 1000)
}

/// "now", "5m", "3h", "2d", "3w", "5mo", "2y": the compact age of a unix time in ms.
pub fn short_age(then_ms: i64, now_ms: i64) -> String {
    if then_ms <= 0 {
        return String::new();
    }
    let secs = (now_ms - then_ms).max(0) / 1000;
    match secs {
        0..=59 => "now".into(),
        60..=3599 => format!("{}m", secs / 60),
        3600..=86_399 => format!("{}h", secs / 3600),
        86_400..=604_799 => format!("{}d", secs / 86_400),
        604_800..=2_591_999 => format!("{}w", secs / 604_800),
        2_592_000..=31_535_999 => format!("{}mo", secs / 2_592_000),
        _ => format!("{}y", secs / 31_536_000),
    }
}

/// `2026-10-16`, for the prompt text.
pub fn date(ms: i64) -> String {
    if ms <= 0 {
        return String::new();
    }
    let z = ms.div_euclid(86_400_000) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}
