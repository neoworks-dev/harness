//! The subagents under the prompt bar, like Claude Code's agent list: while a delegated task
//! runs (and a few seconds after it ends) one row per subagent below the main agent's, with what
//! it is doing, how long it has run and its tokens; arrows select one, Enter opens its transcript
//! (or, for a subagent working as a session of its own in another directory, that session), `x`
//! stops it. The rows are built from the session's log: `subagent_started` / `subagent_ended`
//! mark a delegation, and the events labelled with the subagent's name (or, for a session of its
//! own, the events of that session) say what it does. One started in the background lives on past
//! the turn that started it (`[bg]` after its name); a message to or from a subagent shows as its
//! activity (`← main`, `→ main`).

use neo_protocol::{SessionEvent, SessionEventRecord, StopReason};
use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState};

use crate::chat::{Chat, ChatOpts};
use crate::style::Styles;
use crate::util::{fit_line, truncate, width};

/// How long a finished subagent stays in the list.
pub const LINGER_MS: i64 = 5000;
/// Subagent rows shown before the rest is folded into "+N more".
const MAX_ROWS: usize = 6;
/// Lines Up / Down scroll a transcript by a page key.
const WHEEL: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentState {
    Running,
    Done,
    /// Failed or ended with an error.
    Failed,
    /// Stopped by the user (or by a cancelled turn).
    Stopped,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Agent {
    pub id: String,
    /// Harness name: neo, claude, codex…
    pub harness: String,
    /// The agent definition it runs as (a reviewer on neo), shown instead of the harness.
    pub name: Option<String>,
    pub model: Option<String>,
    pub task: String,
    /// The directory it works in, when that isn't the session's.
    pub cwd: Option<String>,
    /// The session it runs as, when it works in a directory of its own.
    pub child: Option<String>,
    /// Started with `background`: it keeps running when the turn that started it ends.
    pub background: bool,
    pub state: AgentState,
    pub started_ms: i64,
    pub ended_ms: Option<i64>,
    /// Its latest tool call, or what it does between them.
    pub activity: String,
    pub tokens: Option<i64>,
}

impl Agent {
    /// What the list calls it: the agent definition it runs as, else its harness.
    pub fn label(&self) -> &str {
        self.name.as_deref().unwrap_or(&self.harness)
    }
}

/// What a key in the focused list asks the app to do.
#[derive(Debug, Clone, PartialEq)]
pub enum AgentsAction {
    Redraw,
    /// Back to the editor (Esc).
    Leave,
    /// Show this subagent's transcript (Enter).
    Transcript(String),
    /// Switch to the session this subagent runs as (Enter on one in a directory of its own).
    Session(String),
    /// Stop this subagent (`x`).
    Stop(String),
}

/// The session's own agent, the first row.
pub struct MainRow<'a> {
    pub harness: &'a str,
    pub running: bool,
}

#[derive(Debug, Default)]
pub struct Agents {
    rows: Vec<Agent>,
    /// The row the keys are on while the list has focus.
    selected: Option<String>,
    /// Sessions of subagents in directories of their own whose events should be followed.
    wanted: Vec<String>,
}

impl Agents {
    pub fn rows(&self) -> &[Agent] {
        &self.rows
    }

    pub fn get(&self, id: &str) -> Option<&Agent> {
        self.rows.iter().find(|a| a.id == id)
    }

    /// The sessions of subagents with a directory of their own, once each: the app subscribes
    /// to them to see what they do. Cleared when read.
    pub fn take_wanted(&mut self) -> Vec<String> {
        std::mem::take(&mut self.wanted)
    }

    /// Whether an event of this session is one of a subagent's.
    pub fn follows(&self, session: &str) -> bool {
        self.rows
            .iter()
            .any(|a| a.child.as_deref() == Some(session))
    }

    fn shown(&self, now_ms: i64) -> Vec<&Agent> {
        self.rows
            .iter()
            .filter(|a| a.ended_ms.is_none_or(|end| now_ms - end < LINGER_MS))
            .collect()
    }

    /// Whether the list is on screen: a subagent runs, or one ended a moment ago.
    pub fn visible(&self, now_ms: i64) -> bool {
        !self.shown(now_ms).is_empty()
    }

