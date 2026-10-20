//! Git status in the background: the porcelain v2 parser, the commands, and a runner that
//! debounces requests and keeps at most one run in flight. Nothing here touches the terminal;
//! the UI thread only ever calls `Runner::request` and `Runner::poll`, which don't block.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

/// What happened to a path, collapsed from git's two-letter XY status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Change {
    Modified,
    Added,
    Deleted,
    Renamed,
    Untracked,
    Conflicted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GitEntry {
    /// Relative to the repository root, as git prints it.
    pub path: String,
    /// The old path of a rename or copy.
    pub orig: Option<String>,
    pub change: Change,
}

/// Lines added and removed in a file since the last commit (an untracked file: all added).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct LineCount {
    pub added: u32,
    pub removed: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct GitStatus {
    /// Branch name; None when git didn't say.
    pub head: Option<String>,
    pub detached: bool,
    /// Commits ahead of / behind the upstream (0 without one).
    pub ahead: u32,
    pub behind: u32,
    pub entries: Vec<GitEntry>,
    /// Where the session's directory is inside the repository (`src/a/`, empty at the root).
    pub prefix: String,
    /// By path from the repository root, as in `entries`; binary files have none.
    pub lines: BTreeMap<String, LineCount>,
}

impl GitStatus {
    /// "main ↑1 ↓2", "detached".
    pub(crate) fn branch_label(&self) -> String {
        let mut s = if self.detached {
            "detached".to_string()
        } else {
            self.head.clone().unwrap_or_else(|| "?".to_string())
        };
        if self.ahead > 0 {
            s.push_str(&format!(" ↑{}", self.ahead));
        }
        if self.behind > 0 {
            s.push_str(&format!(" ↓{}", self.behind));
        }
        s
    }

    /// `path` (from the repository root) as seen from the session's directory.
    pub(crate) fn rel(&self, path: &str) -> String {
        rel_from_prefix(&self.prefix, path)
    }
}

/// `git status --porcelain=v2 --branch -z` output. Unknown lines are skipped.
pub(crate) fn parse_status(out: &[u8]) -> GitStatus {
    let text = String::from_utf8_lossy(out);
    let mut status = GitStatus::default();
    let mut records = text.split('\0').filter(|r| !r.is_empty());
    while let Some(rec) = records.next() {
        let Some((kind, rest)) = rec.split_once(' ') else {
            continue;
        };
        match kind {
            "#" => {
                let Some((key, value)) = rest.split_once(' ') else {
                    continue;
                };
                match key {
                    "branch.head" => {
                        if value == "(detached)" {
                            status.detached = true;
                        } else {
                            status.head = Some(value.to_string());
                        }
                    }
                    "branch.ab" => {
                        for part in value.split(' ') {
                            if let Some(n) = part.strip_prefix('+') {
                                status.ahead = n.parse().unwrap_or(0);
                            } else if let Some(n) = part.strip_prefix('-') {
                                status.behind = n.parse().unwrap_or(0);
                            }
                        }
                    }
                    _ => {}
                }
            }
            "1" => {
                // 1 <XY> <sub> <mH> <mI> <mW> <hH> <hI> <path>
                let f: Vec<&str> = rest.splitn(8, ' ').collect();
                if f.len() == 8 {
                    status.entries.push(GitEntry {
                        path: f[7].to_string(),
                        orig: None,
                        change: change_of(f[0]),
                    });
                }
            }
            "2" => {
                // 2 <XY> <sub> <mH> <mI> <mW> <hH> <hI> <X><score> <path> NUL <origPath>
                let f: Vec<&str> = rest.splitn(9, ' ').collect();
                let orig = records.next().map(str::to_string);
                if f.len() == 9 {
                    let mut change = change_of(f[0]);
                    if change != Change::Deleted {
                        change = Change::Renamed;
                    }
                    status.entries.push(GitEntry {
                        path: f[8].to_string(),
                        orig,
                        change,
                    });
                }
            }
            "u" => {
                // u <XY> <sub> <m1> <m2> <m3> <mW> <h1> <h2> <h3> <path>
                let f: Vec<&str> = rest.splitn(10, ' ').collect();
                if f.len() == 10 {
                    status.entries.push(GitEntry {
                        path: f[9].to_string(),
                        orig: None,
                        change: Change::Conflicted,
                    });
                }
            }
            "?" => status.entries.push(GitEntry {
                path: rest.to_string(),
                orig: None,
                change: Change::Untracked,
            }),
            _ => {}
        }
    }
    status
}

