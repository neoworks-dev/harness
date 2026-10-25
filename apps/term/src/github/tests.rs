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
        })
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
        || matches!(&g.mode, Mode::Detail(DetailView { diff: Some(d), .. }) if !d.is_loaded())
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
    terminal.draw(|f| g.draw(f, f.area(), NOW, &st)).unwrap();
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
    assert!(calls[1].starts_with("issue list -R acme/site"));
    assert_eq!(calls.len(), 2);
    let list = g.lists.get(&g.current()).unwrap();
    assert_eq!(list.rows.len(), 3);
}

#[test]
fn list_snapshot_issues() {
    let mut g = opened(&Fake::acme());
    snapshot("github-list-issues-100x30", &screen(&mut g, 100, 30));
}

#[test]
fn list_snapshot_pull_requests() {
    let mut g = opened(&Fake::acme());
    ch(&mut g, '2');
    settle(&mut g);
    down(&mut g, 1);
    snapshot("github-list-prs-100x30", &screen(&mut g, 100, 30));
}

fn down(g: &mut Github, n: usize) {
    for _ in 0..n {
        press(g, KeyCode::Down);
    }
}

#[test]
fn list_snapshot_narrow() {
    let mut g = opened(&Fake::acme());
    snapshot("github-list-issues-44x12", &screen(&mut g, 44, 12));
}

