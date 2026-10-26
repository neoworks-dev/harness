//! Widgets as a whole: events in, buffers and lines out. Everything is headless.

use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

use neo_protocol::SessionEventRecord;
use neo_renderer_ratatui::Palette;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use serde_json::json;

use super::diff::DiffView;
use super::git::{Change, GitEntry, GitStatus};
use super::testutil::dump;
use super::*;

fn record(event: serde_json::Value) -> SessionEventRecord {
    serde_json::from_value(json!({"session": "s1", "seq": 1, "ts": 0, "event": event})).unwrap()
}

fn widgets() -> Widgets {
    Widgets::build(PathBuf::from("/work/proj"), Palette::default(), false)
}

fn plan_event(agent: Option<&str>, entries: &[(&str, &str)]) -> SessionEventRecord {
    let entries: Vec<_> = entries
        .iter()
        .map(|(c, s)| json!({"content": c, "status": s}))
        .collect();
    let mut e = json!({"type": "plan", "entries": entries});
    if let Some(a) = agent {
        e["agent"] = json!(a);
    }
    record(e)
}

fn diff_update(id: &str, path: &str, old: Option<&str>) -> SessionEventRecord {
    let mut d = json!({"type": "diff", "path": path, "newText": "x"});
    if let Some(o) = old {
        d["oldText"] = json!(o);
    }
    record(json!({"type": "tool_call_update", "toolCallId": id, "content": [d]}))
}

fn tool_call(id: &str, agent: Option<&str>) -> SessionEventRecord {
    let mut e = json!({"type": "tool_call", "toolCallId": id, "title": "Edit", "kind": "edit", "status": "pending"});
    if let Some(a) = agent {
        e["agent"] = json!(a);
    }
    record(e)
}

fn status(entries: &[(&str, Change)]) -> GitStatus {
    GitStatus {
        head: Some("main".into()),
        ahead: 1,
        entries: entries
            .iter()
            .map(|(p, c)| GitEntry {
                path: p.to_string(),
                orig: None,
                change: *c,
            })
            .collect(),
        ..GitStatus::default()
    }
}

fn text(line: &Line) -> String {
    line.spans.iter().map(|s| s.content.as_ref()).collect()
}

fn sidebar(w: &Widgets, cols: u16, rows: u16, focus: Option<Panel>) -> String {
    let mut t = Terminal::new(TestBackend::new(cols, rows)).unwrap();
    t.draw(|f| w.draw_sidebar(f, f.area(), focus)).unwrap();
    dump(t.backend().buffer())
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::from(code)
}

#[test]
fn plan_events_replace_and_clear_per_agent() {
    let mut w = widgets();
    assert!(!w.wants_sidebar(120));
    w.on_session_event(&plan_event(None, &[("one", "pending")]));
    w.on_session_event(&plan_event(Some("claude"), &[("c1", "in_progress")]));
    w.on_session_event(&plan_event(
        None,
        &[("one", "completed"), ("two", "pending")],
    ));
    let out = sidebar(&w, 30, 10, None);
    assert!(
        out.contains("✓ one") && out.contains("○ two") && out.contains("◐ c1"),
        "{out}"
    );
    w.on_session_event(&plan_event(Some("claude"), &[]));
    let out = sidebar(&w, 30, 10, None);
    assert!(!out.contains("c1") && !out.contains("claude"), "{out}");
    w.on_session_event(&plan_event(None, &[]));
    assert!(!w.wants_sidebar(120));
}

#[test]
fn usage_events_keep_the_last_of_the_main_agent() {
    let mut w = widgets();
    assert!(w.usage_line().is_none());
    let u = |agent: Option<&str>, used: i64, cost: f64| {
        let mut e =
            json!({"type": "usage", "contextUsed": used, "contextSize": 1000, "cost": cost});
        if let Some(a) = agent {
            e["agent"] = json!(a);
        }
        record(e)
    };
    w.on_session_event(&u(Some("claude"), 120, 0.42));
    w.on_session_event(&u(None, 100, 0.05));
    w.on_session_event(&u(None, 350, 0.18));
    assert_eq!(
        text(&w.usage_line().unwrap()),
        "ctx 35% · $0.18",
        "guests' usage is left out"
    );
}