/// `git diff --numstat -z` output: `added TAB removed TAB path NUL`, or for a rename
/// `added TAB removed TAB NUL old NUL new NUL`. Binary files (`-`) are left out.
pub(crate) fn parse_numstat(out: &[u8], into: &mut BTreeMap<String, LineCount>) {
    let text = String::from_utf8_lossy(out);
    let mut records = text.split('\0');
    while let Some(rec) = records.next() {
        let mut f = rec.splitn(3, '\t');
        let (Some(added), Some(removed), Some(path)) = (f.next(), f.next(), f.next()) else {
            continue;
        };
        let path = if path.is_empty() {
            records.next(); // the old path
            match records.next() {
                Some(new) => new,
                None => continue,
            }
        } else {
            path
        };
        let (Ok(added), Ok(removed)) = (added.parse::<u32>(), removed.parse::<u32>()) else {
            continue;
        };
        let count = into.entry(path.to_string()).or_default();
        count.added += added;
        count.removed += removed;
    }
}

/// Untracked files larger than this aren't counted.
const COUNT_LIMIT: u64 = 1 << 20;

/// The lines of a new text file; None for a binary one, a directory or a big file.
fn count_lines(path: &Path) -> Option<u32> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() > COUNT_LIMIT {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    if bytes.contains(&0) {
        return None;
    }
    let newlines = bytes.iter().filter(|b| **b == b'\n').count();
    let unterminated = bytes.last().is_some_and(|b| *b != b'\n');
    u32::try_from(newlines + usize::from(unterminated)).ok()
}

fn change_of(xy: &str) -> Change {
    let mut c = xy.chars();
    let (x, y) = (c.next().unwrap_or('.'), c.next().unwrap_or('.'));
    let has = |ch: char| x == ch || y == ch;
    if has('R') {
        Change::Renamed
    } else if has('D') {
        Change::Deleted
    } else if has('A') || has('C') {
        Change::Added
    } else {
        Change::Modified
    }
}

/// `path` from the repository root, as seen from the directory `prefix` (`a/b/`) inside it.
pub(crate) fn rel_from_prefix(prefix: &str, path: &str) -> String {
    let pre: Vec<&str> = prefix.split('/').filter(|c| !c.is_empty()).collect();
    if pre.is_empty() {
        return path.to_string();
    }
    let comps: Vec<&str> = path.split('/').filter(|c| !c.is_empty()).collect();
    // Only directories can be shared: the last component of the path is the file itself.
    let dirs = comps.len().saturating_sub(1);
    let mut common = 0;
    while common < pre.len() && common < dirs && pre[common] == comps[common] {
        common += 1;
    }
    let mut out = "../".repeat(pre.len() - common);
    out.push_str(&comps[common..].join("/"));
    if path.ends_with('/') {
        out.push('/');
    }
    out
}

/// A `git` command for the session directory: no optional locks (this runs behind the user's
/// back), no pager, literal pathspecs.
pub(crate) fn git(cwd: &Path) -> Command {
    let mut c = Command::new("git");
    c.env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_PAGER", "cat")
        .arg("--literal-pathspecs")
        .arg("-C")
        .arg(cwd)
        .stdin(Stdio::null())
        .stderr(Stdio::null());
    c
}

