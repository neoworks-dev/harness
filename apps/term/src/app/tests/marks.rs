use super::*;

// ---- marks (harpoon) ----

#[test]
fn alt_m_marks_and_alt_digits_jump_between_marked_chats() {
    let mut app = app();
    app.set_sessions(vec![info("s1", "One", 2), info("s2", "Two", 1)]);
    let alt = |c| with(KeyCode::Char(c), KeyModifiers::ALT);
    app.handle_key(alt('m'));
    assert_eq!(app.marks, ["s1"]);
    // Nothing at 2 yet.
    assert!(app.handle_key(alt('2')).is_empty());
    let fx = app.handle_key(alt('1'));
    assert!(fx.is_empty(), "already there");

    app.begin_session("s2", "/home/me/site");
    app.handle_key(alt('m'));
    assert_eq!(app.marks, ["s1", "s2"]);
    let fx = app.handle_key(alt('1'));
    assert!(matches!(&fx[..], [Effect::Switch { to, .. }] if to == "s1"));

    // The list: K moves the second up, d unmarks it.
    app.handle_key(alt('\''));
    assert_eq!(app.overlay, Overlay::Marks);
    app.handle_key(key(KeyCode::Char('j')));
    app.handle_key(key(KeyCode::Char('K')));
    assert_eq!(app.marks, ["s2", "s1"]);
    app.handle_key(key(KeyCode::Char('d')));
    assert_eq!(app.marks, ["s1"]);
    app.handle_key(key(KeyCode::Esc));
    assert_eq!(app.overlay, Overlay::None);

    // Marking the shown chat again unmarks it.
    app.handle_key(alt('m'));
    assert!(app.marks.is_empty());
}

#[test]
fn a_mark_whose_chat_is_gone_is_dropped() {
    let mut app = app();
    app.set_sessions(vec![info("s1", "One", 2)]);
    app.marks = vec!["gone".into()];
    let fx = app.handle_key(with(KeyCode::Char('1'), KeyModifiers::ALT));
    assert!(fx.is_empty());
    assert!(app.marks.is_empty());
}

#[test]
fn the_marks_dialog_lists_the_marked_chats_over_the_chat() {
    let mut app = app();
    let mut asking = info("s3", "Deploy the site", 3);
    asking.state = SessionState::Running;
    asking.asking = Some(1);
    let mut running = info("s4", "Refactor the router", 4);
    running.state = SessionState::Running;
    app.set_sessions(vec![
        info("s1", "Fix the parser", 2),
        info("s2", "Write the docs", 1),
        asking,
        running,
    ]);
    app.marks = vec!["s2".into(), "s1".into(), "s3".into(), "s4".into()];
    app.handle_key(with(KeyCode::Char('\''), KeyModifiers::ALT));
    let s = screen_text(&mut app, 90, 20);
    assert!(
        s.contains("! Deploy the site") && s.contains("needs you"),
        "{s}"
    );
    assert!(s.contains("working"), "{s}");
    snapshot("marks-dialog-90x20", &s);
}

#[test]
fn ctrl_g_then_a_digit_shows_the_marked_chat() {
    let mut app = app();
    app.set_sessions(vec![info("s1", "One", 2), info("s2", "Two", 1)]);
    app.marks = vec!["s2".into()];
    // With the sidebar drawn (it has the keys then) and without.
    screen(&mut app, 120, 30);
    app.handle_key(with(KeyCode::Char('g'), KeyModifiers::CONTROL));
    assert_eq!(app.focus, Some(Focus::Sessions));
    let fx = app.handle_key(key(KeyCode::Char('1')));
    assert!(matches!(&fx[..], [Effect::Switch { to, .. }] if to == "s2"));
    assert_eq!(app.focus, None);

    let mut app = self::app();
    app.set_sessions(vec![info("s1", "One", 2), info("s2", "Two", 1)]);
    app.marks = vec!["s2".into()];
    app.handle_key(with(KeyCode::Char('g'), KeyModifiers::CONTROL));
    let fx = app.handle_key(key(KeyCode::Char('1')));
    assert!(matches!(&fx[..], [Effect::Switch { to, .. }] if to == "s2"));
    // A digit typed later is text.
    app.handle_key(key(KeyCode::Char('1')));
    assert_eq!(app.editor_text(), "1");
}

#[test]
fn ctrl_and_a_digit_shows_the_marked_chat_at_once() {
    let mut app = app();
    app.set_sessions(vec![info("s1", "One", 2), info("s2", "Two", 1)]);
    app.marks = vec!["s2".into()];
    let fx = app.handle_key(with(KeyCode::Char('1'), KeyModifiers::CONTROL));
    assert!(matches!(&fx[..], [Effect::Switch { to, .. }] if to == "s2"));
}

#[test]
fn ctrl_e_opens_the_quick_menu_where_a_marks_the_shown_chat() {
    let mut app = app();
    app.set_sessions(vec![info("s1", "One", 2), info("s2", "Two", 1)]);
    let ctrl_e = with(KeyCode::Char('e'), KeyModifiers::CONTROL);
    app.handle_key(ctrl_e);
    assert_eq!(app.overlay, Overlay::Marks);
    app.handle_key(key(KeyCode::Char('a')));
    assert_eq!(app.marks, ["s1"]);
    app.handle_key(ctrl_e);
    assert_eq!(app.overlay, Overlay::None);
    // The menu is how to get there: Ctrl-E, then the digit.
    app.begin_session("s2", "/home/me/site");
    app.handle_key(ctrl_e);
    let fx = app.handle_key(key(KeyCode::Char('1')));
    assert!(matches!(&fx[..], [Effect::Switch { to, .. }] if to == "s1"));
}
