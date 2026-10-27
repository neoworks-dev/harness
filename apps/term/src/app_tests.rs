use neo_protocol::{
    CommandsAvailable, FocusReason, SessionEvent, SettingChanged, SettingName, SlashCommand,
    StopReason, TurnEnded, UserMessage, ViewItem, ViewOp, ViewPatch, ViewSnapshot,
};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
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

fn with(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
    KeyEvent {
        modifiers,
        ..key(code)
    }
}

fn ctrl(c: char) -> KeyEvent {
    with(KeyCode::Char(c), KeyModifiers::CONTROL)
}

fn type_str(app: &mut App, s: &str) {
    for c in s.chars() {
        assert!(app.handle_key(key(KeyCode::Char(c))).is_empty());
    }
}

fn item(r: &str, kind: &str, text: &str) -> ViewItem {
    serde_json::from_value(json!({
        "ref": r, "kind": kind,
        "component": {"type":"text","blocks":[{"type":"paragraph","spans":[{"text": text}]}]}
    }))
    .unwrap()
}

fn prompt(r: &str, state: &str, labels: &[&str]) -> ViewItem {
    let children: Vec<_> = labels
        .iter()
        .enumerate()
        .map(|(i, l)| {
            json!({"type":"button","label":l,"action":"neo.permit",
                   "args":{"id":r,"option":l},"key":(i + 1).to_string()})
        })
        .collect();
    serde_json::from_value(json!({
        "ref": r, "kind": "tool", "label": "claude: run ls?", "state": state,
        "component": {"type":"row","children": children}
    }))
    .unwrap()
}

fn show(app: &mut App, items: Vec<ViewItem>) {
    app.handle_event(Event::ViewSnapshot(ViewSnapshot {
        agent: None,
        session: "s1".into(),
        seq: 1,
        items,
    }));
}

fn patch(app: &mut App, item: ViewItem) {
    app.handle_event(Event::ViewPatch(ViewPatch {
        agent: None,
        session: "s1".into(),
        seq: 2,
        ops: vec![ViewOp::SetViewOp(neo_protocol::SetViewOp { item })],
    }));
}

fn record(session: &str, seq: i64, ts: i64, event: SessionEvent) -> Event {
    Event::Session(SessionEventRecord {
        session: session.into(),
        seq,
        ts,
        event,
    })
}

fn user_message(ts: i64) -> Event {
    record(
        "s1",
        1,
        ts,
        SessionEvent::UserMessage(UserMessage {
            app: "terminal".into(),
            text: "hi".into(),
            shown: None,
            images: None,
        }),
    )
}

fn turn_ended() -> Event {
    record(
        "s1",
        2,
        NOW,
        SessionEvent::TurnEnded(TurnEnded {
            stop_reason: StopReason::EndTurn,
            error: None,
        }),
    )
}

fn setting(harness: &str, option: SettingName, value: &str) -> Event {
    record(
        "s1",
        3,
        NOW,
        SessionEvent::SettingChanged(SettingChanged {
            harness: harness.into(),
            option,
            value: value.into(),
        }),
    )
}

fn focus(session: &str, app: Option<&str>) -> Event {
    Event::Focus(AgentFocus {
        session: session.into(),
        app: app.map(Into::into),
        from: None,
        reason: FocusReason::Session,
    })
}

fn input(text: &str) -> Effect {
    Effect::Input {
        session: "s1".into(),
        text: text.into(),
        images: Vec::new(),
    }
}

fn screen(app: &mut App, w: u16, h: u16) -> Vec<String> {
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
        .collect()
}

