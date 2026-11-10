//! The masked field for `secret.request`, inside the app and drawn into a test backend.

use neo_protocol::{SecretRequest, SessionState};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::KeyEventState;

use super::*;

const NOW: i64 = 1_000_000;
const KEY: &str = "sk-live-0123456789abcdef";

fn app_with(history: HistoryStore) -> App {
    let mut app = App::new(Config {
        app_id: "terminal".into(),
        session: "s1".into(),
        cwd: "/home/me/site".into(),
        home: Some("/home/me".into()),
        no_color: false,
        history,
        prefs: crate::theme::Prefs::default(),
    });
    app.tick(NOW);
    app.awaiting = None;
    app.last_poll_ms = NOW;
    app.turn_started_ms = NOW;
    app
}

fn app() -> App {
    app_with(HistoryStore::default())
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent {
        code,
        modifiers: KeyModifiers::NONE,
        kind: KeyEventKind::Press,
        state: KeyEventState::NONE,
    }
}

fn ctrl(c: char) -> KeyEvent {
    KeyEvent {
        modifiers: KeyModifiers::CONTROL,
        ..key(KeyCode::Char(c))
    }
}

fn type_str(app: &mut App, s: &str) {
    for c in s.chars() {
        assert!(app.handle_key(key(KeyCode::Char(c))).is_empty());
    }
}

fn screen_text(app: &mut App, w: u16, h: u16) -> String {
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
        .collect::<Vec<_>>()
        .join("\n")
}

fn ask(id: &str, session: &str, placeholder: Option<&str>) -> Event {
    Event::Secret(SecretRequest {
        id: id.into(),
        session: session.into(),
        title: "DeepSeek API key".into(),
        placeholder: placeholder.map(Into::into),
    })
}

fn answered(id: &str, value: Option<&str>) -> Effect {
    Effect::Secret {
        id: id.into(),
        value: value.map(|v| SecretValue::new(v.to_string())),
    }
}

#[test]
fn a_request_opens_a_masked_field_that_never_shows_what_is_typed() {
    let mut app = app();
    assert!(app.handle_event(ask("s1", "s1", Some("sk-…"))).is_empty());
    let empty = screen_text(&mut app, 70, 14);
    assert!(empty.contains("DeepSeek API key"), "{empty}");
    assert!(empty.contains("sk-…"), "{empty}");
    type_str(&mut app, KEY);
    let typed = screen_text(&mut app, 70, 14);
    assert!(!typed.contains("sk-…"), "the placeholder goes: {typed}");
    for part in ["sk-", "live", "0123", "abcdef"] {
        assert!(!typed.contains(part), "{part} in {typed}");
    }
    // Not in the prompt editor either.
    assert_eq!(app.editor_text(), "");
}

#[test]
fn enter_answers_with_the_value_and_esc_without() {
    let mut app = app();
    app.handle_event(ask("s7", "s1", None));
    type_str(&mut app, "  abc");
    app.handle_key(key(KeyCode::Backspace));
    type_str(&mut app, "d ");
    let fx = app.handle_key(key(KeyCode::Enter));
    // Trimmed; Backspace edits.
    assert_eq!(fx, [answered("s7", Some("abd"))]);
    assert!(app.secret.is_none());
    assert_eq!(app.editor_text(), "");

    app.handle_event(ask("s8", "s1", None));
    type_str(&mut app, "abc");
    assert_eq!(app.handle_key(key(KeyCode::Esc)), [answered("s8", None)]);
    assert!(app.secret.is_none());
    // Ctrl-C cancels it too, and does not arm quitting.
    app.handle_event(ask("s9", "s1", None));
    assert_eq!(app.handle_key(ctrl('c')), [answered("s9", None)]);
    assert!(app.handle_key(ctrl('c')).is_empty());
}

#[test]
fn the_sent_value_is_what_was_typed() {
    let mut app = app();
    app.handle_event(ask("s1", "s1", None));
    type_str(&mut app, KEY);
    let fx = app.handle_key(key(KeyCode::Enter));
    let [
        Effect::Secret {
            id,
            value: Some(value),
        },
    ] = fx.as_slice()
    else {
        panic!("{fx:?}");
    };
    assert_eq!(id, "s1");
    assert_eq!(value.reveal(), KEY);
}

