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
    /// The number in the comment's link: what reacting to it needs. 0 when it has none.
    pub id: u64,
    pub author: String,
    pub at_ms: i64,
    pub body: String,
    pub reactions: Vec<Reaction>,
}

/// The reactions of one kind on an item or comment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Reaction {
    /// GitHub's word: `THUMBS_UP`, `HEART`…
    pub content: String,
    pub count: u64,
    /// The user is among them.
    pub mine: bool,
}

/// The reactions GitHub offers: its word for each, REST's word, and the emoji.
pub const REACTIONS: [(&str, &str, &str); 8] = [
    ("THUMBS_UP", "+1", "👍"),
    ("THUMBS_DOWN", "-1", "👎"),
    ("LAUGH", "laugh", "😄"),
    ("HOORAY", "hooray", "🎉"),
    ("CONFUSED", "confused", "😕"),
    ("HEART", "heart", "❤️"),
    ("ROCKET", "rocket", "🚀"),
    ("EYES", "eyes", "👀"),
];

/// The emoji of a reaction by GitHub's word.
pub fn emoji_of(content: &str) -> &'static str {
    REACTIONS
        .iter()
        .find(|r| r.0 == content)
        .map_or("•", |r| r.2)
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
    /// On the old version's line (a removed line): GitHub's `LEFT` side.
    pub old: bool,
    /// The line moved on since (a later push): `line` is where it was.
    pub outdated: bool,
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
    /// Where it is on the web (`detailsUrl` of a check run, `targetUrl` of a status); may be
    /// empty.
    pub url: String,
    /// The GitHub Actions job it is, by id, when the link says so: its log can be read with
    /// `gh run view --job`. 0 for anything else (another CI's status).
    pub job: u64,
    /// When it started and finished, 0 when not (yet) known.
    pub started_ms: i64,
    pub finished_ms: i64,
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
    pub reactions: Vec<Reaction>,
    /// Why it was closed: `completed`, `not planned`, `duplicate`; empty when open or unknown.
    pub state_reason: String,
    pub assignees: Vec<String>,
    pub milestone: String,
    /// The issue type (`Bug`); empty without one.
    pub issue_type: String,
    pub projects: Vec<String>,
    /// People asked to review (pull requests).
    pub reviewers: Vec<String>,
    /// The parent issue's number, 0 without one, and the issues it is blocked by or blocks.
    pub parent: u64,
    pub blocked_by: Vec<u64>,
    pub blocking: Vec<u64>,
    // Pull requests only from here on.
    pub reviews: Vec<Review>,
    pub review_comments: Vec<ReviewComment>,
    pub checks: Vec<Check>,
    pub files: Vec<FileChange>,
    pub base: String,
    pub head: String,
    /// The commit the head branch is at: the files as the pull request has them.
    pub head_sha: String,
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
                id: comment_id(str_of(&c["url"])),
                author: author_of(&c["author"]),
                at_ms: time_of(&c["createdAt"]),
                body: sane(str_of(&c["body"])),
                reactions: reactions_of(&c["reactionGroups"]),
            })
            .collect(),
        reactions: reactions_of(&v["reactionGroups"]),
        state_reason: sane_line(str_of(&v["stateReason"]))
            .to_lowercase()
            .replace('_', " "),
        assignees: array(&v["assignees"])
            .iter()
            .map(author_of)
            .filter(|a| !a.is_empty())
            .collect(),
        milestone: sane_line(str_of(&v["milestone"]["title"])),
        issue_type: sane_line(str_of(&v["issueType"]["name"])),
        projects: array(&v["projectItems"])
            .iter()
            .map(|p| {
                let title = &p["project"]["title"];
                sane_line(str_of(if title.is_null() { &p["title"] } else { title }))
            })
            .filter(|p| !p.is_empty())
            .collect(),
        reviewers: array(&v["reviewRequests"])
            .iter()
            .map(|r| {
                let who = &r["requestedReviewer"];
                let login = if who["login"].is_null() { &r["login"] } else { &who["login"] };
                let name = if login.is_null() { &r["name"] } else { login };
                sane_line(str_of(name))
            })
            .filter(|r| !r.is_empty())
            .collect(),
        parent: v["parent"]["number"].as_u64().unwrap_or(0),
        blocked_by: array(&v["blockedBy"])
            .iter()
            .filter_map(|i| i["number"].as_u64())
            .collect(),
        blocking: array(&v["blocking"])
            .iter()
            .filter_map(|i| i["number"].as_u64())
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
        head_sha: sane_line(str_of(&v["headRefOid"])),
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
            old: str_of(&c["side"]) == "LEFT",
            outdated: c["line"].is_null(),
        })
        .collect();
    out.sort_by_key(|c| c.at_ms);
    out
}

/// An issue field the organization defines (`Priority`), with the options of a single select.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OrgField {
    pub id: u64,
    pub name: String,
    /// `single_select`, `text`, `number`, `date`.
    pub data_type: String,
    pub options: Vec<String>,
}

/// `gh api orgs/o/issue-fields`.
pub fn parse_org_fields(text: &str) -> Vec<OrgField> {
    let Ok(v) = json(text) else {
        return Vec::new();
    };
    array(&v)
        .iter()
        .filter_map(|f| {
            Some(OrgField {
                id: f["id"].as_u64()?,
                name: sane_line(str_of(&f["name"])),
                data_type: sane_line(str_of(&f["data_type"])),
                options: array(&f["options"])
                    .iter()
                    .map(|o| sane_line(str_of(&o["name"])))
                    .filter(|o| !o.is_empty())
                    .collect(),
            })
        })
        .filter(|f| !f.name.is_empty())
        .collect()
}