fn screen_text(app: &mut App, w: u16, h: u16) -> String {
    screen(app, w, h).join("\n")
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

fn scenario() -> App {
    let mut app = app();
    show(
        &mut app,
        vec![
            item("u1", "user", "make the header sticky"),
            item(
                "a1",
                "agent",
                "Done. The header now uses `position: sticky`.",
            ),
        ],
    );
    app
}

// ---- editing and sending ----

#[test]
fn enter_sends_the_trimmed_text_and_clears() {
    let mut app = app();
    type_str(&mut app, "  hello  ");
    let fx = app.handle_key(key(KeyCode::Enter));
    assert_eq!(fx, [input("hello")]);
    assert_eq!(app.editor_text(), "");
    // Nothing to send.
    type_str(&mut app, "   ");
    assert!(app.handle_key(key(KeyCode::Enter)).is_empty());
}

#[test]
fn alt_enter_and_ctrl_j_insert_newlines_and_paste_stays_in_the_editor() {
    let mut app = app();
    type_str(&mut app, "a");
    assert!(
        app.handle_key(with(KeyCode::Enter, KeyModifiers::ALT))
            .is_empty()
    );
    type_str(&mut app, "b");
    assert!(app.handle_key(ctrl('j')).is_empty());
    app.handle_paste("c\nd\r\ne");
    assert_eq!(app.editor_text(), "a\nb\nc\nd\ne");
    let fx = app.handle_key(key(KeyCode::Enter));
    assert_eq!(fx, [input("a\nb\nc\nd\ne")]);
}

#[test]
fn ctrl_v_and_an_empty_paste_read_the_clipboard_image() {
    let mut app = app();
    assert_eq!(app.handle_key(ctrl('v')), [Effect::PasteImage]);
    assert_eq!(app.handle_paste(""), [Effect::PasteImage]);
    app.handle_msg(Msg::Pasted(None));
    assert!(
        app.status_lines(80)[1]
            .to_string()
            .contains("no image on the clipboard")
    );
}

#[test]
fn pasted_images_are_placed_in_the_draft_and_go_with_it() {
    let dir = std::env::temp_dir().join("neo-term-test-attach");
    std::fs::create_dir_all(&dir).unwrap();
    let dropped = dir.join("cat.png");
    std::fs::write(&dropped, b"x").unwrap();
    let dropped = dropped.to_string_lossy().into_owned();
    let mut app = app();
    type_str(&mut app, "compare");
    app.handle_msg(Msg::Pasted(Some("/tmp/neo-term/paste-1.png".into())));
    type_str(&mut app, "with");
    assert!(app.handle_paste(&format!("'{dropped}'")).is_empty());
    assert_eq!(app.editor_text(), "compare [image 1] with [image 2] ");
    let shown = screen(&mut app, 80, 20).join("\n");
    assert!(
        shown.contains("[image 1] paste-1.png  [image 2] cat.png"),
        "{shown}"
    );

    // Backspace after a marker takes all of it, and the image with it.
    app.handle_key(key(KeyCode::Backspace));
    app.handle_key(key(KeyCode::Backspace));
    assert_eq!(app.editor_text(), "compare [image 1] with ");
    let shown = screen(&mut app, 80, 20).join("\n");
    assert!(!shown.contains("cat.png"), "{shown}");

    // Pasted again it is image 3; sent, the markers count from 1 in reading order.
    app.handle_key(ctrl('a'));
    app.handle_paste(&dropped);
    assert_eq!(app.editor_text(), "[image 3] compare [image 1] with ");
    let fx = app.handle_key(key(KeyCode::Enter));
    assert_eq!(
        fx,
        [Effect::Input {
            session: "s1".into(),
            text: "[image 1] compare [image 2] with".into(),
            images: vec![dropped.clone(), "/tmp/neo-term/paste-1.png".into()],
        }]
    );
    assert!(!screen(&mut app, 80, 20).join("\n").contains("[image"));

    // Ctrl-C drops the draft and its images.
    app.handle_paste(&dropped);
    app.handle_key(ctrl('c'));
    assert!(app.handle_key(key(KeyCode::Enter)).is_empty());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn thumbnails_keep_the_pictures_shape() {
    // 6 rows of 20px cells are 120px high; a square picture is 120px wide, 12 cells of 10px.
    assert_eq!(thumbnail_cols(400, 400, 6, (10, 20)), 12);
    assert_eq!(thumbnail_cols(100, 400, 6, (10, 20)), 3);
    assert_eq!(thumbnail_cols(4000, 400, 6, (10, 20)), PREVIEW_COLS);
    assert_eq!(thumbnail_cols(0, 400, 6, (10, 20)), PREVIEW_COLS / 2);
}

#[test]
fn readline_keys() {
    let mut app = app();
    type_str(&mut app, "one two three");
    app.handle_key(ctrl('w'));
    assert_eq!(app.editor_text(), "one two ");
    app.handle_key(with(KeyCode::Char('b'), KeyModifiers::ALT));
    app.handle_key(ctrl('k'));
    assert_eq!(app.editor_text(), "one ");
    app.handle_key(ctrl('a'));
    type_str(&mut app, ">");
    app.handle_key(ctrl('e'));
    type_str(&mut app, "!");
    assert_eq!(app.editor_text(), ">one !");
    app.handle_key(ctrl('u'));
    assert_eq!(app.editor_text(), "");
}

#[test]
fn up_and_down_walk_the_history_with_the_draft_kept() {
    let mut app = app();
    for text in ["first", "second"] {
        type_str(&mut app, text);
        app.handle_key(key(KeyCode::Enter));
    }
    type_str(&mut app, "draft");
    app.handle_key(key(KeyCode::Up));
    assert_eq!(app.editor_text(), "second");
    app.handle_key(key(KeyCode::Up));
    assert_eq!(app.editor_text(), "first");
    app.handle_key(key(KeyCode::Up));
    assert_eq!(app.editor_text(), "first");
    app.handle_key(key(KeyCode::Down));
    app.handle_key(key(KeyCode::Down));
    assert_eq!(app.editor_text(), "draft");
}

#[test]
fn a_recalled_command_does_not_stop_the_walk() {
    let mut app = app();
    for text in ["first", "/model"] {
        type_str(&mut app, text);
        app.handle_key(key(KeyCode::Enter));
    }
    app.handle_key(key(KeyCode::Up));
    assert_eq!(app.editor_text(), "/model");
    assert!(!app.menu_visible());
    app.handle_key(key(KeyCode::Up));
    assert_eq!(app.editor_text(), "first");
    app.handle_key(key(KeyCode::Down));
    assert_eq!(app.editor_text(), "/model");
    // Typing opens it again.
    app.handle_key(key(KeyCode::Backspace));
    assert!(app.menu_visible());
}

#[test]
fn up_moves_between_lines_before_touching_the_history() {
    let mut app = app();
    type_str(&mut app, "old");
    app.handle_key(key(KeyCode::Enter));
    app.handle_paste("one\ntwo");
    app.handle_key(key(KeyCode::Up));
    assert_eq!(app.editor_text(), "one\ntwo", "moved to the first line");
    app.handle_key(key(KeyCode::Up));
    assert_eq!(app.editor_text(), "old");
}

#[test]
fn history_is_per_session() {
    let mut app = app();
    type_str(&mut app, "mine");
    app.handle_key(key(KeyCode::Enter));
    app.handle_event(focus("s2", Some("terminal")));
    app.handle_key(key(KeyCode::Up));
    assert_eq!(app.editor_text(), "");
}

// ---- slash menu ----

#[test]
fn the_slash_menu_filters_completes_and_runs() {
    let mut app = app();
    type_str(&mut app, "/");
    assert_eq!(app.menu_matches().len(), menu::COMMANDS.len());
    type_str(&mut app, "mo");
    let names: Vec<_> = app
        .menu_matches()
        .iter()
        .map(|c| c.name.to_string())
        .collect();
    assert_eq!(names, ["model", "mode"]);
    // Down selects the second; Tab completes it with a trailing space.
    app.handle_key(key(KeyCode::Down));
    app.handle_key(key(KeyCode::Tab));
    assert_eq!(app.editor_text(), "/mode ");
    assert!(!app.menu_visible(), "a space ends the command word");
    // Enter on a partial runs the selected command.
    app.handle_key(ctrl('u'));
    type_str(&mut app, "/mo");
    let fx = app.handle_key(key(KeyCode::Enter));
    assert_eq!(fx, [input("/model")]);
    assert_eq!(app.editor_text(), "");
}

fn offered(harness: &str, commands: &[(&str, &str, Option<&str>)]) -> Event {
    record(
        "s1",
        4,
        NOW,
        SessionEvent::CommandsAvailable(CommandsAvailable {
            harness: harness.into(),
            commands: commands
                .iter()
                .map(|(name, description, hint)| SlashCommand {
                    name: (*name).into(),
                    description: Some((*description).into()),
                    hint: hint.map(Into::into),
                })
                .collect(),
        }),
    )
}

#[test]
fn the_slash_menu_lists_the_commands_of_the_harness_the_session_runs_on() {
    let mut app = app();
    app.handle_event(offered(
        "claude",
        &[
            ("review", "Review the changes", Some("branch or PR")),
            ("model", "Claude's own model picker", None),
        ],
    ));
    app.handle_event(offered("neo", &[("commit", "Create a git commit", None)]));
    // The session runs on neo: its list, not claude's.
    type_str(&mut app, "/rev");
    assert!(app.menu_matches().is_empty());
    app.handle_key(ctrl('u'));
    type_str(&mut app, "/com");
    let names: Vec<_> = app.menu_matches().iter().map(|c| c.name.clone()).collect();
    assert_eq!(names, ["commit"]);
    app.handle_key(ctrl('u'));

    // After /harness claude the menu is claude's, after ours; ours win a name.
    app.handle_event(setting("daemon", SettingName::Harness, "claude"));
    type_str(&mut app, "/rev");
    let found = app.menu_matches();
    assert_eq!(found.len(), 1);
    assert_eq!(
        (found[0].name.as_str(), found[0].args.as_str()),
        ("review", "branch or PR")
    );
    app.handle_key(ctrl('u'));
    type_str(&mut app, "/model");
    let found = app.menu_matches();
    assert_eq!(found.len(), 1);
    assert!(
        found[0].desc.contains("replaces claude's /model"),
        "{}",
        found[0].desc
    );
    app.handle_key(ctrl('u'));

    // Enter runs the harness's command as text; with arguments typed it is sent as typed.
    type_str(&mut app, "/revi");
    assert_eq!(app.handle_key(key(KeyCode::Enter)), [input("/review")]);
    type_str(&mut app, "/review main");
    assert_eq!(app.handle_key(key(KeyCode::Enter)), [input("/review main")]);

    // A later list replaces the earlier one.
    app.handle_event(offered("claude", &[("compact", "Summarise", None)]));
    type_str(&mut app, "/rev");
    assert!(app.menu_matches().is_empty());
}

#[test]
fn commands_that_need_a_task_complete_instead_of_running() {
    let mut app = app();
    type_str(&mut app, "/claude");
    assert!(app.handle_key(key(KeyCode::Enter)).is_empty());
    assert_eq!(app.editor_text(), "/claude ");
    type_str(&mut app, "fix the test");
    let fx = app.handle_key(key(KeyCode::Enter));
    assert_eq!(fx, [input("/claude fix the test")]);
}

#[test]
fn daemon_commands_with_arguments_are_sent_as_typed() {
    let mut app = app();
    type_str(&mut app, "/model claude sonnet");
    assert_eq!(
        app.handle_key(key(KeyCode::Enter)),
        [input("/model claude sonnet")]
    );
    type_str(&mut app, "/approvals auto");
    assert_eq!(
        app.handle_key(key(KeyCode::Enter)),
        [input("/approvals auto")]
    );
}

#[test]
fn unknown_slash_text_is_just_sent() {
    let mut app = app();
    type_str(&mut app, "/usr/bin/env is where?");
    assert_eq!(
        app.handle_key(key(KeyCode::Enter)),
        [input("/usr/bin/env is where?")]
    );
    type_str(&mut app, "/zzz");
    assert!(!app.menu_visible());
    assert_eq!(app.handle_key(key(KeyCode::Enter)), [input("/zzz")]);
}

#[test]
fn local_commands_run_here() {
    let mut app = app();
    type_str(&mut app, "/expand");
    assert!(app.handle_key(key(KeyCode::Enter)).is_empty());
    assert!(app.expanded);
    type_str(&mut app, "/thoughts");
    app.handle_key(key(KeyCode::Enter));
    assert!(app.show_thoughts);
    type_str(&mut app, "/help");
    app.handle_key(key(KeyCode::Enter));
    assert_eq!(app.overlay, Overlay::Help);
    app.handle_key(key(KeyCode::Char('x')));
    assert_eq!(app.overlay, Overlay::None);
    assert_eq!(app.editor_text(), "", "the key only closed the help");
    type_str(&mut app, "/sessions");
    assert_eq!(app.handle_key(key(KeyCode::Enter)), [Effect::ListSessions]);
    assert_eq!(app.overlay, Overlay::Switcher);
    app.handle_key(key(KeyCode::Esc));
    type_str(&mut app, "/quit");
    assert_eq!(app.handle_key(key(KeyCode::Enter)), [Effect::Quit]);
}

#[test]
fn esc_closes_the_menu_without_clearing_the_text() {
    let mut app = app();
    type_str(&mut app, "/mo");
    assert!(app.menu_visible());
    assert!(app.handle_key(key(KeyCode::Esc)).is_empty());
    assert!(!app.menu_visible());
    assert_eq!(app.editor_text(), "/mo");
    type_str(&mut app, "d");
    assert!(app.menu_visible(), "typing again brings it back");
}

// ---- Esc, Ctrl-C, Ctrl-D ----

#[test]
fn esc_cancels_a_running_turn_only_when_nothing_else_is_open() {
    let mut app = app();
    assert!(
        app.handle_key(key(KeyCode::Esc)).is_empty(),
        "idle: nothing"
    );
    app.handle_event(user_message(NOW));
    assert!(app.running());
    let fx = app.handle_key(key(KeyCode::Esc));
    assert_eq!(
        fx,
        [Effect::Cancel {
            session: "s1".into()
        }]
    );
    // The menu takes the first Esc.
    type_str(&mut app, "/c");
    assert!(app.handle_key(key(KeyCode::Esc)).is_empty());
    assert_eq!(app.editor_text(), "/c");
    // Then the turn.
    assert_eq!(
        app.handle_key(key(KeyCode::Esc)),
        [Effect::Cancel {
            session: "s1".into()
        }]
    );
}

#[test]
fn ctrl_c_clears_then_needs_a_second_press_to_quit() {
    let mut app = app();
    type_str(&mut app, "some text");
    assert!(app.handle_key(ctrl('c')).is_empty());
    assert_eq!(app.editor_text(), "");
    assert!(app.handle_key(ctrl('c')).is_empty());
    assert!(
        app.status_lines(60)[1]
            .to_string()
            .contains("press ctrl-c again to quit")
    );
    app.tick(NOW + 1000);
    assert_eq!(app.handle_key(ctrl('c')), [Effect::Quit]);
}

#[test]
fn the_quit_window_closes_and_other_keys_disarm_it() {
    let mut app = app();
    app.handle_key(ctrl('c'));
    app.tick(NOW + 2000);
    assert!(
        app.handle_key(ctrl('c')).is_empty(),
        "too late: it arms again"
    );
    app.tick(NOW + 2500);
    assert_eq!(app.handle_key(ctrl('c')), [Effect::Quit]);
    // Any other key in between disarms.
    let mut app = self::app();
    app.handle_key(ctrl('c'));
    type_str(&mut app, "y");
    assert!(app.handle_key(ctrl('c')).is_empty(), "cleared the text");
    assert!(app.handle_key(ctrl('c')).is_empty(), "and only armed");
    assert_eq!(app.handle_key(ctrl('c')), [Effect::Quit]);
}

#[test]
fn ctrl_d_quits_on_an_empty_editor_only() {
    let mut app = app();
    type_str(&mut app, "abc");
    app.handle_key(key(KeyCode::Left));
    assert!(app.handle_key(ctrl('d')).is_empty());
    assert_eq!(
        app.editor_text(),
        "ab",
        "deleted the character under the cursor"
    );
    app.handle_key(ctrl('u'));
    assert_eq!(app.handle_key(ctrl('d')), [Effect::Quit]);
}

// ---- scrolling ----

fn long_chat(app: &mut App, n: usize) {
    show(
        app,
        (0..n)
            .map(|i| item(&format!("a{i}"), "agent", &format!("message number {i}")))
            .collect(),
    );
    screen(app, 80, 24);
}

#[test]
fn page_keys_scroll_the_chat_and_new_output_leaves_the_view_alone() {
    let mut app = app();
    long_chat(&mut app, 40);
    assert!(app.chat().following());
    let tail = screen(&mut app, 80, 24);
    assert!(tail.iter().any(|l| l.contains("message number 39")));
    app.handle_key(key(KeyCode::PageUp));
    assert!(!app.chat().following());
    let before = screen(&mut app, 80, 24);
    assert!(!before.iter().any(|l| l.contains("message number 39")));
    patch(&mut app, item("new", "agent", "fresh output"));
    let after = screen(&mut app, 80, 24);
    // The scrollbar's thumb moves: there is more to scroll.
    let text = |rows: &[String]| -> Vec<String> {
        rows[..18]
            .iter()
            .map(|l| l.trim_end_matches('│').trim_end().to_string())
            .collect()
    };
    assert_eq!(text(&before), text(&after), "the chat area did not move");
    assert!(
        after.iter().any(|l| l.contains("↓ new output (End)")),
        "{after:#?}"
    );
    // End (empty editor) follows again.
    app.handle_key(key(KeyCode::End));
    let end = screen(&mut app, 80, 24);
    assert!(end.iter().any(|l| l.contains("fresh output")));
    assert!(!end.iter().any(|l| l.contains("new output (End)")));
}

#[test]
fn the_wheel_and_shift_arrows_scroll_the_chat() {
    use ratatui::crossterm::event::{MouseEvent, MouseEventKind};
    let wheel = |kind| MouseEvent {
        kind,
        column: 10,
        row: 5,
        modifiers: KeyModifiers::NONE,
    };
    let mut app = app();
    long_chat(&mut app, 40);
    let bottom = app.chat().top();
    app.handle_mouse(wheel(MouseEventKind::ScrollUp));
    assert!(!app.chat().following());
    assert_eq!(app.chat().top(), bottom - 3);
    app.handle_key(with(KeyCode::Up, KeyModifiers::SHIFT));
    assert_eq!(app.chat().top(), bottom - 4);
    app.handle_key(with(KeyCode::Down, KeyModifiers::SHIFT));
    app.handle_mouse(wheel(MouseEventKind::Moved));
    assert_eq!(
        app.chat().top(),
        bottom - 3,
        "other mouse events do nothing"
    );
    app.handle_mouse(wheel(MouseEventKind::ScrollDown));
    assert!(app.chat().following());
    // Shift-arrows don't walk the input history.
    type_str(&mut app, "draft");
    app.handle_key(with(KeyCode::Up, KeyModifiers::SHIFT));
    assert_eq!(app.editor_text(), "draft");
}

#[test]
fn home_end_belong_to_the_editor_when_it_has_text() {
    let mut app = app();
    long_chat(&mut app, 40);
    type_str(&mut app, "abc");
    app.handle_key(key(KeyCode::Home));
    assert!(
        app.chat().following(),
        "Home moved the cursor, not the chat"
    );
    type_str(&mut app, "X");
    assert_eq!(app.editor_text(), "Xabc");
    app.handle_key(with(KeyCode::Home, KeyModifiers::CONTROL));
    assert!(!app.chat().following(), "ctrl-home always scrolls");
    assert_eq!(app.chat().top(), 0);
    app.handle_key(with(KeyCode::End, KeyModifiers::CONTROL));
    assert!(app.chat().following());
}

#[test]
fn ctrl_u_and_ctrl_d_scroll_halves_when_there_is_nothing_to_edit() {
    let mut app = app();
    long_chat(&mut app, 40);
    let max = app.chat().top();
    app.handle_key(ctrl('u'));
    let up = app.chat().top();
    assert!(up < max && max - up >= 5);
    assert!(
        app.handle_key(ctrl('d')).is_empty(),
        "scrolled up: ctrl-d scrolls instead of quitting"
    );
    assert!(app.chat().top() > up);
    app.handle_key(key(KeyCode::End));
    assert_eq!(app.handle_key(ctrl('d')), [Effect::Quit]);
}

#[test]
fn sending_jumps_back_to_the_tail() {
    let mut app = app();
    long_chat(&mut app, 40);
    app.handle_key(key(KeyCode::PageUp));
    type_str(&mut app, "hi");
    app.handle_key(key(KeyCode::Enter));
    assert!(app.chat().following());
}

#[test]
fn ctrl_o_and_ctrl_t_change_what_is_drawn() {
    let mut app = app();
    let tool: ViewItem = serde_json::from_value(json!({
        "ref":"t1","kind":"tool","label":"bash: seq 1 20","state":"done",
        "component":{"type":"code","code": (1..=20).map(|i| i.to_string()).collect::<Vec<_>>().join("\n")}
    }))
    .unwrap();
    let thought: ViewItem = serde_json::from_value(json!({
        "ref":"th","kind":"thought",
        "component":{"type":"text","blocks":[{"type":"paragraph","spans":[{"text":"pondering deeply"}]}]}
    }))
    .unwrap();
    show(&mut app, vec![thought, tool]);
    let compact = screen_text(&mut app, 80, 30);
    assert!(!compact.contains("pondering"));
    assert!(!compact.contains("\n18"));
    app.handle_key(ctrl('o'));
    app.handle_key(ctrl('t'));
    let full = screen_text(&mut app, 80, 30);
    assert!(full.contains("pondering deeply"));
    assert!(full.contains("18"), "{full}");
}

// ---- prompts ----

#[test]
fn a_pending_prompt_is_active_with_the_first_button_focused() {
    let mut app = app();
    show(
        &mut app,
        vec![
            item("a", "agent", "hi"),
            prompt("p1", "pending", &["Allow once", "Reject"]),
        ],
    );
    assert_eq!(app.active_ref(), Some("p1"));
    assert_eq!(app.prompt.button, 0);
    let text = screen_text(&mut app, 80, 24);
    assert!(text.contains("┃ claude: run ls?"), "{text}");
    app.handle_key(key(KeyCode::Right));
    assert_eq!(app.prompt.button, 1);
    app.handle_key(key(KeyCode::Right));
    assert_eq!(app.prompt.button, 0, "wraps");
    app.handle_key(key(KeyCode::Down));
    assert_eq!(app.prompt.button, 1);
    app.handle_key(key(KeyCode::Up));
    assert_eq!(app.prompt.button, 0);
}

#[test]
fn enter_and_digits_press_buttons() {
    let mut app = app();
    show(
        &mut app,
        vec![prompt("p1", "pending", &["Allow once", "Reject"])],
    );
    app.handle_key(key(KeyCode::Down));
    let fx = app.handle_key(key(KeyCode::Enter));
    assert_eq!(
        fx,
        [Effect::Action {
            session: "s1".into(),
            item: "p1".into(),
            action: "neo.permit".into(),
            args: Some(json!({"id":"p1","option":"Reject"})),
        }]
    );
    assert_eq!(app.active_ref(), None, "answered: the highlight is gone");
    // A new prompt: digit 1 presses the first button.
    show(&mut app, vec![prompt("p2", "pending", &["Yes", "No"])]);
    let fx = app.handle_key(key(KeyCode::Char('1')));
    assert!(
        matches!(&fx[..], [Effect::Action { item, args: Some(a), .. }] if item == "p2" && a["option"] == "Yes")
    );
    // Out of range digits are nothing: the prompt has the keys, the editor is out of reach.
    show(&mut app, vec![prompt("p3", "pending", &["Yes", "No"])]);
    assert!(app.handle_key(key(KeyCode::Char('7'))).is_empty());
    assert_eq!(app.editor_text(), "");
}

// ---- a prompt takes the editor's place ----

/// The text the user has typed, then a prompt arrives `ms` after the last key.
fn typing_then_prompt(ms: i64) -> App {
    let mut app = app();
    type_str(&mut app, "draft");
    app.tick(NOW + ms);
    show(&mut app, vec![prompt("p1", "pending", &["Yes", "No"])]);
    app
}

#[test]
fn a_prompt_arriving_while_idle_takes_the_editors_place_at_once() {
    let mut app = app();
    show(&mut app, vec![prompt("p1", "pending", &["Yes", "No"])]);
    assert!(app.prompt.engaged);
    let text = screen_text(&mut app, 70, 14);
    snapshot("prompt-input-70x14", &text);
    assert!(
        !text.contains("message the agent"),
        "the editor is hidden: {text}"
    );
    assert!(
        text.contains("enter yes · ←/→ choose · tab add a note · esc dismiss"),
        "{text}"
    );
    // Typed characters go nowhere; the digits answer.
    assert!(app.handle_key(key(KeyCode::Char('x'))).is_empty());
    assert_eq!(app.editor_text(), "");
    let fx = app.handle_key(key(KeyCode::Char('2')));
    assert!(matches!(&fx[..], [Effect::Action { args: Some(a), .. }] if a["option"] == "No"));
    // Answered: the editor is back.
    assert!(!app.prompt.engaged);
    assert!(screen_text(&mut app, 70, 14).contains("message the agent"));
}

#[test]
fn a_prompt_right_after_sending_takes_the_editors_place_at_once() {
    let mut app = app();
    type_str(&mut app, "/model");
    app.handle_key(key(KeyCode::Enter));
    app.tick(NOW + 100);
    show(&mut app, vec![prompt("p1", "pending", &["Yes", "No"])]);
    assert!(app.prompt.engaged, "sending ended the typing");
}

#[test]
fn a_prompt_waits_while_the_user_types_and_takes_over_after_a_pause() {
    let mut app = typing_then_prompt(100);
    assert!(!app.prompt.engaged);
    let text = screen_text(&mut app, 70, 14);
    assert!(
        text.contains("permission needed — waiting until you stop typing"),
        "{text}"
    );
    assert!(
        text.contains("┃ draft"),
        "the editor stays meanwhile: {text}"
    );
    snapshot("prompt-waits-70x14", &text);
    // Keys still belong to the editor, digits included.
    app.handle_key(key(KeyCode::Char('2')));
    assert_eq!(app.editor_text(), "draft2");
    // Typing again restarts the pause.
    app.tick(NOW + 700);
    app.handle_key(key(KeyCode::Char('!')));
    app.tick(NOW + 1_400);
    assert!(
        !app.prompt.engaged,
        "700 ms after the last key is not enough"
    );
    app.tick(NOW + 1_500);
    assert!(app.prompt.engaged);
    assert!(app.take_dirty(), "the swap redraws");
    let text = screen_text(&mut app, 70, 14);
    assert!(!text.contains("draft"), "{text}");
    assert!(!text.contains("permission needed"), "{text}");
    // The draft is untouched behind it and comes back once the prompt is gone.
    assert_eq!(app.editor_text(), "draft2!");
    app.handle_key(key(KeyCode::Char('1')));
    assert!(screen_text(&mut app, 70, 14).contains("┃ draft2!"));
}

#[test]
fn a_prompt_after_a_pause_in_typing_takes_over_without_waiting() {
    let app = typing_then_prompt(800);
    assert!(app.prompt.engaged);
}

#[test]
fn enter_answers_the_prompt_never_the_draft_and_esc_gives_the_editor_back() {
    let mut app = typing_then_prompt(1_000);
    app.tick(NOW + 1_000);
    assert!(app.prompt.engaged);
    type_str(&mut app, "more");
    app.handle_key(key(KeyCode::Right));
    let fx = app.handle_key(key(KeyCode::Enter));
    assert!(matches!(&fx[..], [Effect::Action { args: Some(a), .. }] if a["option"] == "No"));
    assert_eq!(app.editor_text(), "draft", "the draft came back as it was");
    // Esc dismisses instead of answering; the draft is intact and Enter sends it.
    let mut app = typing_then_prompt(1_000);
    assert!(app.handle_key(key(KeyCode::Esc)).is_empty());
    assert!(!app.prompt.engaged);
    assert_eq!(app.editor_text(), "draft");
    assert_eq!(app.handle_key(key(KeyCode::Enter)), [input("draft")]);
    // Ctrl-P brings the dismissed prompt back.
    show(&mut app, vec![prompt("p1", "pending", &["Yes", "No"])]);
    app.handle_key(ctrl('p'));
    assert!(app.prompt.engaged);
}

#[test]
fn a_prompt_leaves_the_draft_alone_for_ctrl_c_and_paste() {
    let mut app = typing_then_prompt(1_000);
    app.handle_paste("pasted");
    assert_eq!(app.editor_text(), "draft");
    app.handle_key(ctrl('c'));
    assert_eq!(
        app.editor_text(),
        "draft",
        "ctrl-c arms quit, it doesn't clear the draft"
    );
    assert!(
        app.status_lines(80)[1]
            .to_string()
            .contains("press ctrl-c again to quit")
    );
}

#[test]
fn several_prompts_come_one_at_a_time() {
    let mut app = app();
    show(
        &mut app,
        vec![
            prompt("p1", "pending", &["Yes", "No"]),
            prompt("p2", "pending", &["Yes", "No"]),
            prompt("p3", "pending", &["Yes", "No"]),
        ],
    );
    let text = screen_text(&mut app, 70, 16);
    assert!(text.contains("(1 of 3)"), "{text}");
    let fx = app.handle_key(key(KeyCode::Char('1')));
    assert!(matches!(&fx[..], [Effect::Action { item, .. }] if item == "p1"));
    // The next one is already there: the editor does not flash back in between.
    assert!(app.prompt.engaged);
    assert_eq!(app.active_ref(), Some("p2"));
    let text = screen_text(&mut app, 70, 16);
    assert!(text.contains("(1 of 2)"), "{text}");
    app.handle_key(key(KeyCode::Char('1')));
    assert_eq!(app.active_ref(), Some("p3"));
    assert!(
        !screen_text(&mut app, 70, 16).contains("of"),
        "the last one has no count"
    );
    app.handle_key(key(KeyCode::Char('2')));
    assert!(!app.prompt.engaged);
}

#[test]
fn a_long_diff_stays_compact_in_the_input_area() {
    let mut app = app();
    show(&mut app, vec![permission_with_diff(60)]);
    let lines = screen(&mut app, 70, 30);
    let text = lines.join("\n");
    // The chat shows the item; the input area shows it again, cut short, with the buttons and
    // the keys under it.
    assert!(text.contains("[1 Allow once]"), "{text}");
    assert!(
        text.contains("enter allow once · ←/→ choose · tab add a note · esc dismiss"),
        "{text}"
    );
    let panel_rows = lines
        .iter()
        .rev()
        .take(12)
        .filter(|l| l.starts_with("▎"))
        .count();
    assert!(panel_rows <= 16, "{text}");
    // A tiny terminal still shows the question, the buttons and the keys.
    let tiny = screen(&mut app, 50, 9).join("\n");
    assert!(tiny.contains("[1 Allow once]"), "{tiny}");
}

#[test]
fn esc_dismisses_the_highlight_then_cancels() {
    let mut app = app();
    app.handle_event(user_message(NOW));
    show(&mut app, vec![prompt("p1", "pending", &["Yes", "No"])]);
    assert!(app.active_ref().is_some());
    assert!(app.handle_key(key(KeyCode::Esc)).is_empty());
    assert_eq!(app.active_ref(), None);
    assert_eq!(
        app.handle_key(key(KeyCode::Esc)),
        [Effect::Cancel {
            session: "s1".into()
        }]
    );
    // Digits are text again.
    app.handle_key(key(KeyCode::Char('1')));
    assert_eq!(app.editor_text(), "1");
}

#[test]
fn copy_buttons_copy() {
    let mut app = app();
    let copy: ViewItem = serde_json::from_value(json!({
        "ref":"c","kind":"tool","state":"pending",
        "component":{"type":"button","label":"Copy handle","action":"copy:asset://img-1"}
    }))
    .unwrap();
    show(&mut app, vec![copy]);
    assert_eq!(
        app.handle_key(key(KeyCode::Enter)),
        [Effect::Copy("asset://img-1".into())]
    );
}

#[test]
fn ctrl_p_picks_an_older_prompt_and_ctrl_n_returns() {
    let mut app = app();
    show(
        &mut app,
        vec![
            prompt("old", "done", &["Allow", "Reject"]),
            item("a", "agent", "text"),
            prompt("new", "done", &["A", "B"]),
        ],
    );
    assert_eq!(app.active_ref(), None, "nothing pending");
    app.handle_key(ctrl('p'));
    assert_eq!(app.active_ref(), Some("new"));
    app.handle_key(ctrl('p'));
    assert_eq!(app.active_ref(), Some("old"));
    let fx = app.handle_key(key(KeyCode::Char('2')));
    assert!(matches!(&fx[..], [Effect::Action { item, .. }] if item == "old"));
    app.handle_key(ctrl('p'));
    app.handle_key(ctrl('n'));
    app.handle_key(ctrl('n'));
    assert_eq!(app.active_ref(), None);
}

#[test]
fn a_new_pending_prompt_scrolls_into_view() {
    let mut app = app();
    long_chat(&mut app, 40);
    app.handle_key(key(KeyCode::PageUp));
    assert!(!app.chat().following());
    patch(&mut app, prompt("p9", "pending", &["Yes", "No"]));
    assert!(app.chat().following());
    assert!(screen_text(&mut app, 80, 24).contains("Yes"));
}

// ---- events ----

#[test]
fn turn_state_follows_user_message_and_turn_ended() {
    let mut app = app();
    app.handle_event(user_message(NOW - 12_000));
    assert!(app.running());
    assert!(app.status_lines(80)[1].to_string().contains("working 12s"));
    app.handle_event(turn_ended());
    assert!(!app.running());
    assert!(!app.status_lines(80)[1].to_string().contains("working"));
}

#[test]
fn events_of_other_sessions_are_ignored() {
    let mut app = app();
    app.handle_event(record(
        "other",
        1,
        NOW,
        SessionEvent::UserMessage(UserMessage {
            app: "x".into(),
            text: "t".into(),
            shown: None,
            images: None,
        }),
    ));
    assert!(!app.running());
    app.handle_event(Event::ViewSnapshot(ViewSnapshot {
        agent: None,
        session: "other".into(),
        seq: 1,
        items: vec![item("x", "agent", "no")],
    }));
    assert!(app.chat().items().is_empty());
}

#[test]
fn settings_and_approvals_come_from_setting_changed() {
    let mut app = app();
    app.handle_event(setting("neo", SettingName::Model, "sonnet"));
    app.handle_event(setting("claude", SettingName::Effort, "high"));
    app.handle_event(setting("daemon", SettingName::Approvals, "auto"));
    assert_eq!(app.approvals(), Approvals::Auto);
    let line = app.status_lines(100)[0].to_string();
    assert!(
        line.ends_with("neo · model sonnet"),
        "only the harness's own: {line}"
    );
    assert!(!line.contains("daemon") && !line.contains("auto"));
    assert!(
        app.status_lines(100)[1]
            .to_string()
            .starts_with("⏵⏵ auto mode on")
    );
    app.handle_event(setting("daemon", SettingName::Approvals, "edits"));
    assert_eq!(app.approvals(), Approvals::Edits);
    assert!(
        app.status_lines(100)[1]
            .to_string()
            .starts_with("⏵⏵ accept edits on")
    );
}

#[test]
fn the_harness_comes_from_its_setting_and_defaults_to_pi() {
    let mut app = app();
    assert_eq!(app.harness(), "neo");
    assert!(app.status_lines(100)[0].to_string().ends_with("· neo"));
    app.handle_event(setting("claude", SettingName::Model, "opus"));
    app.on_setting("daemon", "harness", "claude");
    assert_eq!(app.harness(), "claude");
    assert!(
        app.status_lines(100)[0]
            .to_string()
            .ends_with("· claude · model opus")
    );
    app.begin_session("s2", "/w");
    assert_eq!(app.harness(), "neo", "a new session starts over");
}

#[test]
fn shift_tab_cycles_approvals_ask_review_edits_auto() {
    let mut app = app();
    assert_eq!(app.approvals(), Approvals::Ask);
    for (now, next) in [
        ("ask", "review"),
        ("review", "edits"),
        ("edits", "auto"),
        ("auto", "ask"),
    ] {
        app.handle_event(setting("daemon", SettingName::Approvals, now));
        let fx = app.handle_key(with(KeyCode::BackTab, KeyModifiers::SHIFT));
        assert_eq!(fx, [input(&format!("/approvals {next}"))]);
        app.handle_event(setting("daemon", SettingName::Approvals, next));
    }
    assert_eq!(app.approvals(), Approvals::Ask);
    app.handle_event(setting("daemon", SettingName::Approvals, "review"));
    assert_eq!(app.approvals(), Approvals::Review);
    let line = app.status_lines(100)[1].to_string();
    assert!(
        line.starts_with("⏵ review each edit (shift+tab to cycle)"),
        "{line}"
    );
}

#[test]
fn the_agent_elsewhere_shows_in_the_status_line() {
    let mut app = app();
    app.handle_event(focus("s1", Some("photo-editor")));
    assert!(
        app.status_lines(100)[0]
            .to_string()
            .contains("in photo-editor")
    );
    app.handle_event(focus("s1", Some("terminal")));
    assert!(!app.status_lines(100)[0].to_string().contains(" in "));
}

#[test]
fn connection_loss_is_shown() {
    let mut app = app();
    app.handle_event(Event::Connection(false));
    assert!(
        app.status_lines(100)[1]
            .to_string()
            .contains("disconnected — reconnecting")
    );
    app.handle_event(Event::Connection(true));
    assert!(
        !app.status_lines(100)[1]
            .to_string()
            .contains("disconnected")
    );
}

#[test]
fn queued_inputs_are_counted_while_a_turn_runs() {
    let mut app = app();
    app.handle_event(user_message(NOW));
    type_str(&mut app, "later");
    app.handle_key(key(KeyCode::Enter));
    assert!(app.status_lines(100)[1].to_string().contains("1 queued"));
    let fx = app.handle_msg(Msg::Sessions(Ok(vec![SessionInfo {
        harness: None,
        parent: None,
        id: "s1".into(),
        title: None,
        cwd: "/home/me/site".into(),
        created_at: 0,
        last_activity: 0,
        last_seq: 1,
        state: SessionState::Running,
        queued: 3,
        waiting: Some(
            ["later", "and then\nmore", "last"]
                .map(|text| WaitingInput {
                    app: "terminal".into(),
                    text: text.into(),
                    images: None,
                })
                .to_vec(),
        ),
        preview: None,
        added: None,
        removed: None,
    }])));
    assert!(fx.is_empty());
    assert!(app.status_lines(100)[1].to_string().contains("3 queued"));
    // While running the app asks for fresh session info now and then.
    assert!(app.tick(NOW + 500).is_empty());
    assert_eq!(app.tick(NOW + 2500), [Effect::Refresh]);
}

#[test]
fn ctrl_enter_cancels_the_running_turn_and_sends_now() {
    let mut app = app();
    app.handle_event(user_message(NOW));
    type_str(&mut app, "stop, do this");
    assert_eq!(
        app.handle_key(with(KeyCode::Enter, KeyModifiers::CONTROL)),
        [
            Effect::Cancel {
                session: "s1".into()
            },
            input("stop, do this")
        ]
    );
    // Idle, it is just Enter; on an empty editor nothing is cancelled.
    app.handle_event(turn_ended());
    type_str(&mut app, "next");
    assert_eq!(
        app.handle_key(with(KeyCode::Enter, KeyModifiers::CONTROL)),
        [input("next")]
    );
    app.handle_event(user_message(NOW + 1));
    assert!(
        app.handle_key(with(KeyCode::Enter, KeyModifiers::CONTROL))
            .is_empty()
    );
}

#[test]
fn up_takes_the_queued_messages_back_into_the_editor() {
    let mut app = app();
    app.handle_event(user_message(NOW));
    for text in ["first", "second"] {
        app.handle_paste(text);
        app.handle_key(key(KeyCode::Enter));
    }
    assert_eq!(
        app.handle_key(key(KeyCode::Up)),
        [Effect::Unqueue {
            session: "s1".into()
        }]
    );
    type_str(&mut app, "typed meanwhile");
    app.handle_msg(Msg::Unqueued {
        session: "s1".into(),
        inputs: ["first", "second [image 1]"]
            .map(|text| WaitingInput {
                app: "terminal".into(),
                text: text.into(),
                images: text
                    .contains("[image")
                    .then(|| vec!["/tmp/a.png".to_string()]),
            })
            .to_vec(),
    });
    assert_eq!(
        app.editor_text(),
        "first\n\nsecond [image 1]\n\ntyped meanwhile"
    );
    assert!(!app.status_lines(100)[1].to_string().contains("queued"));
    let fx = app.handle_key(key(KeyCode::Enter));
    assert!(matches!(&fx[..], [Effect::Input { images, .. }] if images == &["/tmp/a.png"]));

    // Sent again while the turn runs, it waits again and Up takes it back again.
    assert!(matches!(
        &app.handle_key(key(KeyCode::Up))[..],
        [Effect::Unqueue { .. }]
    ));
}

#[test]
fn waiting_inputs_show_above_the_editor_until_the_turn_takes_them() {
    let mut app = app();
    app.handle_event(user_message(NOW));
    for text in ["first", "second\nline", "third", "fourth"] {
        app.handle_paste(text);
        app.handle_key(key(KeyCode::Enter));
    }
    let shown = screen(&mut app, 60, 20);
    // Not drawn like a sent message: their own marker, and a tag on the first.
    let first = shown.iter().find(|l| l.contains("◷ first")).expect("first");
    assert!(first.ends_with("queued · ↑ edits"), "{first}");
    let shown = shown.join("\n");
    assert!(shown.contains("◷ second …"), "{shown}");
    assert!(shown.contains("… +2 more queued"), "{shown}");
    assert!(!shown.contains("◷ third"), "{shown}");

    // The running turn took the first in: it is the chat's now.
    app.handle_event(record(
        "s1",
        9,
        NOW + 1,
        SessionEvent::UserMessage(UserMessage {
            app: "terminal".into(),
            text: "first".into(),
            shown: None,
            images: None,
        }),
    ));
    let shown = screen(&mut app, 60, 20).join("\n");
    assert!(!shown.contains("◷ first"), "{shown}");
    assert!(shown.contains("◷ third"), "{shown}");
    assert!(app.status_lines(100)[1].to_string().contains("3 queued"));
}

// ---- following the session ----

#[test]
fn clear_starts_a_fresh_view_when_the_new_session_becomes_current() {
    let mut app = app();
    app.widgets.reset("/home/me/site".into());
    show(&mut app, vec![item("a", "agent", "old chat")]);
    type_str(&mut app, "/clear");
    assert_eq!(app.handle_key(key(KeyCode::Enter)), [input("/clear")]);
    let fx = app.handle_event(focus("s2", Some("terminal")));
    assert_eq!(
        fx,
        [Effect::Switch {
            from: Some("s1".into()),
            to: "s2".into(),
            cwd: "/home/me/site".into(),
            move_agent: false
        }]
    );
    assert_eq!(app.session(), "s2");
    assert!(app.chat().items().is_empty());
    // Old events are ignored, new ones shown.
    show(&mut app, vec![]);
    app.handle_event(Event::ViewSnapshot(ViewSnapshot {
        agent: None,
        session: "s2".into(),
        seq: 1,
        items: vec![item("n", "notice", "fresh")],
    }));
    assert_eq!(app.chat().items().len(), 1);
}

#[test]
fn focus_in_another_app_or_on_our_session_changes_nothing() {
    let mut app = app();
    assert!(
        app.handle_event(focus("s2", Some("photo-editor")))
            .is_empty()
    );
    assert_eq!(app.session(), "s1");
    assert!(app.handle_event(focus("s1", Some("terminal"))).is_empty());
}

#[test]
fn focus_events_from_before_our_own_move_are_ignored() {
    let mut app = app();
    app.awaiting = Some(("s1".into(), NOW + 3000));
    assert!(app.handle_event(focus("s0", Some("terminal"))).is_empty());
    assert_eq!(app.session(), "s1");
    assert!(app.handle_event(focus("s1", Some("terminal"))).is_empty());
    assert!(app.awaiting.is_none());
}

/// `/sessions` and Enter: the switcher opens.
fn sessions_command(app: &mut App) -> Vec<Effect> {
    type_str(app, "/sessions");
    app.handle_key(key(KeyCode::Enter))
}

#[test]
fn ctrl_s_puts_the_draft_aside_until_the_next_one_is_sent() {
    let mut app = app();
    app.attach(vec!["/tmp/a.png".into()]);
    type_str(&mut app, "first");
    app.handle_key(ctrl('s'));
    assert_eq!(app.editor.text(), "");
    assert!(app.attachments.is_empty());
    assert!(screen_text(&mut app, 80, 20).contains("draft put aside (ctrl-s)"));

    // Ctrl-S brings it back, and puts what was written meanwhile aside in its place.
    type_str(&mut app, "second");
    app.handle_key(ctrl('s'));
    assert_eq!(app.editor.text(), "[image 1] first");
    assert_eq!(app.attachments, [(1, "/tmp/a.png".to_string())]);
    app.handle_key(ctrl('s'));
    assert_eq!(app.editor.text(), "second");

    // Sending the other one brings it back too.
    let fx = app.handle_key(key(KeyCode::Enter));
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::Input { text, .. } if text == "second"))
    );
    assert_eq!(app.editor.text(), "[image 1] first");
    assert_eq!(app.attachments, [(1, "/tmp/a.png".to_string())]);
    assert!(app.stash.is_none());
}

