//! The setup walkthrough for a new user: a full-screen run through the settings that matter on
//! the first start (and on `/setup`): what the user does with neo, which agent new chats start
//! on, each agent's tools where it has a choice, which subagents may start, and how answers look.
//! Each step is a question with its choices in boxes, and each choice says what to expect of it:
//! what works and what doesn't, how many tokens it takes before the first message, a preview.
//! The agents and their numbers are the daemon's (harnesses.list). The picks become `/settings`
//! and `/harness` commands once it is done.

use neo_protocol::{HarnessInfo, ToolsChoice};
use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Padding, Paragraph, Wrap};

use crate::style::Styles;
use crate::util::width;

/// One thing to expect of a choice: something it does (`✓`), something it doesn't (`✗`), or a
/// plain fact.
#[derive(Debug, Clone, PartialEq)]
pub enum Expect {
    Yes(String),
    No(String),
    Fact(String),
}

fn yes(text: &str) -> Expect {
    Expect::Yes(text.to_string())
}

fn no(text: &str) -> Expect {
    Expect::No(text.to_string())
}

/// One answer to a step's question.
#[derive(Debug, Clone, PartialEq)]
pub struct Choice {
    pub label: String,
    /// What it looks like, drawn above what to expect (show, don't tell).
    pub preview: Vec<String>,
    pub expect: Vec<Expect>,
    pub recommended: bool,
    /// The command that makes it so; None: nothing to change.
    pub command: Option<String>,
}

impl Choice {
    fn new(label: &str, command: &str, expect: Vec<Expect>) -> Choice {
        Choice {
            label: label.to_string(),
            preview: Vec::new(),
            expect,
            recommended: false,
            command: Some(command.to_string()),
        }
    }

    fn recommended(mut self) -> Choice {
        self.recommended = true;
        self
    }
}

/// One question, and the choice it starts on.
#[derive(Debug, Clone, PartialEq)]
pub struct Step {
    pub title: String,
    pub about: String,
    pub choices: Vec<Choice>,
}

impl Step {
    fn new(title: &str, about: &str, choices: Vec<Choice>) -> Step {
        Step {
            title: title.to_string(),
            about: about.to_string(),
            choices,
        }
    }

    /// The recommended choice, else the first.
    fn start(&self) -> usize {
        self.choices.iter().position(|c| c.recommended).unwrap_or(0)
    }
}

/// `17,700` as `~17.7k`.
fn tokens(n: i64) -> String {
    if n >= 1000 {
        format!("~{:.1}k", n as f64 / 1000.0)
    } else {
        format!("~{n}")
    }
}

/// How much the agent has in its context before the first message with these tools.
fn start_tokens(harness: &HarnessInfo, tools: ToolsChoice) -> Expect {
    let found = harness.tools.iter().find(|t| t.id == tools);
    match found.and_then(|t| t.start_tokens) {
        Some(n) => Expect::Fact(format!("{} tokens before your first message", tokens(n))),
        None => {
            Expect::Fact("Tokens before your first message: known after your first chat".into())
        }
    }
}

/// What a harness is, in a line.
fn harness_about(harness: &HarnessInfo) -> Expect {
    match harness.name.as_str() {
        _ if harness.own => yes("Any model you have an account for; /model switches"),
        "claude" => yes("Anthropic's models, with Claude Code's own prompt and tools"),
        "codex" => yes("OpenAI's models, with Codex's own prompt and tools"),
        _ => yes("Its own models, prompt and tools"),
    }
}

fn signed_in(harness: &HarnessInfo) -> Option<Expect> {
    match (harness.signed_in, harness.own) {
        (Some(true), _) => Some(yes("Signed in")),
        (Some(false), true) => Some(no("No account yet: /login adds one")),
        (Some(false), false) => Some(no(&format!(
            "Not signed in: run {} once to log in",
            harness.name
        ))),
        (None, _) => None,
    }
}