/// Runs git status in `cwd`. None when it isn't a repository (or git isn't there).
pub(crate) fn run_status(cwd: &Path) -> Option<GitStatus> {
    let out = git(cwd)
        .args([
            "status",
            "--porcelain=v2",
            "--branch",
            "-z",
            "--untracked-files=normal",
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let mut status = parse_status(&out.stdout);
    if let Ok(o) = git(cwd).args(["rev-parse", "--show-prefix"]).output()
        && o.status.success()
    {
        status.prefix = String::from_utf8_lossy(&o.stdout).trim().to_string();
    }
    // Staged and unstaged together, against the last commit. Without one yet: what is staged
    // (against nothing) plus what isn't.
    let numstat = |args: &[&str]| {
        git(cwd)
            .args(["diff", "--numstat", "-z"])
            .args(args)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| o.stdout)
    };
    let mut lines = BTreeMap::new();
    if let Some(out) = numstat(&["HEAD", "--"]) {
        parse_numstat(&out, &mut lines);
    } else {
        for out in [numstat(&["--cached", "--"]), numstat(&["--"])]
            .into_iter()
            .flatten()
        {
            parse_numstat(&out, &mut lines);
        }
    }
    for e in &status.entries {
        if e.change == Change::Untracked
            && !e.path.ends_with('/')
            && let Some(added) = count_lines(&cwd.join(status.rel(&e.path)))
        {
            lines.insert(e.path.clone(), LineCount { added, removed: 0 });
        }
    }
    status.lines = lines;
    Some(status)
}

pub(crate) type StatusResult = Option<GitStatus>;
type Job = Arc<dyn Fn() -> StatusResult + Send + Sync>;

/// Runs a status job on a thread when asked, never two at once. Requests are debounced (the
/// run starts `debounce` after the first request since the last run); without requests it
/// re-runs every `interval`.
pub(crate) struct Runner {
    job: Job,
    tx: Sender<StatusResult>,
    rx: Receiver<StatusResult>,
    in_flight: bool,
    /// When the next run is due because of a request.
    pending: Option<Instant>,
    last_start: Option<Instant>,
    debounce: Duration,
    interval: Duration,
}

pub(crate) const DEBOUNCE: Duration = Duration::from_millis(200);
pub(crate) const INTERVAL: Duration = Duration::from_secs(10);

impl Runner {
    /// The first run is due immediately (it starts with the first `poll`).
    pub(crate) fn new(job: Job, debounce: Duration, interval: Duration, now: Instant) -> Runner {
        let (tx, rx) = mpsc::channel();
        Runner {
            job,
            tx,
            rx,
            in_flight: false,
            pending: Some(now),
            last_start: None,
            debounce,
            interval,
        }
    }

    pub(crate) fn for_dir(cwd: &Path, now: Instant) -> Runner {
        let cwd = cwd.to_path_buf();
        Runner::new(Arc::new(move || run_status(&cwd)), DEBOUNCE, INTERVAL, now)
    }

    /// Something probably changed: run soon.
    pub(crate) fn request(&mut self, now: Instant) {
        if self.pending.is_none() {
            self.pending = Some(now + self.debounce);
        }
    }

    /// Collects the newest finished result and starts a run if one is due.
    pub(crate) fn poll(&mut self, now: Instant) -> Option<StatusResult> {
        let mut got = None;
        while let Ok(result) = self.rx.try_recv() {
            got = Some(result);
            self.in_flight = false;
        }
        if !self.in_flight {
            let due = match self.pending {
                Some(t) => now >= t,
                None => self
                    .last_start
                    .is_none_or(|l| now.saturating_duration_since(l) >= self.interval),
            };
            if due {
                self.pending = None;
                self.last_start = Some(now);
                self.in_flight = true;
                let job = Arc::clone(&self.job);
                let tx = self.tx.clone();
                std::thread::spawn(move || {
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| job()))
                        .unwrap_or(None);
                    let _ = tx.send(result);
                });
            }
        }
        got
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn numstat_counts_lines_per_path_and_follows_renames() {
        let mut lines = BTreeMap::new();
        parse_numstat(
            b"12\t3\tsrc/a.rs\0-\t-\tlogo.png\x005\t0\t\0old.rs\0new.rs\0",
            &mut lines,
        );
        assert_eq!(
            lines.get("src/a.rs"),
            Some(&LineCount {
                added: 12,
                removed: 3
            })
        );
        assert_eq!(
            lines.get("new.rs"),
            Some(&LineCount {
                added: 5,
                removed: 0
            })
        );
        assert!(!lines.contains_key("logo.png"), "binary");
        assert!(!lines.contains_key("old.rs"));
        // Staged and unstaged runs add up.
        parse_numstat(b"1\t1\tsrc/a.rs\0", &mut lines);
        assert_eq!(
            lines.get("src/a.rs"),
            Some(&LineCount {
                added: 13,
                removed: 4
            })
        );
    }

    #[test]
    fn new_files_count_their_lines_unless_binary() {
        let dir = std::env::temp_dir().join(format!("suite-term-count-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), "one\ntwo\nthree").unwrap();
        std::fs::write(dir.join("b.bin"), b"\x00\x01\n").unwrap();
        assert_eq!(count_lines(&dir.join("a.txt")), Some(3));
        assert_eq!(count_lines(&dir.join("b.bin")), None);
        assert_eq!(count_lines(&dir), None);
        std::fs::remove_dir_all(dir).unwrap();
    }

    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn nul(parts: &[&str]) -> Vec<u8> {
        let mut v = Vec::new();
        for p in parts {
            v.extend_from_slice(p.as_bytes());
            v.push(0);
        }
        v
    }

    #[test]
    fn parses_branch_and_entries() {
        let out = nul(&[
            "# branch.oid 0123abc",
            "# branch.head main",
            "# branch.upstream origin/main",
            "# branch.ab +1 -2",
            "1 .M N... 100644 100644 100644 aaaa bbbb src/a.ts",
            "1 A. N... 000000 100644 100644 0000 cccc new file.rs",
            "1 .D N... 100644 100644 000000 aaaa aaaa gone.txt",
            "2 R. N... 100644 100644 100644 aaaa bbbb R100 src/new name.ts",
            "src/old name.ts",
            "u UU N... 100644 100644 100644 100644 a b c both.txt",
            "? untracked.txt",
            "? dir/",
            "! ignored.o",
        ]);
        let s = parse_status(&out);
        assert_eq!(s.head.as_deref(), Some("main"));
        assert!(!s.detached);
        assert_eq!((s.ahead, s.behind), (1, 2));
        assert_eq!(s.branch_label(), "main ↑1 ↓2");
        let got: Vec<_> = s
            .entries
            .iter()
            .map(|e| (e.path.as_str(), e.change, e.orig.as_deref()))
            .collect();
        assert_eq!(
            got,
            [
                ("src/a.ts", Change::Modified, None),
                ("new file.rs", Change::Added, None),
                ("gone.txt", Change::Deleted, None),
                ("src/new name.ts", Change::Renamed, Some("src/old name.ts")),
                ("both.txt", Change::Conflicted, None),
                ("untracked.txt", Change::Untracked, None),
                ("dir/", Change::Untracked, None),
            ]
        );
    }

    #[test]
    fn detached_and_no_upstream() {
        let s = parse_status(&nul(&["# branch.oid 0123abc", "# branch.head (detached)"]));
        assert!(s.detached);
        assert_eq!(s.branch_label(), "detached");
        let s = parse_status(&nul(&["# branch.oid (initial)", "# branch.head main"]));
        assert_eq!(s.branch_label(), "main");
        assert!(s.entries.is_empty());
        let s = parse_status(&nul(&["# branch.head dev", "# branch.ab +0 -3"]));
        assert_eq!(s.branch_label(), "dev ↓3");
    }

    #[test]
    fn rename_record_consumes_the_orig_path() {
        // The orig path must not be read as a record of its own, even if it looks like one.
        let s = parse_status(&nul(&[
            "2 R. N... 100644 100644 100644 a b R100 new.txt",
            "? looks-like-untracked",
            "? real.txt",
        ]));
        assert_eq!(s.entries.len(), 2);
        assert_eq!(s.entries[0].orig.as_deref(), Some("? looks-like-untracked"));
        assert_eq!(s.entries[1].path, "real.txt");
    }

    #[test]
    fn xy_to_change() {
        assert_eq!(change_of("MM"), Change::Modified);
        assert_eq!(change_of("T."), Change::Modified);
        assert_eq!(change_of("A."), Change::Added);
        assert_eq!(change_of("AD"), Change::Deleted);
        assert_eq!(change_of(".D"), Change::Deleted);
        assert_eq!(change_of("R."), Change::Renamed);
    }

    #[test]
    fn paths_relative_to_the_session_directory() {
        assert_eq!(rel_from_prefix("", "src/a.ts"), "src/a.ts");
        assert_eq!(rel_from_prefix("src/", "src/a.ts"), "a.ts");
        assert_eq!(rel_from_prefix("src/", "README.md"), "../README.md");
        assert_eq!(rel_from_prefix("src/a/", "src/b/x.rs"), "../b/x.rs");
        assert_eq!(rel_from_prefix("src/", "src/dir/"), "dir/");
        assert_eq!(rel_from_prefix("src/", "other/"), "../other/");
    }

    fn counting_job(count: &Arc<AtomicUsize>, sleep: Duration) -> Job {
        let count = Arc::clone(count);
        Arc::new(move || {
            count.fetch_add(1, Ordering::SeqCst);
            std::thread::sleep(sleep);
            Some(GitStatus::default())
        })
    }

    fn wait_result(r: &mut Runner, now: Instant) -> Option<StatusResult> {
        for _ in 0..200 {
            if let Some(x) = r.poll(now) {
                return Some(x);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        None
    }

    #[test]
    fn runner_debounces_and_runs_one_at_a_time() {
        let count = Arc::new(AtomicUsize::new(0));
        let t0 = Instant::now();
        let job = counting_job(&count, Duration::from_millis(60));
        let mut r = Runner::new(job, Duration::from_millis(200), Duration::from_secs(10), t0);
        // The first run starts at once.
        assert!(r.poll(t0).is_none());
        assert!(wait_result(&mut r, t0).is_some());
        assert_eq!(count.load(Ordering::SeqCst), 1);
        // A burst of requests runs once, after the debounce.
        r.request(t0);
        r.request(t0 + Duration::from_millis(50));
        r.poll(t0 + Duration::from_millis(100));
        assert_eq!(count.load(Ordering::SeqCst), 1);
        r.poll(t0 + Duration::from_millis(250));
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(count.load(Ordering::SeqCst), 2);
        // A request while the run is in flight waits for it.
        r.request(t0 + Duration::from_millis(260));
        r.poll(t0 + Duration::from_millis(600));
        assert_eq!(count.load(Ordering::SeqCst), 2);
        assert!(wait_result(&mut r, t0 + Duration::from_millis(600)).is_some());
        r.poll(t0 + Duration::from_millis(700));
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(count.load(Ordering::SeqCst), 3);
        assert!(wait_result(&mut r, t0 + Duration::from_millis(700)).is_some());
        // Nothing requested: the periodic refresh.
        r.poll(t0 + Duration::from_secs(5));
        assert_eq!(count.load(Ordering::SeqCst), 3);
        r.poll(t0 + Duration::from_secs(11));
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(count.load(Ordering::SeqCst), 4);
    }

    #[test]
    fn a_panicking_job_does_not_wedge_the_runner() {
        let t0 = Instant::now();
        let mut r = Runner::new(
            Arc::new(|| panic!("boom")),
            Duration::ZERO,
            Duration::from_secs(10),
            t0,
        );
        r.poll(t0);
        assert_eq!(wait_result(&mut r, t0), Some(None));
    }
}