/// The `key` of each entry of a REST list: `name` for issue types, `title` for milestones.
pub fn parse_names(text: &str, key: &str) -> Vec<String> {
    let Ok(v) = json(text) else {
        return Vec::new();
    };
    array(&v)
        .iter()
        .map(|e| sane_line(str_of(&e[key])))
        .filter(|n| !n.is_empty())
        .collect()
}

/// `gh api repos/o/r/issues/N/issue-field-values`: each field's name and what it is set to.
pub fn parse_field_values(text: &str) -> Vec<(String, String)> {
    let Ok(v) = json(text) else {
        return Vec::new();
    };
    array(&v)
        .iter()
        .filter_map(|f| {
            let name = sane_line(str_of(&f["issue_field_name"]));
            let value = match f["single_select_option"]["name"].as_str() {
                Some(option) => sane_line(option),
                None => match &f["value"] {
                    Value::String(s) => sane_line(s),
                    Value::Number(n) => n.to_string(),
                    _ => String::new(),
                },
            };
            (!name.is_empty() && !value.is_empty()).then_some((name, value))
        })
        .collect()
}

/// `gh project list --format json`: the titles.
pub fn parse_projects(text: &str) -> Vec<String> {
    let Ok(v) = json(text) else {
        return Vec::new();
    };
    array(&v["projects"])
        .iter()
        .map(|p| sane_line(str_of(&p["title"])))
        .filter(|t| !t.is_empty())
        .collect()
}

/// `gh api …/reactions`: each reaction's id, who gave it and its REST word (`heart`).
pub fn parse_reaction_list(text: &str) -> Vec<(u64, String, String)> {
    let Ok(v) = json(text) else {
        return Vec::new();
    };
    array(&v)
        .iter()
        .filter_map(|r| {
            Some((
                r["id"].as_u64()?,
                sane_line(str_of(&r["user"]["login"])),
                sane_line(str_of(&r["content"])),
            ))
        })
        .collect()
}

/// `gh api user`: the login.
pub fn parse_login(text: &str) -> Option<String> {
    let v = json(text).ok()?;
    let login = sane_line(str_of(&v["login"]));
    (!login.is_empty()).then_some(login)
}

/// The number after `#issuecomment-` in a comment's link.
fn comment_id(url: &str) -> u64 {
    url.rsplit_once('-')
        .and_then(|(_, id)| id.parse().ok())
        .unwrap_or(0)
}

/// `reactionGroups`: one entry per kind that somebody used.
fn reactions_of(v: &Value) -> Vec<Reaction> {
    array(v)
        .iter()
        .map(|g| Reaction {
            content: sane_line(str_of(&g["content"])),
            count: g["users"]["totalCount"].as_u64().unwrap_or(0),
            mine: g["viewerHasReacted"].as_bool().unwrap_or(false),
        })
        .filter(|r| r.count > 0)
        .collect()
}

/// A label of the repository, with the color GitHub gives it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Label {
    pub name: String,
    /// `0xRRGGBB`.
    pub rgb: u32,
    pub description: String,
}

/// Somebody who can be assigned in the repository.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Person {
    pub login: String,
    pub avatar_url: String,
}

/// `gh api repos/o/r/labels`. Unreadable output is no labels: they only help typing.
pub fn parse_labels(text: &str) -> Vec<Label> {
    let Ok(v) = json(text) else {
        return Vec::new();
    };
    array(&v)
        .iter()
        .filter(|l| l["name"].is_string())
        .map(|l| Label {
            name: sane_line(str_of(&l["name"])),
            rgb: u32::from_str_radix(str_of(&l["color"]).trim_start_matches('#'), 16)
                .unwrap_or(0x808080),
            description: sane_line(str_of(&l["description"])),
        })
        .collect()
}

/// `gh api repos/o/r/assignees`.
pub fn parse_people(text: &str) -> Vec<Person> {
    let Ok(v) = json(text) else {
        return Vec::new();
    };
    array(&v)
        .iter()
        .filter(|p| p["login"].is_string())
        .map(|p| Person {
            login: sane_line(str_of(&p["login"])),
            avatar_url: sane_line(str_of(&p["avatar_url"])),
        })
        .collect()
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
    let url = if c["__typename"] == "StatusContext" {
        str_of(&c["targetUrl"])
    } else {
        str_of(&c["detailsUrl"])
    };
    Some(Check {
        name: sane_line(name),
        state,
        url: sane_line(url),
        job: job_of(url),
        started_ms: time_of(&c["startedAt"]),
        finished_ms: time_of(&c["completedAt"]),
    })
}

/// Standard base64 (line breaks and padding allowed, as the contents API sends it); None when
/// it is not.
pub fn base64(text: &str) -> Option<Vec<u8>> {
    let value = |c: u8| match c {
        b'A'..=b'Z' => Some(c - b'A'),
        b'a'..=b'z' => Some(c - b'a' + 26),
        b'0'..=b'9' => Some(c - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    };
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0);
    for c in text.bytes() {
        if c.is_ascii_whitespace() || c == b'=' {
            continue;
        }
        acc = (acc << 6) | u32::from(value(c)?);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

/// The job id in the link of a GitHub Actions check:
/// `https://github.com/o/r/actions/runs/123/job/456` is 456; 0 for any other link.
pub fn job_of(url: &str) -> u64 {
    let Some(rest) = url.strip_prefix("https://github.com/") else {
        return 0;
    };
    let parts: Vec<&str> = rest.split(['/', '?', '#']).collect();
    match parts.as_slice() {
        [_, _, "actions", "runs", _, "job", id, ..] => id.parse().unwrap_or(0),
        _ => 0,
    }
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