#[test]
fn the_switcher_lists_filters_and_switches() {
    let mut app = app();
    assert!(sessions_command(&mut app).contains(&Effect::ListSessions));
    let text = screen_text(&mut app, 80, 20);
    assert!(text.contains("loading…"), "{text}");
    let info = |id: &str, title: &str, cwd: &str, last: i64| SessionInfo {
        harness: None,
        parent: None,
        id: id.into(),
        title: Some(title.into()),
        cwd: cwd.into(),
        created_at: 0,
        last_activity: last,
        last_seq: 3,
        state: SessionState::Idle,
        queued: 0,
        waiting: None,
        preview: None,
        added: None,
        removed: None,
    };
    app.handle_msg(Msg::Sessions(Ok(vec![
        info("s1", "this one", "/home/me/site", NOW - 60_000),
        info("s7", "photo logo", "/home/me/logos", NOW - 3_600_000),
    ])));
    let text = screen_text(&mut app, 80, 20);
    assert!(text.contains("● this one"), "{text}");
    assert!(text.contains("photo logo  ~/logos  1h ago"), "{text}");
    type_str(&mut app, "logo");
    assert_eq!(app.switcher.entries().len(), 2);
    let fx = app.handle_key(key(KeyCode::Enter));
    assert_eq!(
        fx,
        [Effect::Switch {
            from: Some("s1".into()),
            to: "s7".into(),
            cwd: "/home/me/logos".into(),
            move_agent: true
        }]
    );
    assert_eq!(app.session(), "s7");
    assert_eq!(app.cwd(), "/home/me/logos");
    assert_eq!(app.overlay, Overlay::None);
}