#[test]
fn tabs_switch_with_tab_and_digits() {
    let fake = Fake::acme();
    let mut g = opened(&fake);
    assert_eq!(g.tab, Kind::Issue);
    press(&mut g, KeyCode::Tab);
    settle(&mut g);
    assert_eq!(g.tab, Kind::Pr);
    assert!(screen(&mut g, 100, 10).contains("[2 Pull requests]"));
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
    assert!(text.contains("› "));
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

#[test]
fn detail_snapshot_issue() {
    let mut g = opened_issue(&Fake::acme());
    snapshot("github-detail-issue-100x30", &screen(&mut g, 100, 30));
}

#[test]
fn detail_snapshot_pull_request() {
    let mut g = opened_pr(&Fake::acme());
    snapshot("github-detail-pr-100x40", &screen(&mut g, 100, 40));
}

#[test]
fn detail_snapshot_narrow() {
    let mut g = opened_pr(&Fake::acme());
    snapshot("github-detail-pr-40x24", &screen(&mut g, 40, 24));
}

#[test]
fn the_detail_scrolls_by_keys_and_wheel() {
    let mut g = opened_pr(&Fake::acme());
    let top = |g: &Github| match &g.mode {
        Mode::Detail(v) => v.scroll,
        Mode::List => unreachable!(),
    };
    let first = screen(&mut g, 60, 12);
    assert!(first.contains("lines 1–"), "{first}");
    ch(&mut g, 'j');
    assert_eq!(top(&g), 1);
    press(&mut g, KeyCode::PageDown);
    assert_eq!(top(&g), 1 + 9);
    ch(&mut g, 'k');
    g.wheel(true);
    assert_eq!(top(&g), 9 + 3);
    ch(&mut g, 'G');
    let end = screen(&mut g, 60, 12);
    assert!(end.contains("src/layout.css"), "{end}");
    let last = top(&g);
    ch(&mut g, 'j');
    assert_eq!(top(&g), last);
    ch(&mut g, 'g');
    assert_eq!(top(&g), 0);
    g.wheel(false);
    assert_eq!(top(&g), 0);
}

#[test]
fn the_pull_request_diff_opens_in_the_diff_view() {
    let fake = Fake::acme();
    let mut g = opened_pr(&fake);
    ch(&mut g, 'd');
    assert!(screen(&mut g, 80, 12).contains("…"));
    settle(&mut g);
    let s = screen(&mut g, 100, 14);
    assert!(s.contains("scrollbar-gutter: stable"), "{s}");
    assert!(s.contains("+2"), "{s}");
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
fn enter_in_a_detail_attaches_the_issue() {
    let mut g = opened_issue(&Fake::acme());
    let text = attach_text(press(&mut g, KeyCode::Enter));
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
    assert!(text.contains("alice (2026-10-16, on src/header.css:12): Use a variable here."));
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

// ---- rendering ----

#[test]
fn markdown_reads_well() {
    let st = styles();
    let src = "<!-- hidden -->\n# Title\n\n**bold** and *it* and `code` and ~~gone~~\n\n- one\n  - nested item that is long enough to wrap around\n1. first\n- [x] done\n> quoted\n\n---\n![shot](https://x/y.png) <br> after\n\n```\nlet x = 1;\n```";
    let lines = md::render(src, 30, 0, &st);
    let text = neo_renderer_ratatui::plain_text(&lines);
    assert_eq!(
        text,
        [
            "Title",
            "",
            "bold and it and code and gone",
            "",
            "• one",
            "  • nested item that is long",
            "    enough to wrap around",
            "1. first",
            "☑ done",
            "│ quoted",
            "",
            "────────────────────────────────────────",
            "[image: shot]",
            "after",
            "",
            "  let x = 1;",
        ]
        .map(|l| l.chars().take(30).collect::<String>())
        .to_vec()
    );
    // Styles survive.
    let bold = &lines[2].spans[1];
    assert!(
        bold.style
            .add_modifier
            .contains(ratatui::style::Modifier::BOLD)
    );
}

#[test]
fn nothing_is_wider_than_the_view() {
    let fake = Fake::acme();
    let st = styles();
    let mut g = opened_pr(&fake);
    let Mode::Detail(v) = &g.mode else { panic!() };
    let d = g
        .details
        .get(&(Kind::Pr, 31))
        .unwrap()
        .detail
        .clone()
        .unwrap();
    let _ = v;
    let mut issue = parse_detail(Kind::Issue, ISSUE_12).unwrap();
    issue.body.push_str("\nhttps://example.com/a/very/long/url/that/cannot/break/anywhere/at/all/ok 日本語日本語日本語日本語");
    for w in [1, 2, 5, 12, 20, 33, 40, 80] {
        for detail in [&d, &issue] {
            for line in doc::build(detail, w, NOW, &st) {
                assert!(line.width() <= w, "width {w}: {:?}", line.to_string());
            }
        }
        let rows = parse_list(Kind::Issue, ISSUES).unwrap();
        let cols = view::Columns::new(&rows, w);
        for r in &rows {
            let line = cols.row(r, true, NOW, &st);
            assert!(
                line.width() <= w.max(2 + 5 + 1 + 6 + 1 + 1),
                "width {w}: {:?}",
                line.to_string()
            );
        }
    }
    // And on screen at silly sizes: no panic, nothing past the edge.
    for (w, h) in [(1, 1), (3, 2), (10, 3), (20, 5), (30, 30), (45, 4)] {
        screen(&mut g, w, h);
        let mut l = opened(&fake);
        screen(&mut l, w, h);
        ch(&mut l, '/');
        screen(&mut l, w, h);
    }
}

#[test]
fn the_footer_keeps_the_most_useful_hints_that_fit() {
    let mut g = opened(&Fake::acme());
    let wide = screen(&mut g, 120, 6);
    assert!(wide.lines().last().unwrap().contains("esc close"));
    let narrow = screen(&mut g, 30, 6);
    let last = narrow.lines().last().unwrap();
    assert!(last.starts_with("enter open"), "{last}");
    assert!(!last.contains("esc close"));
}

#[test]
fn the_search_cursor_sits_in_the_filter_line() {
    let mut g = opened(&Fake::acme());
    ch(&mut g, '/');
    type_str(&mut g, "ab");
    let mut terminal = Terminal::new(TestBackend::new(60, 8)).unwrap();
    let st = styles();
    terminal.draw(|f| g.draw(f, f.area(), NOW, &st)).unwrap();
    terminal.backend_mut().assert_cursor_position((4, 1));
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
    assert!(
        s.contains("[1 Issues]") && s.contains("Header jumps"),
        "{s}"
    );
    // The editor and the status lines stay where they are.
    assert!(s.contains("> "));
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
    assert!(app_key(&mut app, KeyCode::Enter).is_empty());
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
        id: "s2".into(),
        title: None,
        cwd: "/work/other".into(),
        created_at: 0,
        last_activity: 0,
        last_seq: 0,
        state: neo_protocol::SessionState::Idle,
        queued: 0,
        preview: None,
        added: None,
        removed: None,
        parent: None,
    };
    app.handle_msg(crate::app::Msg::Created(Ok(info)));
    assert!(!app.github_mut().is_open());
    ctrl_r(&mut app);
    app_settle(&mut app);
    assert_eq!(fake.count("repo view"), 2);
}

#[test]
fn the_view_leaves_the_terminal_cursor_alone_unless_searching() {
    let fake = Fake::acme();
    let mut app = app(&fake);
    ctrl_r(&mut app);
    app_settle(&mut app);
    app_key(&mut app, KeyCode::Char('/'));
    type_app(&mut app, "ab");
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    terminal.draw(|f| app.draw(f)).unwrap();
    terminal.backend_mut().assert_cursor_position((4, 1));
}
