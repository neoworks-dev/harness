use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use neo_renderer_ratatui::Palette;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

use super::gh::{Gh, GhError, is_read_only};
use super::model::*;
use super::*;
use crate::style::Styles;
use neo_github::fixtures::*;

/// Answers `gh` from recorded output, and checks everything asked is a read.
struct Fake {
    replies: Mutex<Vec<(String, Result<String, GhError>)>>,
    calls: Mutex<Vec<Vec<String>>>,
    /// The writes asked for, with what went on their stdin.
    writes: Mutex<Vec<(Vec<String>, Option<String>)>>,
    /// Writes fail with this, when set.
    refuse: Mutex<Option<GhError>>,
}

impl Fake {
    fn new(replies: &[(&str, Result<&str, GhError>)]) -> Arc<Fake> {
        Arc::new(Fake {
            replies: Mutex::new(
                replies
                    .iter()
                    .map(|(k, r)| ((*k).to_string(), r.clone().map(str::to_string)))
                    .collect(),
            ),
            calls: Mutex::new(Vec::new()),
            writes: Mutex::new(Vec::new()),
            refuse: Mutex::new(None),
        })
    }

    /// The writes as one line each (`issue close 12 -R acme/site --reason completed`), with
    /// the text sent to stdin after ` <<< `.
    fn writes(&self) -> Vec<String> {
        self.writes
            .lock()
            .unwrap()
            .iter()
            .map(|(args, input)| match input {
                Some(text) => format!("{} <<< {text}", args.join(" ")),
                None => args.join(" "),
            })
            .collect()
    }

    /// The usual repository.
    fn acme() -> Arc<Fake> {
        Fake::new(&[
            ("repo view", Ok(REPO)),
            ("issue list", Ok(ISSUES)),
            ("pr list", Ok(PRS)),
            ("issue view 12", Ok(ISSUE_12)),
            ("pr view 31", Ok(PR_31)),
            ("pulls/31/comments", Ok(PR_31_COMMENTS)),
            ("pr diff 31", Ok(DIFF_31)),
            ("contents/src/header.css?ref=abc123", Ok(HEADER_CSS_AT_HEAD)),
            ("--job 7001 -R acme/site --log-failed", Ok(JOB_7001_FAILED)),
            ("--job 7001 -R acme/site --log", Ok(JOB_7001)),
        ])
    }

    fn calls(&self) -> Vec<String> {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .map(|a| a.join(" "))
            .collect()
    }

    fn count(&self, what: &str) -> usize {
        self.calls().iter().filter(|c| c.contains(what)).count()
    }
}

impl Gh for Fake {
    fn run(&self, _cwd: &Path, args: &[String]) -> Result<String, GhError> {
        assert!(is_read_only(args), "not read-only: {args:?}");
        self.calls.lock().unwrap().push(args.to_vec());
        let line = args.join(" ");
        let replies = self.replies.lock().unwrap();
        replies
            .iter()
            .find(|(k, _)| line.contains(k.as_str()))
            .map(|(_, r)| r.clone())
            .unwrap_or_else(|| Err(GhError::Other(format!("no recorded reply for: {line}"))))
    }

    fn write(&self, _cwd: &Path, args: &[String], input: Option<&str>) -> Result<String, GhError> {
        assert!(super::gh::is_allowed_write(args), "not an allowed write: {args:?}");
        self.writes
            .lock()
            .unwrap()
            .push((args.to_vec(), input.map(str::to_string)));
        match self.refuse.lock().unwrap().clone() {
            Some(e) => Err(e),
            None => Ok(String::new()),
        }
    }
}

fn github(fake: &Arc<Fake>) -> Github {
    Github::with_runner(
        PathBuf::from("/work/site"),
        Palette::default(),
        fake.clone(),
    )
}

fn styles() -> Styles {
    Styles::new(&Palette::default(), false)
}

fn busy(g: &Github) -> bool {
    matches!(g.repo, Repo::Loading)
        || g.lists.values().any(|l| l.loading)
        || g.details.values().any(|d| d.loading)
        || g.sending
        || g.mine_asked > 0
        || matches!(&g.mode, Mode::Detail(DetailView { diff: Some(d), .. }) if !d.is_loaded())
        || g.in_flight.get() > 0
}

