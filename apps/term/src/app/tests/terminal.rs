use super::*;
use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};

/// A command `t{n}` that ran and wrote `lines` lines of output.
fn ran(app: &mut App, n: usize, lines: usize) {
    let id = format!("t{n}");
    let output: Vec<String> = (0..lines).map(|i| format!("{id} output {i}")).collect();
    let events = [
        SessionEvent::ToolCall(
            serde_json::from_value(json!({
                "toolCallId": id, "title": format!("bash: make {id}"), "kind": "execute",
                "status": "in_progress", "rawInput": {"command": format!("make {id}")}
            }))
            .unwrap(),
        ),
        SessionEvent::ToolCallUpdate(
            serde_json::from_value(json!({
                "toolCallId": id, "status": "completed",
                "content": [{"type": "text", "text": output.join("\n")}]
            }))
            .unwrap(),
        ),
    ];
    for (i, event) in events.into_iter().enumerate() {
        app.handle_event(record("s1", (n * 10 + i) as i64, NOW, event));
    }
}

fn mouse(kind: MouseEventKind, column: u16, row: u16) -> MouseEvent {
    MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    }
}

#[test]
fn dragging_over_the_terminal_panel_copies_its_text() {
    let mut app = app();
    ran(&mut app, 1, 3);
    let rows = screen(&mut app, 120, 24);
    let area = app.terminal_area.expect("the terminal panel is shown");
    let (y, line) = rows
        .iter()
        .enumerate()
        .find(|(y, l)| *y >= usize::from(area.y) && l.contains("t1 output 0"))
        .expect("its output is on screen");
    let x = line[..line.find("t1 output 0").unwrap()].chars().count() as u16;
    let y = y as u16;
    let left = MouseButton::Left;
    assert!(
        app.handle_mouse(mouse(MouseEventKind::Down(left), x, y))
            .is_empty()
    );
    app.handle_mouse(mouse(MouseEventKind::Drag(left), x + 3, y + 1));
    let fx = app.handle_mouse(mouse(MouseEventKind::Up(left), x + 3, y + 1));
    assert_eq!(fx, [Effect::Copy("t1 output 0\nt1 o".into())]);
    // A click alone copies nothing.
    app.handle_mouse(mouse(MouseEventKind::Down(left), x, y));
    assert!(
        app.handle_mouse(mouse(MouseEventKind::Up(left), x, y))
            .is_empty()
    );
}

#[test]
fn scrolling_the_terminal_panel_takes_the_chat_to_that_command() {
    let mut app = app();
    ran(&mut app, 1, 30);
    ran(&mut app, 2, 30);
    // The first command, a long reply, then the second: the chat ends on the second.
    let mut items = vec![item("c-t1", "tool", "make t1")];
    items.extend((0..40).map(|i| item(&format!("a{i}"), "agent", "a line of the reply")));
    items.push(item("c-t2", "tool", "make t2"));
    show(&mut app, items);
    screen(&mut app, 120, 24);
    assert!(app.chat.items_through_view().is_none(), "following the end");

    // Scrolled up the panel until the first command is on its bottom row: the chat goes to it.
    app.focus = Some(Focus::Panel(Panel::Terminal));
    for _ in 0..3 {
        app.handle_key(key(KeyCode::Home));
        screen(&mut app, 120, 24);
    }
    assert_eq!(app.widgets.terminal_bottom_run().as_deref(), Some("t1"));
    screen(&mut app, 120, 24);
    let seen = app.chat.items_through_view().expect("scrolled up");
    let last_command = seen.iter().rev().find(|i| i.r#ref.starts_with("c-"));
    assert_eq!(last_command.map(|i| i.r#ref.as_str()), Some("c-t1"));
    // The chat's line for it and the panel's rows for it carry the same bar.
    let rows = screen(&mut app, 120, 24);
    let line = rows
        .iter()
        .find(|r| r.contains("make t1") && !r.contains("$"))
        .expect("in chat");
    assert!(line.trim_start().starts_with('▎'), "{line:?}");

    // The chat scrolled again leads again: the panel follows it.
    app.focus = None;
    app.chat.follow();
    screen(&mut app, 120, 24);
    assert!(app.terminal_leads.is_none());
}
