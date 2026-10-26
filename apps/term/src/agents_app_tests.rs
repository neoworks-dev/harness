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

/// neo has been searching tools for 72 s; claude works in a worktree of its own.
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
    app.handle_event(started(
        "d2",
        "claude",
        5_000,
        json!({"model": "sonnet", "cwd": "/home/me/site/.worktrees/fix-login", "child": "s_child"}),
    ));
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
    assert!(lines[at + 2].starts_with("○ claude · sonnet ⎇ fix-login  starting…"));
}

#[test]
fn a_subagent_in_its_own_session_is_followed_and_its_activity_shown() {
    let mut app = app();
    let fx = app.handle_event(started(
        "d2",
        "claude",
        1_000,
        json!({"child": "s_child", "cwd": "/w/x"}),
    ));
    assert_eq!(
        fx,
        [Effect::Watch {
            session: "s_child".into()
        }]
    );
    app.handle_event(event(
        "s_child",
        4,
        NOW,
        json!({"type": "tool_call", "toolCallId": "t", "title": "bash: cargo test",
               "kind": "execute", "status": "pending"}),
    ));
    assert!(screen_text(&mut app, 100, 24).contains("bash: cargo test"));
    // Its events don't leak into the chat's state.
    assert!(!app.running());
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
    assert!(
        text.contains("✗ claude · sonnet ⎇ fix-login  boom"),
        "{text}"
    );
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
    // The latest running one is claude (own session); up to neo.
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
fn enter_on_a_subagent_with_its_own_session_switches_to_it() {
    let mut app = busy();
    app.set_sessions(vec![
        session_info("s1", "/home/me/site", None),
        session_info("s_child", "/home/me/site/.worktrees/fix-login", Some("s1")),
    ]);
    app.handle_key(ctrl('x'));
    let fx = app.handle_key(key(KeyCode::Enter));
    assert_eq!(app.session(), "s_child");
    assert_eq!(
        fx,
        [Effect::Switch {
            from: Some("s1".into()),
            to: "s_child".into(),
            cwd: "/home/me/site/.worktrees/fix-login".into(),
            move_agent: true,
        }]
    );
    // The parent's agents belong to the parent.
    assert!(!screen_text(&mut app, 100, 24).contains("● main"));
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
                   "args": {"id": "p1", "option": l}, "key": (i + 1).to_string()})
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
fn ctrl_brackets_widen_and_narrow_the_terminal_panel() {
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
    app.handle_key(ctrl(']'));
    assert_eq!(column(&mut app), at - 4, "wider by four");
    app.handle_key(ctrl('['));
    app.handle_key(ctrl('['));
    assert_eq!(column(&mut app), at + 4);
    // Not narrower than its least.
    for _ in 0..30 {
        app.handle_key(ctrl('['));
    }
    let least = column(&mut app);
    // 40 columns: the border and a space, then the title.
    assert_eq!(160 - least, 40 - 2, "{least}");
    // Presses past the edge don't pile up: one press back moves it.
    app.handle_key(ctrl(']'));
    assert_eq!(column(&mut app), least - 4);
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
