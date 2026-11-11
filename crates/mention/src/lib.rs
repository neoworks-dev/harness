//! The files an `@` mention could be, for the apps' inputs. `@` alone lists the session's
//! directory (`../` first), `@..` or `@../` the one above it, `@../..` the one above that,
//! `@src/` a folder in it; anything after the last `/` is matched loosely against every file and
//! folder under that directory (`@ap` finds `src/app.rs`). `/cd`'s argument completes the same
//! way, with folders only; `~/` starts at home.
//!
//! The tree is walked once per directory and kept for a few seconds: hidden entries, `.git`,
//! `node_modules`, `target` and the like are left out, and the walk stops after
//! [`MAX_ENTRIES`].

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;

/// Entries a walk keeps at most, and how deep it goes.
pub const MAX_ENTRIES: usize = 20_000;
const MAX_DEPTH: usize = 12;
/// How long a walk is reused.
const FRESH: Duration = Duration::from_secs(10);
/// Matches offered at most.
const MAX_MATCHES: usize = 50;
/// Folders never walked into: build output, dependencies, caches.
const SKIP: [&str; 9] = [
    "node_modules",
    "target",
    "dist",
    "__pycache__",
    "venv",
    "vendor",
    "build",
    "out",
    "coverage",
];

/// One thing to put after the `@`: its path as typed (relative to the session's directory,
/// `../` and all), a folder's ending in `/`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Candidate {
    pub path: String,
    pub dir: bool,
}

/// The query split into the directory it is in (as typed, ending in `/` unless empty) and
/// what to match there.
pub fn split(query: &str) -> (String, &str) {
    // `..`, `../..` and `~` are directories without their last `/`.
    if query == ".." || query.ends_with("/..") || query == "~" {
        return (format!("{query}/"), "");
    }
    match query.rfind('/') {
        Some(i) => (query[..=i].to_string(), &query[i + 1..]),
        None => (String::new(), query),
    }
}

/// Every entry under a directory: relative, folders ending in `/`.
type Entries = Arc<Vec<String>>;

/// Finds candidates, keeping each directory's walk for a few seconds.
#[derive(Debug, Default)]
pub struct Finder {
    /// Walked directories: when, and what was under them.
    walks: Mutex<HashMap<PathBuf, (Instant, Entries)>>,
}

impl Finder {
    /// What to offer for `query` (what follows the `@`) in a session working in `cwd`, with
    /// `home` for `~/`; folders only when `folders` (`/cd`).
    pub fn candidates(
        &self,
        cwd: &Path,
        home: Option<&Path>,
        query: &str,
        folders: bool,
    ) -> Vec<Candidate> {
        let (dir, name) = split(query);
        let base = match (dir.strip_prefix("~/"), home) {
            (Some(rest), Some(home)) => home.join(rest),
            _ if dir.starts_with('/') => PathBuf::from(&dir),
            _ => cwd.join(&dir),
        };
        if name.is_empty() {
            let mut listed = list(&base, &dir);
            if folders {
                listed.retain(|c| c.dir);
            }
            return listed;
        }
        let entries = self.walked(&base);
        let mut scored: Vec<(i64, &String)> = entries
            .iter()
            .filter(|e| !folders || e.ends_with('/'))
            .filter_map(|e| score(e, name).map(|s| (s, e)))
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(b.1)));
        scored
            .into_iter()
            .take(MAX_MATCHES)
            .map(|(_, e)| Candidate {
                path: format!("{dir}{e}"),
                dir: e.ends_with('/'),
            })
            .collect()
    }

    /// Everything under `base`, walked again once the last walk is stale.
    fn walked(&self, base: &Path) -> Entries {
        let mut walks = self
            .walks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match walks.get(base) {
            Some((at, entries)) if at.elapsed() < FRESH => entries.clone(),
            _ => {
                let entries = Arc::new(walk(base));
                walks.insert(base.to_path_buf(), (Instant::now(), entries.clone()));
                entries
            }
        }
    }
}

/// A directory's own entries, folders first, then by name; `../` before them, so the list
/// leads up.
fn list(base: &Path, dir: &str) -> Vec<Candidate> {
    let Ok(read) = std::fs::read_dir(base) else {
        return Vec::new();
    };
    let mut entries: Vec<(bool, String)> = read
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            (!name.starts_with('.')).then(|| (e.path().is_dir(), name))
        })
        .collect();
    entries.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    // Only on the way up (`@`, `@..`): inside a folder `src/../` would go back.
    let upward = dir.split('/').all(|s| s.is_empty() || s == "..");
    let up = (upward && base.parent().is_some()).then(|| Candidate {
        path: format!("{dir}../"),
        dir: true,
    });
    up.into_iter()
        .chain(entries.into_iter().map(|(is_dir, name)| Candidate {
            path: format!("{dir}{name}{}", if is_dir { "/" } else { "" }),
            dir: is_dir,
        }))
        .collect()
}