    /// The seconds on screen change: whether a repaint is due since `last_ms`.
    pub fn ticks(&self, now_ms: i64, last_ms: i64) -> bool {
        self.visible(now_ms) && now_ms / 1000 != last_ms / 1000
    }

    /// Rows the list takes: the main agent's, the subagents' (folded after a few), the keys.
    pub fn height(&self, now_ms: i64, focused: bool) -> usize {
        let n = self.shown(now_ms).len();
        if n == 0 {
            return 0;
        }
        let folded = usize::from(n > MAX_ROWS);
        1 + n.min(MAX_ROWS) + folded + usize::from(focused)
    }

    // ---- the log ----

    /// An event of the shown session.
    pub fn on_record(&mut self, record: &SessionEventRecord) {
        match &record.event {
            SessionEvent::SubagentStarted(e) => {
                if self.get(&e.id).is_some() {
                    return;
                }
                if let Some(child) = &e.child {
                    self.wanted.push(child.clone());
                }
                self.rows.push(Agent {
                    id: e.id.clone(),
                    harness: e.agent.clone(),
                    name: e.name.clone(),
                    model: e.model.clone(),
                    task: e.task.clone(),
                    cwd: e.cwd.clone(),
                    child: e.child.clone(),
                    background: e.background.unwrap_or(false),
                    state: AgentState::Running,
                    started_ms: record.ts,
                    ended_ms: None,
                    activity: "starting…".into(),
                    tokens: None,
                });
            }
            SessionEvent::SubagentEnded(e) => {
                let state = match e.stop_reason {
                    StopReason::EndTurn => AgentState::Done,
                    StopReason::Cancelled => AgentState::Stopped,
                    _ => AgentState::Failed,
                };
                if let Some(agent) = self.rows.iter_mut().find(|a| a.id == e.id) {
                    agent.state = state;
                    agent.ended_ms = Some(record.ts);
                    agent.activity = match (state, &e.error) {
                        (AgentState::Done, _) => "done".into(),
                        (AgentState::Stopped, _) => "stopped".into(),
                        (_, Some(error)) => error.clone(),
                        (_, None) => "failed".into(),
                    };
                }
            }
            // A delegation lives inside a turn: when the turn is over nothing still runs, but for
            // the ones that were started in the background.
            SessionEvent::TurnEnded(_) => {
                for agent in self
                    .rows
                    .iter_mut()
                    .filter(|a| a.state == AgentState::Running && !a.background)
                {
                    agent.state = AgentState::Failed;
                    agent.ended_ms = Some(record.ts);
                    agent.activity = "interrupted".into();
                }
            }
            SessionEvent::AgentMessage(m) => {
                for (id, arrow, other) in [(&m.to, "←", &m.from), (&m.from, "→", &m.to)] {
                    if let Some(agent) = self
                        .rows
                        .iter_mut()
                        .find(|a| a.id == *id && a.state == AgentState::Running)
                    {
                        agent.activity = format!("{arrow} {other}");
                    }
                }
            }
            SessionEvent::ToolCall(c) => self.note(c.agent.as_deref(), Some(&c.title), None),
            SessionEvent::AgentThoughtChunk(c) => self.note(c.agent.as_deref(), None, None),
            SessionEvent::AgentMessageChunk(c) => self.note(c.agent.as_deref(), None, None),
            SessionEvent::Usage(u) => {
                let tokens = match (u.input_tokens, u.output_tokens) {
                    (None, None) => u.context_used,
                    (i, o) => Some(i.unwrap_or(0) + o.unwrap_or(0)),
                };
                self.note(u.agent.as_deref(), None, tokens);
            }
            _ => {}
        }
    }