#[test]
fn tool_diffs_become_tagged_files_and_sidebar_wants_width() {
    let mut w = widgets();
    w.on_session_event(&tool_call("t1", None));
    w.on_session_event(&tool_call("t2", Some("claude")));
    w.on_session_event(&diff_update("t1", "/work/proj/src/a.ts", Some("old")));
    w.on_session_event(&diff_update("t2", "docs/new.md", None));
    w.on_session_event(&diff_update("t2", "/elsewhere/x.txt", Some("o")));
    // An update for a call we never saw is the session's own agent.
    w.on_session_event(&diff_update("t9", "./b.rs", Some("o")));
    assert!(w.branch().is_none());
    assert!(w.wants_sidebar(100));
    assert!(!w.wants_sidebar(99));
    let out = sidebar(&w, 40, 9, Some(Panel::Files));
    let expected = "\
│ Files
│ ▾ /elsewhere/
│   M x.txt (claude)
│ ▾ docs/
│   A new.md (claude)
│ ▾ src/
│   M a.ts (agent)
│ M b.rs (agent)";
    assert_eq!(
        out.lines().take(8).collect::<Vec<_>>().join("\n"),
        expected,
        "{out}"
    );
}

#[test]
fn sidebar_snapshot_plan_above_files() {
    let mut w = widgets();
    w.on_session_event(&plan_event(
        None,
        &[
            ("read the parser", "completed"),
            ("write the porcelain v2 parser with tests", "in_progress"),
            ("wire it up", "pending"),
        ],
    ));
    w.on_session_event(&plan_event(Some("claude"), &[("review", "pending")]));
    w.on_session_event(&diff_update("t1", "src/a.ts", Some("o")));
    w.files.set_status(Some(status(&[
        ("src/a.ts", Change::Modified),
        ("src/b.ts", Change::Added),
        ("old.txt", Change::Deleted),
        ("notes.md", Change::Untracked),
        ("lock.json", Change::Conflicted),
    ])));
    let expected = "\
│ Plan
│ agent
│ ✓ read the parser
│ ◐ write the porcelain
│   v2 parser with tests
│ ○ wire it up
│ claude
│ ○ review
│
│ Files
│ ▾ src/
│   M a.ts (agent)
│   A b.ts
│ U lock.json
│ ? notes.md
│ D old.txt
│";
    assert_eq!(sidebar(&w, 26, 17, None), expected);
    // Squeezed: the plan keeps its in-progress entry, the files are capped.
    let squeezed = "\
│ Plan
│ … 1 earlier
│ ◐ write the porcelain
│   v2 parser with tests
│ … 2 more
│
│ Files
│ ▾ src/
│   M a.ts (agent)
│   A b.ts
│ U lock.json
│ … 2 more";
    assert_eq!(sidebar(&w, 26, 12, None), squeezed);
}

#[test]
fn sidebar_titles_and_selection_follow_focus() {
    let mut w = widgets();
    w.on_session_event(&plan_event(None, &[("a", "pending")]));
    w.files
        .set_status(Some(status(&[("f.rs", Change::Modified)])));
    let p = Palette::default();
    let render = |focus| {
        let mut t = Terminal::new(TestBackend::new(20, 8)).unwrap();
        t.draw(|f| w.draw_sidebar(f, f.area(), focus)).unwrap();
        t.backend().buffer().clone()
    };
    let none = render(None);
    assert_eq!(none[(2, 0)].fg, p.muted);
    assert!(!none[(2, 0)].modifier.contains(Modifier::BOLD));
    assert_eq!(none[(0, 0)].symbol(), "│");
    assert_eq!(none[(0, 0)].fg, p.border);
    let plan = render(Some(Panel::Plan));
    assert!(plan[(2, 0)].modifier.contains(Modifier::BOLD));
    assert!(!plan[(2, 3)].modifier.contains(Modifier::BOLD)); // the Files title
    let files = render(Some(Panel::Files));
    assert!(files[(2, 3)].modifier.contains(Modifier::BOLD));
    assert_eq!(files[(2, 4)].bg, p.surface); // the selected row
    assert_ne!(none[(2, 4)].bg, p.surface);
}