/// Ticks until the background jobs are done; what they produced for the shell.
fn settle(g: &mut Github) -> Vec<Outgoing> {
    let end = Instant::now() + Duration::from_secs(5);
    let mut out = Vec::new();
    loop {
        out.extend(g.tick().out);
        if !busy(g) {
            // One more look: a job's answer may chain the next one.
            out.extend(g.tick().out);
            if !busy(g) {
                return out;
            }
        }
        assert!(Instant::now() < end, "jobs did not finish");
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent {
        code,
        modifiers: KeyModifiers::NONE,
        kind: KeyEventKind::Press,
        state: KeyEventState::NONE,
    }
}

fn press(g: &mut Github, code: KeyCode) -> Option<Outgoing> {
    g.key(key(code))
}

fn ch(g: &mut Github, c: char) -> Option<Outgoing> {
    press(g, KeyCode::Char(c))
}

fn type_str(g: &mut Github, s: &str) {
    s.chars().for_each(|c| {
        ch(g, c);
    });
}

fn opened(fake: &Arc<Fake>) -> Github {
    let mut g = github(fake);
    g.open();
    settle(&mut g);
    g
}

fn screen(g: &mut Github, w: u16, h: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
    let st = styles();
    terminal.draw(|f| g.draw(f, f.area(), NOW, &st, None)).unwrap();
    let buf = terminal.backend().buffer();
    (0..h)
        .map(|y| {
            (0..w)
                .map(|x| buf[(x, y)].symbol())
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn snapshot(name: &str, actual: &str) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/snapshots")
        .join(format!("{name}.txt"));
    if std::env::var_os("UPDATE_SNAPSHOTS").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, format!("{actual}\n")).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&path).unwrap_or_else(|_| {
        panic!("missing snapshot {name} (UPDATE_SNAPSHOTS=1 to write)\n{actual}")
    });
    assert_eq!(actual, expected.trim_end_matches('\n'), "snapshot {name}");
}

fn opened_issue(fake: &Arc<Fake>) -> Github {
    let mut g = opened(fake);
    press(&mut g, KeyCode::Enter);
    settle(&mut g);
    g
}

fn opened_pr(fake: &Arc<Fake>) -> Github {
    let mut g = opened(fake);
    ch(&mut g, '2');
    settle(&mut g);
    press(&mut g, KeyCode::Enter);
    settle(&mut g);
    g
}

// ---- the list ----

#[test]
fn opening_loads_the_repository_then_the_issues() {
    let fake = Fake::acme();
    let mut g = github(&fake);
    assert!(!g.is_open());
    g.open();
    assert!(screen(&mut g, 60, 8).contains("loading…"));
    settle(&mut g);
    let calls = fake.calls();
    assert!(calls[0].starts_with("repo view"));
    assert!(calls.iter().any(|c| c.starts_with("issue list -R acme/site")));
    // The labels (their colors) and the people (the search) come beside the list.
    assert!(calls.iter().any(|c| c.contains("acme/site/labels")));
    assert!(calls.iter().any(|c| c.contains("acme/site/assignees")));
    assert_eq!(calls.len(), 4);
    let list = g.lists.get(&g.current()).unwrap();
    assert_eq!(list.rows.len(), 3);
}

fn down(g: &mut Github, n: usize) {
    for _ in 0..n {
        press(g, KeyCode::Down);
    }
}

#[test]
fn tabs_switch_with_tab_and_digits() {
    let fake = Fake::acme();
    let mut g = opened(&fake);
    assert_eq!(g.tab, Kind::Issue);
    press(&mut g, KeyCode::Tab);
    settle(&mut g);
    assert_eq!(g.tab, Kind::Pr);
    ch(&mut g, '1');
    assert_eq!(g.tab, Kind::Issue);
    press(&mut g, KeyCode::BackTab);
    settle(&mut g);
    assert_eq!(g.tab, Kind::Pr);
    // Each list was fetched once.
    assert_eq!(fake.count("pr list"), 1);
    assert_eq!(fake.count("issue list"), 1);
}

#[test]
fn selection_moves_and_clamps() {
    let mut g = opened(&Fake::acme());
    let sel = |g: &Github| g.lists.get(&g.current()).unwrap().selected;
    ch(&mut g, 'j');
    assert_eq!(sel(&g), 1);
    press(&mut g, KeyCode::Down);
    press(&mut g, KeyCode::Down);
    assert_eq!(sel(&g), 2);
    ch(&mut g, 'k');
    assert_eq!(sel(&g), 1);
    ch(&mut g, 'g');
    assert_eq!(sel(&g), 0);
    ch(&mut g, 'G');
    assert_eq!(sel(&g), 2);
    press(&mut g, KeyCode::PageUp);
    assert_eq!(sel(&g), 0);
    press(&mut g, KeyCode::PageDown);
    assert_eq!(sel(&g), 2);
    press(&mut g, KeyCode::Home);
    g.wheel(true);
    assert_eq!(sel(&g), 2);
    g.wheel(false);
    assert_eq!(sel(&g), 0);
}

#[test]
fn the_selection_scrolls_into_view() {
    let rows: Vec<String> = (1..=40)
        .map(|n| {
            format!(r#"{{"number":{n},"title":"Issue number {n}","state":"OPEN","author":{{"login":"a"}},"labels":[],"comments":[],"updatedAt":"2026-10-16T12:00:00Z"}}"#)
        })
        .collect();
    let json = format!("[{}]", rows.join(","));
    let fake = Fake::new(&[("repo view", Ok(REPO)), ("issue list", Ok(&json))]);
    let mut g = opened(&fake);
    ch(&mut g, 'G');
    let text = screen(&mut g, 60, 10);
    assert!(text.contains("#40"), "{text}");
    assert!(!text.contains("Issue number 1\n"));
}

#[test]
fn slash_searches_and_o_toggles_all() {
    let fake = Fake::acme();
    let mut g = opened(&fake);
    ch(&mut g, '/');
    assert!(g.typing.is_some());
    type_str(&mut g, "label:bug menu");
    assert!(screen(&mut g, 60, 6).contains("/ label:bug menu"));
    press(&mut g, KeyCode::Enter);
    settle(&mut g);
    assert!(g.typing.is_none());
    let last = fake.calls().pop().unwrap();
    assert!(
        last.contains("--search label:bug menu sort:updated-desc"),
        "{last}"
    );
    assert!(screen(&mut g, 80, 6).contains("search: label:bug menu"));

    ch(&mut g, 'o');
    settle(&mut g);
    assert!(fake.calls().pop().unwrap().contains("--state all"));
    assert!(screen(&mut g, 80, 6).contains("all states"));
    ch(&mut g, 'o');
    // Back to a list already fetched: no new run.
    let before = fake.calls().len();
    settle(&mut g);
    assert_eq!(fake.calls().len(), before);
}

#[test]
fn escape_cancels_a_search_and_backspace_edits() {
    let fake = Fake::acme();
    let mut g = opened(&fake);
    ch(&mut g, '/');
    type_str(&mut g, "abc");
    press(&mut g, KeyCode::Backspace);
    assert_eq!(g.typing.as_deref(), Some("ab"));
    g.paste("c\nd");
    assert_eq!(g.typing.as_deref(), Some("abcd"));
    press(&mut g, KeyCode::Esc);
    assert!(g.typing.is_none() && g.query.is_empty() && g.is_open());
    // Typing swallowed the letters: nothing else happened.
    assert_eq!(fake.count("issue list"), 1);
}

#[test]
fn r_reloads_and_keeps_the_selection() {
    let fake = Fake::acme();
    let mut g = opened(&fake);
    down(&mut g, 2);
    ch(&mut g, 'r');
    settle(&mut g);
    assert_eq!(fake.count("issue list"), 2);
    assert_eq!(g.lists.get(&g.current()).unwrap().selected, 2);
}

#[test]
fn closing_and_reopening_uses_the_cache() {
    let fake = Fake::acme();
    let mut g = opened(&fake);
    press(&mut g, KeyCode::Esc);
    assert!(!g.is_open());
    g.open();
    settle(&mut g);
    assert!(g.is_open());
    assert_eq!(fake.count("issue list"), 1);
    assert_eq!(fake.count("repo view"), 1);
    // A new directory starts over; the same one keeps everything.
    g.reset(PathBuf::from("/work/site"));
    g.open();
    settle(&mut g);
    assert_eq!(fake.count("issue list"), 1);
    g.reset(PathBuf::from("/work/other"));
    g.open();
    settle(&mut g);
    assert_eq!(fake.count("issue list"), 2);
    assert_eq!(fake.count("repo view"), 2);
}

#[test]
fn empty_and_unmatched_lists_say_so() {
    let fake = Fake::new(&[("repo view", Ok(REPO)), ("issue list", Ok("[]"))]);
    let mut g = opened(&fake);
    assert!(screen(&mut g, 80, 6).contains("no open issues"));
    ch(&mut g, '/');
    type_str(&mut g, "zzz");
    press(&mut g, KeyCode::Enter);
    settle(&mut g);
    assert!(screen(&mut g, 80, 6).contains("nothing matches"));
}

#[test]
fn escape_and_q_close_the_list() {
    let mut g = opened(&Fake::acme());
    ch(&mut g, 'q');
    assert!(!g.is_open());
    g.open();
    press(&mut g, KeyCode::Esc);
    assert!(!g.is_open());
    g.open();
    g.key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL));
    assert!(!g.is_open());
}

// ---- errors ----

fn error_screen(err: GhError) -> String {
    let fake = Fake::new(&[("repo view", Err(err))]);
    let mut g = opened(&fake);
    screen(&mut g, 70, 8)
}

#[test]
fn a_missing_gh_is_explained() {
    let s = error_screen(GhError::Missing);
    assert!(s.contains("GitHub CLI (gh) is not installed"), "{s}");
    assert!(s.contains("r to try again"));
}

#[test]
fn a_directory_without_a_repository_is_explained() {
    let s = error_screen(gh::classify("no git remotes found"));
    assert!(s.contains("not a GitHub repository"), "{s}");
}

#[test]
fn missing_authentication_is_explained() {
    let s = error_screen(GhError::NotAuth);
    assert!(s.contains("gh auth login"), "{s}");
}

#[test]
fn r_retries_after_an_error() {
    let fake = Fake::new(&[("repo view", Err(GhError::NotAuth))]);
    let mut g = opened(&fake);
    assert!(matches!(g.repo, Repo::Failed(_)));
    *fake.replies.lock().unwrap() = vec![
        ("repo view".into(), Ok(REPO.into())),
        ("issue list".into(), Ok(ISSUES.into())),
    ];
    ch(&mut g, 'r');
    settle(&mut g);
    assert!(screen(&mut g, 80, 8).contains("Header jumps"));
}

#[test]
fn a_failing_list_shows_the_error_inline() {
    let fake = Fake::new(&[
        ("repo view", Ok(REPO)),
        (
            "issue list",
            Err(GhError::Other("HTTP 502: bad gateway".into())),
        ),
        ("pr list", Ok(PRS)),
    ]);
    let mut g = opened(&fake);
    let s = screen(&mut g, 70, 8);
    assert!(s.contains("HTTP 502: bad gateway"), "{s}");
    // The other tab is not affected.
    ch(&mut g, '2');
    settle(&mut g);
    let s = screen(&mut g, 70, 8);
    assert!(s.contains("Fix the sticky header"), "{s}");
}

// ---- the detail ----

#[test]
fn enter_opens_the_detail_and_escape_goes_back() {
    let fake = Fake::acme();
    let mut g = opened(&fake);
    press(&mut g, KeyCode::Enter);
    assert!(screen(&mut g, 80, 10).contains("loading"));
    settle(&mut g);
    assert!(matches!(g.mode, Mode::Detail(_)));
    assert!(screen(&mut g, 80, 10).contains("Issue #12"));
    press(&mut g, KeyCode::Esc);
    assert!(matches!(g.mode, Mode::List) && g.is_open());
    // Open it again: cached.
    press(&mut g, KeyCode::Enter);
    settle(&mut g);
    assert_eq!(fake.count("issue view"), 1);
    ch(&mut g, 'q');
    assert!(!g.is_open());
}

#[test]
fn opened_at_a_number_it_shows_that_one_once_the_repository_is_known() {
    let fake = Fake::acme();
    let mut g = github(&fake);
    assert_eq!(g.repo(), None);
    g.open_number(Kind::Pr, 31);
    assert!(g.is_open());
    settle(&mut g);
    assert_eq!(g.repo(), Some("acme/site"));
    assert_eq!(fake.count("pr view 31"), 1);
    assert!(screen(&mut g, 80, 10).contains("#31"));
    // Esc goes on to the list of pull requests.
    press(&mut g, KeyCode::Esc);
    assert!(matches!(g.mode, Mode::List) && g.tab == Kind::Pr);
}

#[test]
fn a_newer_item_than_the_cached_detail_is_fetched_again() {
    let fake = Fake::acme();
    let mut g = opened_issue(&fake);
    press(&mut g, KeyCode::Esc);
    // The list says the issue changed since.
    g.lists.get_mut(&g.current()).unwrap().rows[0].updated_ms += 60_000;
    press(&mut g, KeyCode::Enter);
    settle(&mut g);
    assert_eq!(fake.count("issue view"), 2);
}

fn top(g: &Github) -> usize {
    match &g.mode {
        Mode::Detail(v) => v.scroll,
        Mode::List => unreachable!(),
    }
}

