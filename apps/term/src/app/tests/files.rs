use super::*;

#[test]
fn comments_from_the_diff_view_go_into_the_prompt_and_follow_their_edits() {
    let mut app = app();
    let some = |s: &str| Some(s.to_string());
    type_str(&mut app, "look at these");
    app.comment_in_prompt(None, some("@src/a.rs:11 use a const"));
    app.comment_in_prompt(None, some("@src/b.rs:2-4 why?"));
    assert_eq!(
        app.editor.text(),
        "look at these\n@src/a.rs:11 use a const\n@src/b.rs:2-4 why?"
    );
    // Edited: in its place, wherever the user moved it.
    app.comment_in_prompt(
        some("@src/a.rs:11 use a const"),
        some("@src/a.rs:11 use a constant"),
    );
    assert!(
        app.editor
            .text()
            .contains("\n@src/a.rs:11 use a constant\n")
    );
    // Deleted: gone with its line.
    app.comment_in_prompt(some("@src/b.rs:2-4 why?"), None);
    assert_eq!(
        app.editor.text(),
        "look at these\n@src/a.rs:11 use a constant"
    );
    // Taken out of the prompt by hand, an edit comes back at the end.
    app.editor.set_text("just this");
    app.comment_in_prompt(some("@src/a.rs:11 use a constant"), some("@src/a.rs:11 x"));
    assert_eq!(app.editor.text(), "just this\n@src/a.rs:11 x");
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
    screen_text(&mut app, 80, 24);
    app.handle_key(key(KeyCode::Tab));
    assert_eq!(app.focus, Some(Focus::Chat));
    assert_eq!(app.chat.picked_file(), Some("src/a.rs"));
    app.handle_key(key(KeyCode::Enter));
    assert!(app.widgets().overlay_open());
    // Ctrl-D reaches the diff, not the editor; `<leader>e` hands the file to the editor.
    assert!(app.handle_key(ctrl('d')).is_empty());
    assert!(app.widgets().overlay_open());
    assert!(app.handle_key(key(KeyCode::Char(' '))).is_empty());
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

#[test]
fn ctrl_o_on_an_edit_in_the_chat_opens_its_file() {
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
    screen_text(&mut app, 80, 24);
    app.handle_key(key(KeyCode::Tab));
    assert_eq!(app.chat.picked_file(), Some("src/a.rs"));
    app.handle_key(ctrl('o'));
    assert!(app.widgets().overlay_open());
    assert!(!app.expanded, "the chat stays as it was");
    // On a reply it expands the chat.
    app.handle_key(key(KeyCode::Esc));
    app.handle_key(key(KeyCode::Up));
    assert_eq!(app.chat.picked_file(), None);
    app.handle_key(ctrl('o'));
    assert!(app.expanded);
}