    /// An event of a subagent's own session: its agent is unlabelled there.
    pub fn on_child_record(&mut self, record: &SessionEventRecord) {
        let Some(agent) = self
            .rows
            .iter_mut()
            .rfind(|a| a.child.as_deref() == Some(record.session.as_str()))
        else {
            return;
        };
        match &record.event {
            SessionEvent::ToolCall(c) if c.agent.is_none() => agent.activity = c.title.clone(),
            SessionEvent::AgentThoughtChunk(c) if c.agent.is_none() => {
                agent.activity = "thinking…".into();
            }
            SessionEvent::AgentMessageChunk(c) if c.agent.is_none() => {
                agent.activity = "thinking…".into();
            }
            SessionEvent::Usage(u) if u.agent.is_none() => {
                agent.tokens = match (u.input_tokens, u.output_tokens) {
                    (None, None) => u.context_used,
                    (i, o) => Some(i.unwrap_or(0) + o.unwrap_or(0)),
                };
            }
            _ => {}
        }
    }

    /// What a labelled event says about the running subagent of that name.
    fn note(&mut self, label: Option<&str>, call: Option<&str>, tokens: Option<i64>) {
        let Some(label) = label else {
            return;
        };
        let Some(agent) = self
            .rows
            .iter_mut()
            .rfind(|a| a.harness == label && a.state == AgentState::Running && a.child.is_none())
        else {
            return;
        };
        match (call, tokens) {
            (Some(title), _) => agent.activity = title.to_string(),
            (None, Some(n)) => agent.tokens = Some(n),
            (None, None) => agent.activity = "thinking…".into(),
        }
    }

    // ---- keys ----

    /// The list gets focus: the keys start on the latest running subagent.
    pub fn focus(&mut self, now_ms: i64) {
        let shown = self.shown(now_ms);
        self.selected = shown
            .iter()
            .rfind(|a| a.state == AgentState::Running)
            .or(shown.last())
            .map(|a| a.id.clone());
    }

    /// Whether the keys are on the last row (Down goes on from there).
    pub fn on_last(&self, now_ms: i64) -> bool {
        let shown = self.shown(now_ms);
        shown.last().map(|a| &a.id) == self.selected.as_ref()
    }

    fn move_by(&mut self, delta: isize, now_ms: i64) {
        let ids: Vec<String> = self.shown(now_ms).iter().map(|a| a.id.clone()).collect();
        if ids.is_empty() {
            return;
        }
        let at = self
            .selected
            .as_ref()
            .and_then(|s| ids.iter().position(|i| i == s))
            .unwrap_or(0) as isize;
        let to = (at + delta).clamp(0, ids.len() as isize - 1) as usize;
        self.selected = Some(ids[to].clone());
    }

    pub fn key(&mut self, key: KeyEvent, now_ms: i64) -> AgentsAction {
        if key.kind == KeyEventKind::Release || key.modifiers.contains(KeyModifiers::CONTROL) {
            return AgentsAction::Redraw;
        }
        // The selection can be a row that has gone: start again from the last one.
        let current = self
            .selected
            .as_deref()
            .and_then(|id| self.shown(now_ms).into_iter().find(|a| a.id == id).cloned());
        let first = self.shown(now_ms).first().map(|a| a.id.clone());
        match key.code {
            // Up from the top row goes back to the editor, as Down came from it.
            KeyCode::Up | KeyCode::Char('k') if current.is_none() || self.selected == first => {
                return AgentsAction::Leave;
            }
            KeyCode::Up | KeyCode::Char('k') => self.move_by(-1, now_ms),
            KeyCode::Down | KeyCode::Char('j') => self.move_by(1, now_ms),
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Left | KeyCode::Char('h') => {
                return AgentsAction::Leave;
            }
            KeyCode::Enter => {
                return match current {
                    Some(a) => match a.child {
                        Some(child) => AgentsAction::Session(child),
                        None => AgentsAction::Transcript(a.id),
                    },
                    None => AgentsAction::Leave,
                };
            }
            KeyCode::Char('x') => {
                if let Some(a) = current.filter(|a| a.state == AgentState::Running) {
                    return AgentsAction::Stop(a.id);
                }
            }
            _ => {}
        }
        AgentsAction::Redraw
    }

    // ---- drawing ----

