//! Session events turned into plain-text lines, the way the Go CLI's `render.go` prints them.
//! No GPUI in here: the view just draws `lines()`. The real renderer arrives with Phase 4.

use std::collections::HashMap;

use suite_protocol::{
    ConnectorKind, SessionEvent, StopReason, ToolCallContent, ToolCallStatus, ToolCallUpdate,
};

/// How many lines of tool output are shown before truncating.
const MAX_TOOL_LINES: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    User,
    Agent,
    Tool,
    /// `[app connected …]`, `[focus …]`, turn errors.
    Note,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub kind: Kind,
    /// May span several rows (streamed text, tool output).
    pub text: String,
}

#[derive(Debug, Default)]
pub struct Transcript {
    lines: Vec<Line>,
    /// The message the last line is still being streamed into.
    streaming: Option<String>,
    /// Tool call id to the index of its line, so the update lands next to the call.
    tools: HashMap<String, usize>,
    running: bool,
}

impl Transcript {
    pub fn lines(&self) -> &[Line] {
        &self.lines
    }

    /// Whether a turn is in progress (a user message without its `turn_ended` yet).
    pub fn running(&self) -> bool {
        self.running
    }

    pub fn clear(&mut self) {
        *self = Transcript::default();
    }

    /// Adds one event. Events this version doesn't draw (thoughts, `harness_message`, unknown
    /// types) change nothing.
    pub fn push(&mut self, event: &SessionEvent) {
        match event {
            SessionEvent::UserMessage(message) => {
                self.running = true;
                let mut text = format!("» {}: {}", message.app, message.text);
                for image in message.images.iter().flatten() {
                    text.push_str(&format!("\n  [image: {image}]"));
                }
                self.add(Kind::User, text);
            }
            SessionEvent::AgentMessageChunk(chunk) => {
                if self.streaming.as_deref() == Some(chunk.message_id.as_str())
                    && let Some(last) = self.lines.last_mut()
                {
                    last.text.push_str(&chunk.text);
                } else {
                    self.add(Kind::Agent, chunk.text.clone());
                    self.streaming = Some(chunk.message_id.clone());
                }
            }
            SessionEvent::ToolCall(call) => {
                self.add(Kind::Tool, format!("⚙ {}", call.title));
                self.tools
                    .insert(call.tool_call_id.clone(), self.lines.len() - 1);
            }
            SessionEvent::ToolCallUpdate(update) => self.tool_update(update),
            SessionEvent::TurnEnded(ended) => {
                self.running = false;
                if ended.stop_reason != StopReason::EndTurn {
                    let reason = stop_reason(ended.stop_reason);
                    self.add(
                        Kind::Note,
                        match ended.error.as_deref() {
                            Some(error) if !error.is_empty() => {
                                format!("[turn {reason}: {error}]")
                            }
                            _ => format!("[turn {reason}]"),
                        },
                    );
                }
            }
            SessionEvent::AppConnected(app) => {
                let kind = match app.kind {
                    ConnectorKind::App => "app",
                    ConnectorKind::Service => "service",
                };
                let mut text = format!("[app connected: {} ({kind} v{})", app.app, app.v);
                if !app.tools.is_empty() {
                    text.push_str(&format!(": {}", app.tools.join(", ")));
                }
                text.push(']');
                self.add(Kind::Note, text);
            }
            SessionEvent::AppDisconnected(app) => {
                self.add(Kind::Note, format!("[app disconnected: {}]", app.app));
            }
            SessionEvent::FocusChanged(focus) => {
                let mut text = format!("[focus: {}", focus.app);
                // BTreeMap: sorted, so the line is stable.
                for (key, value) in focus.context.iter().flatten() {
                    text.push_str(&format!(" {key}={value}"));
                }
                text.push(']');
                self.add(Kind::Note, text);
            }
            SessionEvent::AgentThoughtChunk(_)
            | SessionEvent::HarnessMessage(_)
            | SessionEvent::Unknown(_) => {}
        }
    }

    fn add(&mut self, kind: Kind, text: String) {
        self.lines.push(Line { kind, text });
        self.streaming = None;
    }