/// Everything under `base`, breadth first (so a capped walk keeps what is near), relative to
/// it; folders end in `/`.
fn walk(base: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut queue = std::collections::VecDeque::from([(base.to_path_buf(), String::new(), 0)]);
    while let Some((path, rel, depth)) = queue.pop_front() {
        let Ok(read) = std::fs::read_dir(&path) else {
            continue;
        };
        let mut entries: Vec<_> = read.flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            let Ok(name) = e.file_name().into_string() else {
                continue;
            };
            if name.starts_with('.') {
                continue;
            }
            // Not into symlinked folders: they can loop.
            let is_dir = e.file_type().is_ok_and(|t| t.is_dir());
            let child = format!("{rel}{name}");
            if is_dir {
                if SKIP.contains(&name.as_str()) {
                    continue;
                }
                out.push(format!("{child}/"));
                if depth + 1 < MAX_DEPTH {
                    queue.push_back((e.path(), format!("{child}/"), depth + 1));
                }
            } else {
                out.push(child);
            }
            if out.len() >= MAX_ENTRIES {
                return out;
            }
        }
    }
    out
}

/// How well `path` matches `query` (case doesn't matter), None when it doesn't: the name
/// holding it whole first, then the path holding it, then its letters in order; shorter paths
/// first among equals.
fn score(path: &str, query: &str) -> Option<i64> {
    let path_l = path.to_lowercase();
    let query_l = query.to_lowercase();
    let name = path_l
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(&path_l);
    let len = path.len() as i64;
    if name.starts_with(&query_l) {
        return Some(40_000 - len);
    }
    if name.contains(&query_l) {
        return Some(30_000 - len);
    }
    if path_l.contains(&query_l) {
        return Some(20_000 - len);
    }
    // In order, with the gaps between them counted against it.
    let mut gaps = 0;
    let mut chars = path_l.chars();
    for q in query_l.chars() {
        let mut skipped = 0;
        loop {
            match chars.next() {
                Some(c) if c == q => break,
                Some(_) => skipped += 1,
                None => return None,
            }
        }
        gaps += skipped;
    }
    Some(10_000 - gaps * 10 - len)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_query_splits_into_its_directory_and_the_rest() {
        assert_eq!(split(""), (String::new(), ""));
        assert_eq!(split("ap"), (String::new(), "ap"));
        assert_eq!(split(".."), ("../".into(), ""));
        assert_eq!(split("../.."), ("../../".into(), ""));
        assert_eq!(split("../src/ma"), ("../src/".into(), "ma"));
    }

    /// A tree of its own for the test `name`: tests run side by side.
    fn tree(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("neo-mention-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for dir in ["site/src", "site/node_modules/x", "site/.git", "other"] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
        }
        for file in [
            "site/src/app.rs",
            "site/src/main.rs",
            "site/README.md",
            "site/node_modules/x/app.js",
            "other/notes.txt",
        ] {
            std::fs::write(root.join(file), "").unwrap();
        }
        root
    }

    fn paths(c: Vec<Candidate>) -> Vec<String> {
        c.into_iter().map(|c| c.path).collect()
    }

    #[test]
    fn folders_only_and_home_with_a_tilde() {
        let root = tree("cd");
        let site = root.join("site");
        let f = Finder::default();
        let cd = |query: &str, home: Option<&Path>| paths(f.candidates(&site, home, query, true));
        assert_eq!(cd("", None), ["../", "node_modules/", "src/"]);
        assert_eq!(cd("s", None), ["src/"]);
        assert_eq!(cd("~/", Some(&root)), ["~/other/", "~/site/"]);
        assert_eq!(cd("~", Some(&root)), ["~/other/", "~/site/"]);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn listing_goes_up_with_dots_and_matching_looks_under_it() {
        let root = tree("listing");
        let site = root.join("site");
        let f = Finder::default();
        let at = |query: &str| f.candidates(&site, None, query, false);
        assert_eq!(paths(at("")), ["../", "node_modules/", "src/", "README.md"]);
        assert_eq!(paths(at("..")), ["../../", "../other/", "../site/"]);
        assert_eq!(paths(at("src/")), ["src/app.rs", "src/main.rs"]);
        // Loosely, and not into node_modules or hidden folders.
        assert_eq!(paths(at("app")), ["src/app.rs"]);
        assert_eq!(paths(at("smn"))[0], "src/main.rs");
        assert_eq!(paths(at("../note")), ["../other/notes.txt"]);
        assert_eq!(
            at("sr")[0],
            Candidate {
                path: "src/".into(),
                dir: true
            }
        );
        std::fs::remove_dir_all(&root).unwrap();
    }
}