    /// The list's lines for `w` columns: the main agent, then the subagents.
    pub fn lines(
        &self,
        w: usize,
        now_ms: i64,
        main: &MainRow,
        focused: bool,
        styles: &Styles,
    ) -> Vec<Line<'static>> {
        let shown = self.shown(now_ms);
        if shown.is_empty() || w < 12 {
            return Vec::new();
        }
        let glyph_style = if main.running {
            styles.accent
        } else {
            styles.muted
        };
        let mut out = vec![Line::from(vec![
            Span::styled("●", glyph_style),
            Span::raw(" main"),
            Span::styled(format!(" · {}", main.harness), styles.muted),
        ])];
        for agent in shown.iter().take(MAX_ROWS) {
            let selected = focused && self.selected.as_deref() == Some(agent.id.as_str());
            let mut line = row(agent, w, now_ms, styles);
            if selected {
                let pad = w.saturating_sub(line.width());
                line.spans.push(Span::raw(" ".repeat(pad)));
                for span in &mut line.spans {
                    span.style = span.style.add_modifier(Modifier::REVERSED);
                }
            }
            out.push(line);
        }
        if shown.len() > MAX_ROWS {
            out.push(Line::from(Span::styled(
                format!("  … +{} more", shown.len() - MAX_ROWS),
                styles.muted,
            )));
        }
        if focused {
            out.push(Line::from(Span::styled(
                "↑/↓ select · enter open · x stop · esc back",
                styles.muted,
            )));
        }
        out.into_iter().map(|l| fit_line(l, w)).collect()
    }
}

/// `○ neo · deepseek-flash  Searching tools: weather…  1m 12s · 3.4k tokens`
fn row(agent: &Agent, w: usize, now_ms: i64, styles: &Styles) -> Line<'static> {
    let (glyph, style) = match agent.state {
        AgentState::Running => ("○", styles.accent),
        AgentState::Done => ("✓", styles.success),
        AgentState::Failed => ("✗", styles.danger),
        AgentState::Stopped => ("✗", styles.muted),
    };
    let mut name = agent.label().to_string();
    if let Some(model) = &agent.model {
        // `provider/model-id` reads as the id.
        name.push_str(&format!(" · {}", model.rsplit('/').next().unwrap_or(model)));
    }
    if agent.child.is_some()
        && let Some(dir) = &agent.cwd
    {
        let dir = dir.trim_end_matches('/').rsplit('/').next().unwrap_or(dir);
        name.push_str(&format!(" ⎇ {dir}"));
    }
    if agent.background {
        name.push_str(" [bg]");
    }
    let end = agent.ended_ms.unwrap_or(now_ms);
    let mut stats = elapsed(end - agent.started_ms);
    if let Some(tokens) = agent.tokens {
        stats.push_str(&format!(" · {} tokens", compact(tokens)));
    }
    // Keep the name whole, then the stats, and give the activity what is left.
    let name = truncate(&name, w.saturating_sub(2).max(1));
    let name_w = width(&name) + 2;
    let stats = if name_w + 2 + width(&stats) <= w {
        stats
    } else {
        String::new()
    };
    let room = w.saturating_sub(name_w + 2 + width(&stats) + 2);
    let activity = truncate(&agent.activity, room);
    let used = name_w + 2 + width(&activity) + width(&stats);
    let gap = " ".repeat(
        w.saturating_sub(used)
            .max(if stats.is_empty() { 0 } else { 2 }),
    );
    let activity_style = match agent.state {
        AgentState::Failed => styles.danger,
        _ => styles.muted,
    };
    let mut spans = vec![
        Span::styled(glyph, style),
        Span::raw(format!(" {name}")),
        Span::raw("  "),
        Span::styled(activity, activity_style),
    ];
    if !stats.is_empty() {
        spans.push(Span::raw(gap));
        spans.push(Span::styled(stats, styles.muted));
    }
    Line::from(spans)
}

/// `5s`, `1m 12s`, `2h 03m`.
pub fn elapsed(ms: i64) -> String {
    let secs = (ms / 1000).max(0);
    match secs {
        0..=59 => format!("{secs}s"),
        60..=3599 => format!("{}m {:02}s", secs / 60, secs % 60),
        _ => format!("{}h {:02}m", secs / 3600, secs % 3600 / 60),
    }
}

/// `812`, `3.4k`, `1.2M`.
pub fn compact(n: i64) -> String {
    match n {
        ..=999 => n.to_string(),
        1000..=999_999 => format!("{:.1}k", n as f64 / 1000.0),
        _ => format!("{:.1}M", n as f64 / 1_000_000.0),
    }
}