#[test]
fn the_switcher_can_start_a_new_session_and_follow_it() {
    let mut app = app();
    sessions_command(&mut app);
    app.handle_msg(Msg::Sessions(Ok(vec![])));
    assert_eq!(
        app.handle_key(key(KeyCode::Enter)),
        [Effect::CreateSession {
            cwd: "/home/me/site".into()
        }]
    );
    let fx = app.handle_msg(Msg::Created(Ok(Box::new(SessionInfo {
        harness: None,
        parent: None,
        id: "s9".into(),
        title: None,
        cwd: "/home/me/site".into(),
        created_at: 0,
        last_activity: 0,
        last_seq: 0,
        state: SessionState::Idle,
        queued: 0,
        waiting: None,
        preview: None,
        added: None,
        removed: None,
    }))));
    assert!(matches!(&fx[..], [Effect::Switch { to, move_agent: true, .. }] if to == "s9"));
    assert_eq!(app.session(), "s9");
}

#[test]
fn errors_from_the_worker_show_for_a_while() {
    let mut app = app();
    app.handle_msg(Msg::Error("send failed: connection to daemon lost".into()));
    let line = app.status_lines(100)[1].to_string();
    assert!(line.contains("send failed"), "{line}");
    app.tick(NOW + 10_000);
    assert!(!app.status_lines(100)[1].to_string().contains("send failed"));
}

