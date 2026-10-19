//! The slash-command list and the session switcher's state.

use suite_protocol::{SessionInfo, SessionState};

use crate::util::{relative_time, shorten_home};

/// A command in the slash menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Command {
    pub name: &'static str,
    /// Argument hint shown after the name.
    pub args: &'static str,
    pub desc: &'static str,
    /// Handled by this app instead of being sent to the daemon.
    pub local: bool,
    /// Pointless without arguments (Enter completes it instead of running it).
    pub needs_args: bool,
}

const fn daemon(name: &'static str, args: &'static str, desc: &'static str) -> Command {
    Command {
        name,
        args,
        desc,
        local: false,
        needs_args: false,
    }
}

const fn local(name: &'static str, desc: &'static str) -> Command {
    Command {
        name,
        args: "",
        desc,
        local: true,
        needs_args: false,
    }
}

pub const COMMANDS: &[Command] = &[
    daemon("clear", "", "start a new conversation in this directory"),
    daemon("model", "[harness] [model]", "show or set the model"),
    daemon(
        "effort",
        "[harness] [level]",
        "show or set the reasoning effort",
    ),
    daemon("mode", "[harness] [mode]", "show or set the agent mode"),
    daemon(
        "resume",
        "[claude|codex]",
        "continue an earlier Claude Code / Codex conversation",
    ),
    Command {
        needs_args: true,
        ..daemon("claude", "<task>", "give a task to Claude Code")
    },
    Command {
        needs_args: true,
        ..daemon("codex", "<task>", "give a task to Codex")
    },
    daemon(
        "harness",
        "[neo|claude|codex]",
        "switch this session's agent: neo (based on pi), claude, codex",
    ),
    daemon(
        "approvals",
        "[ask|review|edits|auto]",
        "ask, review each edit, accept edits, or auto (no value: next)",
    ),
    daemon(
        "safesearch",
        "[strict|moderate|off]",
        "how much adult content web search may return",
    ),
    daemon(
        "login",
        "[provider]",
        "add an API key or an account (ChatGPT, Copilot, Claude Pro/Max)",
    ),
    daemon("logout", "[provider]", "remove a stored API key or account"),
    daemon("accounts", "", "list the API keys and accounts in use"),
    local("sessions", "switch to another session (ctrl-s)"),
    local(
        "sidebar",
        "show or hide the sessions sidebar (ctrl-g focuses it)",
    ),
    local("expand", "toggle full tool results (ctrl-o)"),
    local("thoughts", "show or hide the agent's thoughts (ctrl-t)"),
    local("github", "GitHub issues and pull requests (ctrl-r)"),
    local("help", "keys and commands"),
    local(
        "terminal",
        "show or hide the agent's commands and their output next to the chat (tab focuses it)",
    ),
    local(
        "files",
        "the changed files: a tree, enter opens a file (tab also gets there)",
    ),
    Command {
        args: "[dark|light|terminal]",
        ..local("theme", "dark or light background, or the terminal's own")
    },
    local("quit", "leave; the session keeps running"),
];

/// The command named by the first word of `text` ("/model sonnet" → model), if any.
pub fn lookup(text: &str) -> Option<&'static Command> {
    let word = text
        .trim_start()
        .strip_prefix('/')?
        .split_whitespace()
        .next()?;
    COMMANDS.iter().find(|c| c.name == word)
}

/// Commands matching what is typed after the slash: names starting with it first, then ones
/// containing it.
pub fn matching(typed: &str) -> Vec<&'static Command> {
    let q = typed.strip_prefix('/').unwrap_or(typed).to_lowercase();
    let mut out: Vec<&Command> = COMMANDS.iter().filter(|c| c.name.starts_with(&q)).collect();
    if out.is_empty() {
        out = COMMANDS.iter().filter(|c| c.name.contains(&q)).collect();
    }
    out
}

/// Whether the editor text is a command being typed: a slash, no space or newline yet.
pub fn is_slash_word(text: &str) -> bool {
    text.starts_with('/') && !text.contains(char::is_whitespace)
}

/// A row of the session switcher.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Entry {
    /// A new session in the current directory.
    New,
    /// Index into [`Switcher::sessions`].
    Session(usize),
}

/// The overlay for Ctrl-S / `/sessions`.
#[derive(Debug, Clone)]
pub struct Switcher {
    /// None while the list is being fetched.
    pub sessions: Option<Vec<SessionInfo>>,
    pub filter: String,
    pub selected: usize,
    pub current: String,
    pub error: Option<String>,
}

impl Switcher {
    pub fn new(current: &str) -> Switcher {
        Switcher {
            sessions: None,
            filter: String::new(),
            selected: 0,
            current: current.to_string(),
            error: None,
        }
    }

    /// Most recently active first.
    pub fn set_sessions(&mut self, mut sessions: Vec<SessionInfo>) {
        sessions.sort_by_key(|s| std::cmp::Reverse(s.last_activity));
        self.sessions = Some(sessions);
        self.error = None;
        self.selected = 0;
        // Start on the current session when it is listed, else on the newest.
        if let Some(i) = self.entries().iter().position(|e| match e {
            Entry::Session(i) => self
                .sessions
                .as_ref()
                .is_some_and(|s| s[*i].id == self.current),
            Entry::New => false,
        }) {
            self.selected = i;
        } else if self.entries().len() > 1 {
            self.selected = 1;
        }
    }