/// A subagent's transcript, shown over the chat: the daemon's view of just that agent's work,
/// scrolled like the chat.
pub struct Transcript {
    pub agent: Agent,
    pub chat: Chat,
}

impl Transcript {
    pub fn new(agent: Agent) -> Transcript {
        Transcript {
            agent,
            chat: Chat::new(),
        }
    }

    /// Scrolling keys; true when the key was one.
    pub fn scroll_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.chat.scroll_up(1),
            KeyCode::Down | KeyCode::Char('j') => self.chat.scroll_down(1),
            KeyCode::PageUp => self.chat.page_up(),
            KeyCode::PageDown | KeyCode::Char(' ') => self.chat.page_down(),
            KeyCode::Char('u') => self.chat.half_page_up(),
            KeyCode::Char('d') => self.chat.half_page_down(),
            KeyCode::Home | KeyCode::Char('g') => self.chat.to_top(),
            KeyCode::End | KeyCode::Char('G') => self.chat.follow(),
            _ => return false,
        }
        true
    }

    pub fn wheel(&mut self, down: bool) {
        if down {
            self.chat.scroll_down(WHEEL);
        } else {
            self.chat.scroll_up(WHEEL);
        }
    }

    /// A title line, then the transcript with its scrollbar.
    pub fn draw(&mut self, frame: &mut Frame, area: Rect, opts: &ChatOpts, styles: &Styles) {
        if area.height == 0 || area.width == 0 {
            return;
        }
        let a = &self.agent;
        let mut title = vec![
            Span::styled("◂ ", styles.accent),
            Span::styled(
                a.label().to_string(),
                styles.accent.add_modifier(Modifier::BOLD),
            ),
        ];
        if let Some(model) = &a.model {
            title.push(Span::styled(
                format!(" · {}", model.rsplit('/').next().unwrap_or(model)),
                styles.muted,
            ));
        }
        let first = a.task.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
        title.push(Span::styled(format!("  {first}"), styles.muted));
        let bar = Rect { height: 1, ..area };
        frame.render_widget(
            Paragraph::new(fit_line(Line::from(title), usize::from(area.width))),
            bar,
        );
        let hint = "esc back · ↑/↓ pgup/pgdn scroll · ctrl-o full results · ctrl-t thoughts";
        let body = Rect {
            y: area.y + 1,
            height: area.height - 1,
            ..area
        };
        if body.height == 0 {
            return;
        }
        let content = Rect {
            width: body.width.saturating_sub(u16::from(body.width >= 20)),
            ..body
        };
        self.chat.layout(content.width, content.height - 1, opts);
        if self.chat.items().is_empty() {
            let text = if self.chat.loaded() {
                "nothing from this agent yet"
            } else {
                "loading…"
            };
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    truncate(text, usize::from(content.width)),
                    styles.muted,
                ))),
                content,
            );
        } else {
            let lines = Rect {
                height: content.height - 1,
                ..content
            };
            frame.render_widget(Paragraph::new(self.chat.visible()), lines);
            let total = self.chat.total_lines();
            let height = usize::from(lines.height);
            if total > height && body.width >= 20 {
                let track = Rect {
                    x: body.x + body.width - 1,
                    width: 1,
                    height: lines.height,
                    ..body
                };
                let mut state = ScrollbarState::new(total - height).position(self.chat.top());
                frame.render_stateful_widget(
                    Scrollbar::new(ScrollbarOrientation::VerticalRight)
                        .begin_symbol(None)
                        .end_symbol(None)
                        .track_symbol(None)
                        .thumb_symbol("│")
                        .thumb_style(styles.border),
                    track,
                    &mut state,
                );
            }
        }
        let keys = Rect {
            y: body.y + body.height - 1,
            height: 1,
            ..body
        };
        frame.render_widget(
            Paragraph::new(Span::styled(
                truncate(hint, usize::from(keys.width)),
                styles.muted,
            )),
            keys,
        );
    }
}

#[cfg(test)]
#[path = "agents_tests.rs"]
mod tests;
