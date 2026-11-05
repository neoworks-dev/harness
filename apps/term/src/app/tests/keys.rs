use super::*;

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

pub(super) fn long_chat(app: &mut App, n: usize) {
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
fn dragging_over_the_chat_copies_what_it_covers() {
    let mut app = scenario();
    let lines = screen(&mut app, 100, 30);
    let y = lines.iter().position(|l| l.contains("Done.")).unwrap();
    let x = lines[y].find("Done.").unwrap();
    let at = |kind, x: usize| MouseEvent {
        kind,
        column: x as u16,
        row: y as u16,
        modifiers: KeyModifiers::NONE,
    };
    let left = ratatui::crossterm::event::MouseButton::Left;
    assert!(
        app.handle_mouse(at(MouseEventKind::Down(left), x))
            .is_empty()
    );
    app.handle_mouse(at(MouseEventKind::Drag(left), x + 4));
    let shown = screen(&mut app, 100, 30);
    assert_eq!(shown[y], lines[y], "the text stays as it is, only reversed");
    let fx = app.handle_mouse(at(MouseEventKind::Up(left), x + 4));
    assert_eq!(fx, [Effect::Copy("Done.".into())]);
    // A click copies nothing.
    app.handle_mouse(at(MouseEventKind::Down(left), x));
    assert!(app.handle_mouse(at(MouseEventKind::Up(left), x)).is_empty());
}

#[test]
fn tab_picks_a_reply_y_copies_its_markdown_and_enter_opens_it_in_vim_keys() {
    let mut app = scenario();
    screen(&mut app, 100, 30);
    app.handle_key(key(KeyCode::Tab));
    assert_eq!(app.focus, Some(Focus::Chat));
    let markdown = "Done. The header now uses `position: sticky`.";
    assert_eq!(
        app.handle_key(key(KeyCode::Char('y'))),
        [Effect::Copy(markdown.into())]
    );
    app.handle_key(key(KeyCode::Enter));
    let text = screen_text(&mut app, 100, 30);
    assert!(text.contains("reply  markdown"), "{text}");
    assert!(text.contains(markdown), "{text}");
    // Done . The header: the full stop is a word of its own, as in vim.
    type_str(&mut app, "wwwve");
    assert_eq!(
        app.handle_key(key(KeyCode::Char('y'))),
        [Effect::Copy("header".into())]
    );
    app.handle_key(key(KeyCode::Char('q')));
    assert!(!screen_text(&mut app, 100, 30).contains("reply  markdown"));
    assert_eq!(app.focus, Some(Focus::Chat), "back where it was");
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