#[test]
fn too_small_sidebars_draw_nothing_or_clip() {
    let mut w = widgets();
    w.on_session_event(&plan_event(None, &[("a task", "pending")]));
    assert_eq!(sidebar(&w, 5, 4, None), "\n\n\n");
    assert_eq!(sidebar(&w, 12, 1, None), "│ Plan");
    assert_eq!(sidebar(&w, 12, 2, None), "│ Plan\n│ ○ a task");
}

#[test]
fn keys_in_panels() {
    let mut w = widgets();
    w.on_session_event(&plan_event(None, &[("a", "pending")]));
    w.files.set_status(Some(status(&[
        ("a.rs", Change::Modified),
        ("b.rs", Change::Modified),
    ])));
    assert_eq!(
        w.handle_key(Panel::Plan, key(KeyCode::Down)),
        WidgetAction::Redraw
    );
    assert_eq!(
        w.handle_key(Panel::Plan, key(KeyCode::Esc)),
        WidgetAction::Leave
    );
    assert_eq!(
        w.handle_key(Panel::Files, key(KeyCode::Char('j'))),
        WidgetAction::Redraw
    );
    assert_eq!(w.files.selected().unwrap().rel, "b.rs");
    assert_eq!(
        w.handle_key(Panel::Files, key(KeyCode::Char('x'))),
        WidgetAction::Redraw
    );
    assert_eq!(
        w.handle_key(Panel::Files, key(KeyCode::Esc)),
        WidgetAction::Leave
    );
    let mut release = key(KeyCode::Esc);
    release.kind = KeyEventKind::Release;
    assert_eq!(w.handle_key(Panel::Files, release), WidgetAction::Redraw);
    assert!(!w.overlay_open());
}

#[test]
fn overlay_takes_keys_until_closed() {
    let mut w = widgets();
    w.files
        .set_status(Some(status(&[("a.rs", Change::Modified)])));
    let mut t = Terminal::new(TestBackend::new(30, 6)).unwrap();
    t.draw(|f| assert!(!w.draw_overlay(f, f.area()))).unwrap();
    let text: String = std::iter::once("@@ -1 +1 @@\n".to_string())
        .chain((0..20).map(|i| format!("+l{i}\n")))
        .collect();
    w.diff = Some(DiffView::from_text("a.rs", &text));
    assert!(w.overlay_open());
    let mut draw = |w: &Widgets| {
        t.draw(|f| assert!(w.draw_overlay(f, f.area()))).unwrap();
        dump(t.backend().buffer())
    };
    assert_eq!(
        draw(&w),
        "a.rs  +20 −0   lines 1–5 of 21\n    @@ -1 +1 @@\n  1 +l0\n  2 +l1\n  3 +l2\n  4 +l3"
    );
    // Whatever panel is focused, the overlay gets the keys.
    assert_eq!(
        w.handle_key(Panel::Plan, key(KeyCode::Char('j'))),
        WidgetAction::Redraw
    );
    assert!(draw(&w).contains("  1 +l0\n  2 +l1\n  3 +l2\n  4 +l3\n  5 +l4"));
    w.handle_key(Panel::Files, key(KeyCode::Char('G')));
    assert!(draw(&w).ends_with(" 20 +l19"));
    assert_eq!(
        w.handle_key(Panel::Files, key(KeyCode::Esc)),
        WidgetAction::Redraw
    );
    assert!(!w.overlay_open());
    assert_eq!(
        w.handle_key(Panel::Files, key(KeyCode::Esc)),
        WidgetAction::Leave
    );
    let _ = KeyModifiers::NONE;
}

