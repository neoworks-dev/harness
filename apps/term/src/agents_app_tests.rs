//! The subagents list and transcript inside the app, drawn into a test backend.

use neo_protocol::{SessionState, ViewItem, ViewSnapshot};
use neo_renderer_ratatui::Palette as RenderPalette;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyEventState, KeyModifiers};
use serde_json::json;

use super::*;

const NOW: i64 = 1_000_000;

fn app() -> App {
    let mut app = App::new(Config {
        app_id: "terminal".into(),
        session: "s1".into(),
        cwd: "/home/me/site".into(),
        home: Some("/home/me".into()),
        no_color: false,
        history: HistoryStore::default(),
        prefs: crate::theme::Prefs::default(),
    });
    app.tick(NOW);
    app.awaiting = None;
    app.last_poll_ms = NOW;
    app.turn_started_ms = NOW;
    app
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent {
        code,
        modifiers: KeyModifiers::NONE,
        kind: KeyEventKind::Press,
        state: KeyEventState::NONE,
    }
}

fn ctrl(c: char) -> KeyEvent {
    KeyEvent {
        modifiers: KeyModifiers::CONTROL,
        ..key(KeyCode::Char(c))
    }
}

fn screen_text(app: &mut App, w: u16, h: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
    terminal.draw(|f| app.draw(f)).unwrap();
    let buffer = terminal.backend().buffer();
    (0..h)
        .map(|y| {
            (0..w)
                .map(|x| buffer[(x, y)].symbol().to_string())
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn snapshot(name: &str, actual: &str) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
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

fn event(session: &str, seq: i64, ts: i64, event: serde_json::Value) -> Event {
    Event::Session(
        serde_json::from_value(json!({"session": session, "seq": seq, "ts": ts, "event": event}))
            .unwrap(),
    )
}

fn started(id: &str, agent: &str, ago_ms: i64, extra: serde_json::Value) -> Event {
    let mut e = json!({"type": "subagent_started", "id": id, "agent": agent, "task": "Find the weather\nand report it"});
    e.as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    event("s1", 1, NOW - ago_ms, e)
}

/// neo has been searching tools for 72 s; claude has just started.
fn busy() -> App {
    let mut app = app();
    let fx = app.handle_event(started(
        "d1",
        "neo",
        72_000,
        json!({"model": "deepseek/deepseek-v4-flash", "effort": "medium"}),
    ));
    assert!(fx.is_empty());
    app.handle_event(event(
        "s1",
        2,
        NOW - 60_000,
        json!({"type": "tool_call", "toolCallId": "neo:1", "title": "search tools: weather",
               "kind": "search", "status": "in_progress", "agent": "neo"}),
    ));
    app.handle_event(event(
        "s1",
        3,
        NOW - 50_000,
        json!({"type": "usage", "agent": "neo", "inputTokens": 3100, "outputTokens": 300}),
    ));
    app.handle_event(started("d2", "claude", 5_000, json!({"model": "sonnet"})));
    app.tick(NOW);
    app
}

fn session_info(id: &str, cwd: &str, parent: Option<&str>) -> SessionInfo {
    SessionInfo {
        harness: None,
        id: id.into(),
        title: Some("Fix the login".into()),
        cwd: cwd.into(),
        created_at: 0,
        last_activity: NOW,
        last_seq: 1,
        state: SessionState::Running,
        queued: 0,
        waiting: None,
        preview: None,
        added: None,
        removed: None,
        parent: parent.map(Into::into),
    }
}

#[test]
fn the_list_sits_under_the_status_lines_while_subagents_run() {
    let mut app = busy();
    let text = screen_text(&mut app, 100, 24);
    snapshot("agents-100x24", &text);
    let lines: Vec<&str> = text.lines().collect();
    let at = lines.iter().position(|l| l.starts_with("● main")).unwrap();
    // The two status lines come right before it, the first in the input box.
    assert!(lines[at - 2].starts_with("┃ site ·"), "{text}");
    assert!(lines[at + 1].starts_with("○ neo · deepseek-v4-flash  search tools: weather"));
    assert!(lines[at + 1].ends_with("1m 12s · 3.4k tokens"));
    assert!(lines[at + 2].starts_with("○ claude · sonnet  starting…"));
}

#[test]
fn a_subagent_with_a_session_of_its_own_is_only_in_the_sidebar() {
    let mut app = app();
    app.set_sessions(vec![
        session_info("s1", "/home/me/site", None),
        session_info("s_child", "/home/me/site/.worktrees/fix-login", Some("s1")),
    ]);
    let fx = app.handle_event(started(
        "d2",
        "claude",
        1_000,
        json!({"child": "s_child", "cwd": "/home/me/site/.worktrees/fix-login"}),
    ));
    assert!(fx.is_empty(), "its session is not followed: {fx:?}");
    let text = screen_text(&mut app, 100, 24);
    assert!(!text.contains("● main"), "no list under the input: {text}");
    app.handle_key(ctrl('x'));
    assert_eq!(app.focus, None, "nothing to focus");
}

#[test]
fn the_list_goes_five_seconds_after_the_last_subagent_ends() {
    let mut app = busy();
    app.handle_event(event(
        "s1",
        8,
        NOW,
        json!({"type": "subagent_ended", "id": "d1", "stopReason": "end_turn"}),
    ));
    app.handle_event(event(
        "s1",
        9,
        NOW,
        json!({"type": "subagent_ended", "id": "d2", "stopReason": "error", "error": "boom"}),
    ));
    let text = screen_text(&mut app, 100, 24);
    assert!(text.contains("✓ neo · deepseek-v4-flash  done"), "{text}");
    assert!(text.contains("✗ claude · sonnet  boom"), "{text}");
    app.tick(NOW + 4_000);
    assert!(screen_text(&mut app, 100, 24).contains("● main"));
    app.tick(NOW + 6_000);
    assert!(!screen_text(&mut app, 100, 24).contains("● main"));
}

#[test]
fn down_from_an_empty_editor_or_ctrl_x_focuses_the_list() {
    let mut app = busy();
    // Text in the editor keeps Down for itself.
    app.handle_key(key(KeyCode::Char('h')));
    app.handle_key(key(KeyCode::Down));
    assert_eq!(app.focus, None);
    app.handle_key(key(KeyCode::Backspace));
    app.handle_key(key(KeyCode::Down));
    assert_eq!(app.focus, Some(Focus::Agents));
    let text = screen_text(&mut app, 100, 24);
    assert!(
        text.contains("↑/↓ select · enter open · x stop · esc back"),
        "{text}"
    );
    app.handle_key(key(KeyCode::Esc));
    assert_eq!(app.focus, None);
    app.handle_key(ctrl('x'));
    assert_eq!(app.focus, Some(Focus::Agents));
    // The terminal's cursor is not in the editor while the list has the keys.
    assert!(!app.editor_has_cursor());
    app.handle_key(key(KeyCode::Esc));

    // Nothing to focus without subagents.
    let mut quiet = app_without_agents();
    quiet.handle_key(ctrl('x'));
    assert_eq!(quiet.focus, None);
    assert!(
        quiet
            .notice
            .as_ref()
            .is_some_and(|n| n.text.contains("no subagents"))
    );
}

#[test]
fn up_from_the_top_row_goes_back_to_the_editor() {
    let mut app = busy();
    app.handle_key(key(KeyCode::Down));
    assert_eq!(app.focus, Some(Focus::Agents));
    // On claude, the last row; up to neo, then out.
    app.handle_key(key(KeyCode::Up));
    assert_eq!(app.focus, Some(Focus::Agents));
    app.handle_key(key(KeyCode::Up));
    assert_eq!(app.focus, None);
    assert!(app.editor_has_cursor());
}

fn app_without_agents() -> App {
    app()
}

#[test]
fn enter_opens_the_transcript_of_the_selected_subagent_and_esc_closes_it() {
    let mut app = busy();
    app.handle_key(ctrl('x'));
    // The latest running one is claude; up to neo.
    app.handle_key(key(KeyCode::Up));
    let fx = app.handle_key(key(KeyCode::Enter));
    assert_eq!(
        fx,
        [Effect::AgentView {
            session: "s1".into(),
            agent: "neo".into(),
            on: true
        }]
    );
    assert!(app.transcript.is_some());
    let loading = screen_text(&mut app, 100, 24);
    assert!(loading.contains("loading…"), "{loading}");

    // The daemon's view of neo's work arrives; the main chat is untouched.
    let snapshot_items: Vec<ViewItem> = serde_json::from_value(json!([
        {"ref": "s-d1", "kind": "user", "label": "task", "component": {"type": "text", "blocks": [
            {"type": "paragraph", "spans": [{"text": "Find the weather"}]}]}},
        {"ref": "t-neo-1", "kind": "thought", "component": {"type": "text", "tone": "muted", "blocks": [
            {"type": "paragraph", "spans": [{"text": "I should search for a weather tool"}]}]}},
        {"ref": "c-neo:1", "kind": "tool", "label": "search tools: weather", "state": "done",
         "component": {"type": "code", "code": "weather.forecast\nweather.alerts"}},
        {"ref": "m-neo-2", "kind": "agent", "component": {"type": "text", "blocks": [
            {"type": "paragraph", "spans": [{"text": "It is "}, {"text": "sunny", "bold": true}, {"text": " in Berlin."}]}]}}
    ]))
    .unwrap();
    app.handle_event(Event::ViewSnapshot(ViewSnapshot {
        agent: Some("neo".into()),
        session: "s1".into(),
        seq: 4,
        items: snapshot_items,
    }));
    assert!(
        app.chat.items().is_empty(),
        "the chat is not the transcript"
    );
    let text = screen_text(&mut app, 100, 24);
    snapshot("agents-transcript-100x24", &text);
    assert!(text.contains("◂ neo · deepseek-v4-flash  Find the weather"));
    assert!(text.contains("It is sunny in Berlin."));
    assert!(text.contains("esc back"));

    // Keys scroll and expand the transcript, not the editor; Esc goes back.
    app.handle_key(key(KeyCode::Char('x')));
    assert_eq!(app.editor_text(), "");
    app.handle_key(ctrl('o'));
    assert!(app.expanded);
    let fx = app.handle_key(key(KeyCode::Esc));
    assert_eq!(
        fx,
        [Effect::AgentView {
            session: "s1".into(),
            agent: "neo".into(),
            on: false
        }]
    );
    assert!(app.transcript.is_none());
    assert!(screen_text(&mut app, 100, 24).contains("┃ message the agent"));
}

#[test]
fn a_transcript_scrolls_through_its_lines() {
    let mut app = busy();
    app.handle_key(ctrl('x'));
    app.handle_key(key(KeyCode::Up));
    app.handle_key(key(KeyCode::Enter));
    let items: Vec<ViewItem> = (0..60)
        .map(|i| {
            serde_json::from_value(json!({"ref": format!("m{i}"), "kind": "agent", "component": {
                "type": "text", "blocks": [{"type": "paragraph", "spans": [{"text": format!("line number {i}")}]}]}}))
            .unwrap()
        })
        .collect();
    app.handle_event(Event::ViewSnapshot(ViewSnapshot {
        agent: Some("neo".into()),
        session: "s1".into(),
        seq: 4,
        items,
    }));
    let end = screen_text(&mut app, 100, 24);
    assert!(
        end.contains("line number 59") && !end.contains("line number 0\n"),
        "{end}"
    );
    app.handle_key(key(KeyCode::Home));
    let top = screen_text(&mut app, 100, 24);
    assert!(
        top.contains("line number 0") && !top.contains("line number 59"),
        "{top}"
    );
    app.handle_key(key(KeyCode::PageDown));
    assert!(!screen_text(&mut app, 100, 24).contains("line number 0\n"));
    app.handle_key(key(KeyCode::End));
    assert!(screen_text(&mut app, 100, 24).contains("line number 59"));
    // Views of other agents or sessions don't land here.
    let before = screen_text(&mut app, 100, 24);
    app.handle_event(Event::ViewSnapshot(ViewSnapshot {
        agent: Some("claude".into()),
        session: "s1".into(),
        seq: 5,
        items: vec![],
    }));
    assert_eq!(screen_text(&mut app, 100, 24), before);
}

#[test]
fn x_stops_the_selected_subagent() {
    let mut app = busy();
    app.handle_key(ctrl('x'));
    let fx = app.handle_key(key(KeyCode::Char('x')));
    assert_eq!(
        fx,
        [Effect::Action {
            session: "s1".into(),
            item: "agents".into(),
            action: "neo.cancel_agent".into(),
            args: Some(json!({"id": "d2"})),
        }]
    );
}

#[test]
fn the_delegation_gate_is_a_compact_card_with_highlighted_values() {
    let mut app = app();
    let task: Vec<String> = (1..=30)
        .map(|i| format!("step {i}: do the thing"))
        .collect();
    let buttons: Vec<_> = ["Start neo", "Allow neo for this session", "Reject"]
        .iter()
        .enumerate()
        .map(|(i, l)| {
            json!({"type": "button", "label": l, "action": "neo.permit",
                   "args": {"id": "p1", "option": l}, "key": (i + 1).to_string(), "note": true})
        })
        .collect();
    let gate: ViewItem = serde_json::from_value(json!({
        "ref": "p-p1", "kind": "tool", "label": "claude: Start a subagent?", "state": "pending",
        "component": {"type": "stack", "gap": "s", "children": [
            {"type": "kv", "items": [
                {"key": "Harness", "value": "neo", "tone": "accent"},
                {"key": "Model", "value": "deepseek/deepseek-v4-flash", "tone": "accent"},
                {"key": "Effort", "value": "medium", "tone": "accent"},
                {"key": "Directory", "value": "~/site/.worktrees/fix-login (new worktree, branch fix-login)", "tone": "accent"}]},
            {"type": "code", "code": task.join("\n")},
            {"type": "row", "gap": "s", "children": buttons}]}
    }))
    .unwrap();
    app.handle_event(Event::ViewSnapshot(ViewSnapshot {
        agent: None,
        session: "s1".into(),
        seq: 1,
        items: vec![gate],
    }));
    let text = screen_text(&mut app, 100, 30);
    snapshot("delegate-gate-100x30", &text);
    for expected in [
        "claude: Start a subagent?",
        "Harness",
        "Model",
        "Effort",
        "Directory",
    ] {
        assert!(text.contains(expected), "{expected}\n{text}");
    }
    assert!(
        text.contains("step 1: do the thing") && !text.contains("step 30"),
        "{text}"
    );
    assert!(text.contains("more lines"));

    // The highlight is bold in the accent color.
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    terminal.draw(|f| app.draw(f)).unwrap();
    let buffer = terminal.backend().buffer();
    let accent = crate::theme::Theme::default()
        .palette(&RenderPalette::default())
        .accent;
    let styled = (0..30)
        .flat_map(|y| (0..100).map(move |x| (x, y)))
        .any(|(x, y)| {
            let cell = &buffer[(x, y)];
            cell.symbol() == "p" && cell.fg == accent && cell.modifier.contains(Modifier::BOLD)
        });
    assert!(styled, "a highlighted value");
}

#[test]
fn another_session_has_no_agents_of_the_old_one() {
    let mut app = busy();
    app.begin_session("s2", "/w");
    assert!(!app.agents.visible(NOW));
    app.handle_key(ctrl('x'));
    assert_eq!(app.focus, None);
}

#[test]
fn down_lists_a_background_command_and_the_terminal_panel_shows_it() {
    let dir = std::env::temp_dir().join(format!("neo-term-app-jobs-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("b1.output");
    std::fs::write(&path, "tick 1\ntick 2\n").unwrap();
    let mut app = app_without_agents();
    app.handle_event(event(
        "s1",
        1,
        NOW,
        json!({"type": "tool_call", "toolCallId": "c1", "title": "sleep 120", "kind": "execute",
               "status": "in_progress", "rawInput": {"command": "sleep 120"}}),
    ));
    app.handle_event(event(
        "s1",
        2,
        NOW,
        json!({"type": "tool_call_update", "toolCallId": "c1", "status": "completed",
               "content": [{"type": "text", "text": format!(
                   "Command running in background with ID: b1. Output is being written to: {}.",
                   path.display())}]}),
    ));
    app.tick(NOW + 1_000);
    let text = screen_text(&mut app, 140, 30);
    assert!(text.contains("⏵ background · 1 running"), "{text}");
    assert!(text.contains("○ sleep 120  tick 2"), "{text}");

    app.handle_key(key(KeyCode::Down));
    assert_eq!(app.focus, Some(Focus::Jobs));
    let text = screen_text(&mut app, 140, 30);
    assert!(
        text.contains("background command · esc all commands"),
        "{text}"
    );
    assert!(text.contains("enter scroll its output"), "{text}");
    // Enter gives the panel the keys; Esc there shows all commands again.
    app.handle_key(key(KeyCode::Enter));
    assert_eq!(app.focus, Some(Focus::Panel(Panel::Terminal)));
    app.handle_key(key(KeyCode::Esc));
    assert_eq!(app.focus, None);
    let text = screen_text(&mut app, 140, 30);
    assert!(!text.contains("esc all commands"), "{text}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn an_at_lists_files_to_mention_and_tab_puts_one_in() {
    let root = std::env::temp_dir().join(format!("neo-term-app-mention-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("site/src")).unwrap();
    std::fs::write(root.join("site/src/app.rs"), "").unwrap();
    std::fs::write(root.join("notes.txt"), "").unwrap();
    let mut app = app();
    app.cwd = root.join("site").display().to_string();
    for c in "see @".chars() {
        app.handle_key(key(KeyCode::Char(c)));
    }
    let text = screen_text(&mut app, 100, 24);
    assert!(text.contains("@../") && text.contains("@src/"), "{text}");
    // Down to src/, Tab opens it, Tab puts the file in and ends the word.
    app.handle_key(key(KeyCode::Down));
    app.handle_key(key(KeyCode::Tab));
    assert_eq!(app.editor.text(), "see @src/");
    app.handle_key(key(KeyCode::Enter));
    assert_eq!(app.editor.text(), "see @src/app.rs ");
    assert!(!app.mention_visible());
    // One folder up, by name.
    app.editor.set_text("@../no");
    assert_eq!(app.mention_matches()[0].path, "../notes.txt");
    // Esc closes it for this text.
    app.handle_key(key(KeyCode::Esc));
    assert!(!app.mention_visible());
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn cd_completes_folders_and_enter_runs_it_once_one_is_named() {
    let root = std::env::temp_dir().join(format!("neo-term-app-cd-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("site/src")).unwrap();
    std::fs::write(root.join("site/sketch.txt"), "").unwrap();
    let mut app = app();
    app.cwd = root.join("site").display().to_string();
    for c in "/cd s".chars() {
        app.handle_key(key(KeyCode::Char(c)));
    }
    // Folders only, no `@`.
    let text = screen_text(&mut app, 100, 24);
    assert!(
        text.contains("› src/") && !text.contains("sketch"),
        "{text}"
    );
    // Enter picks while a name is being typed…
    assert!(app.handle_key(key(KeyCode::Enter)).is_empty());
    assert_eq!(app.editor.text(), "/cd src/");
    // …and runs the command once the path names a folder.
    let fx = app.handle_key(key(KeyCode::Enter));
    assert!(
        matches!(&fx[..], [Effect::Input { text, .. }] if text == "/cd src/"),
        "{fx:?}"
    );
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn alt_period_and_comma_widen_and_narrow_the_terminal_panel() {
    let alt = |c: char| KeyEvent {
        modifiers: KeyModifiers::ALT,
        ..key(KeyCode::Char(c))
    };
    let mut app = app_without_agents();
    app.handle_event(event(
        "s1",
        1,
        NOW,
        json!({"type": "tool_call", "toolCallId": "c1", "title": "ls", "kind": "execute",
               "status": "completed", "rawInput": {"command": "ls"}}),
    ));
    let column = |app: &mut App| {
        let text = screen_text(app, 160, 30);
        let line = text
            .lines()
            .find(|l| l.contains("Terminal"))
            .unwrap()
            .to_string();
        line[..line.find("Terminal").unwrap()].chars().count()
    };
    let at = column(&mut app);
    app.handle_key(alt('.'));
    assert_eq!(column(&mut app), at - 4, "wider by four");
    app.handle_key(alt(','));
    app.handle_key(alt(','));
    assert_eq!(column(&mut app), at + 4);
    // With the shift: > and <.
    app.handle_key(alt('>'));
    app.handle_key(alt('<'));
    assert_eq!(column(&mut app), at + 4);
    // Not narrower than its least.
    for _ in 0..30 {
        app.handle_key(alt(','));
    }
    let least = column(&mut app);
    // 40 columns: the border and a space, then the title.
    assert_eq!(160 - least, 40 - 2, "{least}");
    // Presses past the edge don't pile up: one press back moves it.
    app.handle_key(alt('.'));
    assert_eq!(column(&mut app), least - 4);
    assert!(app.editor.text().is_empty(), "nothing typed");
}

#[test]
fn the_files_sit_above_the_terminal_in_one_column() {
    let mut app = app_without_agents();
    app.handle_event(event(
        "s1",
        1,
        NOW,
        json!({"type": "tool_call", "toolCallId": "c1", "title": "Edit a.rs", "kind": "edit",
               "status": "in_progress"}),
    ));
    app.handle_event(event(
        "s1",
        2,
        NOW,
        json!({"type": "tool_call_update", "toolCallId": "c1", "status": "completed",
               "content": [{"type": "diff", "path": "/w/a.rs", "oldText": "a", "newText": "b"}]}),
    ));
    app.handle_event(event(
        "s1",
        3,
        NOW,
        json!({"type": "tool_call", "toolCallId": "c2", "title": "ls", "kind": "execute",
               "status": "completed", "rawInput": {"command": "ls"}}),
    ));
    let text = screen_text(&mut app, 160, 30);
    let lines: Vec<&str> = text.lines().collect();
    let find = |title: &str| {
        lines
            .iter()
            .enumerate()
            .find_map(|(y, l)| l.find(title).map(|x| (y, l[..x].chars().count())))
            .unwrap_or_else(|| panic!("no {title}\n{text}"))
    };
    let (files_y, files_x) = find("Files");
    let (terminal_y, terminal_x) = find("Terminal");
    assert_eq!(files_x, terminal_x, "one column\n{text}");
    assert!(files_y < terminal_y, "files on top\n{text}");
    assert!(app.sidebar_shown && app.terminal_area.is_some());
    // Tab walks the column top to bottom: the files, then the terminal.
    let mut seen = Vec::new();
    for _ in 0..3 {
        app.handle_key(key(KeyCode::Tab));
        seen.push(app.focus);
    }
    assert_eq!(
        seen,
        [
            Some(Focus::Panel(Panel::Plan)),
            Some(Focus::Panel(Panel::Files)),
            Some(Focus::Panel(Panel::Terminal)),
        ]
    );
}

#[test]
fn the_shared_todos_show_in_the_side_column_and_the_keys_change_them() {
    let mut app = app_without_agents();
    app.handle_event(event(
        "s1",
        1,
        NOW,
        json!({"type": "todos", "by": "claude", "items": [
            {"id": "t1", "text": "Make the header sticky", "done": false, "by": "claude"}
        ]}),
    ));
    let text = screen_text(&mut app, 140, 30);
    assert!(
        text.contains("Todos") && text.contains("[ ] Make the header sticky"),
        "{text}"
    );

    // /todo adds, silently, as the user.
    for c in "/todo Check mobile".chars() {
        app.handle_key(key(KeyCode::Char(c)));
    }
    let fx = app.handle_key(key(KeyCode::Enter));
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::Action { action, args: Some(a), .. }
            if action == "neo.todos" && a["action"] == "add" && a["text"] == "Check mobile")),
        "{fx:?}"
    );
    assert!(app.editor.text().is_empty());

    // Tab reaches the panel; space checks the selected one off.
    while app.focus != Some(Focus::Panel(Panel::Todos)) {
        app.handle_key(key(KeyCode::Tab));
    }
    let fx = app.handle_key(key(KeyCode::Char(' ')));
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::Action { args: Some(a), .. }
            if a["action"] == "done" && a["id"] == "t1")),
        "{fx:?}"
    );
    // a writes a new one in the editor.
    app.handle_key(key(KeyCode::Char('a')));
    assert_eq!(app.focus, None);
    assert_eq!(app.editor.text(), "/todo ");
}

#[test]
fn a_subagent_starting_flies_in_a_card_and_a_replayed_one_does_not() {
    let mut app = app();
    app.handle_event(started("old", "codex", 60_000, json!({})));
    app.tick(NOW + 1000);
    assert!(!screen_text(&mut app, 100, 30).contains("started"));

    app.handle_event(started("d1", "neo", -1000, json!({"background": true})));
    app.tick(NOW + 1050);
    let entering = screen_text(&mut app, 100, 30);
    app.tick(NOW + 2000);
    let settled = screen_text(&mut app, 100, 30);
    snapshot("toast-subagent-100x30", &settled);
    let at = |s: &str| s.lines().find_map(|l| l.find("● neo"));
    assert!(
        at(&entering).is_none_or(|x| x > at(&settled).unwrap()),
        "{entering}"
    );
    assert!(
        settled.contains("neo started in the background"),
        "{settled}"
    );
    assert!(
        settled.contains("Find the weather and report it"),
        "{settled}"
    );
    // Gone once its time is up.
    app.tick(NOW + 1000 + crate::toast::LIFE_MS + 100);
    assert!(!screen_text(&mut app, 100, 30).contains("● neo"));
}

#[test]
fn a_command_running_a_while_offers_ctrl_b_and_ctrl_b_sends_it_to_the_background() {
    let mut app = app();
    app.handle_event(event(
        "s1",
        1,
        NOW,
        json!({"type": "user_message", "app": "terminal", "text": "build"}),
    ));
    app.handle_event(event(
        "s1",
        2,
        NOW,
        json!({"type": "tool_call", "toolCallId": "t1", "title": "Build the site",
               "kind": "execute", "status": "in_progress",
               "rawInput": {"command": "make all", "description": "Build the site"}}),
    ));
    app.handle_event(Event::ViewSnapshot(ViewSnapshot {
        agent: None,
        session: "s1".into(),
        seq: 2,
        items: vec![
            serde_json::from_value::<ViewItem>(json!({
                "ref": "c-t1", "kind": "tool", "label": "Build the site", "state": "running",
                "command": "make all",
                "component": {"type": "stack", "gap": "s", "children": []}
            }))
            .unwrap(),
        ],
    }));
    let ctrl_b = KeyEvent {
        code: KeyCode::Char('b'),
        modifiers: KeyModifiers::CONTROL,
        kind: KeyEventKind::Press,
        state: KeyEventState::NONE,
    };
    let call_line = |app: &mut App| {
        let text = screen_text(app, 100, 30);
        text.lines()
            .find(|l| l.contains("Build the site"))
            .unwrap()
            .trim_end()
            .to_string()
    };
    // Under a second it says nothing.
    app.tick(NOW + 500);
    let line = call_line(&mut app);
    assert!(line.contains("Build the site   "), "{line}");
    // Not yet: Ctrl-B is the cursor's. How long it has run shows already.
    app.tick(NOW + 1000);
    let line = call_line(&mut app);
    assert!(line.contains("Build the site  1s   "), "{line}");
    assert!(app.handle_key(ctrl_b).is_empty());

    app.tick(NOW + 3500);
    let line = call_line(&mut app);
    assert!(
        line.contains("Build the site  3s · ctrl-b background"),
        "{line}"
    );
    let fx = app.handle_key(ctrl_b);
    assert!(
        matches!(&fx[..], [Effect::Action { item, action, args: Some(args), .. }]
            if item == "c-t1" && action == "neo.background" && args["toolCallId"] == "t1"),
        "{fx:?}"
    );
}