fn focus(g: &Github) -> doc::Part {
    match &g.mode {
        Mode::Detail(v) => v.focus,
        Mode::List => unreachable!(),
    }
}

/// The block in focus is in view, all of it when it fits.
fn focus_in_view(g: &Github) -> bool {
    let Mode::Detail(v) = &g.mode else {
        return false;
    };
    let b = v.block().unwrap();
    if b.end - b.start <= v.rows {
        b.start >= v.scroll && b.end <= v.scroll + v.rows
    } else {
        b.end > v.scroll && b.start < v.scroll + v.rows
    }
}

#[test]
fn j_and_k_step_through_the_blocks_and_the_view_follows() {
    use doc::Part;
    let mut g = opened_pr(&Fake::acme());
    let first = screen(&mut g, 60, 12);
    assert!(first.contains("lines 1–"), "{first}");
    assert_eq!(focus(&g), Part::Head);
    ch(&mut g, 'j');
    assert_eq!(focus(&g), Part::Description);
    // Down to the files at the end, each block in view on the way.
    let mut seen = vec![Part::Head, Part::Description];
    for _ in 0..40 {
        ch(&mut g, 'j');
        screen(&mut g, 60, 12);
        assert!(focus_in_view(&g), "{:?}", focus(&g));
        if seen.last() != Some(&focus(&g)) {
            seen.push(focus(&g));
        }
    }
    assert_eq!(seen.last(), Some(&Part::Files));
    assert!(seen.iter().any(|p| matches!(p, Part::Comment(_))));
    assert!(seen.contains(&Part::Checks));
    assert!(screen(&mut g, 60, 12).contains("src/layout.css"));
    // `k` goes back, `g` is the header again.
    ch(&mut g, 'k');
    assert_eq!(focus(&g), Part::Checks);
    ch(&mut g, 'g');
    assert_eq!((focus(&g), top(&g)), (Part::Head, 0));
    ch(&mut g, 'G');
    assert_eq!(focus(&g), Part::Files);
}

#[test]
fn the_wheel_scrolls_and_the_focus_stays_in_view() {
    let mut g = opened_pr(&Fake::acme());
    screen(&mut g, 60, 12);
    for _ in 0..4 {
        g.wheel(true);
        screen(&mut g, 60, 12);
    }
    assert!(top(&g) > 0);
    assert_ne!(focus(&g), doc::Part::Head, "the header left the view");
    let Mode::Detail(v) = &g.mode else { unreachable!() };
    let b = v.block().unwrap();
    assert!(b.end > v.scroll && b.start < v.scroll + v.rows);
    for _ in 0..10 {
        g.wheel(false);
    }
    assert_eq!(top(&g), 0);
}

#[test]
fn keys_only_act_on_the_block_they_belong_to() {
    let fake = with_links();
    let mut g = opened_issue(&fake);
    screen(&mut g, 70, 24);
    // On the header: no reacting or editing text, only where it goes says the footer.
    ch(&mut g, '+');
    assert!(g.writing.is_none());
    assert!(screen(&mut g, 70, 24).contains("+ works on the description or a comment"));
    ch(&mut g, 'E');
    assert!(g.writing.is_none());
    // On the description: no closing, no title.
    ch(&mut g, 'j');
    ch(&mut g, 'x');
    ch(&mut g, 'S');
    assert!(g.writing.is_none() && g.note.is_some());
    ch(&mut g, '+');
    assert!(matches!(g.writing, Some(write::Writing::React)));
    press(&mut g, KeyCode::Esc);
    // The footer offers what the block takes.
    let shown = screen(&mut g, 200, 24);
    assert!(shown.contains("E edit") && !shown.contains("x close"), "{shown}");
    ch(&mut g, 'k');
    let shown = screen(&mut g, 200, 24);
    assert!(shown.contains("x close") && !shown.contains("E edit"), "{shown}");
    assert!(fake.writes().is_empty());
}

#[test]
fn greater_than_quotes_the_whole_block_into_the_input() {
    let fake = with_links();
    let mut g = opened_issue(&fake);
    screen(&mut g, 70, 24);
    ch(&mut g, 'j');
    ch(&mut g, 'j');
    ch(&mut g, '>');
    assert_eq!(g.draft.text(), "> I can reproduce this.\n\n");
    assert!(matches!(g.writing, Some(write::Writing::Comment)));
}

#[test]
fn the_input_is_always_there_and_tab_moves_in_and_out() {
    let fake = with_links();
    let mut g = opened_issue(&fake);
    assert!(screen(&mut g, 70, 24).contains("tab to comment on #12"));
    press(&mut g, KeyCode::Tab);
    type_str(&mut g, "hi");
    press(&mut g, KeyCode::Tab);
    assert!(g.writing.is_none());
    // Still shown while reading, and kept.
    assert!(screen(&mut g, 70, 24).contains("› hi"));
    // Keys read the detail again: j moves the focus, not the text.
    ch(&mut g, 'j');
    assert_eq!(g.draft.text(), "hi");
    assert_eq!(focus(&g), doc::Part::Description);
}

#[test]
fn the_pull_request_diff_opens_a_file_at_a_time() {
    let fake = Fake::acme();
    let mut g = opened_pr(&fake);
    ch(&mut g, 'd');
    assert!(screen(&mut g, 80, 12).contains("…"));
    settle(&mut g);
    // The first file, only it, with its review comments on their lines.
    let s = screen(&mut g, 100, 30);
    assert!(s.contains("src/header.css  1/2"), "{s}");
    assert!(s.contains("scrollbar-gutter: stable") && !s.contains("margin: 0"), "{s}");
    assert!(s.contains("Use a variable here.") && s.contains("↳ bob"), "{s}");
    assert!(s.contains("Why not fixed?"), "on the removed line: {s}");
    // `]` is the next file, `[` back; past the ends they stay.
    ch(&mut g, ']');
    let s = screen(&mut g, 100, 30);
    assert!(s.contains("src/layout.css  2/2") && s.contains("margin: 0"), "{s}");
    ch(&mut g, ']');
    assert!(screen(&mut g, 100, 30).contains("2/2"));
    ch(&mut g, '[');
    assert!(screen(&mut g, 100, 30).contains("1/2"));
    // The diff's own keys work; Esc comes back to the detail, not out of the view.
    ch(&mut g, 's');
    g.wheel(true);
    press(&mut g, KeyCode::Esc);
    assert!(screen(&mut g, 80, 12).contains("Pull request #31"));
    assert!(g.is_open());
    // Cached for the next time.
    ch(&mut g, 'd');
    assert!(matches!(&g.mode, Mode::Detail(v) if v.diff.as_ref().is_some_and(|d| d.is_loaded())));
    assert_eq!(fake.count("pr diff"), 1);
}

/// Walks the detail's focus to `part`.
fn focus_on(g: &mut Github, part: doc::Part) {
    screen(g, 100, 40);
    for _ in 0..40 {
        if focus(g) == part {
            return;
        }
        ch(g, 'j');
        screen(g, 100, 40);
    }
    panic!("no {part:?}");
}

#[test]
fn a_file_is_picked_from_the_files_and_f_shows_it_whole() {
    let fake = Fake::acme();
    let mut g = opened_pr(&fake);
    focus_on(&mut g, doc::Part::Files);
    assert!(screen(&mut g, 100, 40).contains("3 comments"));
    keys(&mut g, "\nj\n");
    settle(&mut g);
    assert!(screen(&mut g, 100, 30).contains("src/layout.css  2/2"));
    ch(&mut g, '[');
    // The whole file, from GitHub at the head commit, once; the changes stay marked.
    ch(&mut g, 'f');
    settle(&mut g);
    let calls = fake.calls();
    assert!(calls.iter().any(|c| c.contains("contents/src/header.css?ref=abc123")), "{calls:?}");
    let s = screen(&mut g, 100, 30);
    assert!(s.contains("scrollbar-gutter: stable") && s.contains("Use a variable here."), "{s}");
    ch(&mut g, 'f');
    ch(&mut g, 'f');
    assert_eq!(fake.count("contents/"), 1, "kept");
}

/// The view's own (unposted) comments in the open diff.
fn own_comments(g: &Github) -> usize {
    match &g.mode {
        Mode::Detail(v) => v.diff.as_ref().map_or(0, |d| d.comment_count()),
        Mode::List => 0,
    }
}

