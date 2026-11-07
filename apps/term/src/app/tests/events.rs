use super::*;

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
fn a_turn_left_open_for_a_background_subagent_says_the_subagent_works() {
    let mut app = app();
    app.handle_event(user_message(NOW - 12_000));
    let waiting = |on: Option<Vec<&str>>| {
        let info: SessionInfo = serde_json::from_value(json!({
            "id": "s1", "cwd": "/home/me/site", "createdAt": 0, "lastActivity": NOW,
            "lastSeq": 2, "state": "running", "queued": 0,
            "waitingOn": on,
        }))
        .unwrap();
        Event::SessionChanged(info)
    };
    app.handle_event(waiting(Some(vec!["Research the cache"])));
    let text = screen_text(&mut app, 100, 30);
    assert!(
        text.contains("a subagent is still working: Research the cache"),
        "{text}"
    );
    assert!(!text.contains("esc to interrupt"), "{text}");
    // The agent writes again: its own work row is back.
    app.handle_event(waiting(None));
    assert!(screen_text(&mut app, 100, 30).contains("esc to interrupt"));
    // The turn's end takes both away.
    app.handle_event(waiting(Some(vec!["Research the cache"])));
    app.handle_event(turn_ended());
    assert!(!screen_text(&mut app, 100, 30).contains("still working"));
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
        before: None,
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
        asking: None,
        waiting_on: None,
        unseen: None,
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
fn ctrl_enter_sends_into_the_running_turn_without_cancelling_it() {
    let steered = |text: &str| Effect::Input {
        session: "s1".into(),
        text: text.into(),
        images: Vec::new(),
        steer: true,
    };
    let mut app = app();
    app.handle_event(user_message(NOW));
    type_str(&mut app, "also do this");
    assert_eq!(
        app.handle_key(with(KeyCode::Enter, KeyModifiers::CONTROL)),
        [steered("also do this")]
    );
    // Idle, it is just Enter; on an empty editor nothing is sent.
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
    // Alt-Enter does the same, for terminals that can't tell Ctrl-Enter from Enter.
    type_str(&mut app, "now");
    assert_eq!(
        app.handle_key(with(KeyCode::Enter, KeyModifiers::ALT)),
        [steered("now")]
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
    let shown = screen(&mut app, 60, 20).join("\n");
    assert!(shown.contains("◷ first"), "{shown}");
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
        before: None,
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
pub(super) fn sessions_command(app: &mut App) -> Vec<Effect> {
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
        asking: None,
        waiting_on: None,
        unseen: None,
        preview: None,
        added: None,
        removed: None,
    };
    app.handle_msg(Msg::Sessions(Ok(vec![
        info("s1", "this one", "/home/me/site", NOW - 60_000),
        info("s7", "photo logo", "/home/me/logos", NOW - 3_600_000),
    ])));
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
        asking: None,
        waiting_on: None,
        unseen: None,
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