    /// Shows how a tool call ended: ✓ or ✗ on its line, then its output. Progress updates
    /// without a final status are noise in a text transcript.
    fn tool_update(&mut self, update: &ToolCallUpdate) {
        let mark = match update.status {
            Some(ToolCallStatus::Completed) => "✓",
            Some(ToolCallStatus::Failed) => "✗",
            _ => return,
        };
        let mut details = String::new();
        for content in update.content.iter().flatten() {
            match content {
                ToolCallContent::TextContent(text) => tool_text(&mut details, &text.text),
                ToolCallContent::DiffContent(diff) => {
                    let (added, removed) =
                        diff_counts(diff.old_text.as_deref().unwrap_or(""), &diff.new_text);
                    details.push_str(&format!(
                        "\n    diff {} (+{added} -{removed} lines)",
                        diff.path
                    ));
                }
                ToolCallContent::Unknown(_) => {}
            }
        }
        match self.tools.get(&update.tool_call_id) {
            Some(&index) => {
                let line = &mut self.lines[index];
                line.text.push_str(&format!(" {mark}{details}"));
                // A second final update would stack; later ones can't reach the line again.
                self.tools.remove(&update.tool_call_id);
            }
            // The call scrolled out of what we saw (a subscription that started late).
            None => self.add(Kind::Tool, format!("  {mark}{details}")),
        }
    }
}

fn stop_reason(reason: StopReason) -> &'static str {
    match reason {
        StopReason::EndTurn => "end_turn",
        StopReason::MaxTokens => "max_tokens",
        StopReason::Cancelled => "cancelled",
        StopReason::Refusal => "refusal",
        StopReason::Error => "error",
    }
}

/// Appends indented output lines, cut after [`MAX_TOOL_LINES`].
fn tool_text(out: &mut String, text: &str) {
    let text = text.trim_end_matches('\n');
    if text.is_empty() {
        return;
    }
    let lines: Vec<&str> = text.split('\n').collect();
    for line in lines.iter().take(MAX_TOOL_LINES) {
        out.push_str(&format!("\n    {line}"));
    }
    if lines.len() > MAX_TOOL_LINES {
        out.push_str(&format!(
            "\n    … ({} more lines)",
            lines.len() - MAX_TOOL_LINES
        ));
    }
}

