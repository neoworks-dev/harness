use neo_renderer_ratatui::Palette;
use ratatui::crossterm::event::{KeyEventState, KeyModifiers};
use serde_json::json;

use super::*;

fn rec(session: &str, seq: i64, ts: i64, event: serde_json::Value) -> SessionEventRecord {
    serde_json::from_value(json!({"session": session, "seq": seq, "ts": ts, "event": event}))
        .unwrap()
}

fn started(id: &str, agent: &str, ts: i64, extra: serde_json::Value) -> SessionEventRecord {
    let mut event = json!({"type": "subagent_started", "id": id, "agent": agent, "task": "do it"});
    event
        .as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    rec("s1", 1, ts, event)
}

fn ended(id: &str, ts: i64, stop: &str) -> SessionEventRecord {
    rec(
        "s1",
        9,
        ts,
        json!({"type": "subagent_ended", "id": id, "stopReason": stop}),
    )
}

fn styles() -> Styles {
    Styles::new(&Palette::default(), false)
}

fn text(lines: &[Line<'_>]) -> Vec<String> {
    lines.iter().map(|l| l.to_string()).collect()
}

fn press(code: KeyCode) -> KeyEvent {
    KeyEvent {
        code,
        modifiers: KeyModifiers::NONE,
        kind: KeyEventKind::Press,
        state: KeyEventState::NONE,
    }
}

/// `left`, then `right` flush against the `w`th column.
fn flush(left: &str, right: &str, w: usize) -> String {
    format!(
        "{left}{}{right}",
        " ".repeat(w - width(left) - width(right))
    )
}

const MAIN: MainRow<'static> = MainRow {
    harness: "claude",
    running: true,
};

#[test]
fn a_row_shows_the_agent_definition_it_runs_as() {
    let mut a = Agents::default();
    a.on_record(&started(
        "d1",
        "neo",
        1_000,
        json!({"name": "reviewer", "model": "deepseek/deepseek-v4-flash"}),
    ));
    // Its events are still labelled with the harness.
    a.on_record(&rec(
        "s1",
        2,
        2_000,
        json!({"type": "agent_thought_chunk", "messageId": "neo-1", "text": "hm", "agent": "neo"}),
    ));
    assert_eq!(a.get("d1").unwrap().activity, "thinking…");
    let got = text(&a.lines(80, 3_000, &MAIN, false, &styles()));
    assert_eq!(
        got,
        [
            "● main · claude".to_string(),
            flush("○ reviewer · deepseek-v4-flash  thinking…", "2s", 80)
        ]
    );
}

#[test]
fn a_row_says_what_the_subagent_does_how_long_and_its_tokens() {
    let mut a = Agents::default();
    a.on_record(&started(
        "d1",
        "neo",
        1_000,
        json!({"model": "deepseek/deepseek-v4-flash"}),
    ));
    // Nothing yet: it is starting.
    let now = 1_000 + 2_000;
    let got = text(&a.lines(80, now, &MAIN, false, &styles()));
    assert_eq!(
        got,
        [
            "● main · claude".to_string(),
            flush("○ neo · deepseek-v4-flash  starting…", "2s", 80)
        ]
    );

    a.on_record(&rec(
        "s1",
        2,
        2_000,
        json!({"type": "agent_thought_chunk", "messageId": "neo-1", "text": "hm", "agent": "neo"}),
    ));
    assert_eq!(a.get("d1").unwrap().activity, "thinking…");
    a.on_record(&rec(
        "s1",
        3,
        3_000,
        json!({
            "type": "tool_call", "toolCallId": "neo:t1", "title": "search tools: weather in Berlin",
            "kind": "search", "status": "in_progress", "agent": "neo"
        }),
    ));
    a.on_record(&rec(
        "s1",
        4,
        4_000,
        json!({
            "type": "usage", "agent": "neo", "inputTokens": 3000, "outputTokens": 412
        }),
    ));
    let now = 1_000 + 72_000;
    let got = text(&a.lines(80, now, &MAIN, false, &styles()));
    assert_eq!(
        got[1],
        flush(
            "○ neo · deepseek-v4-flash  search tools: weather in Berlin",
            "1m 12s · 3.4k tokens",
            80
        )
    );
    // Events of another agent's name don't touch it.
    a.on_record(&rec("s1", 5, 5_000, json!({"type": "tool_call", "toolCallId": "x:1", "title": "other", "kind": "other", "status": "pending", "agent": "claude"})));
    assert_eq!(
        a.get("d1").unwrap().activity,
        "search tools: weather in Berlin"
    );
    // The session's own agent's events aren't a subagent's either.
    a.on_record(&rec("s1", 6, 6_000, json!({"type": "tool_call", "toolCallId": "m", "title": "bash: ls", "kind": "execute", "status": "pending"})));
    assert_eq!(
        a.get("d1").unwrap().activity,
        "search tools: weather in Berlin"
    );
}