// ---- panels ----

#[test]
fn tab_does_nothing_without_a_sidebar_and_completes_commands() {
    let mut app = app();
    screen(&mut app, 100, 30);
    assert!(app.handle_key(key(KeyCode::Tab)).is_empty());
    assert_eq!(app.focus, None);
    type_str(&mut app, "/cl");
    app.handle_key(key(KeyCode::Tab));
    assert_eq!(app.editor_text(), "/clear ");
}

// ---- permission previews and review cards ----

/// A permission request as the daemon's session view builds it: the diff it would apply above
/// one button per option.
fn permission_with_diff(changed: usize) -> ViewItem {
    let mut lines = vec![json!({"kind":"hunk","text":"@@ -1,40 +1,40 @@"})];
    for i in 0..changed {
        lines.push(json!({"kind":"remove","text":format!("const old{i} = {i};")}));
        lines.push(json!({"kind":"add","text":format!("const next{i} = {i};")}));
    }
    let options = [
        ("allow", "Allow once"),
        ("always", "Always allow"),
        ("reject", "Reject"),
    ];
    let buttons: Vec<_> = options
        .iter()
        .enumerate()
        .map(|(i, (id, name))| {
            json!({"type":"button","label":name,"action":"neo.permit",
                   "args":{"id":"7","option":id},"key":(i + 1).to_string()})
        })
        .collect();
    serde_json::from_value(json!({
        "ref": "p-7", "kind": "tool", "label": "Edit src/app.ts", "state": "pending",
        "component": {"type":"stack","gap":"s","children":[
            {"type":"diff","path":"src/app.ts","lines":lines},
            {"type":"row","gap":"s","children":buttons}]}
    }))
    .unwrap()
}

#[test]
fn a_permission_previews_its_diff_and_the_buttons_answer_from_the_keyboard() {
    let mut app = app();
    show(&mut app, vec![permission_with_diff(2)]);
    let text = screen_text(&mut app, 70, 18);
    snapshot("permission-diff-70x18", &text);
    assert_eq!(app.active_ref(), Some("p-7"));
    // Right moves over the buttons, a digit or Enter presses.
    app.handle_key(key(KeyCode::Right));
    let fx = app.handle_key(key(KeyCode::Enter));
    assert!(
        matches!(&fx[..], [Effect::Action { item, action, args: Some(a), .. }]
            if item == "p-7" && action == "neo.permit" && a["option"] == "always"),
        "{fx:?}"
    );
    let mut app = self::app();
    show(&mut app, vec![permission_with_diff(2)]);
    let fx = app.handle_key(key(KeyCode::Char('3')));
    assert!(matches!(&fx[..], [Effect::Action { args: Some(a), .. }] if a["option"] == "reject"));
}

/// The per-edit card of review mode, as the daemon's session view builds it: the diff of an edit
/// that is already written above Keep and Revert.
fn edit_review_card() -> ViewItem {
    let lines = vec![
        json!({"kind":"hunk","text":"@@ -3,2 +3,2 @@"}),
        json!({"kind":"remove","text":"  return a - b;"}),
        json!({"kind":"add","text":"  return a + b;"}),
    ];
    let buttons = [("keep", "Keep"), ("revert", "Revert")]
        .iter()
        .enumerate()
        .map(|(i, (id, name))| {
            json!({"type":"button","label":name,"action":"neo.permit",
                   "args":{"id":"9","option":id},"key":(i + 1).to_string()})
        })
        .collect::<Vec<_>>();
    serde_json::from_value(json!({
        "ref": "p-9", "kind": "tool", "label": "Edited src/a.ts — keep it?",
        "state": "pending",
        "component": {"type":"stack","gap":"s","children":[
            {"type":"diff","path":"src/a.ts","lines":lines},
            {"type":"row","gap":"s","children":buttons}]}
    }))
    .unwrap()
}

#[test]
fn the_review_mode_edit_card_shows_its_diff_with_keep_and_revert() {
    let mut app = app();
    show(&mut app, vec![edit_review_card()]);
    let text = screen_text(&mut app, 70, 14);
    snapshot("edit-review-card-70x14", &text);
    assert!(text.contains("[1 Keep]  [2 Revert]"), "{text}");
    // Digits answer straight away; Revert is the second.
    let fx = app.handle_key(key(KeyCode::Char('2')));
    assert!(
        matches!(&fx[..], [Effect::Action { item, action, args: Some(a), .. }]
            if item == "p-9" && action == "neo.permit" && a["option"] == "revert"
                && a.get("note").is_none()),
        "{fx:?}"
    );
}