/// Approximates a diff by dropping the lines the two texts share at both ends and counting
/// the rest.
fn diff_counts(old: &str, new: &str) -> (usize, usize) {
    let lines = |text: &'_ str| -> Vec<String> {
        if text.is_empty() {
            return Vec::new();
        }
        text.strip_suffix('\n')
            .unwrap_or(text)
            .split('\n')
            .map(String::from)
            .collect()
    };
    let (old, new) = (lines(old), lines(new));
    let head = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
    let (old, new) = (&old[head..], &new[head..]);
    let tail = old
        .iter()
        .rev()
        .zip(new.iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    (new.len() - tail, old.len() - tail)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn event(value: serde_json::Value) -> SessionEvent {
        serde_json::from_value(value).expect("a session event")
    }

    fn chunk(id: &str, text: &str) -> SessionEvent {
        event(json!({"type": "agent_message_chunk", "messageId": id, "text": text}))
    }

    fn texts(transcript: &Transcript) -> Vec<&str> {
        transcript.lines().iter().map(|l| l.text.as_str()).collect()
    }

    fn feed(events: &[SessionEvent]) -> Transcript {
        let mut transcript = Transcript::default();
        for event in events {
            transcript.push(event);
        }
        transcript
    }

    #[test]
    fn user_message_and_streamed_reply() {
        let transcript = feed(&[
            event(json!({"type": "user_message", "app": "pad", "text": "hi",
                         "images": ["/tmp/a.png"]})),
            chunk("m1", "Hel"),
            chunk("m1", "lo"),
            chunk("m2", "Second"),
        ]);
        assert_eq!(
            texts(&transcript),
            ["» pad: hi\n  [image: /tmp/a.png]", "Hello", "Second"]
        );
        assert_eq!(transcript.lines()[0].kind, Kind::User);
        assert_eq!(transcript.lines()[1].kind, Kind::Agent);
    }

    #[test]
    fn a_message_resumed_after_a_tool_call_starts_a_new_line() {
        let transcript = feed(&[
            chunk("m1", "before"),
            event(
                json!({"type": "tool_call", "toolCallId": "t", "title": "ls",
                         "kind": "execute", "status": "pending"}),
            ),
            chunk("m1", "after"),
        ]);
        assert_eq!(texts(&transcript), ["before", "⚙ ls", "after"]);
    }

    #[test]
    fn tool_results_land_on_their_call() {
        let call = event(
            json!({"type": "tool_call", "toolCallId": "t1", "title": "run ls",
                                "kind": "execute", "status": "pending"}),
        );
        let progress = event(json!({"type": "tool_call_update", "toolCallId": "t1",
                                    "status": "in_progress"}));
        let output: String = (1..=12).map(|n| format!("line {n}\n")).collect();
        let done = event(json!({"type": "tool_call_update", "toolCallId": "t1",
                                "status": "completed",
                                "content": [{"type": "text", "text": output}]}));
        let transcript = feed(&[call, progress, done]);
        let [line] = transcript.lines() else {
            panic!("one line, got {:?}", transcript.lines());
        };
        assert!(
            line.text.starts_with("⚙ run ls ✓\n    line 1\n"),
            "{}",
            line.text
        );
        assert!(line.text.ends_with("    line 10\n    … (2 more lines)"));
    }

    #[test]
    fn failed_tool_and_unknown_call() {
        let transcript = feed(&[
            event(
                json!({"type": "tool_call", "toolCallId": "t", "title": "rm",
                         "kind": "delete", "status": "pending"}),
            ),
            event(json!({"type": "tool_call_update", "toolCallId": "t", "status": "failed"})),
            event(json!({"type": "tool_call_update", "toolCallId": "gone",
                         "status": "completed"})),
        ]);
        assert_eq!(texts(&transcript), ["⚙ rm ✗", "  ✓"]);
    }

    #[test]
    fn diffs_are_summarised() {
        let transcript = feed(&[
            event(
                json!({"type": "tool_call", "toolCallId": "t", "title": "edit",
                         "kind": "edit", "status": "pending"}),
            ),
            event(
                json!({"type": "tool_call_update", "toolCallId": "t", "status": "completed",
                         "content": [{"type": "diff", "path": "a.rs",
                                      "oldText": "a\nb\nc\n", "newText": "a\nB\nB2\nc\n"}]}),
            ),
        ]);
        assert_eq!(
            texts(&transcript),
            ["⚙ edit ✓\n    diff a.rs (+2 -1 lines)"]
        );
    }

    #[test]
    fn turn_end_and_running_state() {
        let mut transcript = Transcript::default();
        assert!(!transcript.running());
        transcript.push(&event(
            json!({"type": "user_message", "app": "cli", "text": "x"}),
        ));
        assert!(transcript.running());
        transcript.push(&event(
            json!({"type": "turn_ended", "stopReason": "end_turn"}),
        ));
        assert!(!transcript.running());
        assert_eq!(transcript.lines().len(), 1, "a normal end draws nothing");

        transcript.push(&event(
            json!({"type": "turn_ended", "stopReason": "error", "error": "boom"}),
        ));
        transcript.push(&event(
            json!({"type": "turn_ended", "stopReason": "cancelled"}),
        ));
        assert_eq!(
            &texts(&transcript)[1..],
            ["[turn error: boom]", "[turn cancelled]"]
        );
    }

    #[test]
    fn connector_and_focus_notes() {
        let transcript = feed(&[
            event(
                json!({"type": "app_connected", "app": "pad", "kind": "app", "v": 2,
                         "tools": ["open", "reveal"]}),
            ),
            event(
                json!({"type": "app_connected", "app": "lsp", "kind": "service", "v": 1,
                         "tools": []}),
            ),
            event(json!({"type": "app_disconnected", "app": "pad"})),
            event(json!({"type": "focus_changed", "app": "browser",
                         "context": {"url": "https://x", "tab": "1"}})),
        ]);
        assert_eq!(
            texts(&transcript),
            [
                "[app connected: pad (app v2): open, reveal]",
                "[app connected: lsp (service v1)]",
                "[app disconnected: pad]",
                "[focus: browser tab=1 url=https://x]",
            ]
        );
        assert!(transcript.lines().iter().all(|l| l.kind == Kind::Note));
    }

    #[test]
    fn ignored_events() {
        let transcript = feed(&[
            event(json!({"type": "harness_message", "harness": "pi", "message": {}})),
            event(json!({"type": "agent_thought_chunk", "messageId": "m", "text": "hmm"})),
            event(json!({"type": "from_the_future", "x": 1})),
        ]);
        assert!(transcript.lines().is_empty());
    }

    #[test]
    fn clear_forgets_everything() {
        let mut transcript = feed(&[
            chunk("m", "a"),
            event(json!({
            "type": "user_message", "app": "pad", "text": "x"})),
        ]);
        transcript.clear();
        assert!(transcript.lines().is_empty() && !transcript.running());
        transcript.push(&chunk("m", "b"));
        assert_eq!(texts(&transcript), ["b"]);
    }
}