/// The steps for this machine, from the agents the daemon can run.
pub fn steps(harnesses: &[HarnessInfo]) -> Vec<Step> {
    let mut steps = vec![Step::new(
        "What do you want to do with Neo?",
        "This decides which tools Neo works with. /settings changes it later.",
        vec![
            Choice::new(
                "Coding",
                "/settings work coding",
                vec![
                    yes("Runs commands in a shell: builds, tests, git"),
                    yes("Language servers: errors, definitions and renames"),
                    yes("Reads, writes and edits files, searches the web"),
                ],
            )
            .recommended(),
            Choice::new(
                "Computer work",
                "/settings work computer",
                vec![
                    yes("Reads, writes and edits files and documents"),
                    yes("Searches and reads the web, works with your other apps"),
                    no("No shell and no language servers"),
                ],
            ),
        ],
    )];
    let installed: Vec<&HarnessInfo> = harnesses.iter().filter(|h| h.installed).collect();
    let guests = installed.iter().any(|h| !h.own);
    if installed.len() > 1 {
        // Neo when it can work, else the first agent that is signed in.
        let pick = installed
            .iter()
            .position(|h| h.own && h.signed_in != Some(false))
            .or_else(|| installed.iter().position(|h| h.signed_in == Some(true)));
        let choices = installed
            .iter()
            .enumerate()
            .map(|(i, h)| {
                let current = h
                    .tools
                    .iter()
                    .find(|t| t.current == Some(true))
                    .map_or(ToolsChoice::Own, |t| t.id);
                let mut expect = vec![harness_about(h), start_tokens(h, current)];
                expect.extend(signed_in(h));
                let choice = Choice::new(&h.title, &format!("/harness {}", h.name), expect);
                if pick == Some(i) {
                    choice.recommended()
                } else {
                    choice
                }
            })
            .collect();
        steps.push(Step::new(
            "Which agent should new chats start on?",
            "The others stay a /harness away, and every agent can hand work to the others.",
            choices,
        ));
    }
    for h in installed.iter().filter(|h| h.tools.len() > 1) {
        let own = start_tokens(h, ToolsChoice::Own);
        let neo = start_tokens(h, ToolsChoice::Neo);
        let size = |t: ToolsChoice| {
            h.tools
                .iter()
                .find(|x| x.id == t)
                .and_then(|x| x.start_tokens)
        };
        // The one that starts smaller, when both are known; else its own.
        let neo_smaller =
            matches!((size(ToolsChoice::Own), size(ToolsChoice::Neo)), (Some(a), Some(b)) if b < a);
        let mut own_choice = Choice::new(
            "Its own",
            &format!("/settings {}-tools own", h.name),
            vec![
                yes(&format!(
                    "{}'s Read, Edit and Write, as it was trained on them",
                    h.title
                )),
                yes("Its edits come to you as diffs to approve"),
                own,
            ],
        );
        let mut neo_choice = Choice::new(
            "Neo's",
            &format!("/settings {}-tools neo", h.name),
            vec![
                yes("Hashline edits: each line has a tag, so an edit lands where it was meant"),
                yes("Reads images and the files other apps hand over"),
                neo,
            ],
        );
        if neo_smaller {
            neo_choice.recommended = true;
        } else {
            own_choice.recommended = true;
        }
        steps.push(Step::new(
            &format!("Which file tools should {} use?", h.title),
            "Both read and change files. They differ in what they cost and what they can do.",
            vec![own_choice, neo_choice],
        ));
    }
    if guests {
        steps.push(Step::new(
            "Which subagents may the agents start?",
            "A subagent takes on part of the work beside the agent you talk to.",
            vec![
                Choice::new(
                    "Both",
                    "/settings subagents both",
                    vec![
                        yes("Each agent picks what suits the task"),
                        yes("Neo's delegate when you should see and steer the work"),
                        yes("Their own for quick side tasks"),
                    ],
                )
                .recommended(),
                Choice::new(
                    "Neo's delegate",
                    "/settings subagents delegate",
                    vec![
                        yes("Every subagent shows in Neo: you watch, message and stop it"),
                        yes("Any agent can take the task: Neo, Claude Code or Codex"),
                        no("Claude Code's and Codex's own subagents are off"),
                    ],
                ),
                Choice::new(
                    "Their own",
                    "/settings subagents native",
                    vec![
                        yes("What Claude Code and Codex were trained to use"),
                        no("They work out of sight: you see the result, not the steps"),
                        yes("Neo itself keeps delegate"),
                    ],
                ),
            ],
        ));
    }
    let mut show = Choice::new(
        "Show, don't tell",
        "/settings show-dont-tell on",
        vec![
            yes("Numbers, comparisons and steps come as cards, tables and charts"),
            yes("Buttons and forms when the agent needs a choice from you"),
            no("A few more tokens per answer"),
        ],
    )
    .recommended();
    show.preview = vec![
        "Disk usage".into(),
        "/home  ████████░░ 82%".into(),
        "/      ███░░░░░░░ 31%".into(),
    ];
    let mut text = Choice::new(
        "Mostly text",
        "/settings show-dont-tell off",
        vec![
            yes("Plain sentences, quick to read anywhere"),
            yes("Cards and charts only when you ask for them"),
        ],
    );
    text.preview = vec!["/home is 82% full, / is 31% full.".into()];
    steps.push(Step::new(
        "How should answers look?",
        "Agents can answer with cards, tables, charts and forms, drawn in the chat.",
        vec![show, text],
    ));
    steps
}