#[test]
fn tab_on_a_card_types_a_note_that_goes_with_the_focused_button() {
    let mut app = app();
    show(&mut app, vec![edit_review_card()]);
    // Move to Revert, then Tab: the buttons wait, a one-line input opens under the card.
    app.handle_key(key(KeyCode::Right));
    assert!(app.handle_key(key(KeyCode::Tab)).is_empty());
    let empty = screen_text(&mut app, 70, 14);
    assert!(empty.contains("note ›"), "{empty}");
    assert!(
        empty.contains("for the agent · enter sends · esc cancels"),
        "{empty}"
    );
    snapshot("edit-review-note-empty-70x14", &empty);
    // Digits, arrows and Tab are text or nothing now, not answers.
    type_str(&mut app, "1 wrong file, use b.ts");
    app.handle_key(key(KeyCode::Left));
    app.handle_key(key(KeyCode::Tab));
    let typed = screen_text(&mut app, 70, 14);
    snapshot("edit-review-note-typed-70x14", &typed);
    assert!(typed.contains("note › 1 wrong file, use b.ts"), "{typed}");
    // Backspace edits it; Enter answers with the focused button and the note.
    app.handle_key(key(KeyCode::Backspace));
    app.handle_key(key(KeyCode::Char('c')));
    let fx = app.handle_key(key(KeyCode::Enter));
    assert_eq!(
        fx,
        [Effect::Action {
            session: "s1".into(),
            item: "p-9".into(),
            action: "neo.permit".into(),
            args: Some(json!({"id":"9","option":"revert","note":"1 wrong file, use b.tc"})),
        }]
    );
    assert!(app.note.is_none());
    assert_eq!(app.active_ref(), None);
}

#[test]
fn esc_leaves_the_note_but_keeps_the_card_and_an_empty_note_is_no_note() {
    let mut app = app();
    show(&mut app, vec![permission_with_diff(1)]);
    app.handle_key(key(KeyCode::Tab));
    type_str(&mut app, "never mind");
    app.handle_key(key(KeyCode::Esc));
    assert!(app.note.is_none());
    assert_eq!(app.active_ref(), Some("p-7"), "the card is still waiting");
    assert!(!screen_text(&mut app, 70, 18).contains("note ›"));
    // The digits answer again.
    app.handle_key(key(KeyCode::Tab));
    type_str(&mut app, "   ");
    let fx = app.handle_key(key(KeyCode::Enter));
    assert!(
        matches!(&fx[..], [Effect::Action { args: Some(a), .. }]
            if a["option"] == "allow" && a.get("note").is_none()),
        "{fx:?}"
    );
    // Pasting goes into the note, on one line.
    let mut app = self::app();
    show(&mut app, vec![permission_with_diff(1)]);
    app.handle_key(key(KeyCode::Tab));
    app.handle_paste("use\ntabs");
    let fx = app.handle_key(key(KeyCode::Enter));
    assert!(
        matches!(&fx[..], [Effect::Action { args: Some(a), .. }] if a["note"] == "usetabs"),
        "{fx:?}"
    );
}

#[test]
fn a_note_is_dropped_when_the_card_is_answered_elsewhere() {
    let mut app = app();
    show(&mut app, vec![edit_review_card()]);
    app.handle_key(key(KeyCode::Tab));
    type_str(&mut app, "hm");
    let mut answered = edit_review_card();
    answered.state = Some(neo_protocol::ItemState::Done);
    answered.component = serde_json::from_value(json!({
        "type":"text","blocks":[{"type":"paragraph","spans":[{"text":"Keep"}]}]
    }))
    .unwrap();
    patch(&mut app, answered);
    assert!(app.note.is_none());
    type_str(&mut app, "x");
    assert_eq!(app.editor_text(), "x", "typing goes to the editor again");
}

#[test]
fn tab_still_moves_between_the_buttons_of_other_prompts() {
    let mut app = app();
    let other: ViewItem = serde_json::from_value(json!({
        "ref": "o", "kind": "tool", "label": "photo", "state": "pending",
        "component": {"type":"row","children":[
            {"type":"button","label":"Crop","action":"photo.crop"},
            {"type":"button","label":"Undo","action":"photo.undo"}]}
    }))
    .unwrap();
    show(&mut app, vec![other]);
    app.handle_key(key(KeyCode::Tab));
    assert!(app.note.is_none());
    assert_eq!(app.prompt.button, 1);
}

#[test]
fn a_long_diff_keeps_the_buttons_in_reach_until_expanded() {
    let mut app = app();
    show(&mut app, vec![permission_with_diff(60)]);
    // The prompt scrolls into view: the buttons are on screen, the middle of the diff is not.
    let text = screen_text(&mut app, 70, 30);
    assert!(
        text.contains("[1 Allow once]  [2 Always allow]  [3 Reject]"),
        "{text}"
    );
    assert!(text.contains("more lines (expand to see all)"), "{text}");
    assert!(!text.contains("const next50"), "{text}");
    // Ctrl-O shows all of it; the buttons are still at the end, and the layout holds.
    app.handle_key(ctrl('o'));
    let lines = screen(&mut app, 70, 30);
    assert!(
        lines.iter().any(|l| l.contains("[1 Allow once]")),
        "{lines:#?}"
    );
    assert!(lines.iter().all(|l| l.chars().count() <= 70));
    assert!(app.chat().total_lines() > 100);
    // Narrow terminals wrap the diff and stack the buttons without breaking out of the pane.
    let narrow = screen(&mut app, 24, 30);
    assert!(
        narrow.iter().any(|l| l.contains("[3 Reject]")),
        "{narrow:#?}"
    );
}

#[test]
fn the_review_card_after_an_edits_turn_is_answered_with_keys() {
    let option = |i: usize, id: &str, name: &str| {
        json!({"type":"button","label":name,"action":"neo.choose",
               "args":{"id":"r1","option":id},"key":(i + 1).to_string()})
    };
    let card: ViewItem = serde_json::from_value(json!({
        "ref": "q-r1", "kind": "tool", "label": "The agent changed 2 files: keep them?",
        "state": "pending",
        "component": {"type":"stack","gap":"s","children":[
            option(0, "keep", "Keep"),
            option(1, "revert", "Revert all"),
            option(2, "revert:a.ts", "Revert a.ts"),
            option(3, "revert:b.ts", "Revert b.ts")]}
    }))
    .unwrap();
    let mut app = app();
    show(&mut app, vec![card]);
    let text = screen_text(&mut app, 60, 12);
    snapshot("review-card-60x12", &text);
    // Down walks the buttons (the first is focused), Enter presses.
    app.handle_key(key(KeyCode::Down));
    app.handle_key(key(KeyCode::Down));
    let fx = app.handle_key(key(KeyCode::Enter));
    assert_eq!(
        fx,
        [Effect::Action {
            session: "s1".into(),
            item: "q-r1".into(),
            action: "neo.choose".into(),
            args: Some(json!({"id":"r1","option":"revert:a.ts"})),
        }]
    );
}

// ---- sessions sidebar ----

fn info(id: &str, title: &str, last: i64) -> SessionInfo {
    SessionInfo {
        harness: None,
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
        preview: None,
        added: None,
        removed: None,
    }
}

