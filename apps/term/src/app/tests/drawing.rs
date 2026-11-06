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
    snapshot("sidebar-100x30", &text);
    // The chat keeps the left part; the side column the right.
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
    let s = screen_text(&mut app, 80, 24);
    assert!(s.contains("enter change · ←/→ choose · esc close"), "{s}");
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
    assert!(screen_text(&mut app, 80, 24).contains("[2 Charts · on]"));
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
    assert!(app.handle_key(key(KeyCode::Esc)).is_empty());
    assert_eq!(app.active_ref(), None);
}