/// What a key did to the walkthrough.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// Still going.
    Stay,
    /// Finished: these commands carry out the picks.
    Done(Vec<String>),
    /// Left without changing anything.
    Skipped,
}

/// The walkthrough: its steps, the step shown and the choice picked in each.
#[derive(Debug, Clone)]
pub struct Onboarding {
    steps: Vec<Step>,
    /// The step shown; `steps.len()` is the summary at the end.
    at: usize,
    picks: Vec<usize>,
}

impl Onboarding {
    pub fn new(steps: Vec<Step>) -> Onboarding {
        let picks = steps.iter().map(Step::start).collect();
        Onboarding {
            steps,
            at: 0,
            picks,
        }
    }

    fn on_summary(&self) -> bool {
        self.at >= self.steps.len()
    }

    /// ←/→ (h/l, Tab) choose, a digit picks and goes on, Enter goes on (and on the summary
    /// finishes), Backspace or Shift-Tab goes back, Esc leaves without changing anything.
    pub fn key(&mut self, key: KeyEvent) -> Outcome {
        if key.code == KeyCode::Esc
            || key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL)
        {
            return Outcome::Skipped;
        }
        if key.code == KeyCode::Enter && self.on_summary() {
            return Outcome::Done(self.commands());
        }
        let n = self.steps.get(self.at).map_or(0, |s| s.choices.len());
        match (key.code, self.picks.get_mut(self.at)) {
            (KeyCode::Left | KeyCode::Up | KeyCode::Char('h' | 'k'), Some(p)) => {
                *p = (*p + n - 1) % n;
            }
            (KeyCode::Right | KeyCode::Down | KeyCode::Tab | KeyCode::Char('l' | 'j'), Some(p)) => {
                *p = (*p + 1) % n;
            }
            (KeyCode::Char(c @ '1'..='9'), Some(p)) => {
                let at = c as usize - '1' as usize;
                if at < n {
                    *p = at;
                    self.at += 1;
                }
            }
            (KeyCode::Backspace | KeyCode::BackTab, _) => self.at = self.at.saturating_sub(1),
            (KeyCode::Enter, _) => self.at += 1,
            _ => {}
        }
        Outcome::Stay
    }

    /// The commands of every pick, in the order of the steps.
    pub fn commands(&self) -> Vec<String> {
        self.steps
            .iter()
            .zip(&self.picks)
            .filter_map(|(step, &p)| step.choices.get(p)?.command.clone())
            .collect()
    }

    pub fn draw(&self, frame: &mut Frame, area: Rect, styles: &Styles) {
        frame.render_widget(Clear, area);
        crate::style::fill(frame.buffer_mut(), area, styles.panel);
        let w = area.width.saturating_sub(4).min(110);
        let top = area.height.saturating_sub(30) / 3;
        let page = Rect {
            x: area.x + (area.width - w) / 2,
            y: area.y + top,
            width: w,
            height: area.height - top,
        };
        let total = self.steps.len() + 1;
        let mut dots = Vec::new();
        for i in 0..total {
            let style = if i == self.at {
                styles.accent
            } else {
                styles.muted
            };
            dots.push(Span::styled(if i == self.at { "●" } else { "○" }, style));
            dots.push(Span::raw(" "));
        }
        dots.push(Span::styled(
            format!(" {} of {total}", self.at.min(total - 1) + 1),
            styles.muted,
        ));
        let (title, about) = match self.steps.get(self.at) {
            Some(step) => (step.title.as_str(), step.about.as_str()),
            None => (
                "You're set.",
                "Neo starts with these settings. /setup runs through them again and /settings has them all.",
            ),
        };
        let head = vec![
            Line::from(Span::styled(
                "Welcome to Neo",
                styles.accent.add_modifier(Modifier::BOLD),
            )),
            Line::from(dots),
            Line::default(),
            Line::from(Span::styled(title.to_string(), Modifier::BOLD)),
            Line::from(Span::styled(about.to_string(), styles.muted)),
        ];
        let head_h = 6;
        frame.render_widget(
            Paragraph::new(head).wrap(Wrap { trim: true }),
            Rect {
                height: head_h.min(page.height),
                ..page
            },
        );
        let body = Rect {
            y: page.y + head_h + 1,
            height: page.height.saturating_sub(head_h + 3),
            ..page
        };
        let keys_y = match self.steps.get(self.at) {
            Some(step) => draw_choices(frame, body, step, self.picks[self.at], styles),
            None => self.draw_summary(frame, body, styles),
        };
        let keys = if self.on_summary() {
            "Enter Start · Backspace Back · Esc Leave without changes"
        } else if self.at == 0 {
            "←/→ Choose · Enter Next · Esc Leave without changes"
        } else {
            "←/→ Choose · Enter Next · Backspace Back · Esc Leave without changes"
        };
        if keys_y < area.bottom() {
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(keys, styles.muted))),
                Rect {
                    y: keys_y,
                    height: 1,
                    ..page
                },
            );
        }
    }

    /// What was picked in each step; returns the row under it.
    fn draw_summary(&self, frame: &mut Frame, area: Rect, styles: &Styles) -> u16 {
        let label_w = self
            .steps
            .iter()
            .map(|s| width(&s.title))
            .max()
            .unwrap_or(0);
        let lines: Vec<Line> = self
            .steps
            .iter()
            .zip(&self.picks)
            .filter_map(|(step, &p)| {
                let choice = step.choices.get(p)?;
                Some(Line::from(vec![
                    Span::styled(format!("{:<label_w$}  ", step.title), styles.muted),
                    Span::styled(choice.label.clone(), Modifier::BOLD),
                ]))
            })
            .collect();
        let h = (lines.len() as u16).min(area.height);
        frame.render_widget(Paragraph::new(lines), Rect { height: h, ..area });
        area.y + h + 1
    }
}

