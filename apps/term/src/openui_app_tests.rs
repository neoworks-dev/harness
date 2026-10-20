//! OpenUI docs in the app: focus, typing, keys and what is sent to the daemon, headless.

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyEventState, KeyModifiers};
use serde_json::{Value, json};
use suite_protocol::{ViewItem, ViewOp, ViewPatch, ViewSnapshot};
use suite_renderer_ratatui::{UiKind, UiTarget};

use super::*;

const NOW: i64 = 1_000_000;

fn app() -> App {
    let mut app = App::new(Config {
        app_id: "terminal".into(),
        session: "s1".into(),
        cwd: "/home/me/site".into(),
        home: Some("/home/me".into()),
        no_color: false,
        history: HistoryStore::default(),
        prefs: crate::theme::Prefs::default(),
    });
    app.tick(NOW);
    app.awaiting = None;
    app.last_poll_ms = NOW;
    app.turn_started_ms = NOW;
    app
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent {
        code,
        modifiers: KeyModifiers::NONE,
        kind: KeyEventKind::Press,
        state: KeyEventState::NONE,
    }
}

fn ch(c: char) -> KeyEvent {
    key(KeyCode::Char(c))
}

fn ctrl(c: char) -> KeyEvent {
    KeyEvent {
        modifiers: KeyModifiers::CONTROL,
        ..key(KeyCode::Char(c))
    }
}

