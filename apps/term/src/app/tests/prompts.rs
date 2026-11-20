use super::*;

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
    let placeholder = app.placeholder();
    let text = screen_text(&mut app, 70, 14);
    assert!(!text.contains(&placeholder), "the editor is hidden: {text}");
    // Typed characters go nowhere; the digits answer.
    assert!(app.handle_key(key(KeyCode::Char('x'))).is_empty());
    assert_eq!(app.editor_text(), "");
    let fx = app.handle_key(key(KeyCode::Char('2')));
    assert!(matches!(&fx[..], [Effect::Action { args: Some(a), .. }] if a["option"] == "No"));
    // Answered: the editor is back.
    assert!(!app.prompt.engaged);
    let placeholder = app.placeholder();
    assert!(screen_text(&mut app, 70, 14).contains(&placeholder));
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

/// A prompt that arrived over a draft, taken by hand with Ctrl-P.
fn prompt_over_draft() -> App {
    let mut app = typing_then_prompt(1_000);
    app.handle_key(ctrl('p'));
    assert!(app.prompt.engaged);
    app
}

#[test]
fn a_prompt_waits_while_there_is_a_draft_and_ctrl_p_takes_it() {
    let mut app = typing_then_prompt(100);
    assert!(!app.prompt.engaged);
    let text = screen_text(&mut app, 70, 14);
    assert!(text.contains("draft"), "the editor stays meanwhile: {text}");
    // Keys still belong to the editor, digits included.
    app.handle_key(key(KeyCode::Char('2')));
    assert_eq!(app.editor_text(), "draft2");
    // A pause in typing is not enough while there is a draft.
    app.tick(NOW + 60_000);
    assert!(!app.prompt.engaged, "the draft holds it back");
    // Ctrl-P takes it now; the draft is untouched behind it and comes back once it is gone.
    app.handle_key(ctrl('p'));
    assert!(app.prompt.engaged);
    assert_eq!(
        app.active_ref(),
        Some("p1"),
        "the waiting one, not an older one"
    );
    assert!(app.take_dirty(), "the swap redraws");
    let text = screen_text(&mut app, 70, 14);
    assert!(!text.contains("draft"), "{text}");
    assert!(!text.contains("permission needed"), "{text}");
    assert_eq!(app.editor_text(), "draft2");
    app.handle_key(key(KeyCode::Char('1')));
    assert!(screen_text(&mut app, 70, 14).contains("draft2"));
}

#[test]
fn a_prompt_takes_over_once_the_draft_is_gone_and_typing_paused() {
    let mut app = typing_then_prompt(1_000);
    for _ in 0.."draft".len() {
        app.handle_key(key(KeyCode::Backspace));
    }
    assert!(!app.prompt.engaged, "the user may be typing on");
    app.tick(NOW + 1_800);
    assert!(app.prompt.engaged);
}

#[test]
fn stashing_the_draft_hands_the_editor_to_the_prompt_at_once() {
    let mut app = typing_then_prompt(100);
    app.handle_key(ctrl('s'));
    assert!(app.prompt.engaged);
    assert_eq!(app.editor_text(), "");
    app.handle_key(key(KeyCode::Char('1')));
    assert!(!app.prompt.engaged);
    app.handle_key(ctrl('s'));
    assert_eq!(app.editor_text(), "draft", "the stash comes back");
}

#[test]
fn sending_the_draft_lets_the_prompt_take_over() {
    let mut app = typing_then_prompt(100);
    app.handle_key(key(KeyCode::Enter));
    assert!(app.prompt.engaged);
}

#[test]
fn enter_answers_the_prompt_never_the_draft_and_esc_gives_the_editor_back() {
    let mut app = prompt_over_draft();
    type_str(&mut app, "more");
    app.handle_key(key(KeyCode::Right));
    let fx = app.handle_key(key(KeyCode::Enter));
    assert!(matches!(&fx[..], [Effect::Action { args: Some(a), .. }] if a["option"] == "No"));
    assert_eq!(app.editor_text(), "draft", "the draft came back as it was");
    // Esc dismisses instead of answering; the draft is intact and Enter sends it.
    let mut app = prompt_over_draft();
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
    let mut app = prompt_over_draft();
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
fn a_long_diff_is_not_repeated_in_the_input_area_once_expanded() {
    let mut app = app();
    show(&mut app, vec![permission_with_diff(60)]);
    let text = screen(&mut app, 100, 30).join("\n");
    // The edit has its own file view above the input.
    assert!(app.review_shown, "{text}");

    // Expanded, the chat has the whole diff: the panel keeps only the question and buttons.
    app.handle_key(ctrl('o'));
    let text = screen(&mut app, 70, 60).join("\n");
    // The chat follows its end; the panel's preview would have repeated the diff's start.
    assert_eq!(text.matches("const next59 = 59;").count(), 1, "{text}");
    assert!(!text.contains("const next0 = 0;"), "no preview: {text}");
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

#[test]
fn the_placeholder_follows_what_the_session_is_doing_and_keeps_to_one_wording() {
    let mut app = app();
    let fresh = app.placeholder();
    assert!(fresh.ends_with("· / for commands"), "{fresh}");
    assert_eq!(
        app.placeholder(),
        fresh,
        "it does not change while the session is open"
    );
    show(&mut app, vec![item("a1", "agent", "hello")]);
    let after = app.placeholder();
    assert!(
        ["What's next?", "Anything to change?"].contains(&after.as_str()),
        "{after}"
    );
    app.running = true;
    assert_eq!(app.placeholder(), "Queue a message · alt-enter sends now");
}
