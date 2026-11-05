use super::*;

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

pub(super) fn picked(fx: &[Effect]) -> String {
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
    // And the choice leaves the chat with it.
    let s = screen_text(&mut app, 80, 24);
    assert!(!s.contains("Model") && !s.contains("Haiku"), "{s}");
}

#[test]
fn a_setting_takes_no_note() {
    let mut app = app();
    model_choice(&mut app);
    type_str(&mut app, "gemini");
    let s = screen_text(&mut app, 80, 24);
    assert!(!s.contains("note"), "{s}");
    app.handle_key(key(KeyCode::Tab));
    assert!(!screen_text(&mut app, 80, 24).contains("note ›"));
    let fx = app.handle_key(key(KeyCode::Enter));
    match &fx[..] {
        [Effect::Action { args: Some(a), .. }] => {
            assert!(a["option"].as_str().unwrap().ends_with("Gemini 2.5"), "{a}");
            assert!(a.get("note").is_none(), "{a}");
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