#[test]
fn a_comment_written_in_a_files_diff_is_posted_as_a_review_comment() {
    let fake = Fake::acme();
    let mut g = opened_pr(&fake);
    ch(&mut g, 'd');
    settle(&mut g);
    screen(&mut g, 100, 30);
    // Down from the hunk header to line 1, which is in both versions: the new one's.
    keys(&mut g, "jcnit");
    press(&mut g, KeyCode::Enter);
    settle(&mut g);
    let writes = fake.writes();
    assert_eq!(writes.len(), 1, "{writes:?}");
    let (args, json) = writes[0].split_once(" <<< ").unwrap();
    assert_eq!(args, "api -X POST repos/acme/site/pulls/31/comments --input -");
    let v: serde_json::Value = serde_json::from_str(json).unwrap();
    assert_eq!(v["body"], "nit");
    assert_eq!(v["path"], "src/header.css");
    assert_eq!(v["commit_id"], "abc123");
    assert_eq!((v["line"].as_u64(), v["side"].as_str()), (Some(1), Some("RIGHT")));
    assert!(v["start_line"].is_null());
    // It is GitHub's now: not the view's own, and the pull request was fetched again.
    assert_eq!(own_comments(&g), 0);
    assert_eq!(fake.count("pr view 31"), 2);
    assert!(screen(&mut g, 100, 30).contains("review comment posted"));
}

#[test]
fn a_review_comment_that_could_not_be_posted_is_back_on_its_lines() {
    let fake = Fake::acme();
    *fake.refuse.lock().unwrap() = Some(GhError::Other("HTTP 422: line not in the diff".into()));
    let mut g = opened_pr(&fake);
    ch(&mut g, 'd');
    settle(&mut g);
    screen(&mut g, 100, 30);
    // Lines 2 and 3 of the new version, selected.
    keys(&mut g, "jjvjcmore");
    press(&mut g, KeyCode::Enter);
    settle(&mut g);
    let json = fake.writes()[0].split_once(" <<< ").unwrap().1.to_string();
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!((v["start_line"].as_u64(), v["line"].as_u64()), (Some(2), Some(3)));
    assert_eq!(own_comments(&g), 1, "kept to try again");
    assert!(screen(&mut g, 100, 30).contains("line not in the diff"));
}

#[test]
fn a_reload_after_a_push_fetches_the_diff_again() {
    let fake = Fake::acme();
    let mut g = opened_pr(&fake);
    ch(&mut g, 'd');
    settle(&mut g);
    // Nothing new: the diff is kept.
    g.load_detail(Kind::Pr, 31);
    settle(&mut g);
    assert_eq!(fake.count("pr diff"), 1);
    // A push: a new head commit. The diff is fetched again, the file stays open.
    let pushed = PR_31.replace("abc123", "def456");
    fake.replies
        .lock()
        .unwrap()
        .insert(0, ("pr view 31".into(), Ok(pushed)));
    g.load_detail(Kind::Pr, 31);
    settle(&mut g);
    assert_eq!(fake.count("pr diff"), 2);
    assert!(screen(&mut g, 100, 30).contains("src/header.css  1/2"));
    // And from the detail too: `d` after `r` uses the new one.
    press(&mut g, KeyCode::Esc);
    ch(&mut g, 'd');
    settle(&mut g);
    assert_eq!(fake.count("pr diff"), 2, "the new diff is the one cached");
}

#[test]
fn a_check_opens_its_log_failed_steps_first_or_copies_its_link() {
    let fake = Fake::acme();
    let mut g = opened_pr(&fake);
    focus_on(&mut g, doc::Part::Checks);
    let s = screen(&mut g, 100, 40);
    assert!(s.contains("✗ test (ubuntu)  3m 20s"), "{s}");
    // The failed one is first: Enter picks it, Enter opens its log.
    keys(&mut g, "\n\n");
    settle(&mut g);
    assert!(fake.calls().iter().any(|c| c == "run view --job 7001 -R acme/site --log-failed"));
    let s = screen(&mut g, 100, 20);
    assert!(s.contains("test (ubuntu) · failed steps") && s.contains("FAIL header"), "{s}");
    assert!(!s.contains("2026-10-16T"), "no times: {s}");
    // An `f` typed into a search is the search's.
    keys(&mut g, "/fail\n");
    assert!(screen(&mut g, 100, 20).contains("failed steps"));
    assert!(!fake.calls().iter().any(|c| c.ends_with("--log")), "the whole log not asked for");
    // `f` is the whole log; `y` copies the line.
    ch(&mut g, 'f');
    settle(&mut g);
    assert!(screen(&mut g, 100, 20).contains("all steps"));
    let copied = keys_out(&mut g, "yy");
    assert!(matches!(copied, Some(Outgoing::Copy(_))), "{copied:?}");
    // q is back to the detail, the check still picked.
    ch(&mut g, 'q');
    assert!(g.is_open() && screen(&mut g, 100, 40).contains("Pull request #31"));
    // A status from another CI: its link is copied.
    keys(&mut g, "jj");
    let out = press(&mut g, KeyCode::Enter);
    assert!(matches!(out, Some(Outgoing::Copy(url)) if url == "https://deploy.example.com/31"));
}

/// Presses the keys, what the last one gave.
fn keys_out(g: &mut Github, text: &str) -> Option<Outgoing> {
    let mut out = None;
    for c in text.chars() {
        out = ch(g, c);
    }
    out
}

#[test]
fn issues_have_no_diff() {
    let fake = Fake::acme();
    let mut g = opened_issue(&fake);
    ch(&mut g, 'd');
    assert_eq!(fake.count("pr diff"), 0);
    assert!(matches!(&g.mode, Mode::Detail(v) if v.diff.is_none()));
}

#[test]
fn a_failing_diff_says_why() {
    let fake = Fake::new(&[
        ("repo view", Ok(REPO)),
        ("pr list", Ok(PRS)),
        ("pr view 31", Ok(PR_31)),
        ("pulls/31", Err(GhError::Other("HTTP 404".into()))),
        ("pr diff", Err(GhError::Other("diff too large".into()))),
    ]);
    let mut g = opened(&fake);
    ch(&mut g, '2');
    settle(&mut g);
    press(&mut g, KeyCode::Enter);
    settle(&mut g);
    ch(&mut g, 'd');
    settle(&mut g);
    assert!(screen(&mut g, 80, 8).contains("diff too large"));
}

#[test]
fn a_failing_detail_shows_the_error_and_r_retries() {
    let fake = Fake::new(&[
        ("repo view", Ok(REPO)),
        ("issue list", Ok(ISSUES)),
        ("issue view 12", Err(GhError::Other("HTTP 500".into()))),
    ]);
    let mut g = opened(&fake);
    press(&mut g, KeyCode::Enter);
    settle(&mut g);
    assert!(screen(&mut g, 70, 8).contains("HTTP 500"));
    fake.replies
        .lock()
        .unwrap()
        .insert(0, ("issue view 12".into(), Ok(ISSUE_12.into())));
    ch(&mut g, 'r');
    settle(&mut g);
    assert!(screen(&mut g, 70, 8).contains("Header jumps"));
}

// ---- to the model ----

fn attach_text(out: Option<Outgoing>) -> String {
    match out {
        Some(Outgoing::Attach(t)) => t,
        other => panic!("expected an attach, got {other:?}"),
    }
}

#[test]
fn a_in_a_detail_attaches_the_issue() {
    let mut g = opened_issue(&Fake::acme());
    let text = attach_text(ch(&mut g, 'a'));
    assert!(!g.is_open());
    assert!(text.starts_with(
        "<github-issue repo=\"acme/site\" number=\"12\" url=\"https://github.com/acme/site/issues/12\" state=\"open\">\n"
    ));
    assert!(text.ends_with("</github-issue>"));
    assert!(text.contains("Title: Header jumps when the menu opens"));
    assert!(text.contains("Labels: bug, ui"));
    assert!(text.contains("Opening the **menu**"));
    // Template comments are noise for the model.
    assert!(!text.contains("<!--") && !text.contains("please describe"));
    assert!(text.contains("bob (2026-10-11): I can reproduce this on main."));
    assert!(text.contains("carol (2026-10-16): Probably the scrollbar gutter.\nTrying a fix."));
    snapshot("github-attach-issue", &text);
}

#[test]
fn a_attaches_the_pull_request_with_reviews_and_checks() {
    let mut g = opened_pr(&Fake::acme());
    let text = attach_text(ch(&mut g, 'a'));
    assert!(text.starts_with("<github-pr repo=\"acme/site\" number=\"31\""));
    assert!(
        text.contains("Branch: fix-header -> main · +14 -3 in 2 files · Review: CHANGES_REQUESTED")
    );
    assert!(text.contains("Checks: 2 passed, 1 failed, 1 pending (failing: test (ubuntu))"));
    assert!(text.contains("src/header.css (+10 -3)"));
    assert!(text.contains(
        "alice (2026-10-16, review changes requested): The gutter should be a variable."
    ));
    assert!(text.contains("alice (2026-10-16, on src/header.css:3): Use a variable here."));
    // A review that only says "commented" is left out.
    assert!(!text.contains("carol (2026-10-16, review commented)"));
    snapshot("github-attach-pr", &text);
}