#[test]
fn reset_forgets_everything() {
    let mut w = widgets();
    w.on_session_event(&plan_event(None, &[("a", "pending")]));
    w.on_session_event(&record(json!({"type": "usage", "cost": 1.0})));
    w.on_session_event(&diff_update("t", "a.rs", None));
    w.files
        .set_status(Some(status(&[("a.rs", Change::Modified)])));
    w.diff = Some(DiffView::from_text("a.rs", ""));
    w.reset(PathBuf::from("/other"));
    assert!(w.usage_line().is_none());
    assert!(!w.wants_sidebar(200));
    assert!(w.branch().is_none());
    assert!(!w.overlay_open());
    // The new directory is used to relativize paths.
    w.on_session_event(&diff_update("t", "/other/x.rs", None));
    assert_eq!(w.files.entries()[0].rel, "x.rs");
}

#[test]
fn palette_change_restyles() {
    let mut w = widgets();
    w.on_session_event(&record(
        json!({"type": "usage", "contextUsed": 90, "contextSize": 100}),
    ));
    let p = Palette {
        danger: ratatui::style::Color::Rgb(1, 2, 3),
        ..Palette::default()
    };
    w.set_palette(p);
    assert_eq!(w.usage_line().unwrap().spans[0].style.fg, Some(p.danger));
}