    /// Rows that pass the filter; "new session" is always first.
    pub fn entries(&self) -> Vec<Entry> {
        let mut out = vec![Entry::New];
        let q = self.filter.to_lowercase();
        for (i, s) in self.sessions.iter().flatten().enumerate() {
            if q.is_empty() || haystack(s).contains(&q) {
                out.push(Entry::Session(i));
            }
        }
        out
    }

    pub fn selected_entry(&self) -> Option<Entry> {
        self.entries().get(self.selected).copied()
    }

    pub fn type_char(&mut self, ch: char) {
        self.filter.push(ch);
        self.selected = usize::from(self.entries().len() > 1);
    }

    pub fn backspace(&mut self) {
        self.filter.pop();
        self.selected = usize::from(!self.filter.is_empty() && self.entries().len() > 1);
    }

    pub fn move_by(&mut self, delta: isize) {
        let n = self.entries().len();
        if n == 0 {
            return;
        }
        self.selected = (self.selected as isize + delta).rem_euclid(n as isize) as usize;
    }
}

fn haystack(s: &SessionInfo) -> String {
    format!("{} {} {}", s.title.as_deref().unwrap_or(""), s.cwd, s.id).to_lowercase()
}

/// The session's title on one line, else "session " and the start of its id.
pub fn session_title(s: &SessionInfo) -> String {
    match s.title.as_deref().filter(|t| !t.trim().is_empty()) {
        Some(t) => t.split_whitespace().collect::<Vec<_>>().join(" "),
        None => format!("session {}", s.id.chars().take(8).collect::<String>()),
    }
}

/// "title  ~/dir  5m ago  running" for a switcher row, minus the width fitting.
pub fn session_label(s: &SessionInfo, home: Option<&str>, now_ms: i64) -> (String, String) {
    let name = session_title(s);
    let mut rest = format!(
        "{}  {}",
        shorten_home(&s.cwd, home),
        relative_time(s.last_activity, now_ms)
    );
    if s.state == SessionState::Running {
        rest.push_str("  running");
    }
    if s.queued > 0 {
        rest.push_str(&format!("  +{} queued", s.queued));
    }
    (name, rest)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(id: &str, title: Option<&str>, cwd: &str, last: i64) -> SessionInfo {
        SessionInfo {
            parent: None,
            id: id.into(),
            title: title.map(Into::into),
            cwd: cwd.into(),
            created_at: 0,
            last_activity: last,
            last_seq: 1,
            state: SessionState::Idle,
            queued: 0,
            preview: None,
            added: None,
            removed: None,
        }
    }

    #[test]
    fn slash_matching() {
        let names = |q: &str| matching(q).iter().map(|c| c.name).collect::<Vec<_>>();
        assert_eq!(names("/").len(), COMMANDS.len());
        assert_eq!(names("/c"), ["clear", "claude", "codex"]);
        assert_eq!(names("/mod"), ["model", "mode"]);
        assert_eq!(names("/h"), ["harness", "help"]);
        assert_eq!(names("/model"), ["model"]);
        // No prefix match falls back to containing.
        assert_eq!(names("/ssion"), ["sessions"]);
        assert!(names("/zzz").is_empty());
    }

    #[test]
    fn slash_words() {
        assert!(is_slash_word("/"));
        assert!(is_slash_word("/mod"));
        assert!(!is_slash_word("/model "));
        assert!(!is_slash_word("/a\nb"));
        assert!(!is_slash_word("hi /x"));
        assert_eq!(lookup("/model sonnet").map(|c| c.name), Some("model"));
        assert!(lookup("/nope").is_none());
        assert!(lookup("model").is_none());
    }

    #[test]
    fn switcher_sorts_filters_and_wraps() {
        let mut s = Switcher::new("b");
        assert_eq!(s.entries(), [Entry::New]);
        s.set_sessions(vec![
            info("a", Some("old fix"), "/w/a", 10),
            info("b", None, "/w/b", 30),
            info("c", Some("new site"), "/w/c", 20),
        ]);
        let ids: Vec<_> = s
            .entries()
            .iter()
            .map(|e| match e {
                Entry::New => "+".to_string(),
                Entry::Session(i) => s.sessions.as_ref().unwrap()[*i].id.clone(),
            })
            .collect();
        assert_eq!(ids, ["+", "b", "c", "a"]);
        assert_eq!(s.selected, 1, "starts on the current session");
        s.type_char('s');
        s.type_char('i');
        assert_eq!(s.entries().len(), 2, "only 'new site' matches (plus new)");
        assert_eq!(s.selected, 1);
        s.backspace();
        s.backspace();
        s.move_by(-1);
        assert_eq!(s.selected, 3, "wraps to the last row");
    }

    #[test]
    fn labels() {
        let mut i = info("abcdef123456", None, "/home/me/site", 0);
        i.state = SessionState::Running;
        i.queued = 2;
        let (name, rest) = session_label(&i, Some("/home/me"), 5 * 60_000);
        assert_eq!(name, "session abcdef12");
        assert_eq!(rest, "~/site  5m ago  running  +2 queued");
    }
}