#[test]
fn s_sends_with_an_instruction_line() {
    let mut g = opened_issue(&Fake::acme());
    let Some(Outgoing::Send(text)) = ch(&mut g, 's') else {
        panic!("expected a send");
    };
    assert!(text.starts_with("Here is issue #12 from GitHub.\n\n<github-issue "));
    assert!(!g.is_open());
    let mut g = opened_pr(&Fake::acme());
    let Some(Outgoing::Send(text)) = ch(&mut g, 's') else {
        panic!("expected a send");
    };
    assert!(text.starts_with("Here is pull request #31 from GitHub.\n\n<github-pr "));
}

#[test]
fn a_from_the_list_fetches_then_attaches() {
    let fake = Fake::acme();
    let mut g = opened(&fake);
    assert_eq!(ch(&mut g, 'a'), None);
    assert!(screen(&mut g, 80, 6).contains("fetching #12…"));
    let out = settle(&mut g);
    assert_eq!(out.len(), 1);
    assert!(matches!(&out[0], Outgoing::Attach(t) if t.contains("number=\"12\"")));
    assert!(!g.is_open());
    // A second time the detail is cached: immediate.
    g.open();
    let out = ch(&mut g, 's');
    assert!(matches!(out, Some(Outgoing::Send(_))));
    assert_eq!(fake.count("issue view"), 1);
}

#[test]
fn a_failed_fetch_for_an_attach_is_reported_in_the_footer() {
    let fake = Fake::new(&[
        ("repo view", Ok(REPO)),
        ("issue list", Ok(ISSUES)),
        ("issue view", Err(GhError::Other("rate limited".into()))),
    ]);
    let mut g = opened(&fake);
    ch(&mut g, 's');
    let out = settle(&mut g);
    assert!(out.is_empty() && g.is_open());
    assert!(screen(&mut g, 80, 6).contains("could not load #12: rate limited"));
    ch(&mut g, 'j');
    assert!(!screen(&mut g, 80, 6).contains("rate limited"));
}

// ---- in the app ----

use crate::app::{App, Config, Effect};
use crate::history::HistoryStore;

fn app(fake: &Arc<Fake>) -> App {
    let mut app = App::new(Config {
        app_id: "terminal".into(),
        session: "s1".into(),
        cwd: "/work/site".into(),
        home: None,
        no_color: false,
        history: HistoryStore::default(),
        prefs: crate::theme::Prefs::default(),
    });
    *app.github_mut() = github(fake);
    app.tick(NOW);
    app
}