#[test]
fn an_empty_field_does_not_submit_and_ctrl_u_starts_over() {
    let mut app = app();
    app.handle_event(ask("s1", "s1", None));
    assert!(app.handle_key(key(KeyCode::Enter)).is_empty());
    assert!(app.secret.is_some());
    type_str(&mut app, "wrong");
    app.handle_key(ctrl('u'));
    type_str(&mut app, "right");
    assert_eq!(
        app.handle_key(key(KeyCode::Enter)),
        [answered("s1", Some("right"))]
    );
}

#[test]
fn the_field_takes_every_key_and_paste_but_leaves_the_editor_alone() {
    let mut app = app();
    type_str(&mut app, "draft");
    app.handle_event(ask("s1", "s1", None));
    // Keys that mean something elsewhere are only text or nothing here.
    for k in [
        key(KeyCode::Up),
        key(KeyCode::Tab),
        key(KeyCode::PageUp),
        ctrl('g'),
    ] {
        assert!(app.handle_key(k).is_empty());
    }
    type_str(&mut app, "/?");
    app.handle_paste("sk-pasted\n");
    assert_eq!(app.editor_text(), "draft");
    let fx = app.handle_key(key(KeyCode::Enter));
    assert_eq!(fx, [answered("s1", Some("/?sk-pasted"))]);
    assert_eq!(app.editor_text(), "draft");
}

#[test]
fn a_second_request_replaces_the_first_which_is_answered_as_cancelled() {
    let mut app = app();
    app.handle_event(ask("a", "s1", None));
    type_str(&mut app, "first");
    assert_eq!(
        app.handle_event(ask("b", "s1", None)),
        [answered("a", None)]
    );
    // The new field starts empty.
    assert!(app.handle_key(key(KeyCode::Enter)).is_empty());
    type_str(&mut app, "second");
    assert_eq!(
        app.handle_key(key(KeyCode::Enter)),
        [answered("b", Some("second"))]
    );
}

#[test]
fn another_session_is_cancelled_a_lost_connection_and_a_switch_drop_it() {
    let mut app = app();
    assert_eq!(
        app.handle_event(ask("x", "other", None)),
        [answered("x", None)]
    );
    assert!(app.secret.is_none());

    app.handle_event(ask("y", "s1", None));
    type_str(&mut app, "abc");
    assert!(app.handle_event(Event::Connection(false)).is_empty());
    assert!(app.secret.is_none());
    assert!(app.handle_key(key(KeyCode::Enter)).is_empty());

    app.handle_event(ask("z", "s1", None));
    let info = SessionInfo {
        harness: None,
        usage: None,
        id: "s2".into(),
        title: None,
        cwd: "/w".into(),
        created_at: 0,
        last_activity: NOW,
        last_seq: 1,
        state: SessionState::Idle,
        queued: 0,
        waiting: None,
        asking: None,
        waiting_on: None,
        unseen: None,
        preview: None,
        added: None,
        removed: None,
        parent: None,
    };
    let mut fx = Vec::new();
    app.open_session(info, &mut fx);
    assert_eq!(fx[0], answered("z", None));
    assert!(app.secret.is_none());
}

#[test]
fn a_secret_stays_out_of_history_effects_debug_and_frames() {
    let dir = std::env::temp_dir().join(format!("neo-term-secret-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut app = app_with(HistoryStore::new(Some(dir.clone())));
    // One real prompt so the history file exists to look in.
    type_str(&mut app, "hello");
    app.handle_key(key(KeyCode::Enter));
    app.handle_event(ask("s1", "s1", Some("sk-…")));
    type_str(&mut app, KEY);
    let mut frames = String::new();
    for (w, h) in [(70, 14), (30, 8), (100, 30)] {
        frames.push_str(&screen_text(&mut app, w, h));
    }
    assert!(!format!("{:?}", app.secret).contains("sk-live"));
    let fx = app.handle_key(key(KeyCode::Enter));
    assert!(!format!("{fx:?}").contains("sk-live"), "{fx:?}");
    assert!(!frames.contains("sk-live") && !frames.contains("0123456789"));
    // Up through history gives "hello" and nothing else.
    app.handle_key(key(KeyCode::Up));
    assert_eq!(app.editor_text(), "hello");
    app.handle_key(key(KeyCode::Up));
    assert_eq!(app.editor_text(), "hello");
    let mut on_disk = String::new();
    for entry in std::fs::read_dir(&dir).unwrap() {
        on_disk.push_str(&std::fs::read_to_string(entry.unwrap().path()).unwrap());
    }
    assert!(on_disk.contains("hello"));
    assert!(
        !on_disk.contains("sk-live") && !on_disk.contains("0123"),
        "{on_disk}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
