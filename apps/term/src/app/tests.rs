use neo_protocol::{
    CommandsAvailable, FocusReason, SessionEvent, SettingChanged, SettingName, SlashCommand,
    StopReason, TurnEnded, UserMessage, ViewItem, ViewOp, ViewPatch, ViewSnapshot,
};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use serde_json::json;

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

fn with(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
    KeyEvent {
        modifiers,
        ..key(code)
    }
}

fn ctrl(c: char) -> KeyEvent {
    with(KeyCode::Char(c), KeyModifiers::CONTROL)
}

fn type_str(app: &mut App, s: &str) {
    for c in s.chars() {
        assert!(app.handle_key(key(KeyCode::Char(c))).is_empty());
    }
}

fn item(r: &str, kind: &str, text: &str) -> ViewItem {
    serde_json::from_value(json!({
        "ref": r, "kind": kind,
        "component": {"type":"text","blocks":[{"type":"paragraph","spans":[{"text": text}]}]}
    }))
    .unwrap()
}

fn prompt(r: &str, state: &str, labels: &[&str]) -> ViewItem {
    let children: Vec<_> = labels
        .iter()
        .enumerate()
        .map(|(i, l)| {
            json!({"type":"button","label":l,"action":"neo.permit",
                   "args":{"id":r,"option":l},"key":(i + 1).to_string(),"note":true})
        })
        .collect();
    serde_json::from_value(json!({
        "ref": r, "kind": "tool", "label": "claude: run ls?", "state": state,
        "component": {"type":"row","children": children}
    }))
    .unwrap()
}

fn show(app: &mut App, items: Vec<ViewItem>) {
    app.handle_event(Event::ViewSnapshot(ViewSnapshot {
        before: None,
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
        ops: vec![ViewOp::SetViewOp(neo_protocol::SetViewOp {
            item,
            appended: None,
        })],
    }));
}

fn record(session: &str, seq: i64, ts: i64, event: SessionEvent) -> Event {
    Event::Session(SessionEventRecord {
        session: session.into(),
        seq,
        ts,
        event,
    })
}

fn user_message(ts: i64) -> Event {
    record(
        "s1",
        1,
        ts,
        SessionEvent::UserMessage(UserMessage {
            app: "terminal".into(),
            text: "hi".into(),
            shown: None,
            images: None,
        }),
    )
}

fn turn_ended() -> Event {
    record(
        "s1",
        2,
        NOW,
        SessionEvent::TurnEnded(TurnEnded {
            stop_reason: StopReason::EndTurn,
            error: None,
        }),
    )
}

fn setting(harness: &str, option: SettingName, value: &str) -> Event {
    record(
        "s1",
        3,
        NOW,
        SessionEvent::SettingChanged(SettingChanged {
            harness: harness.into(),
            option,
            value: value.into(),
        }),
    )
}

fn focus(session: &str, app: Option<&str>) -> Event {
    Event::Focus(AgentFocus {
        session: session.into(),
        app: app.map(Into::into),
        from: None,
        reason: FocusReason::Session,
    })
}

fn input(text: &str) -> Effect {
    Effect::Input {
        session: "s1".into(),
        text: text.into(),
        images: Vec::new(),
        steer: false,
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

fn screen_text(app: &mut App, w: u16, h: u16) -> String {
    screen(app, w, h).join("\n")
}

fn scenario() -> App {
    let mut app = app();
    show(
        &mut app,
        vec![
            item("u1", "user", "make the header sticky"),
            item(
                "a1",
                "agent",
                "Done. The header now uses `position: sticky`.",
            ),
        ],
    );
    app
}

// The tests, by the part of the app they are about.
mod cards;
mod choices;
mod drawing;
mod editing;
mod events;
mod files;
mod keys;
mod marks;
mod misc;
mod prompts;
mod sidebar;
mod terminal;

// Helpers of one part that others use too.
use cards::permission_with_diff;
use drawing::plan_event;
use events::sessions_command;
use keys::long_chat;
use sidebar::{info, overview};
