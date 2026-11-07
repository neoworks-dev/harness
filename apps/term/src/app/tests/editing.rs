use super::*;

// ---- editing and sending ----

#[test]
fn enter_sends_the_trimmed_text_and_clears() {
    let mut app = app();
    type_str(&mut app, "  hello  ");
    let fx = app.handle_key(key(KeyCode::Enter));
    assert_eq!(fx, [input("hello")]);
    assert_eq!(app.editor_text(), "");
    // Nothing to send. (Typed by hand: three Spaces at once would be a held one, see voice.rs.)
    for at in [1_000, 1_300, 1_600] {
        app.tick(at);
        type_str(&mut app, " ");
    }
    assert!(app.handle_key(key(KeyCode::Enter)).is_empty());
}

#[test]
fn holding_space_records_and_letting_go_writes_it_out() {
    let mut app = app();
    type_str(&mut app, "fix");
    app.tick(1_000);
    type_str(&mut app, " ");
    // The key repeat: its delay, then a Space every 33 ms.
    app.tick(1_500);
    type_str(&mut app, " ");
    app.tick(1_533);
    type_str(&mut app, " ");
    app.tick(1_566);
    assert_eq!(
        app.handle_key(key(KeyCode::Char(' '))),
        [Effect::VoiceStart]
    );
    // The Spaces it typed before it was clear came back out; more repeats type nothing.
    assert_eq!(app.editor_text(), "fix");
    app.tick(1_600);
    assert!(app.handle_key(key(KeyCode::Char(' '))).is_empty());
    assert_eq!(app.editor_text(), "fix");
    // The repeats stop: Space was let go.
    assert!(app.tick(1_700).is_empty());
    assert_eq!(app.tick(2_100), [Effect::VoiceStop]);
    app.handle_msg(Msg::Transcribed(Ok("the failing test".into())));
    assert_eq!(app.editor_text(), "fix the failing test");
}

#[test]
fn a_space_let_go_under_kittys_protocol_stops_at_once() {
    let mut app = app();
    app.tick(1_000);
    type_str(&mut app, " ");
    app.tick(1_500);
    let repeat = KeyEvent {
        kind: KeyEventKind::Repeat,
        ..key(KeyCode::Char(' '))
    };
    assert_eq!(app.handle_key(repeat), [Effect::VoiceStart]);
    assert_eq!(app.editor_text(), "");
    let release = KeyEvent {
        kind: KeyEventKind::Release,
        ..key(KeyCode::Char(' '))
    };
    assert_eq!(app.handle_key(release), [Effect::VoiceStop]);
    app.handle_msg(Msg::Transcribed(
        Err("speech to text failed: no key".into()),
    ));
    assert_eq!(app.editor_text(), "");
}

#[test]
fn shift_enter_and_ctrl_j_insert_newlines_and_paste_stays_in_the_editor() {
    let mut app = app();
    type_str(&mut app, "a");
    assert!(
        app.handle_key(with(KeyCode::Enter, KeyModifiers::SHIFT))
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
            steer: false,
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
    // Ctrl-E is the marks' quick menu: End goes to the line's end.
    app.handle_key(key(KeyCode::End));
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