/// Ticks the app until the view's jobs are done; the effects they caused.
fn app_settle(app: &mut App) -> Vec<Effect> {
    let end = Instant::now() + Duration::from_secs(5);
    let mut fx = Vec::new();
    loop {
        fx.extend(app.tick(NOW));
        if !busy(app.github_mut()) {
            fx.extend(app.tick(NOW));
            return fx;
        }
        assert!(Instant::now() < end, "jobs did not finish");
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn app_screen(app: &mut App, w: u16, h: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
    terminal.draw(|f| app.draw(f)).unwrap();
    let buf = terminal.backend().buffer();
    (0..h)
        .map(|y| {
            (0..w)
                .map(|x| buf[(x, y)].symbol())
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn app_key(app: &mut App, code: KeyCode) -> Vec<Effect> {
    app.handle_key(key(code))
}

fn ctrl_r(app: &mut App) {
    assert!(
        app.handle_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL))
            .is_empty()
    );
}

fn type_app(app: &mut App, s: &str) {
    for c in s.chars() {
        app_key(app, KeyCode::Char(c));
    }
}

#[test]
fn ctrl_r_opens_the_view_over_the_chat() {
    let fake = Fake::acme();
    let mut app = app(&fake);
    assert!(!app_screen(&mut app, 100, 30).contains("Issues"));
    ctrl_r(&mut app);
    app_settle(&mut app);
    let s = app_screen(&mut app, 100, 30);
    assert!(s.contains("Issues") && s.contains("Header jumps"), "{s}");
    // Typing goes to the view, not the editor.
    type_app(&mut app, "zz");
    assert_eq!(app.editor_text(), "");
    app_key(&mut app, KeyCode::Esc);
    assert!(!app.github_mut().is_open());
    assert!(!app_screen(&mut app, 100, 30).contains("Issues"));
}

#[test]
fn the_github_command_opens_it_and_is_in_the_menu_and_help() {
    let fake = Fake::acme();
    let mut app = app(&fake);
    type_app(&mut app, "/gith");
    assert!(app_screen(&mut app, 100, 30).contains("/github"));
    assert!(app_key(&mut app, KeyCode::Enter).is_empty());
    assert!(app.github_mut().is_open());
    assert_eq!(app.editor_text(), "");
    app_settle(&mut app);
    ctrl_r(&mut app);
    assert!(!app.github_mut().is_open());
    type_app(&mut app, "?");
    let help = app_screen(&mut app, 100, 60);
    assert!(help.contains("ctrl-r"), "{help}");
}

#[test]
fn attaching_puts_the_block_in_the_editor_for_instructions() {
    let fake = Fake::acme();
    let mut app = app(&fake);
    type_app(&mut app, "please fix");
    ctrl_r(&mut app);
    app_settle(&mut app);
    app_key(&mut app, KeyCode::Enter);
    app_settle(&mut app);
    assert!(app_key(&mut app, KeyCode::Char('a')).is_empty());
    assert!(!app.github_mut().is_open());
    let text = app.editor_text().to_string();
    assert!(
        text.starts_with("please fix\n\n<github-issue repo=\"acme/site\" number=\"12\""),
        "{text}"
    );
    assert!(text.ends_with("</github-issue>\n\n"));
    // The editor shows the end of the block, with the cursor below it.
    let s = app_screen(&mut app, 100, 30);
    assert!(s.contains("</github-issue>"), "{s}");
}

#[test]
fn attach_from_the_list_waits_for_the_fetch_in_the_app() {
    let fake = Fake::acme();
    let mut app = app(&fake);
    ctrl_r(&mut app);
    app_settle(&mut app);
    app_key(&mut app, KeyCode::Char('a'));
    assert_eq!(app.editor_text(), "");
    app_settle(&mut app);
    assert!(app.editor_text().starts_with("<github-issue "));
    assert!(!app.github_mut().is_open());
}

#[test]
fn s_sends_through_the_editor_path() {
    let fake = Fake::acme();
    let mut app = app(&fake);
    ctrl_r(&mut app);
    app_settle(&mut app);
    app_key(&mut app, KeyCode::Char('s'));
    let fx = app_settle(&mut app);
    let [Effect::Input { session, text, .. }] = fx.as_slice() else {
        panic!("expected one input, got {fx:?}");
    };
    assert_eq!(session, "s1");
    assert!(text.starts_with("Here is issue #12 from GitHub.\n\n<github-issue "));
    assert!(!app.github_mut().is_open());
    assert_eq!(app.editor_text(), "");
    // From a detail the key gives the effect at once.
    ctrl_r(&mut app);
    app_key(&mut app, KeyCode::Enter);
    app_settle(&mut app);
    let fx = app_key(&mut app, KeyCode::Char('s'));
    assert!(
        matches!(fx.as_slice(), [Effect::Input { text, .. }] if text.contains("number=\"12\""))
    );
}

#[test]
fn the_wheel_reaches_the_view() {
    use ratatui::crossterm::event::{MouseEvent, MouseEventKind};
    let fake = Fake::acme();
    let mut app = app(&fake);
    ctrl_r(&mut app);
    app_settle(&mut app);
    app_key(&mut app, KeyCode::Enter);
    app_settle(&mut app);
    app_screen(&mut app, 80, 12);
    let wheel = MouseEvent {
        kind: MouseEventKind::ScrollDown,
        column: 1,
        row: 1,
        modifiers: KeyModifiers::NONE,
    };
    app.handle_mouse(wheel);
    let s = app_screen(&mut app, 80, 12);
    assert!(s.contains("lines 4–"), "{s}");
}

#[test]
fn a_session_switch_closes_the_view_and_drops_the_cache() {
    let fake = Fake::acme();
    let mut app = app(&fake);
    ctrl_r(&mut app);
    app_settle(&mut app);
    // Another session in another directory.
    let info = neo_protocol::SessionInfo {
        harness: None,
        usage: None,
        id: "s2".into(),
        title: None,
        cwd: "/work/other".into(),
        created_at: 0,
        last_activity: 0,
        last_seq: 0,
        state: neo_protocol::SessionState::Idle,
        queued: 0,
        waiting: None,
        asking: None,
        waiting_on: None,
        unseen: None,
        preview: None,
        added: None,
        removed: None,
        parent: None,
    };
    app.handle_msg(crate::app::Msg::Created(Ok(Box::new(info))));
    assert!(!app.github_mut().is_open());
    ctrl_r(&mut app);
    app_settle(&mut app);
    assert_eq!(fake.count("repo view"), 2);
}

#[test]
fn the_detail_leaves_room_for_the_avatars_of_its_people_when_pictures_are_drawn() {
    let detail = neo_github::model::parse_detail(Kind::Issue, ISSUE_12).unwrap();
    let st = crate::style::Styles::new(&neo_renderer_ratatui::Palette::default(), false);
    let colors = view::Colors::new();
    let mut options = doc::With {
        colors: &colors,
        pictures: false,
        fields: &[],
    };
    let plain = doc::build(&detail, 60, NOW, &st, &options);
    assert!(plain.avatars.is_empty());
    options.pictures = true;
    let with = doc::build(&detail, 60, NOW, &st, &options);
    let avatars = with.avatars;
    assert_eq!(with.lines.len(), plain.lines.len(), "the room is in the lines, not more of them");
    let comments = with.blocks.iter().filter(|b| matches!(b.part, doc::Part::Comment(_)));
    assert_eq!(comments.count(), detail.comments.len());
    // The creator first, then each commenter, each on a line of its own.
    assert_eq!(avatars.len(), 1 + detail.comments.len());
    assert_eq!(avatars[0].login, detail.author);
    for (a, c) in avatars[1..].iter().zip(&detail.comments) {
        assert_eq!(a.login, c.author);
    }
    assert!(avatars.windows(2).all(|w| w[0].line < w[1].line));
}

// ---- writing ----

/// An issue whose comments have links, so they can be reacted to.
const ISSUE_WITH_LINKS: &str = r#"{
 "number":12,"title":"Header jumps","state":"OPEN","author":{"login":"alice"},"labels":[],
 "url":"https://github.com/acme/site/issues/12",
 "createdAt":"2026-10-10T09:00:00Z","updatedAt":"2026-10-16T12:00:00Z",
 "body":"Opening the menu shifts the header.\nSecond line.",
 "comments":[
  {"url":"https://github.com/acme/site/issues/12#issuecomment-501","author":{"login":"bob"},"createdAt":"2026-10-11T09:00:00Z","body":"I can reproduce this."},
  {"url":"https://github.com/acme/site/issues/12#issuecomment-502","author":{"login":"carol"},"createdAt":"2026-10-16T12:00:00Z","body":"The scrollbar gutter."}
 ]}"#;

fn with_links() -> Arc<Fake> {
    Fake::new(&[
        ("repo view", Ok(REPO)),
        ("issue list", Ok(ISSUES)),
        ("pr list", Ok(PRS)),
        ("issue view 12", Ok(ISSUE_WITH_LINKS)),
        ("labels", Ok("[]")),
        ("assignees", Ok("[]")),
        ("api user", Ok(r#"{"login":"me"}"#)),
        ("/reactions?per_page=100", Ok("[]")),
    ])
}

fn ctrl(g: &mut Github, c: char) {
    g.key(KeyEvent {
        modifiers: KeyModifiers::CONTROL,
        ..key(KeyCode::Char(c))
    });
}

#[test]
fn a_comment_is_typed_in_a_box_and_posted_with_ctrl_s() {
    let fake = with_links();
    let mut g = opened_issue(&fake);
    ch(&mut g, 'c');
    type_str(&mut g, "first");
    press(&mut g, KeyCode::Enter);
    type_str(&mut g, "second");
    let shown = screen(&mut g, 70, 24);
    assert!(shown.contains("ctrl-s post") && shown.contains("second"), "{shown}");
    ctrl(&mut g, 's');
    settle(&mut g);
    assert_eq!(
        fake.writes(),
        ["issue comment 12 -R acme/site --body-file - <<< first\nsecond"]
    );
    // The issue is fetched again to show it, and the box is gone.
    assert_eq!(fake.count("issue view 12"), 2);
    assert!(g.writing.is_none());
    assert!(screen(&mut g, 70, 24).contains("comment posted"));
}

#[test]
fn esc_gives_up_a_comment_and_an_empty_one_is_not_posted() {
    let fake = with_links();
    let mut g = opened_issue(&fake);
    ch(&mut g, 'c');
    ctrl(&mut g, 's');
    assert!(g.writing.is_some(), "still open: it needs some text");
    assert!(g.note.is_some());
    type_str(&mut g, "x");
    press(&mut g, KeyCode::Esc);
    assert!(g.writing.is_none());
    assert!(fake.writes().is_empty());
}

#[test]
fn a_line_of_a_comment_is_selected_and_added_to_the_message() {
    let fake = with_links();
    let mut g = opened_issue(&fake);
    screen(&mut g, 70, 24);
    // Down to the second comment and into it; its second line is its text, `V` selects it,
    // `a` adds it.
    keys(&mut g, "jjj\n");
    ch(&mut g, 'j');
    ch(&mut g, 'V');
    ch(&mut g, 'a');
    assert!(matches!(g.writing, Some(write::Writing::Comment)));
    assert_eq!(g.draft.text(), "> The scrollbar gutter.\n\n");
    type_str(&mut g, "agreed");
    ctrl(&mut g, 's');
    settle(&mut g);
    assert_eq!(
        fake.writes(),
        ["issue comment 12 -R acme/site --body-file - <<< > The scrollbar gutter.\n\nagreed"]
    );
    assert!(g.draft.is_empty(), "posted, so the draft is empty");
}

/// Goes into the block with the first line that has `text` in it, the vim cursor on that line,
/// as after walking there.
fn goto_text(g: &mut Github, text: &str) {
    let Mode::Detail(v) = &mut g.mode else {
        panic!("a detail");
    };
    let doc = v.doc.as_ref().expect("drawn");
    let row = doc
        .lines
        .iter()
        .position(|l| l.spans.iter().any(|s| s.content.contains(text)))
        .unwrap_or_else(|| panic!("{text:?} on screen"));
    let b = *doc.blocks.iter().find(|b| row >= b.start && row < b.end).expect("in a block");
    if v.focus != b.part || v.cur.is_none() {
        v.focus = b.part;
        v.enter();
    }
    v.cur.as_mut().unwrap().go_to_row(row - b.start);
}

#[test]
fn a_block_is_walked_with_vim_keys_and_selected_in_visual_mode() {
    let fake = with_links();
    let mut g = opened_issue(&fake);
    screen(&mut g, 70, 24);
    goto_text(&mut g, "Opening the menu");
    // `the menu` out of `Opening the menu shifts the header.`
    for c in "^wvwe".chars() {
        ch(&mut g, c);
    }
    assert!(screen(&mut g, 70, 24).contains("a add to message"));
    ch(&mut g, 'a');
    assert_eq!(g.draft.text(), "> the menu\n\n");
    // Esc goes back to reading, the draft kept; another selection is added after it.
    press(&mut g, KeyCode::Esc);
    assert!(g.writing.is_none());
    goto_text(&mut g, "Second line.");
    for c in "^V".chars() {
        ch(&mut g, c);
    }
    ch(&mut g, 'a');
    assert_eq!(g.draft.text(), "> the menu\n\n> Second line.\n\n");
    type_str(&mut g, "both");
    ctrl(&mut g, 's');
    settle(&mut g);
    assert_eq!(
        fake.writes(),
        ["issue comment 12 -R acme/site --body-file - <<< > the menu\n\n> Second line.\n\nboth"]
    );
}

#[test]
fn esc_leaves_visual_mode_then_the_block_then_the_detail_and_ctrl_d_discards_a_draft() {
    let fake = with_links();
    let mut g = opened_issue(&fake);
    screen(&mut g, 70, 24);
    // Not in a block: Esc goes back to the list.
    press(&mut g, KeyCode::Esc);
    assert!(matches!(g.mode, Mode::List));
    let mut g = opened_issue(&fake);
    screen(&mut g, 70, 24);
    let inside = |g: &Github| matches!(&g.mode, Mode::Detail(v) if v.cur.is_some());
    keys(&mut g, "j\nv");
    press(&mut g, KeyCode::Esc);
    assert!(inside(&g), "only out of visual mode");
    press(&mut g, KeyCode::Esc);
    assert!(!inside(&g) && matches!(g.mode, Mode::Detail(_)), "out of the block");
    // A draft stays until it is posted or thrown away.
    ch(&mut g, 'c');
    type_str(&mut g, "half");
    press(&mut g, KeyCode::Esc);
    assert_eq!(g.draft.text(), "half");
    assert!(screen(&mut g, 120, 24).contains("tab write (draft)"));
    ch(&mut g, 'c');
    ctrl(&mut g, 'd');
    assert!(g.draft.is_empty() && g.writing.is_none());
}

#[test]
fn reactions_go_on_the_comment_in_focus_or_the_description() {
    let fake = with_links();
    let mut g = opened_issue(&fake);
    screen(&mut g, 70, 24);
    ch(&mut g, 'j');
    ch(&mut g, '+');
    ch(&mut g, '6');
    settle(&mut g);
    screen(&mut g, 70, 24);
    ch(&mut g, 'j');
    ch(&mut g, '+');
    ch(&mut g, '1');
    settle(&mut g);
    assert_eq!(
        fake.writes(),
        [
            "api -X POST repos/acme/site/issues/12/reactions -f content=heart",
            "api -X POST repos/acme/site/issues/comments/501/reactions -f content=+1",
        ]
    );
    // Not one of the eight: nothing happens, the choice stays.
    ch(&mut g, '+');
    ch(&mut g, '9');
    assert!(g.writing.is_some());
    press(&mut g, KeyCode::Esc);
    assert_eq!(fake.writes().len(), 2);
}

#[test]
fn an_issue_closes_as_completed_not_planned_or_duplicate() {
    for (keys, expected) in [
        ("x1", "issue close 12 -R acme/site --reason completed"),
        ("x2", "issue close 12 -R acme/site --reason not planned"),
        (
            "x37\n",
            "issue close 12 -R acme/site --reason duplicate --duplicate-of 7",
        ),
    ] {
        let fake = with_links();
        let mut g = opened_issue(&fake);
        for c in keys.chars() {
            match c {
                '\n' => press(&mut g, KeyCode::Enter),
                c => ch(&mut g, c),
            };
        }
        settle(&mut g);
        assert_eq!(fake.writes(), [expected], "{keys:?}");
    }
    // Esc gives up, a duplicate of itself is refused.
    let fake = with_links();
    let mut g = opened_issue(&fake);
    ch(&mut g, 'x');
    press(&mut g, KeyCode::Esc);
    ch(&mut g, 'x');
    ch(&mut g, '3');
    ch(&mut g, '1');
    ch(&mut g, '2');
    press(&mut g, KeyCode::Enter);
    assert!(fake.writes().is_empty());
    assert!(g.note.is_some());
}

#[test]
fn a_closed_issue_is_reopened_and_a_pull_request_closes_without_a_reason() {
    let closed = ISSUE_WITH_LINKS.replace(r#""state":"OPEN""#, r#""state":"CLOSED""#);
    let fake = Fake::new(&[
        ("repo view", Ok(REPO)),
        ("issue list", Ok(ISSUES)),
        ("issue view 12", Ok(&closed)),
        ("labels", Ok("[]")),
        ("assignees", Ok("[]")),
    ]);
    let mut g = opened_issue(&fake);
    ch(&mut g, 'x');
    ch(&mut g, '1');
    settle(&mut g);
    assert_eq!(fake.writes(), ["issue reopen 12 -R acme/site"]);

    let fake = Fake::acme();
    let mut g = opened_pr(&fake);
    ch(&mut g, 'x');
    ch(&mut g, '1');
    settle(&mut g);
    assert_eq!(fake.writes(), ["pr close 31 -R acme/site"]);
}

#[test]
fn the_same_reaction_again_takes_it_back_and_only_the_users_own_is_removed() {
    let fake = Fake::new(&[
        ("repo view", Ok(REPO)),
        ("issue list", Ok(ISSUES)),
        ("issue view 12", Ok(ISSUE_WITH_LINKS)),
        ("labels", Ok("[]")),
        ("assignees", Ok("[]")),
        ("api user", Ok(r#"{"login":"Me"}"#)),
        (
            "issues/12/reactions?per_page=100",
            Ok(r#"[{"id":5,"user":{"login":"bob"},"content":"heart"},
                   {"id":77,"user":{"login":"me"},"content":"heart"}]"#),
        ),
    ]);
    let mut g = opened_issue(&fake);
    screen(&mut g, 70, 24);
    ch(&mut g, 'j');
    ch(&mut g, '+');
    ch(&mut g, '6');
    settle(&mut g);
    assert_eq!(
        fake.writes(),
        ["api -X DELETE repos/acme/site/issues/12/reactions/77"]
    );
    assert!(screen(&mut g, 70, 24).contains("reaction taken back"));
    // Somebody else's heart is not the user's: a thumbs up is new.
    ch(&mut g, '+');
    ch(&mut g, '1');
    settle(&mut g);
    assert_eq!(fake.writes().len(), 2);
    assert!(fake.writes()[1].ends_with("content=+1"));
}

#[test]
fn the_users_own_reactions_are_found_and_marked() {
    let json = ISSUE_WITH_LINKS
        .replacen(
            r#""body":"Opening"#,
            r#""reactionGroups":[{"content":"HEART","users":{"totalCount":2}}],"body":"Opening"#,
            1,
        )
        .replace(
            r#""author":{"login":"bob"},"#,
            r#""reactionGroups":[{"content":"THUMBS_UP","users":{"totalCount":1}},{"content":"EYES","users":{"totalCount":1}}],"author":{"login":"bob"},"#,
        );
    let fake = Fake::new(&[
        ("repo view", Ok(REPO)),
        ("issue list", Ok(ISSUES)),
        ("issue view 12", Ok(&json)),
        ("labels", Ok("[]")),
        ("assignees", Ok("[]")),
        ("api user", Ok(r#"{"login":"me"}"#)),
        (
            "issues/12/reactions?per_page=100",
            Ok(r#"[{"id":1,"user":{"login":"bob"},"content":"heart"},{"id":2,"user":{"login":"me"},"content":"heart"}]"#),
        ),
        (
            "comments/501/reactions?per_page=100",
            Ok(r#"[{"id":3,"user":{"login":"me"},"content":"eyes"},{"id":4,"user":{"login":"bob"},"content":"+1"}]"#),
        ),
    ]);
    let g = opened_issue(&fake);
    let detail = g.details[&(Kind::Issue, 12)].detail.as_ref().unwrap();
    assert!(
        detail.reactions[0].mine,
        "the item's heart is the user's: {:?} {:?}",
        fake.calls(),
        detail.reactions
    );
    let comment = &detail.comments[0].reactions;
    let mine: Vec<(&str, bool)> = comment.iter().map(|r| (r.content.as_str(), r.mine)).collect();
    assert_eq!(mine, [("THUMBS_UP", false), ("EYES", true)]);
    // An item nobody reacted to costs no extra asks.
    let quiet = Fake::acme();
    opened_issue(&quiet);
    assert_eq!(quiet.count("api user"), 0);
}

// ---- editing: assignees, labels, milestone, type, fields, relations ----

const ISSUE_SETUP: &str = r#"{
 "number":12,"title":"Header jumps","state":"OPEN","author":{"login":"alice"},
 "labels":[{"name":"bug"}],"assignees":[{"login":"bob"}],
 "milestone":{"title":"v1"},"issueType":{"name":"Bug"},"projectItems":[{"project":{"title":"Roadmap"}}],
 "parent":{"number":3},"blockedBy":[{"number":4}],"blocking":[],
 "url":"https://github.com/acme/site/issues/12",
 "createdAt":"2026-10-10T09:00:00Z","updatedAt":"2026-10-16T12:00:00Z",
 "body":"The header jumps.","comments":[]}"#;

const ORG_FIELDS: &str = r#"[
 {"id":11,"name":"Priority","data_type":"single_select","options":[{"name":"High"},{"name":"Low"}]},
 {"id":12,"name":"Target date","data_type":"date","options":[]}]"#;

fn editable() -> Arc<Fake> {
    Fake::new(&[
        ("repo view", Ok(REPO)),
        ("issue list", Ok(ISSUES)),
        ("issue view 12", Ok(ISSUE_SETUP)),
        ("repos/acme/site/labels", Ok(r#"[{"name":"bug","color":"d73a4a"},{"name":"docs","color":"0075ca"}]"#)),
        ("assignees", Ok(r#"[{"login":"alice","avatar_url":""},{"login":"bob","avatar_url":""}]"#)),
        ("orgs/acme/issue-fields", Ok(ORG_FIELDS)),
        ("orgs/acme/issue-types", Ok(r#"[{"name":"Bug"},{"name":"Feature"}]"#)),
        ("repos/acme/site/milestones", Ok(r#"[{"title":"v1"},{"title":"v2"}]"#)),
        (
            "issue-field-values",
            Ok(r#"[{"issue_field_name":"Priority","single_select_option":{"name":"Low"},"value":2}]"#),
        ),
        ("project list", Ok(r#"{"projects":[{"title":"Roadmap"},{"title":"Ideas"}]}"#)),
    ])
}

fn keys(g: &mut Github, text: &str) {
    for c in text.chars() {
        match c {
            '\n' => press(g, KeyCode::Enter),
            c => ch(g, c),
        };
    }
}

#[test]
fn the_detail_says_what_is_set_on_the_issue() {
    let fake = editable();
    let mut g = opened_issue(&fake);
    let shown = screen(&mut g, 140, 24);
    for part in ["assigned to bob", "milestone v1", "type Bug", "Priority Low", "projects Roadmap", "parent #3", "blocked by #4"] {
        assert!(shown.contains(part), "{part}\n{shown}");
    }
}

#[test]
fn you_or_someone_is_assigned_from_a_list_that_toggles() {
    let fake = editable();
    let mut g = opened_issue(&fake);
    // Yourself is first in the list: Space toggles it, Enter applies.
    keys(&mut g, "A");
    assert!(screen(&mut g, 80, 24).contains("@me (yourself)"));
    keys(&mut g, " \n");
    settle(&mut g);
    assert_eq!(fake.writes(), ["issue edit 12 -R acme/site --add-assignee @me"]);
    // Filter by typing, toggle, and take bob (already on) off.
    let fake = editable();
    let mut g = opened_issue(&fake);
    keys(&mut g, "Aalice ");
    keys(&mut g, "\n");
    settle(&mut g);
    assert_eq!(fake.writes(), ["issue edit 12 -R acme/site --add-assignee alice"]);
    let fake = editable();
    let mut g = opened_issue(&fake);
    keys(&mut g, "Abob \n");
    settle(&mut g);
    assert_eq!(fake.writes(), ["issue edit 12 -R acme/site --remove-assignee bob"]);
    // Nothing toggled is no write.
    let fake = editable();
    let mut g = opened_issue(&fake);
    keys(&mut g, "A\n");
    assert!(fake.writes().is_empty());
}

#[test]
fn labels_toggle_in_their_own_colors() {
    let fake = editable();
    let mut g = opened_issue(&fake);
    settle(&mut g);
    keys(&mut g, "Ldocs ");
    keys(&mut g, "\n");
    settle(&mut g);
    assert_eq!(fake.writes(), ["issue edit 12 -R acme/site --add-label docs"]);
    let fake = editable();
    let mut g = opened_issue(&fake);
    settle(&mut g);
    keys(&mut g, "Lbug \n");
    settle(&mut g);
    assert_eq!(fake.writes(), ["issue edit 12 -R acme/site --remove-label bug"]);
}

#[test]
fn milestone_and_type_are_chosen_or_taken_away() {
    for (keys_to_press, expected) in [
        ("Mv2\n", "issue edit 12 -R acme/site --milestone v2"),
        ("M(none)\n", "issue edit 12 -R acme/site --remove-milestone"),
        ("TFeature\n", "issue edit 12 -R acme/site --type Feature"),
        ("T(none)\n", "issue edit 12 -R acme/site --remove-type"),
    ] {
        let fake = editable();
        let mut g = opened_issue(&fake);
        keys(&mut g, &keys_to_press[..1]);
        settle(&mut g);
        keys(&mut g, &keys_to_press[1..]);
        settle(&mut g);
        assert_eq!(fake.writes(), [expected], "{keys_to_press:?}");
    }
}

#[test]
fn an_issue_field_is_set_from_its_options_or_a_typed_value() {
    let fake = editable();
    let mut g = opened_issue(&fake);
    keys(&mut g, "F");
    settle(&mut g);
    let shown = screen(&mut g, 80, 24);
    assert!(shown.contains("Priority: Low"), "{shown}");
    keys(&mut g, "Prio\n");
    keys(&mut g, "High\n");
    settle(&mut g);
    assert_eq!(
        fake.writes(),
        ["api -X POST repos/acme/site/issues/12/issue-field-values --input - <<< {\"issue_field_values\":[{\"field_id\":11,\"value\":\"High\"}]}"]
    );
    // A field that is not a single select takes what is typed.
    let fake = editable();
    let mut g = opened_issue(&fake);
    keys(&mut g, "F");
    settle(&mut g);
    keys(&mut g, "Target\n");
    keys(&mut g, "2026-12-01\n");
    settle(&mut g);
    assert!(fake.writes()[0].ends_with("{\"issue_field_values\":[{\"field_id\":12,\"value\":\"2026-12-01\"}]}"));
}

#[test]
fn relations_take_the_number_of_another_issue() {
    let fake = editable();
    let mut g = opened_issue(&fake);
    keys(&mut g, "RSet the parent\n7\n");
    settle(&mut g);
    assert_eq!(fake.writes(), ["issue edit 12 -R acme/site --parent 7"]);
    let fake = editable();
    let mut g = opened_issue(&fake);
    keys(&mut g, "RAdd blocked by\n12\n");
    // Not itself: it asks again.
    assert!(g.writing.is_some() && fake.writes().is_empty());
    press(&mut g, KeyCode::Backspace);
    press(&mut g, KeyCode::Backspace);
    keys(&mut g, "9\n");
    settle(&mut g);
    assert_eq!(fake.writes(), ["issue edit 12 -R acme/site --add-blocked-by 9"]);
    let fake = editable();
    let mut g = opened_issue(&fake);
    keys(&mut g, "RRemove the parent\n");
    settle(&mut g);
    assert_eq!(fake.writes(), ["issue edit 12 -R acme/site --remove-parent"]);
}

#[test]
fn projects_come_from_gh_and_a_failure_says_how_to_fix_it() {
    let fake = editable();
    let mut g = opened_issue(&fake);
    keys(&mut g, "P");
    settle(&mut g);
    keys(&mut g, "Ideas \n");
    settle(&mut g);
    assert_eq!(fake.writes(), ["issue edit 12 -R acme/site --add-project Ideas"]);
    // Without the project scope.
    let fake = Fake::new(&[
        ("repo view", Ok(REPO)),
        ("issue list", Ok(ISSUES)),
        ("issue view 12", Ok(ISSUE_SETUP)),
        ("labels", Ok("[]")),
        ("assignees", Ok("[]")),
        ("project list", Err(GhError::Other("missing required scopes [read:project]".into()))),
    ]);
    let mut g = opened_issue(&fake);
    keys(&mut g, "P");
    settle(&mut g);
    assert!(g.note.as_deref().unwrap_or("").contains("gh auth refresh -s project"));
    assert!(g.writing.is_none());
}

#[test]
fn m_shows_the_keys_that_edit() {
    let fake = editable();
    let mut g = opened_issue(&fake);
    keys(&mut g, "m");
    let shown = screen(&mut g, 100, 24);
    assert!(shown.contains("assignees") && shown.contains("relations"), "{shown}");
    keys(&mut g, "A");
    assert!(matches!(g.writing, Some(write::Writing::Pick(_))));
}

#[test]
fn the_description_is_edited_in_a_box_that_starts_with_its_text() {
    let fake = with_links();
    let mut g = opened_issue(&fake);
    screen(&mut g, 70, 24);
    ch(&mut g, 'j');
    ch(&mut g, 'E');
    assert_eq!(g.body_edit.text(), "Opening the menu shifts the header.\nSecond line.");
    assert!(screen(&mut g, 70, 24).contains("Edit the description of #12"));
    type_str(&mut g, " More.");
    ctrl(&mut g, 's');
    settle(&mut g);
    assert_eq!(
        fake.writes(),
        ["issue edit 12 -R acme/site --body-file - <<< Opening the menu shifts the header.\nSecond line. More."]
    );
    // Esc drops the changes.
    let fake = with_links();
    let mut g = opened_issue(&fake);
    screen(&mut g, 70, 24);
    ch(&mut g, 'j');
    ch(&mut g, 'E');
    type_str(&mut g, "x");
    press(&mut g, KeyCode::Esc);
    assert!(g.writing.is_none() && fake.writes().is_empty());
}

#[test]
fn the_comment_in_focus_is_edited_by_its_number() {
    let fake = with_links();
    let mut g = opened_issue(&fake);
    screen(&mut g, 70, 24);
    keys(&mut g, "jjE");
    assert_eq!(g.body_edit.text(), "I can reproduce this.");
    type_str(&mut g, "!");
    ctrl(&mut g, 's');
    settle(&mut g);
    assert_eq!(
        fake.writes(),
        ["api -X PATCH repos/acme/site/issues/comments/501 --input - <<< {\"body\":\"I can reproduce this.!\"}"]
    );
    assert!(screen(&mut g, 70, 24).contains("comment saved"));
}

#[test]
fn the_title_is_a_line_that_starts_with_the_old_one() {
    let fake = with_links();
    let mut g = opened_issue(&fake);
    ch(&mut g, 'S');
    assert!(screen(&mut g, 70, 24).contains("Title: Header jumps"));
    for _ in 0.."jumps".len() {
        press(&mut g, KeyCode::Backspace);
    }
    type_str(&mut g, "flickers");
    press(&mut g, KeyCode::Enter);
    settle(&mut g);
    assert_eq!(fake.writes(), ["issue edit 12 -R acme/site --title Header flickers"]);
    // The same title, or none, is not a write.
    let fake = with_links();
    let mut g = opened_issue(&fake);
    ch(&mut g, 'S');
    press(&mut g, KeyCode::Enter);
    assert!(fake.writes().is_empty());
}