#[test]
fn a_finished_row_shows_its_outcome_for_five_seconds() {
    let mut a = Agents::default();
    a.on_record(&started("d1", "neo", 0, json!({})));
    a.on_record(&started("d2", "claude", 0, json!({"model": "sonnet"})));
    a.on_record(&started("d3", "codex", 0, json!({})));
    a.on_record(&ended("d1", 10_000, "end_turn"));
    a.on_record(&ended("d2", 11_000, "cancelled"));
    a.on_record(&rec("s1", 12, 12_000, json!({"type": "subagent_ended", "id": "d3", "stopReason": "end_turn", "error": "model refused"})));
    let at = |now| text(&a.lines(60, now, &MAIN, false, &styles()));
    assert_eq!(
        at(13_000),
        [
            "● main · claude".to_string(),
            flush("✓ neo  done", "10s", 60),
            flush("✗ claude · sonnet  stopped", "11s", 60),
            flush("✓ codex  done", "12s", 60),
        ]
    );
    assert!(a.visible(14_999));
    // neo's row is gone after five seconds, the others follow.
    assert_eq!(at(15_500).len(), 3);
    assert!(a.visible(16_000));
    assert!(!a.visible(17_100));
    assert_eq!(a.height(17_100, false), 0);
    assert!(at(17_100).is_empty());
}

#[test]
fn a_turn_that_ended_leaves_nothing_running() {
    let mut a = Agents::default();
    a.on_record(&started("d1", "neo", 0, json!({})));
    a.on_record(&rec(
        "s1",
        5,
        4_000,
        json!({"type": "turn_ended", "stopReason": "error", "error": "crash"}),
    ));
    let row = a.get("d1").unwrap();
    assert_eq!(
        (row.state, row.activity.as_str()),
        (AgentState::Failed, "interrupted")
    );
    assert_eq!(row.ended_ms, Some(4_000));
}

#[test]
fn a_background_subagent_runs_on_past_the_end_of_a_turn() {
    let mut a = Agents::default();
    a.on_record(&started("d1", "claude", 0, json!({"background": true})));
    a.on_record(&started("d2", "neo", 0, json!({})));
    a.on_record(&rec(
        "s1",
        5,
        4_000,
        json!({"type": "turn_ended", "stopReason": "end_turn"}),
    ));
    assert_eq!(a.get("d1").unwrap().state, AgentState::Running);
    assert_eq!(a.get("d2").unwrap().state, AgentState::Failed);
    let got = text(&a.lines(80, 5_000, &MAIN, false, &styles()));
    assert!(got[1].starts_with("○ claude [bg]  starting…"), "{got:?}");
    // It ends when it says so.
    a.on_record(&ended("d1", 6_000, "end_turn"));
    assert_eq!(a.get("d1").unwrap().state, AgentState::Done);
}

#[test]
fn a_message_shows_as_the_activity_of_the_subagent_it_is_to_or_from() {
    let mut a = Agents::default();
    a.on_record(&started("d1", "claude", 0, json!({"background": true})));
    let message = |from: &str, to: &str| {
        rec(
            "s1",
            3,
            100,
            json!({"type": "agent_message", "from": from, "to": to, "text": "hi"}),
        )
    };
    a.on_record(&message("main", "d1"));
    assert_eq!(a.get("d1").unwrap().activity, "← main");
    a.on_record(&message("d1", "main"));
    assert_eq!(a.get("d1").unwrap().activity, "→ main");
    // Between two other agents: nothing here.
    a.on_record(&message("d8", "d9"));
    assert_eq!(a.get("d1").unwrap().activity, "→ main");
}