/// `text` in rows of at most `w` columns, broken between words (a word longer than a row is
/// cut).
fn wrap(text: &str, w: usize) -> Vec<String> {
    let w = w.max(1);
    let mut rows = vec![String::new()];
    for word in text.split_whitespace() {
        let row = rows.last_mut().expect("never empty");
        if !row.is_empty() && width(row) + 1 + width(word) > w {
            rows.push(String::new());
        }
        let row = rows.last_mut().expect("never empty");
        if !row.is_empty() {
            row.push(' ');
        }
        row.push_str(word);
    }
    rows
}

/// A choice's lines in `w` columns: its name (and whether it is recommended), its preview, what
/// to expect, each broken under its text.
fn choice_lines(
    choice: &Choice,
    i: usize,
    selected: bool,
    w: usize,
    styles: &Styles,
) -> Vec<Line<'static>> {
    let edge = if selected {
        styles.accent
    } else {
        styles.border
    };
    let name = if selected {
        styles.accent.add_modifier(Modifier::BOLD)
    } else {
        Style::new().add_modifier(Modifier::BOLD)
    };
    let mut head = vec![
        Span::styled(
            format!("{} {} ", if selected { "◉" } else { "○" }, i + 1),
            edge,
        ),
        Span::styled(choice.label.clone(), name),
    ];
    if choice.recommended {
        head.push(Span::styled("  Recommended", styles.success));
    }
    let mut lines = vec![Line::from(head), Line::default()];
    if !choice.preview.is_empty() {
        for (n, row) in choice.preview.iter().enumerate() {
            let style = if n == 0 && choice.preview.len() > 1 {
                Style::new().add_modifier(Modifier::BOLD)
            } else {
                Style::new()
            };
            lines.push(Line::from(vec![
                Span::styled("│ ", styles.border),
                Span::styled(row.clone(), style),
            ]));
        }
        lines.push(Line::default());
    }
    for expect in &choice.expect {
        let (mark, tone, text) = match expect {
            Expect::Yes(t) => ("✓ ", styles.success, t),
            Expect::No(t) => ("✗ ", styles.danger, t),
            Expect::Fact(t) => ("· ", styles.muted, t),
        };
        for (n, row) in wrap(text, w.saturating_sub(2)).into_iter().enumerate() {
            let lead = if n == 0 {
                Span::styled(mark, tone)
            } else {
                Span::raw("  ")
            };
            lines.push(Line::from(vec![lead, Span::styled(row, styles.muted)]));
        }
    }
    lines
}