fn screen(app: &mut App, w: u16, h: u16) -> Vec<String> {
    let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
    terminal.draw(|f| app.draw(f)).unwrap();
    let buffer = terminal.backend().buffer();
    (0..h)
        .map(|y| {
            (0..w)
                .map(|x| buffer[(x, y)].symbol().to_string())
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect()
}

fn text(app: &mut App) -> String {
    screen(app, 80, 36).join("\n")
}

fn node(component: &str, props: Value) -> Value {
    json!({"component": component, "props": props})
}

fn keyed(component: &str, key: &str, props: Value) -> Value {
    json!({"component": component, "key": key, "props": props})
}

fn control(label: &str, input: Value) -> Value {
    node("FormControl", json!({"label": label, "input": input}))
}

fn option(value: &str, label: &str) -> Value {
    node("Option", json!({"value": value, "label": label}))
}

/// An agent message with a form in it.
fn form_item(r: &str, streaming: bool, email: &str) -> ViewItem {
    let form = node(
        "Form",
        json!({"name": "deploy", "children": [
            control("Service", keyed("Select", "service", json!({"name": "service", "placeholder": "Pick", "required": true,
                "options": [option("api", "API"), option("web", "Web app"), option("worker", "Worker")], "value": "api"}))),
            control("Email", keyed("Input", "email", json!({"name": "email", "placeholder": "you@company.com", "value": email}))),
            control("Environment", keyed("RadioGroup", "env", json!({"name": "env", "value": "staging",
                "options": [option("staging", "Staging"), option("prod", "Production")]}))),
            control("Notes", keyed("TextArea", "notes", json!({"name": "notes"}))),
            control("Canary", keyed("Slider", "canary", json!({"name": "canary", "min": 0, "max": 50, "step": 5, "value": 10}))),
            keyed("Switch", "notify", json!({"name": "notify", "label": "Post to #deploys", "value": false})),
            keyed("CheckBox", "agree", json!({"name": "agree", "label": "I checked", "value": false})),
        ], "buttons": node("Buttons", json!({"buttons": [
            keyed("Button", "deploy", json!({"label": "Deploy", "variant": "primary"})),
            keyed("Button", "docs", json!({"label": "Runbook", "url": "https://example.com/runbook"})),
            keyed("Button", "copy", json!({"label": "Copy id", "copy": "abc-123"})),
        ]}))}),
    );
    serde_json::from_value(json!({
        "ref": r, "kind": "agent",
        "component": {"type": "text", "blocks": [
            {"type": "paragraph", "spans": [{"text": "Fill this in:"}]},
            {"type": "openui", "ui": {"id": "0", "root": form, "streaming": streaming}},
        ]}
    }))
    .unwrap()
}

fn user(r: &str, text: &str) -> ViewItem {
    serde_json::from_value(json!({
        "ref": r, "kind": "user",
        "component": {"type":"text","blocks":[{"type":"paragraph","spans":[{"text": text}]}]}
    }))
    .unwrap()
}

fn show(app: &mut App, items: Vec<ViewItem>) {
    app.handle_event(Event::ViewSnapshot(ViewSnapshot {
        agent: None,
        session: "s1".into(),
        seq: 1,
        items,
    }));
}

fn patch(app: &mut App, item: ViewItem) {
    app.handle_event(Event::ViewPatch(ViewPatch {
        agent: None,
        session: "s1".into(),
        seq: 2,
        ops: vec![ViewOp::SetViewOp(suite_protocol::SetViewOp { item })],
    }));
}

fn with_form() -> App {
    let mut app = app();
    show(
        &mut app,
        vec![user("u1", "deploy it"), form_item("f1", false, "")],
    );
    // The user's message came before the form.
    app
}

fn press(app: &mut App, k: KeyEvent) -> Vec<Effect> {
    app.handle_key(k)
}

fn tabs(app: &mut App, n: usize) {
    for _ in 0..n {
        press(app, key(KeyCode::Tab));
    }
}

fn typed(app: &mut App, s: &str) {
    for c in s.chars() {
        press(app, ch(c));
    }
}

fn input(doc: &str, k: &str, value: Value) -> Effect {
    Effect::Action {
        session: "s1".into(),
        item: "f1".into(),
        action: "openui.input".into(),
        args: Some(json!({"doc": doc, "key": k, "value": value})),
    }
}

fn pressed(doc: &str, k: &str) -> Effect {
    Effect::Action {
        session: "s1".into(),
        item: "f1".into(),
        action: "openui.press".into(),
        args: Some(json!({"doc": doc, "key": k})),
    }
}

fn focused(app: &App) -> Option<String> {
    let targets = app.form_targets("f1");
    app.forms.focused("f1", &targets).map(|t| t.id.clone())
}

#[test]
fn a_form_takes_the_editor_s_place_and_starts_on_its_first_field() {
    let mut app = with_form();
    assert!(app.prompt.engaged);
    assert_eq!(focused(&app).as_deref(), Some("service"));
    let screen = text(&mut app);
    assert!(screen.contains("Service *"), "{screen}");
    assert!(screen.contains("[ API"), "{screen}");
    // The keys line stands where the editor was.
    assert!(
        screen.contains("Service · enter opens · ←/→ choose · tab next · esc input"),
        "{screen}"
    );
    assert!(!screen.contains("message the agent"), "{screen}");
}

#[test]
fn tab_and_arrows_move_between_targets_and_wrap() {
    let mut app = with_form();
    let order = [
        "service", "email", "env", "notes", "canary", "notify", "agree", "deploy", "docs", "copy",
    ];
    for want in &order[1..] {
        press(&mut app, key(KeyCode::Tab));
        assert_eq!(focused(&app).as_deref(), Some(*want));
    }
    press(&mut app, key(KeyCode::Tab));
    assert_eq!(focused(&app).as_deref(), Some("service"));
    press(&mut app, key(KeyCode::BackTab));
    assert_eq!(focused(&app).as_deref(), Some("copy"));
    // Down is the next one in a text field too; Up the one before.
    tabs(&mut app, 2);
    assert_eq!(focused(&app).as_deref(), Some("email"));
    press(&mut app, key(KeyCode::Down));
    assert_eq!(focused(&app).as_deref(), Some("env"));
    press(&mut app, key(KeyCode::Up));
    assert_eq!(focused(&app).as_deref(), Some("email"));
}

#[test]
fn typing_edits_the_focused_field_and_is_sent_after_a_pause() {
    let mut app = with_form();
    tabs(&mut app, 1);
    typed(&mut app, "ops@x");
    assert!(app.tick(NOW + 100).is_empty());
    assert!(text(&mut app).contains("[ ops@x"));
    // Typing again pushes the pause back.
    typed(&mut app, ".io");
    assert!(app.tick(NOW + 200).is_empty());
    assert_eq!(
        app.tick(NOW + 300),
        [input("0", "email", json!("ops@x.io"))]
    );
    assert!(app.tick(NOW + 400).is_empty());
    // The editor never saw it.
    assert_eq!(app.editor_text(), "");
    // Cursor keys and deleting.
    press(&mut app, key(KeyCode::Left));
    press(&mut app, key(KeyCode::Left));
    press(&mut app, key(KeyCode::Backspace));
    assert!(text(&mut app).contains("[ ops@xio"));
    assert_eq!(app.tick(NOW + 600), [input("0", "email", json!("ops@xio"))]);
    press(&mut app, ctrl('u'));
    assert_eq!(app.tick(NOW + 800), [input("0", "email", json!("io"))]);
}

#[test]
fn pasting_goes_into_the_field() {
    let mut app = with_form();
    tabs(&mut app, 1);
    app.handle_paste("pasted\ntext");
    assert!(text(&mut app).contains("[ pasted"));
    let fx = app.tick(NOW + 200);
    assert_eq!(fx.len(), 1);
}

#[test]
fn the_daemon_s_value_is_ignored_while_the_field_has_focus() {
    let mut app = with_form();
    tabs(&mut app, 1);
    typed(&mut app, "abc");
    // The daemon echoes an older text (or a normalized one) while typing goes on.
    patch(&mut app, form_item("f1", false, "ab"));
    let screen = text(&mut app);
    assert!(screen.contains("[ abc"), "{screen}");
    typed(&mut app, "d");
    assert!(text(&mut app).contains("[ abcd"));
    // Away from the field the daemon's text shows again.
    tabs(&mut app, 1);
    patch(&mut app, form_item("f1", false, "from the daemon"));
    assert!(text(&mut app).contains("[ from the daemon"));
}

#[test]
fn space_toggles_at_once_and_the_value_shows_before_the_daemon_echoes() {
    let mut app = with_form();
    tabs(&mut app, 5);
    assert_eq!(focused(&app).as_deref(), Some("notify"));
    let fx = press(&mut app, ch(' '));
    assert_eq!(fx, [input("0", "notify", json!(true))]);
    assert!(text(&mut app).contains("● on  Post to #deploys"));
    // Toggled twice before the daemon answered: the local value counts.
    let fx = press(&mut app, ch(' '));
    assert_eq!(fx, [input("0", "notify", json!(false))]);
    let fx = press(&mut app, ch(' '));
    assert_eq!(fx, [input("0", "notify", json!(true))]);
    tabs(&mut app, 1);
    assert_eq!(focused(&app).as_deref(), Some("agree"));
    let fx = press(&mut app, ch(' '));
    assert_eq!(fx, [input("0", "agree", json!(true))]);
    assert!(text(&mut app).contains("[x] I checked"));
}

#[test]
fn left_and_right_pick_radio_options_and_move_sliders() {
    let mut app = with_form();
    tabs(&mut app, 2);
    assert_eq!(focused(&app).as_deref(), Some("env"));
    assert_eq!(
        press(&mut app, key(KeyCode::Right)),
        [input("0", "env", json!("prod"))]
    );
    assert!(text(&mut app).contains("(•) Production"));
    // At the end it stays.
    assert!(press(&mut app, key(KeyCode::Right)).is_empty());
    assert_eq!(
        press(&mut app, key(KeyCode::Left)),
        [input("0", "env", json!("staging"))]
    );
    tabs(&mut app, 2);
    assert_eq!(focused(&app).as_deref(), Some("canary"));
    assert_eq!(
        press(&mut app, key(KeyCode::Right)),
        [input("0", "canary", json!(15))]
    );
    assert_eq!(
        press(&mut app, key(KeyCode::Left)),
        [input("0", "canary", json!(10))]
    );
    assert_eq!(
        press(&mut app, key(KeyCode::End)),
        [input("0", "canary", json!(50))]
    );
    assert!(press(&mut app, key(KeyCode::Right)).is_empty());
}

#[test]
fn enter_opens_a_select_and_chooses_from_its_list() {
    let mut app = with_form();
    assert!(press(&mut app, key(KeyCode::Enter)).is_empty());
    let screen = text(&mut app);
    assert!(screen.contains("› API ✓"), "{screen}");
    assert!(screen.contains("Worker"), "{screen}");
    assert!(screen.contains("↑/↓ choose · enter picks"), "{screen}");
    press(&mut app, key(KeyCode::Down));
    press(&mut app, key(KeyCode::Down));
    assert_eq!(
        press(&mut app, key(KeyCode::Enter)),
        [input("0", "service", json!("worker"))]
    );
    let screen = text(&mut app);
    assert!(screen.contains("[ Worker"), "{screen}");
    assert!(!screen.contains("Web app"), "{screen}");
    // Esc closes the list without choosing, the next Esc leaves the form.
    press(&mut app, key(KeyCode::Enter));
    press(&mut app, key(KeyCode::Up));
    assert!(press(&mut app, key(KeyCode::Esc)).is_empty());
    assert!(app.prompt.engaged);
    assert!(text(&mut app).contains("[ Worker"));
    press(&mut app, key(KeyCode::Esc));
    assert!(!app.prompt.engaged);
    // Left and right pick without opening it (the choice is held until the daemon echoes it).
    press(&mut app, ctrl('p'));
    assert!(app.prompt.engaged);
    assert_eq!(
        press(&mut app, key(KeyCode::Left)),
        [input("0", "service", json!("web"))]
    );
}

#[test]
fn enter_in_a_text_area_is_a_newline_and_elsewhere_it_presses_the_first_button() {
    let mut app = with_form();
    tabs(&mut app, 3);
    assert_eq!(focused(&app).as_deref(), Some("notes"));
    typed(&mut app, "one");
    assert!(press(&mut app, key(KeyCode::Enter)).is_empty());
    typed(&mut app, "two");
    assert_eq!(
        app.tick(NOW + 200),
        [input("0", "notes", json!("one\ntwo"))]
    );
    // Up and Down move in the text, then on.
    press(&mut app, key(KeyCode::Up));
    assert_eq!(focused(&app).as_deref(), Some("notes"));
    press(&mut app, key(KeyCode::Up));
    assert_eq!(focused(&app).as_deref(), Some("env"));
    // In an input, enter submits, and what was typed goes first.
    press(&mut app, key(KeyCode::Up));
    typed(&mut app, "me@x.dev");
    let fx = press(&mut app, key(KeyCode::Enter));
    assert_eq!(
        fx,
        [
            input("0", "email", json!("me@x.dev")),
            pressed("0", "deploy")
        ]
    );
    // Nothing is sent twice.
    assert!(app.tick(NOW + 500).is_empty());
    // The form stays up for the answer (errors, a second press).
    assert!(app.prompt.engaged);
}

#[test]
fn a_button_presses_with_enter_opens_its_link_or_copies() {
    let mut app = with_form();
    tabs(&mut app, 7);
    assert_eq!(focused(&app).as_deref(), Some("deploy"));
    let screen = text(&mut app);
    assert!(screen.contains("[ Deploy ]"), "{screen}");
    assert!(screen.contains("[ Runbook ↗ ]"), "{screen}");
    assert_eq!(
        press(&mut app, key(KeyCode::Enter)),
        [pressed("0", "deploy")]
    );
    assert_eq!(press(&mut app, ch(' ')), [pressed("0", "deploy")]);
    // Right goes on to the next button; its link is the client's.
    press(&mut app, key(KeyCode::Right));
    assert_eq!(focused(&app).as_deref(), Some("docs"));
    assert_eq!(
        press(&mut app, key(KeyCode::Enter)),
        [Effect::OpenUrl("https://example.com/runbook".into())]
    );
    press(&mut app, key(KeyCode::Right));
    assert_eq!(
        press(&mut app, key(KeyCode::Enter)),
        [Effect::Copy("abc-123".into())]
    );
    assert!(text(&mut app).contains("copied"));
}

#[test]
fn only_links_a_browser_can_open_are_opened() {
    let mut app = with_form();
    let mut fx = Vec::new();
    app.open_url("file:///etc/passwd".into(), &mut fx);
    assert!(fx.is_empty());
    assert!(text(&mut app).contains("link: file:///etc/passwd"));
    app.open_url("mailto:a@b.c".into(), &mut fx);
    assert_eq!(fx, [Effect::OpenUrl("mailto:a@b.c".into())]);
}

#[test]
fn esc_gives_the_editor_back_with_its_draft() {
    let mut app = app();
    type_draft(&mut app, "half a thought");
    show(
        &mut app,
        vec![user("u1", "deploy it"), form_item("f1", false, "")],
    );
    // The user is typing: the form waits.
    assert!(!app.prompt.engaged);
    assert!(text(&mut app).contains("form open"));
    app.tick(NOW + 1000);
    assert!(app.prompt.engaged);
    press(&mut app, key(KeyCode::Esc));
    assert!(!app.prompt.engaged);
    assert_eq!(app.editor_text(), "half a thought");
    // Typing goes to the editor again.
    typed(&mut app, "!");
    assert_eq!(app.editor_text(), "half a thought!");
}

fn type_draft(app: &mut App, s: &str) {
    for c in s.chars() {
        app.handle_key(ch(c));
    }
}

#[test]
fn a_streaming_doc_takes_no_focus() {
    let mut app = app();
    show(&mut app, vec![user("u1", "go"), form_item("f1", true, "")]);
    assert!(!app.prompt.engaged);
    typed(&mut app, "hello");
    assert_eq!(app.editor_text(), "hello");
    // Once it is complete it can be used.
    app.editor.clear();
    patch(&mut app, form_item("f1", false, ""));
    app.tick(NOW + 2000);
    assert!(app.prompt.engaged);
}

#[test]
fn a_form_the_user_has_spoken_after_waits_to_be_picked() {
    let mut app = app();
    show(
        &mut app,
        vec![form_item("f1", false, ""), user("u1", "never mind")],
    );
    assert!(!app.prompt.engaged);
    // It can still be picked by hand.
    press(&mut app, ctrl('p'));
    assert!(app.prompt.engaged);
    assert_eq!(focused(&app).as_deref(), Some("service"));
}

#[test]
fn tabs_and_accordions_are_reached_with_ctrl_p_and_switch_with_the_arrows() {
    let dashboard: ViewItem = serde_json::from_value(json!({
        "ref": "d1", "kind": "tool", "label": "analytics.revenue", "state": "done",
        "component": {"type": "openui", "ui": {"id": "0", "root": node("Tabs", json!({"tabs": [
            node("Tab", json!({"label": "Trend", "children": [node("Text", json!({"text": "t", "blocks": [{"type":"paragraph","spans":[{"text":"trend body"}]}]}))]})),
            node("Tab", json!({"label": "Mix", "children": [node("Text", json!({"text": "m", "blocks": [{"type":"paragraph","spans":[{"text":"mix body"}]}]}))]})),
        ]}))}}
    }))
    .unwrap();
    let mut app = app();
    show(&mut app, vec![user("u1", "show me"), dashboard]);
    // Nothing to fill in: the editor keeps the keys and the result is one line.
    assert!(!app.prompt.engaged);
    let screen = text(&mut app);
    assert!(screen.contains("⎿ "), "{screen}");
    assert!(!screen.contains("trend body"), "{screen}");
    press(&mut app, ctrl('p'));
    assert!(app.prompt.engaged);
    let screen = text(&mut app);
    assert!(screen.contains("│ Trend │ Mix │"), "{screen}");
    assert!(screen.contains("trend body"), "{screen}");
    assert!(screen.contains("←/→ switch tab"), "{screen}");
    assert!(press(&mut app, key(KeyCode::Right)).is_empty());
    let screen = text(&mut app);
    assert!(
        screen.contains("mix body") && !screen.contains("trend body"),
        "{screen}"
    );
    // Left to Trend, enter cycles on to Mix; no daemon is told.
    press(&mut app, key(KeyCode::Left));
    assert!(text(&mut app).contains("trend body"));
    assert!(press(&mut app, key(KeyCode::Enter)).is_empty());
    assert!(text(&mut app).contains("mix body"));
    // Esc: back to the editor, the tab stays, the result is a line again.
    press(&mut app, key(KeyCode::Esc));
    assert!(!app.prompt.engaged);
    assert!(!text(&mut app).contains("mix body"));
}

#[test]
fn the_chat_scrolls_to_the_focused_field() {
    let mut app = app();
    let mut items: Vec<ViewItem> = (0..30)
        .map(|i| user(&format!("u{i}"), &format!("line {i}")))
        .collect();
    items.push(form_item("f1", false, ""));
    show(&mut app, items);
    // 12 rows is too small for the form: the focus line must stay in view.
    let rows = |app: &mut App| screen(app, 80, 12).join("\n");
    let first = rows(&mut app);
    assert!(first.contains("Service"), "{first}");
    tabs(&mut app, 7);
    let on_button = rows(&mut app);
    assert!(on_button.contains("[ Deploy ]"), "{on_button}");
    assert_eq!(focused(&app).as_deref(), Some("deploy"));
    press(&mut app, key(KeyCode::BackTab));
    press(&mut app, key(KeyCode::BackTab));
    assert_eq!(focused(&app).as_deref(), Some("notify"));
    let on_switch = rows(&mut app);
    assert!(on_switch.contains("Post to #deploys"), "{on_switch}");
}

#[test]
fn the_form_is_drawn_into_the_chat_on_a_wide_screen() {
    let mut app = with_form();
    tabs(&mut app, 1);
    typed(&mut app, "ops@");
    let screen = screen(&mut app, 80, 34).join("\n");
    assert!(screen.contains("▎   Service *"), "{screen}");
    // Only the chat's margin bar marks the form being answered.
    assert!(screen.contains("▎ "), "{screen}");
    assert!(
        screen.contains("▎ Email · type · enter submits · tab next · esc input"),
        "{screen}"
    );
}

fn target(id: &str, kind: UiKind, value: Value) -> UiTarget {
    UiTarget {
        doc: "0".into(),
        id: id.into(),
        kind,
        label: id.into(),
        value,
        form: None,
    }
}

#[test]
fn a_value_the_daemon_has_echoed_is_not_held_here_any_more() {
    let mut forms = Forms::default();
    let off = [target("n", UiKind::Switch, json!(false))];
    forms.enter("r", &off, 0);
    forms.key("r", &off, ch(' '), 0);
    assert_eq!(forms.doc_states("r")["0"].values["n"], json!(true));
    // Not echoed yet, or something else: held.
    forms.refresh("r", &off, 10);
    assert_eq!(forms.doc_states("r")["0"].values["n"], json!(true));
    let on = [target("n", UiKind::Switch, json!(true))];
    forms.refresh("r", &on, 20);
    assert!(forms.doc_states("r")["0"].values.is_empty());
    // A slider at 10 is 10.0 to the daemon.
    let slider = |v: Value| {
        target(
            "s",
            UiKind::Slider {
                min: 0.0,
                max: 50.0,
                step: 5.0,
            },
            v,
        )
    };
    let mut forms = Forms::default();
    let both = [slider(json!(5))];
    forms.enter("r", &both, 0);
    forms.key("r", &both, key(KeyCode::Right), 0);
    forms.refresh("r", &[slider(json!(10.0))], 5);
    assert!(forms.doc_states("r")["0"].values.is_empty());
}

#[test]
fn focus_moves_on_when_the_daemon_takes_the_focused_field_away() {
    let mut app = with_form();
    tabs(&mut app, 1);
    assert_eq!(focused(&app).as_deref(), Some("email"));
    // The form comes back without the email field.
    let mut gone = form_item("f1", false, "");
    if let suite_protocol::Component::TextComponent(t) = &mut gone.component
        && let suite_protocol::Block::OpenUiBlock(b) = &mut t.blocks[1]
        && let Some(root) = b.ui.root.as_mut()
        && let Some(children) = root.props.get_mut("children").and_then(Value::as_array_mut)
    {
        children.remove(1);
    }
    patch(&mut app, gone);
    assert_eq!(focused(&app).as_deref(), Some("service"));
}

fn snapshot(name: &str, actual: &str) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/snapshots")
        .join(format!("{name}.txt"));
    if std::env::var_os("UPDATE_SNAPSHOTS").is_some() {
        std::fs::write(&path, format!("{actual}\n")).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&path).unwrap_or_else(|_| {
        panic!("missing snapshot {name} (UPDATE_SNAPSHOTS=1 to write)\n{actual}")
    });
    assert_eq!(actual, expected.trim_end_matches('\n'), "snapshot {name}");
}

#[test]
fn screens() {
    let mut app = with_form();
    snapshot("openui-form-70x30", &screen(&mut app, 70, 30).join("\n"));
    tabs(&mut app, 1);
    typed(&mut app, "ops@x.io");
    press(&mut app, key(KeyCode::Tab));
    press(&mut app, key(KeyCode::Right));
    snapshot(
        "openui-form-typed-70x30",
        &screen(&mut app, 70, 30).join("\n"),
    );
    // A pane too short for the form shows the focused field.
    tabs(&mut app, 5);
    snapshot(
        "openui-form-short-70x12",
        &screen(&mut app, 70, 12).join("\n"),
    );
}
