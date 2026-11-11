use super::*;

// ---- sessions sidebar ----

pub(super) fn info(id: &str, title: &str, last: i64) -> SessionInfo {
    SessionInfo {
        harness: None,
        usage: None,
        parent: None,
        id: id.into(),
        title: Some(title.into()),
        cwd: "/home/me/site".into(),
        created_at: 0,
        last_activity: last,
        last_seq: 3,
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

pub(super) fn overview() -> App {
    let mut app = scenario();
    let mut s2 = info("s2", "Fix the login form", NOW - 60_000);
    s2.state = SessionState::Running;
    s2.preview = Some("Looking at the validation".into());
    s2.added = Some(12);
    s2.removed = Some(3);
    let mut s1 = info("s1", "Sticky header", NOW - 30_000);
    s1.preview = Some("Done. The header now uses position: sticky.".into());
    app.set_sessions(vec![s2, s1, info("s3", "Docs", NOW - 3_600_000)]);
    app
}

#[test]
fn context_and_cost_are_in_the_sidebar_while_it_shows_else_under_the_input() {
    let mut app = overview();
    app.handle_event(record(
        "s1",
        9,
        NOW,
        SessionEvent::Usage(
            serde_json::from_value(serde_json::json!({
                "contextUsed": 190000, "contextSize": 1000000, "cost": 5.61
            }))
            .unwrap(),
        ),
    ));
    let text = screen_text(&mut app, 150, 40);
    assert!(app.sessions_shown);
    assert!(text.contains("ctx 19%  190,000 / 1,000,000"), "{text}");
    assert!(!app.status_lines(150)[1].to_string().contains("ctx"));
    // Too narrow for the sidebar: under the input again.
    screen_text(&mut app, 70, 40);
    assert!(!app.sessions_shown);
    assert!(
        app.status_lines(70)[1]
            .to_string()
            .contains("ctx 19% · $5.61")
    );
}

#[test]
fn the_sessions_sidebar_hides_with_its_focus_when_too_narrow() {
    let mut app = overview();
    screen_text(&mut app, 120, 24);
    assert!(app.sessions_shown);
    // Too narrow for the chat beside it: hidden, and so is its focus.
    app.handle_key(ctrl('g'));
    assert_eq!(app.focus, Some(Focus::Sessions));
    let narrow = screen_text(&mut app, 80, 24);
    assert!(!narrow.contains("Sessions"), "{narrow}");
    assert_eq!(app.focus, None);
    // No sessions known (yet): nothing to show.
    let mut empty = scenario();
    assert!(!screen_text(&mut empty, 120, 24).contains("Sessions"));
}

#[test]
fn down_past_the_last_session_reaches_the_apps_and_their_more_row() {
    let mut app = overview();
    let apps: Vec<neo_protocol::ConnectorInfo> = serde_json::from_value(json!(
        (0..9)
            .map(
                |i| json!({"app": format!("app-{i}"), "kind": "app", "connected": false,
                            "v": 0, "tools": []})
            )
            .collect::<Vec<_>>()
    ))
    .unwrap();
    app.handle_event(Event::Connectors(apps));
    screen_text(&mut app, 120, 40);
    app.handle_key(ctrl('g'));
    app.handle_key(key(KeyCode::Char('G')));
    screen_text(&mut app, 120, 40);
    app.handle_key(key(KeyCode::Char('j')));
    assert_eq!(app.apps_sel, Some(0));
    for _ in 0..10 {
        app.handle_key(key(KeyCode::Char('j')));
    }
    // Four apps, then the row for the rest, where the keys stop.
    assert_eq!(app.apps_sel, Some(4));
    app.handle_key(key(KeyCode::Enter));
    assert!(app.apps_open);
    screen_text(&mut app, 120, 40);
    assert_eq!(app.app_rows.apps, 10);
    // Up from the first app is back on the sessions.
    for _ in 0..5 {
        app.handle_key(key(KeyCode::Char('k')));
    }
    assert_eq!(app.apps_sel, None);
    assert_eq!(app.focus, Some(Focus::Sessions));
}

#[test]
fn session_changed_updates_or_adds_one_entry_live() {
    let mut app = overview();
    let titles = |app: &mut App| {
        screen(app, 120, 24)
            .iter()
            .filter_map(|l| {
                let side: String = l.chars().take(29).collect();
                ["Fix the login form", "Sticky header", "Docs", "Brand new"]
                    .into_iter()
                    .find(|t| side.contains(t))
            })
            .collect::<Vec<_>>()
    };
    // The working one first, then newest first.
    assert_eq!(
        titles(&mut app),
        ["Fix the login form", "Sticky header", "Docs"]
    );
    // Docs got a message: it shows what was said but keeps its place.
    let mut docs = info("s3", "Docs", NOW);
    docs.preview = Some("Rewrote the README".into());
    docs.added = Some(40);
    app.handle_event(Event::SessionChanged(docs));
    assert_eq!(
        titles(&mut app),
        ["Fix the login form", "Sticky header", "Docs"]
    );
    let text = screen_text(&mut app, 120, 24);
    assert!(text.contains("Rewrote the README"), "{text}");
    assert!(text.contains("+40"), "{text}");
    // A session nobody listed yet appears.
    app.handle_event(Event::SessionChanged(info("s9", "Brand new", NOW + 1)));
    assert_eq!(app.sessions().len(), 4);
    assert_eq!(titles(&mut app)[1], "Brand new");
    // One the daemon deleted goes again.
    app.handle_event(Event::SessionRemoved("s9".into()));
    assert_eq!(app.sessions().len(), 3);
    assert_eq!(titles(&mut app)[1], "Sticky header");
    // The shown session's own queue and turn state follow it too.
    app.running = true;
    app.turn_started_ms = NOW - 10_000;
    app.now_ms = NOW;
    let mut mine = info("s1", "Sticky header", NOW);
    mine.queued = 1;
    mine.waiting = Some(vec![WaitingInput {
        app: "gui".into(),
        text: "from the desktop".into(),
        images: None,
        agent: None,
    }]);
    app.handle_event(Event::SessionChanged(mine));
    assert_eq!(app.waiting.len(), 1);
    assert!(!app.running, "the daemon says idle");
}

#[test]
fn a_session_that_moved_takes_the_app_along() {
    let mut app = overview();
    app.handle_event(Event::Session(
        serde_json::from_value(json!({"session": "s1", "seq": 9, "ts": NOW, "event": {
            "type": "todos", "items": [{"id": "t1", "text": "Ship it", "done": false}]
        }}))
        .unwrap(),
    ));
    let mut moved = info("s1", "Sticky header", NOW);
    moved.cwd = "/home/me/site/docs".into();
    app.handle_event(Event::SessionChanged(moved));
    assert_eq!(app.cwd(), "/home/me/site/docs");
    // The session's own state stays: only where it works changed.
    assert!(app.widgets().has_todos());
    assert_eq!(app.session(), "s1");
}

#[test]
fn running_sessions_animate_in_the_sidebar() {
    let mut app = overview();
    app.tick(NOW);
    screen(&mut app, 120, 24);
    app.take_dirty();
    app.tick(NOW + 200);
    assert!(app.take_dirty(), "the spinner needs a frame");
    let mut idle = overview();
    idle.set_sessions(vec![info("s1", "Sticky header", NOW)]);
    idle.tick(NOW);
    screen(&mut idle, 120, 24);
    idle.take_dirty();
    idle.tick(NOW + 200);
    assert!(!idle.take_dirty());
}

#[test]
fn ctrl_g_focuses_the_sidebar_and_enter_switches() {
    let mut app = overview();
    screen(&mut app, 120, 24);
    app.handle_key(ctrl('g'));
    assert_eq!(app.focus, Some(Focus::Sessions));
    // The keys are the sidebar's: typing does not reach the editor.
    type_str(&mut app, "x");
    assert_eq!(app.editor_text(), "");
    // Starts on the shown session (s1, under the working s2); j goes to s3, k back to s1, k
    // again to s2.
    app.handle_key(key(KeyCode::Char('j')));
    app.handle_key(key(KeyCode::Char('k')));
    app.handle_key(key(KeyCode::Up));
    let text = screen_text(&mut app, 120, 24);
    assert!(text.contains("Fix the login form"), "{text}");
    let fx = app.handle_key(key(KeyCode::Enter));
    assert_eq!(
        fx,
        [Effect::Switch {
            from: Some("s1".into()),
            to: "s2".into(),
            cwd: "/home/me/site".into(),
            move_agent: true,
        }]
    );
    assert_eq!(app.session(), "s2");
    assert_eq!(app.focus, None, "the editor has the keys again");
    type_str(&mut app, "x");
    assert_eq!(app.editor_text(), "x");
}

#[test]
fn new_chat_in_the_sidebar_creates_a_session_here() {
    let mut app = overview();
    screen(&mut app, 120, 24);
    let create = [Effect::CreateSession {
        cwd: app.cwd().to_string(),
    }];
    // The row above the sessions, then Enter.
    app.handle_key(ctrl('g'));
    app.handle_key(key(KeyCode::Char('g')));
    let text = screen_text(&mut app, 120, 24);
    assert!(text.contains("+ New chat"), "{text}");
    assert_eq!(app.handle_key(key(KeyCode::Enter)), create);
    assert_eq!(app.focus, None, "the editor has the keys again");
    // n from anywhere in the list.
    app.handle_key(ctrl('g'));
    assert_eq!(app.handle_key(key(KeyCode::Char('n'))), create);
}

#[test]
fn the_sidebar_gives_the_keys_back() {
    let mut app = overview();
    screen(&mut app, 120, 24);
    app.handle_key(ctrl('g'));
    app.handle_key(key(KeyCode::Esc));
    assert_eq!(app.focus, None);
    app.handle_key(ctrl('g'));
    app.handle_key(ctrl('g'));
    assert_eq!(app.focus, None);
    // Enter on the shown session just returns to the editor.
    app.handle_key(ctrl('g'));
    assert!(app.handle_key(key(KeyCode::Enter)).is_empty());
    assert_eq!(app.focus, None);
    assert_eq!(app.session(), "s1");
}

#[test]
fn tab_walks_sessions_plan_files_and_back() {
    let mut app = overview();
    app.handle_event(plan_event());
    screen(&mut app, 150, 30);
    assert!(app.sessions_shown && app.sidebar_shown);
    let mut seen = Vec::new();
    for _ in 0..5 {
        app.handle_key(key(KeyCode::Tab));
        seen.push(app.focus);
    }
    assert_eq!(
        seen,
        [
            // The chat's replies first.
            Some(Focus::Chat),
            Some(Focus::Sessions),
            Some(Focus::Panel(Panel::Plan)),
            Some(Focus::Panel(Panel::Files)),
            None
        ]
    );
}

#[test]
fn shift_tab_walks_the_panels_back_to_the_editor() {
    let mut app = overview();
    app.handle_event(plan_event());
    screen(&mut app, 150, 30);
    for _ in 0..4 {
        app.handle_key(key(KeyCode::Tab));
    }
    assert_eq!(app.focus, Some(Focus::Panel(Panel::Files)));
    let approvals = app.approvals;
    let mut seen = Vec::new();
    for _ in 0..4 {
        assert!(app.handle_key(key(KeyCode::BackTab)).is_empty());
        seen.push(app.focus);
    }
    assert_eq!(
        seen,
        [
            Some(Focus::Panel(Panel::Plan)),
            Some(Focus::Sessions),
            Some(Focus::Chat),
            None
        ]
    );
    assert_eq!(app.approvals, approvals, "the approvals mode stays");
    // In the editor Shift-Tab is the approvals' again.
    assert!(!app.handle_key(key(KeyCode::BackTab)).is_empty());
}

fn command_event() -> Event {
    record(
        "s1",
        6,
        NOW,
        SessionEvent::ToolCall(
            serde_json::from_value(serde_json::json!({
                "toolCallId": "t1", "title": "bash: cargo build", "kind": "execute",
                "status": "in_progress", "rawInput": {"command": "cargo build --release"}
            }))
            .unwrap(),
        ),
    )
}

#[test]
fn the_agents_commands_show_next_to_the_chat() {
    let mut app = app();
    assert!(!screen_text(&mut app, 120, 24).contains("Terminal"));
    app.handle_event(command_event());
    let text = screen_text(&mut app, 120, 24);
    assert!(
        text.contains("Terminal") && text.contains("$ cargo build --release"),
        "{text}"
    );
    // Tab gets there, Esc gives the keys back.
    app.handle_key(key(KeyCode::Tab));
    assert_eq!(app.focus, Some(Focus::Panel(Panel::Terminal)));
    app.handle_key(key(KeyCode::Esc));
    assert_eq!(app.focus, None);
    // Too narrow for both: the chat keeps its room.
    assert!(!screen_text(&mut app, 80, 24).contains("Terminal"));
    // /terminal hides it.
    type_str(&mut app, "/terminal");
    app.handle_key(key(KeyCode::Enter));
    assert!(!screen_text(&mut app, 120, 24).contains("Terminal"));
}

#[test]
fn the_plan_column_wins_when_both_do_not_fit() {
    let mut app = overview();
    app.handle_event(plan_event());
    let text = screen_text(&mut app, 110, 24);
    assert!(
        text.contains("crop the logo") && !text.contains("Sessions"),
        "{text}"
    );
    let text = screen_text(&mut app, 150, 24);
    assert!(
        text.contains("crop the logo") && text.contains("Sessions"),
        "{text}"
    );
}

#[test]
fn sidebar_command_hides_and_shows_it() {
    let mut app = overview();
    assert!(screen_text(&mut app, 120, 24).contains("Sessions"));
    type_str(&mut app, "/sidebar");
    assert!(app.handle_key(key(KeyCode::Enter)).is_empty());
    assert!(!screen_text(&mut app, 120, 24).contains("Sessions"));
    // Ctrl-G switches it on again and takes the keys.
    app.handle_key(ctrl('g'));
    assert!(screen_text(&mut app, 120, 24).contains("Sessions"));
    assert_eq!(app.focus, Some(Focus::Sessions));
    // Too narrow: it says so instead of doing nothing.
    app.focus = None;
    screen(&mut app, 70, 24);
    app.handle_key(ctrl('g'));
    screen(&mut app, 70, 24);
    assert_eq!(app.focus, None);
    assert!(app.status_lines(100)[1].to_string().contains("too narrow"));
}

#[test]
fn the_session_list_is_refreshed_by_list_sessions_answers() {
    let mut app = overview();
    app.handle_msg(Msg::Sessions(Ok(vec![info("s1", "Sticky header", NOW)])));
    assert_eq!(app.sessions().len(), 1);
    app.handle_msg(Msg::Created(Ok(Box::new(info("s5", "Fresh", NOW + 5)))));
    assert_eq!(app.sessions().len(), 2);
}
