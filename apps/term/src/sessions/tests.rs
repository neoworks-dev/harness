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
        waiting_on: None,
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
    // The shown and the running session stay, the running one first; the two idle ones fold.
    assert_eq!(rows(&mut s, &l), ["◐ Busy", "Fresh", "Shown", "▸ 2 older"]);
    l.focused = true;
    s.focus("shown");
    s.key(press(KeyCode::Down));
    assert_eq!(s.selected, Some(Pick::Older("/w".into())));
    assert_eq!(s.key(press(KeyCode::Enter)), SessionsAction::Redraw);
    assert_eq!(
        rows(&mut s, &l),
        ["◐ Busy", "Fresh", "Shown", "▾ 2 older", "Stale", "Staler"]
    );
    s.key(press(KeyCode::Down));
    match s.key(press(KeyCode::Enter)) {
        SessionsAction::Open(i) => assert_eq!(i.id, "old"),
        other => panic!("{other:?}"),
    }
    s.key(press(KeyCode::Up));
    s.key(press(KeyCode::Enter));
    assert_eq!(rows(&mut s, &l), ["◐ Busy", "Fresh", "Shown", "▸ 2 older"]);
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
    assert!(lines[0].to_string().contains("●1"), "{:?}", text(&lines));
    assert!(
        lines[4].to_string().contains("your turn"),
        "{:?}",
        text(&lines)
    );
    // The shown session is being looked at, whatever the daemon said last.
    let lines = s.render(29, 9, &look("a", false), &styles());
    assert!(!lines[0].to_string().contains('●'), "{:?}", text(&lines));
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
    assert!(
        lines[3].to_string().contains("! Deploy"),
        "{:?}",
        text(&lines)
    );
    assert!(
        lines[4].to_string().contains("needs you"),
        "{:?}",
        text(&lines)
    );
    // Folded, the repository shows the most pressing one.
    s.folded.insert("/w".into());
    s.build();
    let lines = s.render(29, 20, &look("x", false), &styles());
    assert!(lines[2].to_string().contains('!'), "{:?}", text(&lines));
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
    child.state = SessionState::Running;
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
    // A new session there doesn't pass a repository where one works.
    s.upsert(at("g", "Ideas", "/w/notes", 100));
    assert_eq!(order(&s), ["site", "  fix-login", "  dark", "notes"]);
    // Once nothing works there, the newer one leads.
    s.upsert(at("c", "Fix the login", "/w/site/.worktrees/fix-login", 50));
    s.upsert(at("d", "Write tests", "/w/site/.worktrees/fix-login", 45));
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
    // Its five sessions are out of the list, the other repository stays.
    let folded = lines(&mut s, false).join("\n");
    assert!(
        !folded.contains("Docs") && !folded.contains("Dark mode"),
        "{folded}"
    );
    assert!(folded.contains("Scratch"), "{folded}");
    // The keys go from the folded header straight to the next repository.
    s.key(press(KeyCode::Down));
    assert_eq!(s.selected, Some(Pick::Cluster("/w/notes".into())));
    s.key(press(KeyCode::Char('[')));
    assert_eq!(s.selected, Some(Pick::Cluster("/w/site".into())));
    // Focused again, the keys start on the header standing for the shown session.
    s.focus("a");
    assert_eq!(s.selected, Some(Pick::Cluster("/w/site".into())));
    s.key(press(KeyCode::Enter));
    let unfolded = lines(&mut s, false).join("\n");
    assert!(
        unfolded.contains("Docs") && unfolded.contains("Dark mode"),
        "{unfolded}"
    );
}

#[test]
fn a_delegated_session_leaves_once_its_work_is_done_unless_it_is_shown() {
    let mut s = Sessions::default();
    let mut child = at("k", "Run the build", "/w/site", 20);
    child.parent = Some("p".into());
    child.state = SessionState::Running;
    s.set_all(vec![at("p", "Main chat", "/w/site", 30), child.clone()]);
    let titles = |s: &mut Sessions, current: &str| {
        text(&s.render(29, 20, &look(current, false), &styles()))
            .join("\n")
            .contains("Run the build")
    };
    assert!(titles(&mut s, "p"));
    child.state = SessionState::Idle;
    child.unseen = Some(true);
    s.upsert(child);
    assert!(!titles(&mut s, "p"));
    assert_eq!(s.shown_count(), 1);
    assert!(titles(&mut s, "k"), "the shown one stays");
}

#[test]
fn the_list_scrolls_to_show_the_selected_sessions_group_header() {
    let mut s = grouped();
    s.focus("e");
    // 5 lines: a group header and one session at a time fit under `+ New chat`.
    let got = text(&s.render(29, 5, &look("a", true), &styles()));
    assert!(got[2].contains("worktree dark"), "{got:?}");
    assert!(got[3].contains("Dark mode"), "{got:?}");
    s.focus("f");
    let got = text(&s.render(29, 5, &look("a", true), &styles()));
    assert!(got[2].contains("notes"), "{got:?}");
    assert!(got[3].contains("Scratch"), "{got:?}");
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