fn overview() -> App {
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
fn the_sessions_sidebar_lists_every_session_newest_first() {
    let mut app = overview();
    let text = screen_text(&mut app, 120, 24);
    snapshot("sessions-120x24", &text);
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
    assert_eq!(
        titles(&mut app),
        ["Sticky header", "Fix the login form", "Docs"]
    );
    // Docs got a message: it shows what was said but keeps its place.
    let mut docs = info("s3", "Docs", NOW);
    docs.preview = Some("Rewrote the README".into());
    docs.added = Some(40);
    app.handle_event(Event::SessionChanged(docs));
    assert_eq!(
        titles(&mut app),
        ["Sticky header", "Fix the login form", "Docs"]
    );
    let text = screen_text(&mut app, 120, 24);
    assert!(text.contains("Rewrote the README"), "{text}");
    assert!(text.contains("+40"), "{text}");
    // A session nobody listed yet appears.
    app.handle_event(Event::SessionChanged(info("s9", "Brand new", NOW + 1)));
    assert_eq!(app.sessions().len(), 4);
    assert_eq!(titles(&mut app)[0], "Brand new");
    // One the daemon deleted goes again.
    app.handle_event(Event::SessionRemoved("s9".into()));
    assert_eq!(app.sessions().len(), 3);
    assert_eq!(titles(&mut app)[0], "Sticky header");
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
    // Starts on the shown session (s1, the first row); j goes to s2 and s3, k back to s2.
    app.handle_key(key(KeyCode::Char('j')));
    app.handle_key(key(KeyCode::Down));
    app.handle_key(key(KeyCode::Char('k')));
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
    for _ in 0..4 {
        app.handle_key(key(KeyCode::Tab));
        seen.push(app.focus);
    }
    assert_eq!(
        seen,
        [
            Some(Focus::Sessions),
            Some(Focus::Panel(Panel::Plan)),
            Some(Focus::Panel(Panel::Files)),
            None
        ]
    );
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

// ---- drawing ----

fn plan_event() -> Event {
    record(
        "s1",
        5,
        NOW,
        SessionEvent::Plan(neo_protocol::Plan {
            agent: None,
            entries: vec![
                neo_protocol::PlanEntry {
                    content: "crop the logo".into(),
                    status: neo_protocol::PlanEntryStatus::Completed,
                    priority: None,
                },
                neo_protocol::PlanEntry {
                    content: "wire it into the header".into(),
                    status: neo_protocol::PlanEntryStatus::InProgress,
                    priority: None,
                },
            ],
        }),
    )
}

#[test]
fn the_sidebar_appears_with_a_plan_and_tab_cycles_focus() {
    let mut app = scenario();
    let narrow = screen_text(&mut app, 60, 20);
    assert!(!narrow.contains("crop the logo"));
    app.handle_event(plan_event());
    let text = screen_text(&mut app, 100, 30);
    assert!(text.contains("crop the logo"), "{text}");
    snapshot("sidebar-100x30", &text);
    // The chat keeps the left part; the side column the right.
    assert!(app.sidebar_shown);
    app.handle_key(key(KeyCode::Tab));
    assert_eq!(app.focus, Some(Focus::Panel(Panel::Plan)));
    app.handle_key(key(KeyCode::Tab));
    assert_eq!(app.focus, Some(Focus::Panel(Panel::Files)));
    app.handle_key(key(KeyCode::Tab));
    assert_eq!(app.focus, None);
    // Typing is not possible while a panel has focus; Esc leaves it.
    app.handle_key(key(KeyCode::Tab));
    type_str(&mut app, "x");
    assert_eq!(app.editor_text(), "");
    app.handle_key(key(KeyCode::Esc));
    assert_eq!(app.focus, None);
    type_str(&mut app, "x");
    assert_eq!(app.editor_text(), "x");
    // A terminal too narrow for the column drops it and the focus with it.
    app.handle_key(key(KeyCode::Tab));
    screen(&mut app, 60, 20);
    assert_eq!(app.focus, None);
}

#[test]
fn layout_at_100x30() {
    let mut app = scenario();
    app.handle_event(setting("neo", SettingName::Model, "sonnet"));
    let text = screen_text(&mut app, 100, 30);
    snapshot("layout-100x30", &text);
}

#[test]
fn layout_at_60x20_while_working_with_a_menu_open() {
    let mut app = scenario();
    app.handle_event(user_message(NOW - 12_000));
    app.handle_event(setting("daemon", SettingName::Approvals, "auto"));
    app.tick(NOW);
    type_str(&mut app, "/c");
    let text = screen_text(&mut app, 60, 20);
    snapshot("menu-60x20", &text);
}

#[test]
fn layout_with_a_prompt_and_help() {
    let mut app = app();
    show(
        &mut app,
        vec![
            item("u", "user", "run the tests"),
            prompt("p1", "pending", &["Allow once", "Always allow", "Reject"]),
        ],
    );
    app.handle_key(key(KeyCode::Right));
    let text = screen_text(&mut app, 70, 14);
    snapshot("prompt-70x14", &text);
}

#[test]
fn help_lists_keys_and_commands() {
    let mut app = scenario();
    type_str(&mut app, "/help");
    app.handle_key(key(KeyCode::Enter));
    snapshot("help-100x48", &screen_text(&mut app, 100, 48));
    app.handle_key(key(KeyCode::Esc));
    app.handle_key(key(KeyCode::Char('?')));
    assert_eq!(app.overlay, Overlay::Help, "? on an empty editor");
}

#[test]
fn an_empty_chat_says_what_to_do() {
    let mut app = app();
    let text = screen_text(&mut app, 80, 12);
    assert!(text.contains("connecting to the session"));
    show(&mut app, vec![]);
    let text = screen_text(&mut app, 80, 12);
    assert!(text.contains("enter send  ·  / commands"), "{text}");
    assert!(text.contains("┃ message the agent · / for commands"));
}

#[test]
fn the_editor_grows_and_wraps_up_to_eight_rows() {
    let mut app = scenario();
    app.handle_paste(&"line\n".repeat(12));
    let rows = screen(&mut app, 60, 24);
    // The rows between the input box's top row and the status line in it.
    let top = rows.iter().position(|l| l == "┃").unwrap();
    let status = rows.iter().position(|l| l.starts_with("┃ site ·")).unwrap();
    let editor_rows = status - top - 1;
    assert_eq!(editor_rows, 8, "{rows:#?}");
    // The status lines stay at the bottom.
    assert!(rows[23].contains("shift+tab"), "{rows:#?}");
}

#[test]
fn nothing_panics_on_tiny_terminals() {
    for (w, h) in [(1, 1), (5, 3), (20, 4), (30, 6), (10, 40)] {
        let mut app = scenario();
        app.handle_paste("some text\nmore");
        type_str(&mut app, "/");
        screen(&mut app, w, h);
        app.handle_key(ctrl('s'));
        screen(&mut app, w, h);
        sessions_command(&mut app);
        screen(&mut app, w, h);
        app.handle_key(key(KeyCode::Esc));
        app.handle_key(ctrl('c'));
        app.handle_key(key(KeyCode::Char('?')));
        screen(&mut app, w, h);
        app.handle_key(key(KeyCode::Enter));
        screen(&mut app, w, h);
    }
}

#[test]
fn no_color_uses_no_colors() {
    let mut app = App::new(Config {
        app_id: "terminal".into(),
        session: "s1".into(),
        cwd: "/w".into(),
        home: None,
        no_color: true,
        history: HistoryStore::default(),
        prefs: crate::theme::Prefs::default(),
    });
    show(
        &mut app,
        vec![item("u", "user", "hello"), item("a", "agent", "world")],
    );
    let mut terminal = Terminal::new(TestBackend::new(60, 14)).unwrap();
    terminal.draw(|f| app.draw(f)).unwrap();
    let buffer = terminal.backend().buffer();
    for cell in buffer.content() {
        assert_eq!(cell.fg, ratatui::style::Color::Reset, "{cell:?}");
        assert_eq!(cell.bg, ratatui::style::Color::Reset);
    }
}

#[test]
fn theme_paints_the_whole_screen_and_switches() {
    use ratatui::style::Color;
    let backgrounds = |app: &mut App| {
        let mut terminal = Terminal::new(TestBackend::new(60, 14)).unwrap();
        terminal.draw(|f| app.draw(f)).unwrap();
        let buffer = terminal.backend().buffer();
        let mut seen: Vec<Color> = buffer.content().iter().map(|c| c.bg).collect();
        seen.dedup();
        (seen, buffer[(59, 0)].fg)
    };
    let mut app = scenario();
    // Dark by default: near-black where nothing else is painted, the theme's text color.
    let (seen, fg) = backgrounds(&mut app);
    assert!(seen.contains(&Color::Rgb(0x0f, 0x0e, 0x0c)));
    assert!(!seen.contains(&Color::Reset));
    assert_ne!(fg, Color::Reset);

    type_str(&mut app, "/theme light");
    app.handle_key(key(KeyCode::Enter));
    let (seen, _) = backgrounds(&mut app);
    assert!(seen.contains(&Color::Rgb(0xf2, 0xf1, 0xee)));
    assert!(!seen.contains(&Color::Reset));

    // No name: the next one, which leaves the terminal's own colors.
    type_str(&mut app, "/theme");
    app.handle_key(key(KeyCode::Enter));
    let (seen, _) = backgrounds(&mut app);
    assert!(seen.contains(&Color::Reset));

    type_str(&mut app, "/theme sepia");
    app.handle_key(key(KeyCode::Enter));
    assert!(screen_text(&mut app, 60, 14).contains("usage: /theme [dark|light|terminal]"));
}

#[test]
fn the_cursor_sits_in_the_editor() {
    let mut app = scenario();
    type_str(&mut app, "héllo");
    let mut terminal = Terminal::new(TestBackend::new(60, 20)).unwrap();
    terminal.draw(|f| app.draw(f)).unwrap();
    let pos = terminal.get_cursor_position().unwrap();
    assert_eq!((pos.x, pos.y), (2 + 5, 20 - 2 - 1));
}

#[test]
fn a_prompt_answered_below_the_chat_leaves_one_line_in_the_chat_and_starts_on_the_current_value() {
    let option = |i: usize, id: &str, name: &str, current: bool| {
        let mut b = json!({"type":"button","label":name,"action":"neo.choose",
               "args":{"id":"m1","option":id},"key":(i + 1).to_string()});
        if current {
            b["current"] = json!(true);
        }
        b
    };
    let picker: ViewItem = serde_json::from_value(json!({
        "ref": "q-m1", "kind": "tool", "label": "Claude Code model (now: haiku)",
        "state": "pending",
        "component": {"type":"stack","gap":"s","children":[
            option(0, "default", "Default", false),
            option(1, "opus", "Opus", false),
            option(2, "haiku", "Haiku", true)]}
    }))
    .unwrap();
    let mut app = app();
    show(&mut app, vec![picker]);
    assert!(app.prompt.engaged);
    let rows = screen(&mut app, 70, 20);
    let titles = rows
        .iter()
        .filter(|l| l.contains("Claude Code model"))
        .count();
    assert_eq!(
        titles, 2,
        "one line in the chat, the picker below: {rows:#?}"
    );
    assert!(
        rows.iter()
            .any(|l| l.contains("Claude Code model (now: haiku) · answer below")),
        "{rows:#?}"
    );
    assert_eq!(
        rows.iter().filter(|l| l.contains("[2 Opus]")).count(),
        1,
        "the buttons only once: {rows:#?}"
    );
    assert!(rows.iter().any(|l| l.contains("[3 Haiku] ✓")), "{rows:#?}");
    // Enter keeps what is in effect.
    let fx = app.handle_key(key(KeyCode::Enter));
    assert_eq!(
        fx,
        [Effect::Action {
            session: "s1".into(),
            item: "q-m1".into(),
            action: "neo.choose".into(),
            args: Some(json!({"id":"m1","option":"haiku"})),
        }]
    );
}

#[test]
fn a_long_choice_scrolls_to_the_focused_button() {
    let mut app = app();
    long_chat(&mut app, 10);
    let rows: Vec<_> = (0..120)
        .map(|i| {
            json!({"type":"button","label":format!("model-{i:03}"),"action":"neo.choose",
                   "args":{"id":"c","option":format!("m{i}")},"current": i == 0})
        })
        .collect();
    let item: ViewItem = serde_json::from_value(json!({
        "ref":"q-c","kind":"tool","label":"Model","state":"pending",
        "component":{"type":"stack","gap":"s","children":rows}}))
    .unwrap();
    patch(&mut app, item);
    // Opens on the current one, at the top of the list.
    let s = screen_text(&mut app, 80, 24);
    assert!(s.contains("model-000") && !s.contains("model-119"), "{s}");
    // Up wraps to the last one: the list follows to the bottom.
    app.handle_key(key(KeyCode::Up));
    let s = screen_text(&mut app, 80, 24);
    assert!(s.contains("model-119") && !s.contains("model-000"), "{s}");
    // Back down to the top; going down then scrolls only once the focus leaves the window.
    for _ in 0..2 {
        app.handle_key(key(KeyCode::Down));
        screen_text(&mut app, 80, 24);
    }
    let s = screen_text(&mut app, 80, 24);
    assert!(s.contains("model-000") && s.contains("model-001"), "{s}");
    for _ in 0..30 {
        app.handle_key(key(KeyCode::Down));
        screen_text(&mut app, 80, 24);
    }
    let s = screen_text(&mut app, 80, 24);
    assert!(
        s.contains("enter model-031") && s.contains("[model-031]"),
        "{s}"
    );
    assert!(!s.contains("model-000"), "{s}");
}

// ---- type to find an option of a long choice ----

/// `/model` with several providers: 12 options, the third and fourth with a description, the
/// first one in effect.
fn model_choice(app: &mut App) {
    long_chat(app, 3);
    let providers = ["claude", "openrouter", "openai"];
    let names = ["Haiku 4.5", "Sonnet 4.5", "GPT 5", "Gemini 2.5"];
    let rows: Vec<_> = (0..12)
        .map(|i| {
            let (provider, name) = (providers[i % 3], names[i % 4]);
            let button = json!({"type":"button","label":format!("{name} · {provider}"),
                "action":"neo.choose","args":{"id":"c","option":format!("{provider}/{i}/{name}")},
                "current": i == 0, "key": (i < 9).then(|| (i + 1).to_string())});
            if i == 3 || i == 4 {
                json!({"type":"row","children":[button,
                    {"type":"text","tone":"muted","blocks":[{"type":"paragraph",
                        "spans":[{"text": format!("reasons about code {i}")}]}]}]})
            } else {
                button
            }
        })
        .collect();
    let item: ViewItem = serde_json::from_value(json!({
        "ref":"q-c","kind":"tool","label":"Model","state":"pending",
        "component":{"type":"stack","gap":"s","children":rows}}))
    .unwrap();
    patch(app, item);
}

fn picked(fx: &[Effect]) -> String {
    match fx {
        [Effect::Action { args: Some(a), .. }] => a["option"].as_str().unwrap().to_string(),
        other => panic!("not one action: {other:?}"),
    }
}

#[test]
fn typing_filters_a_long_choice_and_enter_picks_the_match() {
    let mut app = app();
    model_choice(&mut app);
    let s = screen_text(&mut app, 80, 24);
    assert!(
        s.contains("filter › ") && s.contains("type to find an option"),
        "{s}"
    );
    assert!(s.contains("[Haiku 4.5 · claude] ✓"), "{s}");
    // Case does not matter, every word must occur, and the order stays.
    type_str(&mut app, "OPENROUTER sonnet");
    let s = screen_text(&mut app, 80, 24);
    assert!(s.contains("filter › OPENROUTER sonnet"), "{s}");
    assert!(s.contains("[Sonnet 4.5 · openrouter]"), "{s}");
    assert!(!s.contains("Haiku") && !s.contains("GPT"), "{s}");
    assert!(s.contains("enter sonnet 4.5 · openrouter"), "{s}");
    snapshot("choice-filter-70x14", &screen_text(&mut app, 70, 14));
    let fx = app.handle_key(key(KeyCode::Enter));
    assert_eq!(picked(&fx), "openrouter/1/Sonnet 4.5");
}

#[test]
fn the_filter_takes_digits_and_edits_with_backspace_and_ctrl_u() {
    let mut app = app();
    model_choice(&mut app);
    // 7 would press the seventh button on a short card; here it is text, found in the id.
    assert!(app.handle_key(key(KeyCode::Char('7'))).is_empty());
    assert!(app.active_ref().is_some());
    let s = screen_text(&mut app, 80, 24);
    assert!(
        s.contains("filter › 7") && s.contains("Gemini 2.5 · openrouter"),
        "{s}"
    );
    assert!(!s.contains("Haiku"), "{s}");
    app.handle_key(key(KeyCode::Backspace));
    let s = screen_text(&mut app, 80, 24);
    assert!(s.contains("type to find an option"), "{s}");
    type_str(&mut app, "gpt");
    app.handle_key(ctrl('u'));
    // Cleared: the focus is back on the option in effect.
    let s = screen_text(&mut app, 80, 24);
    assert!(s.contains("type to find an option"), "{s}");
    assert_eq!(
        picked(&app.handle_key(key(KeyCode::Enter))),
        "claude/0/Haiku 4.5"
    );
}

#[test]
fn a_description_is_searched_and_the_arrows_stay_among_the_matches() {
    let mut app = app();
    model_choice(&mut app);
    type_str(&mut app, "reasons");
    // Options 3 and 4 have the text; the first match has the focus.
    let s = screen_text(&mut app, 80, 24);
    assert!(
        s.contains("[Gemini 2.5 · claude]") && s.contains("reasons about code 3"),
        "{s}"
    );
    assert!(
        s.contains("[Haiku 4.5 · openrouter]") && !s.contains("Sonnet"),
        "{s}"
    );
    // Down, down (wraps), down: on the second match.
    for _ in 0..3 {
        app.handle_key(key(KeyCode::Down));
    }
    let fx = app.handle_key(key(KeyCode::Enter));
    assert_eq!(picked(&fx), "openrouter/4/Haiku 4.5");
}

#[test]
fn nothing_matching_says_so_and_enter_does_nothing() {
    let mut app = app();
    model_choice(&mut app);
    type_str(&mut app, "zzz");
    let s = screen_text(&mut app, 80, 24);
    assert!(s.contains("no match") && !s.contains("Haiku"), "{s}");
    assert!(!s.contains("enter "), "{s}");
    snapshot("choice-no-match-70x14", &screen_text(&mut app, 70, 14));
    assert!(app.handle_key(key(KeyCode::Enter)).is_empty());
    assert!(app.handle_key(key(KeyCode::Tab)).is_empty());
    assert!(app.note.is_none());
    assert!(app.active_ref().is_some());
    // Backspacing back to a match brings the list back.
    for _ in 0..3 {
        app.handle_key(key(KeyCode::Backspace));
    }
    assert_eq!(
        picked(&app.handle_key(key(KeyCode::Enter))),
        "claude/0/Haiku 4.5"
    );
}

#[test]
fn esc_clears_the_filter_first_and_then_dismisses() {
    let mut app = app();
    model_choice(&mut app);
    type_str(&mut app, "gpt");
    app.handle_key(key(KeyCode::Esc));
    assert!(app.active_ref().is_some(), "the first Esc only clears");
    let s = screen_text(&mut app, 80, 24);
    assert!(
        s.contains("type to find an option") && s.contains("Haiku"),
        "{s}"
    );
    app.handle_key(key(KeyCode::Esc));
    assert_eq!(app.active_ref(), None, "the second one dismisses");
}

#[test]
fn a_note_goes_with_the_focused_match() {
    let mut app = app();
    model_choice(&mut app);
    type_str(&mut app, "gemini");
    app.handle_key(key(KeyCode::Tab));
    type_str(&mut app, "thanks");
    let fx = app.handle_key(key(KeyCode::Enter));
    match &fx[..] {
        [Effect::Action { args: Some(a), .. }] => {
            assert_eq!(a["option"], "claude/3/Gemini 2.5");
            assert_eq!(a["note"], "thanks");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_choice_of_nine_or_a_permission_has_no_filter() {
    let mut app = app();
    show(
        &mut app,
        vec![prompt("p1", "pending", &["Allow once", "Reject"])],
    );
    let s = screen_text(&mut app, 80, 24);
    assert!(!s.contains("filter ›"), "{s}");
    app.handle_key(key(KeyCode::Char('x')));
    assert!(app.active_ref().is_some());
    let rows: Vec<_> = (0..9)
        .map(|i| {
            json!({"type":"button","label":format!("m{i}"),"action":"neo.choose",
                        "args":{"id":"c","option":format!("m{i}")}})
        })
        .collect();
    let item: ViewItem = serde_json::from_value(json!({
        "ref":"q-c","kind":"tool","label":"Model","state":"pending",
        "component":{"type":"stack","children":rows}}))
    .unwrap();
    show(&mut app, vec![item]);
    let s = screen_text(&mut app, 80, 24);
    assert!(!s.contains("filter ›") && s.contains("[9 m8]"), "{s}");
    assert_eq!(picked(&app.handle_key(key(KeyCode::Char('9')))), "m8");
}

#[test]
fn a_filtered_list_scrolls_to_the_focused_match() {
    let mut app = app();
    long_chat(&mut app, 10);
    let rows: Vec<_> = (0..120)
        .map(|i| {
            json!({"type":"button","label":format!("model-{i:03}"),"action":"neo.choose",
                   "args":{"id":"c","option":format!("m{i}")}})
        })
        .collect();
    let item: ViewItem = serde_json::from_value(json!({
        "ref":"q-c","kind":"tool","label":"Model","state":"pending",
        "component":{"type":"stack","gap":"s","children":rows}}))
    .unwrap();
    patch(&mut app, item);
    // 40 matches (those with a 1 in them: 1x, x1x ...) do not fit; the filter stays in view.
    type_str(&mut app, "model-01");
    let s = screen_text(&mut app, 80, 18);
    assert!(
        s.contains("filter › model-01") && s.contains("model-010"),
        "{s}"
    );
    assert!(!s.contains("model-000") && !s.contains("model-020"), "{s}");
    app.handle_key(key(KeyCode::Up));
    let s = screen_text(&mut app, 80, 18);
    assert!(
        s.contains("filter › model-01") && s.contains("model-019"),
        "{s}"
    );
    assert!(s.contains("enter model-019"), "{s}");
    assert_eq!(picked(&app.handle_key(key(KeyCode::Enter))), "m19");
}

#[test]
fn tab_reaches_the_files_in_the_chat_and_enter_opens_one() {
    let mut app = app();
    let edit: ViewItem = serde_json::from_value(json!({
        "ref": "e1", "kind": "tool", "label": "edit src/a.rs", "state": "done",
        "component": {"type":"diff","path":"src/a.rs","lines":[{"kind":"add","text":"x"}]}
    }))
    .unwrap();
    show(
        &mut app,
        vec![item("u1", "user", "go"), edit, item("a1", "agent", "Done.")],
    );
    let s = screen_text(&mut app, 80, 24);
    assert!(s.contains("1 file changed  +1 −0"), "{s}");
    app.handle_key(key(KeyCode::Tab));
    assert_eq!(app.focus, Some(Focus::Chat));
    assert_eq!(app.chat.picked_file(), Some("src/a.rs"));
    app.handle_key(key(KeyCode::Enter));
    assert!(app.widgets().overlay_open());
    // Ctrl-D reaches the diff, not the editor; `e` hands the file to the editor.
    assert!(app.handle_key(ctrl('d')).is_empty());
    assert!(app.widgets().overlay_open());
    assert_eq!(
        app.handle_key(key(KeyCode::Char('e'))),
        [Effect::Edit {
            path: "/home/me/site/src/a.rs".into(),
            line: None,
        }]
    );
    // The diff takes the keys until it closes; then the chat has them again.
    app.handle_key(key(KeyCode::Esc));
    assert!(!app.widgets().overlay_open());
    assert_eq!(app.focus, Some(Focus::Chat));
    app.handle_key(key(KeyCode::Esc));
    assert_eq!(app.focus, None);
}

/// Not a test: writes a frame of a busy session as HTML to `$PREVIEW_HTML` (in the theme
/// `$PREVIEW_THEME`, dark by default), to look at the design in a browser.
/// `PREVIEW_HTML=/tmp/term.html cargo test -p neo-term preview -- --ignored`
#[test]
#[ignore]
fn preview() {
    let Some(path) = std::env::var_os("PREVIEW_HTML") else {
        return;
    };
    let mut app = overview();
    if let Ok(theme) = std::env::var("PREVIEW_THEME") {
        type_str(&mut app, &format!("/theme {theme}"));
        app.handle_key(key(KeyCode::Enter));
    }
    app.handle_event(Event::Session(
        serde_json::from_value(json!({"session": "s1", "seq": 9, "ts": NOW, "event": {
        "type": "todos", "items": [
            {"id": "t1", "text": "Make the header sticky", "done": true, "by": "claude"},
            {"id": "t2", "text": "Check the login form on a phone", "done": false}
        ]}}))
        .unwrap(),
    ));
    type_str(&mut app, "and the footer too");
    let (w, h) = (140, 36);
    let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
    terminal.draw(|f| app.draw(f)).unwrap();
    let buf = terminal.backend().buffer();
    let css = |c: ratatui::style::Color, or: &str| match c {
        ratatui::style::Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        _ => or.to_string(),
    };
    let mut html = String::from(
        "<html><body style=\"margin:0;background:#000\"><pre style=\"margin:0;font:15px/1.3 'JetBrains Mono',monospace\">",
    );
    for y in 0..h {
        for x in 0..w {
            let cell = &buf[(x, y)];
            let m = cell.modifier;
            let (mut fg, mut bg) = (css(cell.fg, "#ccc"), css(cell.bg, "#000"));
            if m.contains(Modifier::REVERSED) {
                std::mem::swap(&mut fg, &mut bg);
            }
            let mut style = format!("color:{fg};background:{bg}");
            if m.contains(Modifier::BOLD) {
                style.push_str(";font-weight:bold");
            }
            if m.contains(Modifier::ITALIC) {
                style.push_str(";font-style:italic");
            }
            if m.contains(Modifier::DIM) {
                style.push_str(";opacity:.6");
            }
            if m.contains(Modifier::CROSSED_OUT) {
                style.push_str(";text-decoration:line-through");
            }
            let sym = match cell.symbol() {
                "<" => "&lt;",
                ">" => "&gt;",
                "&" => "&amp;",
                s => s,
            };
            html.push_str(&format!("<span style=\"{style}\">{sym}</span>"));
        }
        html.push('\n');
    }
    html.push_str("</pre></body></html>");
    std::fs::write(path, html).unwrap();
}