fn git_available() -> bool {
    Command::new("git")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

fn run(dir: &std::path::Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn wait_until(w: &mut Widgets, what: &str, mut done: impl FnMut(&Widgets) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        w.tick(0);
        if done(w) {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("timed out waiting for {what}");
}

#[test]
fn real_git_end_to_end() {
    if !git_available() {
        eprintln!("git not found, skipping");
        return;
    }
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("neo-term-git-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    run(&dir, &["init", "-q"]);
    run(&dir, &["symbolic-ref", "HEAD", "refs/heads/main"]);
    std::fs::write(dir.join("src/a.txt"), "one\ntwo\n").unwrap();
    std::fs::write(dir.join("gone.txt"), "bye\n").unwrap();
    run(&dir, &["add", "."]);
    run(&dir, &["commit", "-q", "-m", "init"]);

    let mut w = Widgets::new(dir.clone(), Palette::default());
    wait_until(&mut w, "the clean repo", |w| w.branch().is_some());
    assert_eq!(w.branch().as_deref(), Some("main"));
    assert!(!w.wants_sidebar(120), "a clean repo has nothing to show");

    std::fs::write(dir.join("src/a.txt"), "one\nchanged\n").unwrap();
    std::fs::write(dir.join("new.txt"), "fresh\n").unwrap();
    std::fs::remove_file(dir.join("gone.txt")).unwrap();
    w.on_file_changed("src/a.txt");
    wait_until(&mut w, "the changes", |w| w.files.entries().len() == 3);
    let got: Vec<_> = w
        .files
        .entries()
        .iter()
        .map(|e| (e.rel.clone(), e.mark))
        .collect();
    use files::Mark;
    assert_eq!(
        got,
        [
            ("gone.txt".to_string(), Mark::Deleted),
            ("src/a.txt".to_string(), Mark::Modified),
            ("new.txt".to_string(), Mark::Untracked),
        ]
    );
    assert!(w.wants_sidebar(120));

    // The tree: src/ and its file, then the files at the top. The modified file, whole with
    // its change marked.
    w.handle_key(Panel::Files, key(KeyCode::Char('j')));
    assert_eq!(w.files.selected().unwrap().rel, "src/a.txt");
    assert_eq!(
        w.handle_key(Panel::Files, key(KeyCode::Enter)),
        WidgetAction::Redraw
    );
    assert!(w.overlay_open());
    wait_until(&mut w, "the diff", |w| {
        w.diff.as_ref().is_some_and(DiffView::is_loaded)
    });
    let d = w.diff.as_ref().unwrap();
    assert_eq!(d.counts(), (1, 1));
    assert!(d.lines().iter().any(|l| l.text == "+changed"));
    assert!(
        d.lines().iter().any(|l| l.text == " one"),
        "the rest of the file"
    );
    w.handle_key(Panel::Files, key(KeyCode::Esc));
    assert!(!w.overlay_open());

    // The untracked file shows whole.
    w.handle_key(Panel::Files, key(KeyCode::Char('G')));
    assert_eq!(w.files.selected().unwrap().rel, "new.txt");
    w.handle_key(Panel::Files, key(KeyCode::Enter));
    wait_until(&mut w, "the untracked diff", |w| {
        w.diff.as_ref().is_some_and(DiffView::is_loaded)
    });
    let d = w.diff.as_ref().unwrap();
    assert_eq!(d.counts(), (1, 0));
    assert!(d.lines().iter().any(|l| l.text == "+fresh"));
    w.handle_key(Panel::Files, key(KeyCode::Char('q')));

    // A session in a subdirectory sees paths relative to it, from the same repository.
    let mut sub = Widgets::new(dir.join("src"), Palette::default());
    wait_until(&mut sub, "the subdirectory", |w| {
        w.files.entries().len() == 3
    });
    let rels: Vec<_> = sub.files.entries().iter().map(|e| e.rel.as_str()).collect();
    assert_eq!(rels, ["../gone.txt", "a.txt", "../new.txt"]);
    // ../ with the two files outside, then a.txt.
    sub.handle_key(Panel::Files, key(KeyCode::Char('G')));
    assert_eq!(sub.files.selected().unwrap().rel, "a.txt");
    sub.handle_key(Panel::Files, key(KeyCode::Enter));
    wait_until(&mut sub, "the sub diff", |w| {
        w.diff.as_ref().is_some_and(DiffView::is_loaded)
    });
    assert_eq!(sub.diff.as_ref().unwrap().counts(), (1, 1));

    // Not a repository: git fails, only the agent's files show.
    let plain = std::env::temp_dir().join(format!("neo-term-plain-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&plain).unwrap();
    let mut p = Widgets::new(plain.clone(), Palette::default());
    p.on_session_event(&diff_update(
        "t",
        &format!("{}/f.txt", plain.display()),
        None,
    ));
    std::fs::write(plain.join("f.txt"), "hello\n").unwrap();
    std::thread::sleep(Duration::from_millis(300));
    for _ in 0..20 {
        p.tick(0);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(p.branch().is_none());
    assert_eq!(p.files.entries().len(), 1);
    assert_eq!(p.files.entries()[0].rel, "f.txt");
    // A created file still shows up as a whole-file diff without a repository.
    p.handle_key(Panel::Files, key(KeyCode::Enter));
    wait_until(&mut p, "the plain diff", |w| {
        w.diff.as_ref().is_some_and(DiffView::is_loaded)
    });
    assert!(
        p.diff
            .as_ref()
            .unwrap()
            .lines()
            .iter()
            .any(|l| l.text == "+hello")
    );

    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&plain);
}

#[test]
fn area_offsets_are_respected() {
    // The sidebar may sit anywhere in the frame.
    let mut w = widgets();
    w.on_session_event(&plan_event(None, &[("a", "pending")]));
    let mut t = Terminal::new(TestBackend::new(20, 4)).unwrap();
    t.draw(|f| w.draw_sidebar(f, Rect::new(10, 1, 10, 3), None))
        .unwrap();
    assert_eq!(
        dump(t.backend().buffer()),
        "\n          │ Plan\n          │ ○ a\n          │"
    );
}