#[test]
fn a_subagent_with_a_session_of_its_own_is_left_to_the_sidebar() {
    let mut a = Agents::default();
    a.on_record(&started(
        "d1",
        "claude",
        0,
        json!({"cwd": "/w/site/.worktrees/fix-login", "child": "s_child"}),
    ));
    assert!(a.get("d1").is_none());
    assert!(!a.visible(1_000));
    assert_eq!(a.height(1_000, false), 0);
}

#[test]
fn keys_select_open_and_stop() {
    let mut a = Agents::default();
    a.on_record(&started("d1", "neo", 0, json!({})));
    a.on_record(&started("d2", "claude", 0, json!({})));
    a.on_record(&started("d3", "codex", 0, json!({})));
    a.on_record(&ended("d3", 100, "end_turn"));
    let now = 1_000;
    // Starts on the latest running one.
    a.focus(now);
    assert_eq!(
        a.key(press(KeyCode::Enter), now),
        AgentsAction::Transcript("d2".into())
    );
    a.key(press(KeyCode::Up), now);
    assert_eq!(
        a.key(press(KeyCode::Enter), now),
        AgentsAction::Transcript("d1".into())
    );
    assert_eq!(
        a.key(press(KeyCode::Char('x')), now),
        AgentsAction::Stop("d1".into())
    );
    // Up stops at the top, Down goes through the finished one and stops at the end.
    a.key(press(KeyCode::Up), now);
    assert_eq!(
        a.key(press(KeyCode::Char('x')), now),
        AgentsAction::Stop("d1".into())
    );
    a.key(press(KeyCode::Down), now);
    a.key(press(KeyCode::Down), now);
    a.key(press(KeyCode::Down), now);
    // A finished subagent can't be stopped; its transcript can be opened.
    assert_eq!(a.key(press(KeyCode::Char('x')), now), AgentsAction::Redraw);
    assert_eq!(
        a.key(press(KeyCode::Enter), now),
        AgentsAction::Transcript("d3".into())
    );
    assert_eq!(a.key(press(KeyCode::Esc), now), AgentsAction::Leave);
    // Keys with Ctrl belong to the app.
    let ctrl_x = KeyEvent {
        modifiers: KeyModifiers::CONTROL,
        ..press(KeyCode::Char('x'))
    };
    assert_eq!(a.key(ctrl_x, now), AgentsAction::Redraw);
}

#[test]
fn the_focused_list_has_its_keys_and_a_reversed_selection() {
    let mut a = Agents::default();
    a.on_record(&started("d1", "neo", 0, json!({})));
    a.on_record(&started("d2", "claude", 0, json!({})));
    a.focus(1_000);
    let lines = a.lines(60, 1_000, &MAIN, true, &styles());
    assert_eq!(lines.len(), a.height(1_000, true));
    assert_eq!(
        lines.last().unwrap().to_string(),
        "↑/↓ select · enter open · x stop · esc back"
    );
    let reversed: Vec<bool> = lines
        .iter()
        .map(|l| {
            l.spans
                .iter()
                .any(|s| s.style.add_modifier.contains(Modifier::REVERSED))
        })
        .collect();
    assert_eq!(reversed, [false, false, true, false]);
}

#[test]
fn many_subagents_fold_and_nothing_overflows() {
    let mut a = Agents::default();
    for i in 0..9 {
        a.on_record(&started(
            &format!("d{i}"),
            "neo",
            0,
            json!({"model": "a-very-long-model-name-indeed"}),
        ));
    }
    a.on_record(&rec("s1", 2, 1, json!({"type": "tool_call", "toolCallId": "t", "title": "日本語の長いタイトルがここにあります", "kind": "other", "status": "pending", "agent": "neo"})));
    let lines = a.lines(60, 100_000, &MAIN, true, &styles());
    assert_eq!(lines.len(), a.height(100_000, true));
    assert!(text(&lines).contains(&"  … +3 more".to_string()));
    for w in [12, 20, 40, 60, 120] {
        for line in a.lines(w, 100_000, &MAIN, true, &styles()) {
            assert!(line.width() <= w, "{w}: {line}");
        }
    }
}

#[test]
fn durations_and_counts_are_short() {
    assert_eq!(elapsed(0), "0s");
    assert_eq!(elapsed(59_999), "59s");
    assert_eq!(elapsed(72_000), "1m 12s");
    assert_eq!(elapsed(3_723_000), "1h 02m");
    assert_eq!(compact(812), "812");
    assert_eq!(compact(3_412), "3.4k");
    assert_eq!(compact(1_250_000), "1.2M");
}
