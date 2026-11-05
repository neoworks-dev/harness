use super::*;
use neo_renderer_ratatui::Palette;

fn info(id: &str, title: &str, last: i64) -> SessionInfo {
    SessionInfo {
        harness: None,
        parent: None,
        id: id.into(),
        title: (!title.is_empty()).then(|| title.into()),
        cwd: "/w".into(),
        created_at: 0,
        last_activity: last,
        last_seq: 1,
        state: SessionState::Idle,
        queued: 0,
        waiting: None,
        asking: None,
        unseen: None,
        preview: None,
        added: None,
        removed: None,
    }
}

fn text(lines: &[Line<'_>]) -> Vec<String> {
    lines.iter().map(|l| l.to_string()).collect()
}

fn press(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn styles() -> Styles {
    Styles::new(&Palette::default(), false)
}

/// Activity times in the tests are a few ms, so they all read "now".
fn look(current: &str, focused: bool) -> Look<'_> {
    Look {
        current,
        focused,
        spinner: '◐',
        now_ms: 60_000,
        marks: &[],
    }
}

#[test]
fn a_marked_session_has_its_slot_before_its_title() {
    let mut s = Sessions::default();
    s.set_all(vec![info("a", "Alpha", 2), info("b", "Beta", 1)]);
    let marks = ["b".to_string()];
    let look = Look {
        marks: &marks,
        ..look("a", false)
    };
    let text = neo_renderer_ratatui::plain_text(&s.render(40, 20, &look, &styles())).join("\n");
    assert!(text.contains("1 Beta"), "{text}");
    assert!(!text.contains("1 Alpha"), "{text}");
}

#[test]
fn a_working_session_sits_on_a_blue_band() {
    let mut s = Sessions::default();
    let mut running = info("b", "Beta", 1);
    running.state = SessionState::Running;
    s.set_all(vec![info("a", "Alpha", 2), running]);
    // A theme that paints the pane.
    let styles = Styles {
        panel: ratatui::style::Color::Rgb(0x1c, 0x1d, 0x20),
        ..styles()
    };
    let lines = s.render(40, 20, &look("a", false), &styles);
    let bg = |title: &str| {
        let line = lines
            .iter()
            .find(|l| l.to_string().contains(title))
            .unwrap();
        line.spans.iter().map(|s| s.style.bg).collect::<Vec<_>>()
    };
    let band = crate::chat::mix(styles.panel, styles.info.fg.unwrap(), RUNNING_TINT);
    assert!(band.is_some());
    assert!(bg("Beta").iter().all(|c| *c == band), "{:?}", bg("Beta"));
    assert!(bg("Alpha").iter().all(|c| *c != band));
}

#[test]
fn newest_first_and_activity_does_not_move_them() {
    let mut s = Sessions::default();
    s.set_all(vec![info("a", "old", 10), info("b", "new", 30)]);
    let ids = |s: &Sessions| s.list.iter().map(|i| i.id.clone()).collect::<Vec<_>>();
    assert_eq!(ids(&s), ["b", "a"]);
    s.upsert(info("a", "old", 40));
    assert_eq!(ids(&s), ["b", "a"]);
    assert_eq!(s.get("a").unwrap().last_activity, 40);
    s.upsert(info("c", "fresh", 50));
    assert_eq!(ids(&s), ["c", "b", "a"]);
    assert_eq!(s.len(), 3);
    assert!(!s.any_running());
    let mut running = info("a", "old", 60);
    running.state = SessionState::Running;
    s.upsert(running);
    assert!(s.any_running());
    assert_eq!(ids(&s), ["c", "b", "a"]);
    // A fresh list keeps the places too.
    s.set_all(vec![
        info("a", "old", 70),
        info("b", "new", 30),
        info("c", "fresh", 50),
    ]);
    assert_eq!(ids(&s), ["c", "b", "a"]);
}

#[test]
fn the_shown_sessions_title_goes_on_under_itself() {
    let mut s = Sessions::default();
    let mut a = info("a", "Term TUI highlighting, icons and polish", 30);
    a.preview = Some("Both are committed".into());
    let b = info("b", "Cheap modern hotels in Hamburg", 20);
    s.set_all(vec![a, b]);
    let lines = s.render(29, 12, &look("a", false), &styles());
    let got: Vec<_> = text(&lines)
        .iter()
        .map(|l| l.trim_end().to_string())
        .collect();
    assert_eq!(
        got,
        [
            " Sessions · 2",
            "   + New chat",
            " w ──────────────────────────",
            "▎  Term TUI highlighting, now",
            "▎  icons and polish",
            "▎  Both are committed",
            "",
            "   Cheap modern hotels i… now",
            "",
        ]
    );
}

#[test]
fn rows_show_state_title_preview_and_changed_lines() {
    let mut s = Sessions::default();
    let mut a = info("a", "Fix the login form", 30);
    a.state = SessionState::Running;
    a.queued = 2;
    a.preview = Some("Looking at the validation".into());
    a.added = Some(12);
    a.removed = Some(3);
    let mut b = info("b", "", 20);
    b.preview = Some("Done. I changed the form and ran the tests twice".into());
    b.added = Some(7);
    let mut c = info("cafe0123456", "Docs", 10);
    c.removed = Some(40);
    s.set_all(vec![a, b, c]);
    let lines = s.render(29, 12, &look("b", false), &styles());
    let got: Vec<_> = text(&lines)
        .iter()
        .map(|l| l.trim_end().to_string())
        .collect();
    assert_eq!(
        got,
        [
            " Sessions · 3             ◐1",
            "   + New chat",
            " w ──────────────────────────",
            " ◐ Fix the login form     now",
            "   2 queued · Looking… +12 −3",
            "",
            "▎  New Session            now",
            "▎  Done. I changed the fo… +7",
            "",
            "   Docs                   now",
            "                          −40",
            "",
        ]
    );
    // Green and red for the changed lines, the spinner in blue.
    let st = styles();
    assert_eq!(lines[3].spans[1].style, st.info);
    let spans = &lines[4].spans;
    assert_eq!(
        spans.iter().find(|s| s.content == "+12").unwrap().style,
        st.success
    );
    assert_eq!(
        spans.iter().find(|s| s.content == "−3").unwrap().style,
        st.danger
    );
    assert_eq!(
        lines[6].spans[0].content, "▎",
        "the shown session has a bar"
    );
}

#[test]
fn sessions_idle_for_over_a_day_fade() {
    let mut s = Sessions::default();
    let day = 24 * 3600 * 1000;
    let mut old = info("old", "Stale", day);
    old.preview = Some("said long ago".into());
    s.set_all(vec![info("new", "Fresh", 10 * day), old]);
    let mut l = look("x", false);
    l.now_ms = 10 * day + 1000;
    s.render(29, 20, &l, &styles());
    s.opened.insert("/w".into());
    s.build();
    let lines = s.render(29, 20, &l, &styles());
    let title = |text: &str| {
        lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .find(|s| s.content.contains(text))
            .unwrap()
            .style
    };
    assert!(!title("Fresh").add_modifier.contains(Modifier::DIM));
    assert_eq!(title("Stale"), styles().muted);
    assert!(title("said long ago").add_modifier.contains(Modifier::DIM));
}

#[test]
fn sessions_idle_for_over_a_day_fold_into_an_older_row_that_enter_opens() {
    let mut s = Sessions::default();
    let day = 24 * 3600 * 1000;
    let mut running = info("run", "Busy", day);
    running.state = SessionState::Running;
    s.set_all(vec![
        info("new", "Fresh", 10 * day),
        info("old", "Stale", 2 * day),
        info("older", "Staler", day),
        info("shown", "Shown", day),
        running,
    ]);
    let mut l = look("shown", false);
    l.now_ms = 10 * day + 1000;
    let rows = |s: &mut Sessions, l: &Look| {
        text(&s.render(29, 40, l, &styles()))
            .iter()
            .map(|l| l.trim().trim_start_matches('▎').trim().to_string())
            .filter(|l| {
                !l.is_empty()
                    && !l.starts_with("Sessions")
                    && !l.starts_with('+')
                    && !l.ends_with('─')
            })
            // Without the age.
            .map(|l| {
                l.trim_end_matches(|c: char| c.is_ascii_digit() || c == 'd')
                    .trim_end_matches("now")
                    .trim_end()
                    .to_string()
            })
            .collect::<Vec<_>>()
    };
    // The shown and the running session stay; the two idle ones fold.
    assert_eq!(rows(&mut s, &l), ["Fresh", "◐ Busy", "Shown", "▸ 2 older"]);
    l.focused = true;
    s.focus("shown");
    s.key(press(KeyCode::Down));
    assert_eq!(s.selected, Some(Pick::Older("/w".into())));
    assert_eq!(s.key(press(KeyCode::Enter)), SessionsAction::Redraw);
    assert_eq!(
        rows(&mut s, &l),
        ["Fresh", "◐ Busy", "Shown", "▾ 2 older", "Stale", "Staler"]
    );
    s.key(press(KeyCode::Down));
    match s.key(press(KeyCode::Enter)) {
        SessionsAction::Open(i) => assert_eq!(i.id, "old"),
        other => panic!("{other:?}"),
    }
    s.key(press(KeyCode::Up));
    s.key(press(KeyCode::Enter));
    assert_eq!(rows(&mut s, &l), ["Fresh", "◐ Busy", "Shown", "▸ 2 older"]);
}

#[test]
fn ages_are_short() {
    assert_eq!(age(0, 59_000), "now");
    assert_eq!(age(0, 5 * 60_000), "5m");
    assert_eq!(age(0, 3 * 3_600_000), "3h");
    assert_eq!(age(0, 2 * 86_400_000), "2d");
}

#[test]
fn a_turn_nobody_looked_at_says_it_is_your_turn() {
    let mut s = Sessions::default();
    let mut a = info("a", "Background job", 30);
    a.unseen = Some(true);
    s.set_all(vec![a, info("b", "Here", 10)]);
    let lines = s.render(29, 9, &look("b", false), &styles());
    assert_eq!(lines[0].to_string(), " Sessions · 2             ●1");
    assert_eq!(lines[3].spans[1].content, "●");
    assert_eq!(lines[3].spans[1].style, styles().success);
    assert!(
        lines[3].spans[3]
            .style
            .add_modifier
            .contains(Modifier::BOLD)
    );
    assert!(
        lines[4].to_string().contains("your turn"),
        "{:?}",
        text(&lines)
    );
    // The shown session is being looked at, whatever the daemon said last.
    let lines = s.render(29, 9, &look("a", false), &styles());
    assert_eq!(lines[0].to_string(), " Sessions · 2");
}

#[test]
fn a_session_waiting_for_an_answer_says_so_over_its_other_states() {
    let mut s = Sessions::default();
    let mut a = info("a", "Deploy", 30);
    a.state = SessionState::Running;
    a.asking = Some(1);
    a.preview = Some("Pushing the tag".into());
    let mut b = info("b", "Build", 20);
    b.state = SessionState::Running;
    let mut c = info("c", "Tests", 10);
    c.unseen = Some(true);
    s.set_all(vec![a, b, c]);
    let lines = s.render(29, 20, &look("x", false), &styles());
    let got: Vec<_> = text(&lines)
        .iter()
        .map(|l| l.trim_end().to_string())
        .collect();
    assert_eq!(
        got[..7],
        [
            " Sessions · 3       !1 ◐1 ●1",
            "   + New chat",
            " w ──────────────────────────",
            " ! Deploy                 now",
            "   needs you · Pushing the t…",
            "",
            " ◐ Build                  now",
        ]
    );
    let st = styles();
    let warning = st.warning.add_modifier(Modifier::BOLD);
    assert_eq!(lines[3].spans[1].style, warning);
    assert_eq!(lines[3].spans[3].style, warning, "the title too");
    assert_eq!(lines[4].spans[3].style, st.warning);
    // Folded, the repository shows the most pressing one.
    s.folded.insert("/w".into());
    s.build();
    let lines = s.render(29, 20, &look("x", false), &styles());
    assert_eq!(
        lines[2].to_string().trim_end(),
        " ▸ w · 3 ! ──────────────────"
    );
}

#[test]
fn keys_move_open_and_leave() {
    let mut s = Sessions::default();
    s.set_all(vec![
        info("a", "one", 30),
        info("b", "two", 20),
        info("c", "three", 10),
    ]);
    s.focus("b");
    assert_eq!(s.key(press(KeyCode::Char('j'))), SessionsAction::Redraw);
    match s.key(press(KeyCode::Enter)) {
        SessionsAction::Open(i) => assert_eq!(i.id, "c"),
        other => panic!("{other:?}"),
    }
    s.key(press(KeyCode::Down));
    match s.key(press(KeyCode::Enter)) {
        SessionsAction::Open(i) => assert_eq!(i.id, "c", "stops at the end"),
        other => panic!("{other:?}"),
    }
    // `+ New chat` comes first.
    s.key(press(KeyCode::Char('g')));
    s.key(press(KeyCode::Char('k')));
    assert_eq!(
        s.key(press(KeyCode::Enter)),
        SessionsAction::New,
        "stops at the start"
    );
    // Then the directory's header, then its first session.
    s.key(press(KeyCode::Char('j')));
    assert_eq!(s.selected, Some(Pick::Cluster("/w".into())));
    s.key(press(KeyCode::Char('j')));
    match s.key(press(KeyCode::Enter)) {
        SessionsAction::Open(i) => assert_eq!(i.id, "a"),
        other => panic!("{other:?}"),
    }
    s.key(press(KeyCode::Char('G')));
    match s.key(press(KeyCode::Enter)) {
        SessionsAction::Open(i) => assert_eq!(i.id, "c"),
        other => panic!("{other:?}"),
    }
    assert_eq!(s.key(press(KeyCode::Char('n'))), SessionsAction::New);
    assert_eq!(s.key(press(KeyCode::Esc)), SessionsAction::Leave);
    assert_eq!(s.key(press(KeyCode::Char('x'))), SessionsAction::Redraw);
}

#[test]
fn the_list_scrolls_to_the_selected_or_shown_session() {
    let mut s = Sessions::default();
    s.set_all(
        (0..10)
            .map(|i| SessionInfo {
                preview: Some("said".into()),
                ..info(&format!("s{i}"), &format!("title {i}"), 100 - i)
            })
            .collect(),
    );
    // Room for 3 sessions: header, `+ New chat`, the group's name and 9 rows.
    let shown = |s: &mut Sessions, current: &str, focused: bool| {
        text(&s.render(29, 12, &look(current, focused), &styles()))
            .iter()
            .filter(|l| l.contains("title"))
            .map(|l| {
                let l = l.trim().trim_start_matches('▎').trim();
                l.trim_end_matches("now").trim().to_string()
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(
        shown(&mut s, "s0", false),
        ["title 0", "title 1", "title 2"]
    );
    assert_eq!(
        shown(&mut s, "s7", false),
        ["title 5", "title 6", "title 7"]
    );
    s.focus("s7");
    for _ in 0..2 {
        s.key(press(KeyCode::Down));
    }
    assert_eq!(shown(&mut s, "s7", true), ["title 7", "title 8", "title 9"]);
    for _ in 0..9 {
        s.key(press(KeyCode::Up));
    }
    assert_eq!(shown(&mut s, "s7", true), ["title 0", "title 1", "title 2"]);
}

#[test]
fn the_selected_row_is_reversed_only_while_focused() {
    let mut s = Sessions::default();
    let mut b = info("b", "two", 20);
    b.preview = Some("said".into());
    s.set_all(vec![info("a", "one", 30), b]);
    s.focus("b");
    let reversed = |lines: &[Line<'_>]| {
        lines
            .iter()
            .filter(|l| {
                l.spans
                    .iter()
                    .any(|sp| sp.style.add_modifier.contains(Modifier::REVERSED))
            })
            .count()
    };
    assert_eq!(reversed(&s.render(29, 9, &look("a", true), &styles())), 2);
    assert_eq!(reversed(&s.render(29, 9, &look("a", false), &styles())), 0);
}

#[test]
fn nothing_overflows_narrow_or_short_areas() {
    let mut s = Sessions::default();
    let mut a = info("a", "A very long title that cannot possibly fit", 30);
    a.preview = Some("日本語のプレビューもあります。とても長い".into());
    a.added = Some(123_456);
    a.removed = Some(98_765);
    a.queued = 12;
    s.set_all(vec![a]);
    for w in [8, 12, 20, 29] {
        for h in [1, 2, 3, 9] {
            for line in s.render(w, h, &look("a", true), &styles()) {
                assert!(line.width() <= w, "{w}x{h}: {line}");
            }
        }
    }
}

// ---- groups ----

fn at(id: &str, title: &str, cwd: &str, last: i64) -> SessionInfo {
    SessionInfo {
        cwd: cwd.into(),
        ..info(id, title, last)
    }
}

/// Two repositories; the first has a main checkout and two worktrees.
fn grouped() -> Sessions {
    let mut s = Sessions::default();
    let mut login = at("c", "Fix the login", "/w/site/.worktrees/fix-login", 50);
    login.state = SessionState::Running;
    login.preview = Some("Editing the form".into());
    let mut child = at("d", "Write tests", "/w/site/.worktrees/fix-login", 45);
    child.parent = Some("c".into());
    s.set_all(vec![
        at("a", "Plan the release", "/w/site", 10),
        at("b", "Docs", "/w/site", 20),
        login,
        child,
        at("e", "Dark mode", "/w/site/.worktrees/dark/src", 30),
        at("f", "Scratch", "/w/notes", 40),
    ]);
    s
}

#[test]
fn sessions_group_by_repository_and_worktree_newest_group_first() {
    let mut s = grouped();
    let lines = text(&s.render(29, 40, &look("a", false), &styles()));
    let got: Vec<_> = lines.iter().map(|l| l.trim_end().to_string()).collect();
    assert_eq!(
        got,
        [
            " Sessions · 6             ◐1",
            "   + New chat",
            " site ───────────────────────",
            "   Docs                   now",
            "",
            "▎  Plan the release       now",
            "",
            "   worktree fix-login",
            " ◐ Fix the login          now",
            "   Editing the form",
            "",
            "   ↳ Write tests          now",
            "",
            "   worktree dark",
            "   Dark mode              now",
            "",
            " notes ──────────────────────",
            "   Scratch                now",
            "",
        ]
    );
}

#[test]
fn the_cluster_with_the_newest_session_leads_and_worktrees_follow_their_repo() {
    let mut s = grouped();
    // The notes directory is quiet; the site repo has the newest session in a worktree.
    let mut f = at("f", "Scratch", "/w/notes", 5);
    f.preview = None;
    s.upsert(f);
    let order = |s: &Sessions| {
        s.entries
            .iter()
            .filter_map(|e| match e {
                Entry::Cluster { name, .. } => Some(name.clone()),
                Entry::Group(n) => Some(format!("  {n}")),
                Entry::Session { .. } | Entry::Older { .. } => None,
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(order(&s), ["site", "  fix-login", "  dark", "notes"]);
    // Activity in the notes directory doesn't move its cluster.
    s.upsert(at("f", "Scratch", "/w/notes", 99));
    assert_eq!(order(&s), ["site", "  fix-login", "  dark", "notes"]);
    // A new session there does.
    s.upsert(at("g", "Ideas", "/w/notes", 100));
    assert_eq!(order(&s), ["notes", "site", "  fix-login", "  dark"]);
    // Within a repo, the worktree with the newest session comes first.
    s.upsert(at("h", "Contrast", "/w/site/.worktrees/dark", 200));
    assert_eq!(order(&s), ["site", "  dark", "  fix-login", "notes"]);
}

#[test]
fn keys_skip_headers_and_jump_between_groups() {
    let mut s = grouped();
    s.focus("b");
    let selected = |s: &Sessions| match s.selected.clone().unwrap() {
        Pick::Session(id) => id,
        Pick::Older(group) | Pick::Cluster(group) => group,
        Pick::New => "new".into(),
    };
    let mut walk = vec![selected(&s)];
    for _ in 0..6 {
        s.key(press(KeyCode::Down));
        walk.push(selected(&s));
    }
    // Main checkout (b, a: newest first), the worktree's session and its child (c, d), then
    // dark (e), the notes' header and its session (f): the order on screen. Worktree
    // headers are skipped.
    assert_eq!(walk, ["b", "a", "c", "d", "e", "/w/notes", "f"]);
    s.key(press(KeyCode::Down));
    assert_eq!(selected(&s), "f", "stops at the end");

    // Group jumps: first session of the previous / next group.
    s.focus("a");
    s.key(press(KeyCode::Char(']')));
    assert_eq!(selected(&s), "c");
    s.key(press(KeyCode::Down));
    s.key(press(KeyCode::Char(']')));
    assert_eq!(selected(&s), "e");
    s.key(KeyEvent::new(KeyCode::Down, KeyModifiers::CONTROL));
    assert_eq!(selected(&s), "f");
    s.key(KeyEvent::new(KeyCode::Down, KeyModifiers::CONTROL));
    assert_eq!(selected(&s), "f", "no group after the last");
    s.key(press(KeyCode::Char('[')));
    assert_eq!(selected(&s), "e");
    s.key(KeyEvent::new(KeyCode::Up, KeyModifiers::CONTROL));
    assert_eq!(selected(&s), "c");
    s.key(press(KeyCode::Char('[')));
    assert_eq!(selected(&s), "b");
    // Enter opens what is selected.
    match s.key(press(KeyCode::Enter)) {
        SessionsAction::Open(i) => assert_eq!(i.id, "b"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn enter_on_a_repositorys_header_folds_it_and_its_worktrees() {
    let mut s = grouped();
    let lines = |s: &mut Sessions, focused: bool| {
        text(&s.render(29, 40, &look("a", focused), &styles()))
            .iter()
            .map(|l| l.trim_end().to_string())
            .collect::<Vec<_>>()
    };
    s.focus("b");
    s.key(press(KeyCode::Up));
    assert_eq!(s.selected, Some(Pick::Cluster("/w/site".into())));
    assert_eq!(s.key(press(KeyCode::Enter)), SessionsAction::Redraw);
    // Five sessions in it, one running.
    assert_eq!(
        lines(&mut s, false),
        [
            " Sessions · 6             ◐1",
            "   + New chat",
            " ▸ site · 5 ◐ ───────────────",
            " notes ──────────────────────",
            "   Scratch                now",
            "",
        ]
    );
    // The keys go from the folded header straight to the next repository.
    s.key(press(KeyCode::Down));
    assert_eq!(s.selected, Some(Pick::Cluster("/w/notes".into())));
    s.key(press(KeyCode::Char('[')));
    assert_eq!(s.selected, Some(Pick::Cluster("/w/site".into())));
    // Focused again, the keys start on the header standing for the shown session.
    s.focus("a");
    assert_eq!(s.selected, Some(Pick::Cluster("/w/site".into())));
    s.key(press(KeyCode::Enter));
    assert_eq!(lines(&mut s, false)[2], " site ───────────────────────");
    assert_eq!(lines(&mut s, false).len(), 19);
}

#[test]
fn a_child_of_a_session_elsewhere_is_marked_but_stays_in_its_own_group() {
    let mut s = Sessions::default();
    let mut child = at("k", "Run the build", "/w/site/.worktrees/ci", 20);
    child.parent = Some("p".into());
    s.set_all(vec![at("p", "Main chat", "/w/site", 30), child]);
    let lines = text(&s.render(29, 20, &look("p", false), &styles()));
    let got: Vec<_> = lines.iter().map(|l| l.trim_end().to_string()).collect();
    assert_eq!(
        got,
        [
            " Sessions · 2",
            "   + New chat",
            " site ───────────────────────",
            "▎  Main chat              now",
            "",
            "   worktree ci",
            "   ↳ Run the build        now",
            "",
        ]
    );
}

#[test]
fn the_list_scrolls_to_show_the_selected_sessions_group_header() {
    let mut s = grouped();
    s.focus("e");
    // 5 lines: a group header and one session at a time fit under `+ New chat`.
    let got = text(&s.render(29, 5, &look("a", true), &styles()));
    let got: Vec<_> = got.iter().map(|l| l.trim_end().to_string()).collect();
    assert_eq!(
        got,
        [
            " Sessions · 6             ◐1",
            "   + New chat",
            "   worktree dark",
            "   Dark mode              now",
            ""
        ]
    );
    s.focus("f");
    let got = text(&s.render(29, 5, &look("a", true), &styles()));
    assert_eq!(got[2].trim_end(), " notes ──────────────────────");
    assert_eq!(got[3].trim_end(), "   Scratch                now");
}

#[test]
fn places_come_from_the_worktrees_convention_and_from_git_files() {
    let root = std::env::temp_dir().join(format!("neo-term-places-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let repo = root.join("app");
    let linked = root.join("app-hotfix");
    let admin = repo.join(".git/worktrees/app-hotfix");
    std::fs::create_dir_all(repo.join("src")).unwrap();
    std::fs::create_dir_all(&admin).unwrap();
    std::fs::create_dir_all(&linked).unwrap();
    std::fs::write(admin.join("HEAD"), "ref: refs/heads/hotfix-1\n").unwrap();
    std::fs::write(
        linked.join(".git"),
        format!("gitdir: {}\n", admin.display()),
    )
    .unwrap();
    let (repo_s, linked_s) = (repo.to_string_lossy(), linked.to_string_lossy());

    // A subdirectory belongs to the checkout around it.
    let main = place_of(&format!("{repo_s}/src"));
    assert_eq!((main.repo.as_str(), main.worktree), (&*repo_s, None));
    // A linked worktree elsewhere groups under its repository, named by its branch.
    let other = place_of(&linked_s);
    assert_eq!(other.repo, repo_s);
    assert_eq!(other.worktree.as_deref(), Some("hotfix-1"));
    // The convention needs no disk access.
    let named = place_of("/nowhere/proj/.worktrees/fix-login/crates/a");
    assert_eq!(named.repo, "/nowhere/proj");
    assert_eq!(named.worktree.as_deref(), Some("fix-login"));
    // Outside any repository: the directory itself.
    let plain = place_of("/nowhere/notes/");
    assert_eq!(plain.repo, "/nowhere/notes");
    assert_eq!(plain.worktree, None);
    let _ = std::fs::remove_dir_all(&root);
}
