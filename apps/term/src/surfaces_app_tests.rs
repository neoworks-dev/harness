//! Surfaces in the app, headless: the strip over the input, a page in the chat's place, and the
//! keys that reach their fields and buttons.

use neo_protocol::{ViewItem, ViewSnapshot};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyEventState, KeyModifiers};
use serde_json::{Value, json};

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

fn text(app: &mut App) -> String {
    let (w, h) = (80, 30);
    let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
    terminal.draw(|f| app.draw(f)).unwrap();
    let buffer = terminal.backend().buffer();
    (0..h)
        .map(|y| {
            (0..w)
                .map(|x| buffer[(x, y)].symbol().to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn node(component: &str, props: Value) -> Value {
    json!({"component": component, "props": props})
}

fn surface(name: &str, place: &str, title: &str, root: Value) -> ViewItem {
    serde_json::from_value(json!({
        "ref": format!("s-{name}"), "kind": "tool", "place": place, "label": title,
        "component": {"type": "neoui", "ui": {"id": "0", "root": root}}
    }))
    .unwrap()
}

fn agent(r: &str, said: &str) -> ViewItem {
    serde_json::from_value(json!({
        "ref": r, "kind": "agent",
        "component": {"type": "text", "blocks": [{"type": "paragraph", "spans": [{"text": said}]}]}
    }))
    .unwrap()
}

fn show(app: &mut App, surfaces: Vec<ViewItem>) {
    app.handle_event(Event::ViewSnapshot(ViewSnapshot {
        surfaces: Some(surfaces),
        before: None,
        agent: None,
        session: "s1".into(),
        seq: 1,
        items: vec![agent("m1", "the chat says hello")],
    }));
}

fn knobs() -> ViewItem {
    surface(
        "knobs",
        "page",
        "Knobs",
        node(
            "Stack",
            json!({"children": [
                node("Header", json!({"title": "Learning rate"})),
                {"component": "Button", "key": "apply", "props": {"label": "Apply"}},
            ]}),
        ),
    )
}

#[test]
fn a_bar_is_drawn_over_the_input_under_its_title_and_the_chat_stays() {
    let mut app = app();
    show(
        &mut app,
        vec![surface(
            "run",
            "bottom",
            "Training",
            node("Header", json!({"title": "epoch 3 of 8"})),
        )],
    );
    let screen = text(&mut app);
    assert!(screen.contains("the chat says hello"));
    assert!(screen.contains("Training"));
    assert!(screen.contains("epoch 3 of 8"));
}

#[test]
fn a_page_that_opens_takes_the_chat_s_place_until_esc_and_its_line_brings_it_back() {
    let mut app = app();
    show(&mut app, vec![]);
    // Opened after the session was shown: it is new, so it shows.
    app.handle_event(Event::ViewPatch(neo_protocol::ViewPatch {
        agent: None,
        session: "s1".into(),
        seq: 2,
        ops: vec![neo_protocol::ViewOp::SetViewOp(neo_protocol::SetViewOp {
            item: knobs(),
            appended: None,
        })],
    }));
    let page = text(&mut app);
    assert!(page.contains("Learning rate"));
    assert!(!page.contains("the chat says hello"));

    app.handle_key(key(KeyCode::Esc));
    let chat = text(&mut app);
    assert!(chat.contains("the chat says hello"));
    assert!(chat.contains("▸ Knobs (page)"));

    // Tab to the strip, Enter on the page's line: the page is back.
    app.handle_key(key(KeyCode::Tab));
    assert_eq!(app.focus, Some(Focus::Surfaces));
    app.handle_key(key(KeyCode::Enter));
    assert!(text(&mut app).contains("Learning rate"));
}

#[test]
fn a_surface_s_button_is_pressed_from_the_keys_and_sent_with_its_ref() {
    let mut app = app();
    show(&mut app, vec![knobs()]);
    // A session shown with the page open: it waits as a line, not in the chat's place.
    assert!(text(&mut app).contains("▸ Knobs (page)"));
    app.handle_key(key(KeyCode::Tab));
    app.handle_key(key(KeyCode::Enter));
    text(&mut app);
    // The page shown has the keys; its button is the first target.
    let effects = app.handle_key(key(KeyCode::Enter));
    assert!(effects.iter().any(|e| matches!(
        e,
        Effect::Action { item, action, .. } if item == "s-knobs" && action == "neoui.press"
    )));
}

#[test]
fn ctrl_d_closes_the_focused_surface_through_the_daemon() {
    let mut app = app();
    show(
        &mut app,
        vec![surface(
            "run",
            "bottom",
            "Training",
            node("Header", json!({"title": "epoch 3"})),
        )],
    );
    text(&mut app);
    app.handle_key(key(KeyCode::Tab));
    assert_eq!(app.focus, Some(Focus::Surfaces));
    assert!(text(&mut app).contains("ctrl-d closes"));
    let effects = app.handle_key(KeyEvent {
        modifiers: KeyModifiers::CONTROL,
        ..key(KeyCode::Char('d'))
    });
    assert!(effects.iter().any(|e| matches!(
        e,
        Effect::Action { item, action, .. } if item == "s-run" && action == "neo.surface.close"
    )));
    assert_eq!(app.focus, None);
}