/// The choices as boxes side by side where they fit, else under each other; returns the row
/// under them.
fn draw_choices(frame: &mut Frame, area: Rect, step: &Step, picked: usize, styles: &Styles) -> u16 {
    let n = step.choices.len().max(1) as u16;
    let gap = 2;
    let side = (area.width.saturating_sub(gap * (n - 1))) / n;
    let across = side >= 30;
    let box_w = if across { side } else { area.width };
    let inner = usize::from(box_w.saturating_sub(4)).max(1);
    let all: Vec<Vec<Line>> = step
        .choices
        .iter()
        .enumerate()
        .map(|(i, c)| choice_lines(c, i, i == picked, inner, styles))
        .collect();
    let tallest = all.iter().map(Vec::len).max().unwrap_or(1);
    let box_h = u16::try_from(tallest + 2).unwrap_or(u16::MAX);
    let mut bottom = area.y;
    for (i, lines) in all.into_iter().enumerate() {
        let at = i as u16;
        let rect = if across {
            Rect {
                x: area.x + at * (box_w + gap),
                y: area.y,
                width: box_w,
                height: box_h.min(area.height),
            }
        } else {
            let h = u16::try_from(lines.len() + 2).unwrap_or(u16::MAX);
            let y = bottom.max(area.y) + u16::from(i > 0);
            Rect {
                x: area.x,
                y,
                width: box_w,
                height: h,
            }
        };
        if rect.bottom() > area.bottom() {
            break;
        }
        let edge = if i == picked {
            styles.accent
        } else {
            styles.border
        };
        let block = Block::new()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(edge)
            .style(Style::new().bg(styles.raised))
            .padding(Padding::horizontal(1));
        frame.render_widget(Paragraph::new(lines).block(block), rect);
        bottom = bottom.max(rect.bottom());
    }
    bottom + 1
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn harnesses(value: serde_json::Value) -> Vec<HarnessInfo> {
        serde_json::from_value(value).unwrap()
    }

    fn neo(signed_in: bool) -> serde_json::Value {
        json!({"name": "neo", "title": "Neo", "own": true, "installed": true,
               "signedIn": signed_in, "tools": [{"id": "own", "current": true, "startTokens": 2600}]})
    }

    fn claude(installed: bool) -> serde_json::Value {
        json!({"name": "claude", "title": "Claude Code", "own": false, "installed": installed,
               "signedIn": true, "tools": [
                   {"id": "own", "current": true, "startTokens": 17700},
                   {"id": "neo", "current": false, "startTokens": 15000}]})
    }

    #[test]
    fn the_steps_are_the_daemons_agents_that_are_installed() {
        // Neo alone: nothing to pick between, no tools or subagents to ask about.
        assert_eq!(steps(&harnesses(json!([neo(true)]))).len(), 2);
        assert_eq!(
            steps(&harnesses(json!([neo(true), claude(false)]))).len(),
            2
        );
        let both = steps(&harnesses(json!([neo(true), claude(true)])));
        let titles: Vec<&str> = both.iter().map(|s| s.title.as_str()).collect();
        assert_eq!(
            titles,
            [
                "What do you want to do with Neo?",
                "Which agent should new chats start on?",
                "Which file tools should Claude Code use?",
                "Which subagents may the agents start?",
                "How should answers look?",
            ]
        );
    }

    #[test]
    fn the_recommendation_follows_what_works_and_what_costs_less() {
        let agent = |list: serde_json::Value| {
            let steps = steps(&harnesses(list));
            steps[1].choices[steps[1].start()].label.clone()
        };
        assert_eq!(agent(json!([neo(true), claude(true)])), "Neo");
        // Neo without an account: the agent that is signed in.
        assert_eq!(agent(json!([neo(false), claude(true)])), "Claude Code");
        // Neo's tools start smaller here: they are the recommended ones.
        let steps = steps(&harnesses(json!([neo(true), claude(true)])));
        assert_eq!(steps[2].choices[steps[2].start()].label, "Neo's");
    }

    #[test]
    fn the_picks_become_commands_once_the_summary_is_confirmed() {
        let mut o = Onboarding::new(steps(&harnesses(json!([neo(true)]))));
        assert_eq!(o.key(key(KeyCode::Right)), Outcome::Stay);
        assert_eq!(o.key(key(KeyCode::Enter)), Outcome::Stay);
        // Back and on again keeps the pick.
        assert_eq!(o.key(key(KeyCode::Backspace)), Outcome::Stay);
        assert_eq!(o.key(key(KeyCode::Enter)), Outcome::Stay);
        // A digit picks and goes on: the summary.
        assert_eq!(o.key(key(KeyCode::Char('2'))), Outcome::Stay);
        assert_eq!(
            o.key(key(KeyCode::Enter)),
            Outcome::Done(vec![
                "/settings work computer".to_string(),
                "/settings show-dont-tell off".to_string(),
            ])
        );
        let mut left = Onboarding::new(steps(&[]));
        assert_eq!(left.key(key(KeyCode::Esc)), Outcome::Skipped);
    }
}
