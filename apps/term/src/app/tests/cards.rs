use super::*;

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
pub(super) fn permission_with_diff(changed: usize) -> ViewItem {
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
                   "args":{"id":"7","option":id},"key":(i + 1).to_string(),"note":true})
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
fn an_edit_request_shows_whole_above_the_input_and_its_comments_go_with_the_answer() {
    let dir = std::env::temp_dir().join("neo-term-test-review");
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("src/a.ts"), "one\ntwo\nthree\nfour\n").unwrap();
    let mut app = app();
    app.widgets.reset(dir);
    let mut item = edit_review_card();
    item.label = Some("Edit src/a.ts".into());
    item.component = serde_json::from_value(json!({"type":"stack","children":[
        {"type":"diff","path":"src/a.ts","lines":[
            {"kind":"context","text":"two","old":2,"new":2},
            {"kind":"remove","text":"three","old":3},
            {"kind":"add","text":"3","new":3}]},
        {"type":"row","children":[
            {"type":"button","label":"Allow","action":"neo.permit",
             "args":{"id":"9","option":"allow"},"key":"1","note":true},
            {"type":"button","label":"Reject","action":"neo.permit",
             "args":{"id":"9","option":"reject"},"key":"2","note":true}]}]}))
    .unwrap();
    show(&mut app, vec![item]);
    assert_eq!(app.widgets.reviewing(), Some("p-9"));
    let text = screen_text(&mut app, 80, 30);
    assert!(text.contains("proposed edit"), "{text}");
    assert!(
        text.contains("one") && text.contains("four"),
        "the whole file: {text}"
    );
    // Visual mode and the leader's c: a comment on the lines, above them.
    // It starts on the change; the digits answer the card, so no `2G`.
    type_str(&mut app, "kVjj");
    app.handle_key(key(KeyCode::Char(' ')));
    app.handle_key(key(KeyCode::Char('c')));
    type_str(&mut app, "keep the word");
    app.handle_key(key(KeyCode::Enter));
    let text = screen_text(&mut app, 80, 30);
    assert!(text.contains("╭ @src/a.ts:2-3"), "{text}");
    assert!(text.contains("keep the word"), "{text}");
    // q doesn't take the view away: it goes with the card.
    app.handle_key(key(KeyCode::Char('q')));
    assert_eq!(app.widgets.reviewing(), Some("p-9"));
    // Not into the prompt: with the answer.
    assert_eq!(app.editor_text(), "");
    assert_eq!(app.widgets.review_note_count("p-9"), 1);
    // The card's keys answer while the view is there.
    let fx = app.handle_key(key(KeyCode::Char('2')));
    assert!(
        matches!(&fx[..], [Effect::Action { args: Some(a), .. }]
            if a["option"] == "reject" && a["note"] == "@src/a.ts:2-3 keep the word"),
        "{fx:?}"
    );
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
                   "args":{"id":"9","option":id},"key":(i + 1).to_string(),"note":true})
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
               "args":{"id":"r1","option":id},"key":(i + 1).to_string(),"note":true})
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
