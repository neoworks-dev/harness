use super::*;

// ---- drawing ----

pub(super) fn plan_event() -> Event {
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
    assert!(app.sidebar_shown);
    app.handle_key(key(KeyCode::Tab));
    assert_eq!(app.focus, Some(Focus::Chat), "the reply first");
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
    app.handle_key(key(KeyCode::Tab));
    assert_eq!(app.focus, Some(Focus::Panel(Panel::Plan)));
    screen(&mut app, 60, 20);
    assert_eq!(app.focus, None);
}

#[test]
fn help_opens_by_command_and_by_question_mark() {
    let mut app = scenario();
    type_str(&mut app, "/help");
    app.handle_key(key(KeyCode::Enter));
    app.handle_key(key(KeyCode::Esc));
    app.handle_key(key(KeyCode::Char('?')));
    assert_eq!(app.overlay, Overlay::Help, "? on an empty editor");
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
fn theme_switches_by_name_or_to_the_next() {
    use crate::theme::Theme;
    let mut app = scenario();
    // Dark by default.
    assert_eq!(app.theme, Theme::Dark);

    type_str(&mut app, "/theme light");
    app.handle_key(key(KeyCode::Enter));
    assert_eq!(app.theme, Theme::Light);

    // No name: the next one.
    type_str(&mut app, "/theme");
    app.handle_key(key(KeyCode::Enter));
    assert_eq!(app.theme, Theme::Terminal);

    type_str(&mut app, "/theme sepia");
    app.handle_key(key(KeyCode::Enter));
    assert!(screen_text(&mut app, 60, 14).contains("usage: /theme [dark|light|terminal]"));
    assert_eq!(app.theme, Theme::Terminal);
}

#[test]
fn a_prompt_answered_below_the_chat_starts_on_the_current_value() {
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
        s.contains("Enter model-031") && s.contains("[model-031]"),
        "{s}"
    );
    assert!(!s.contains("model-000"), "{s}");
}

/// `/settings`: three settings, each a button that stays; `on` says which are on.
fn settings_choice(app: &mut App, on: [bool; 3]) {
    let rows: Vec<_> = ["Diffs", "Charts", "Edits"]
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let state = if on[i] { "on" } else { "off" };
            json!({"type":"button","label":format!("{name} · {state}"),"action":"neo.choose",
                "args":{"id":"c","option":name.to_lowercase()},"key":(i + 1).to_string(),
                "stays":true})
        })
        .collect();
    let item: ViewItem = serde_json::from_value(json!({
        "ref":"q-s","kind":"tool","label":"Settings","state":"pending",
        "component":{"type":"stack","gap":"s","children":rows}}))
    .unwrap();
    patch(app, item);
}

#[test]
fn enter_or_space_changes_a_setting_and_the_card_stays_until_esc() {
    let mut app = app();
    long_chat(&mut app, 3);
    settings_choice(&mut app, [false; 3]);
    app.handle_key(key(KeyCode::Down));
    match &app.handle_key(key(KeyCode::Char(' ')))[..] {
        [Effect::Action { args: Some(a), .. }] => {
            assert_eq!(a["option"], "charts");
            assert_eq!(a["stay"], true);
        }
        other => panic!("{other:?}"),
    }
    // Still open, on the same option, drawn again with the value as it is now.
    assert_eq!(app.active_ref(), Some("q-s"));
    settings_choice(&mut app, [false, true, false]);
    assert_eq!(app.active_ref(), Some("q-s"));
    match &app.handle_key(key(KeyCode::Char(' ')))[..] {
        [Effect::Action { args: Some(a), .. }] => assert_eq!(a["option"], "charts"),
        other => panic!("{other:?}"),
    }
    // Enter and the digits change it too, and the card stays; Esc closes it.
    let fx = app.handle_key(key(KeyCode::Enter));
    assert!(matches!(&fx[..], [Effect::Action { args: Some(a), .. }]
        if a["option"] == "charts" && a["stay"] == true));
    let fx = app.handle_key(key(KeyCode::Char('3')));
    assert!(matches!(&fx[..], [Effect::Action { args: Some(a), .. }]
        if a["option"] == "edits" && a["stay"] == true));
    assert_eq!(app.active_ref(), Some("q-s"));
    // Closed in the daemon too, so the session isn't left waiting for the user.
    let fx = app.handle_key(key(KeyCode::Esc));
    assert!(
        matches!(&fx[..], [Effect::Action { action, args: Some(a), .. }]
            if action == "neo.close" && a == &json!({"id": "c"})),
        "{fx:?}"
    );
    assert_eq!(app.active_ref(), None);
}

#[test]
fn a_setting_that_is_text_is_written_in_the_editor() {
    let mut app = app();
    long_chat(&mut app, 1);
    type_str(&mut app, "half a message");
    let card: ViewItem = serde_json::from_value(json!({
        "ref":"q-s","kind":"tool","label":"Settings","state":"pending",
        "component":{"type":"stack","children":[
            {"type":"button","label":"Diffs · off","action":"neo.choose",
             "args":{"id":"c","option":"diffs"},"key":"1","stays":true},
            {"type":"button","label":"Your words · 1 line","action":"neo.choose",
             "args":{"id":"c","option":"words"},"key":"2","stays":true,
             "input":{"value":"Be brief.","multiline":true}}]}}))
    .unwrap();
    patch(&mut app, card);
    app.handle_key(with(KeyCode::Char('p'), KeyModifiers::CONTROL));
    assert!(app.prompt.engaged);
    // The digit opens the editor on its text, the draft put aside.
    assert!(app.handle_key(key(KeyCode::Char('2'))).is_empty());
    assert_eq!(app.editor_text(), "Be brief.");
    app.handle_key(with(KeyCode::Enter, KeyModifiers::SHIFT));
    type_str(&mut app, "In German.");
    let fx = app.handle_key(key(KeyCode::Enter));
    assert!(
        matches!(&fx[..], [Effect::Action { args: Some(a), .. }]
            if a["option"] == "words" && a["stay"] == true
                && a["value"] == "Be brief.\nIn German."),
        "{fx:?}"
    );
    // The card is back, the draft under it.
    assert!(app.prompt.engaged);
    assert_eq!(app.editor_text(), "half a message");
    // Esc gives up on it: nothing sent.
    app.handle_key(key(KeyCode::Char('2')));
    type_str(&mut app, " Never mind.");
    assert!(app.handle_key(key(KeyCode::Esc)).is_empty());
    assert_eq!(app.editor_text(), "half a message");
    assert_eq!(app.active_ref(), Some("q-s"));
}
