//! The setup walkthrough: a full-screen run through the settings that matter on the first start
//! (and on `/setup`): what the user does with neo, which agent new chats start on, Claude Code
//! and Codex when they are installed, and how answers look. Each step is a question with its
//! choices in boxes; the picks become `/settings` and `/harness` commands once it is done.

use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap};

use crate::style::Styles;
use crate::util::width;

/// One answer to a step's question.
#[derive(Debug, Clone, PartialEq)]
pub struct Choice {
    pub label: &'static str,
    pub about: &'static str,
    /// The command that makes it so; None: nothing to change.
    pub command: Option<String>,
}

/// One question.
#[derive(Debug, Clone, PartialEq)]
pub struct Step {
    pub title: &'static str,
    pub about: &'static str,
    pub choices: Vec<Choice>,
}

/// Which harnesses are on this machine, for the steps about them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Installed {
    pub claude: bool,
    pub codex: bool,
}

impl Installed {
    /// Looks for `claude` and `codex` on the PATH.
    pub fn find() -> Installed {
        Installed {
            claude: on_path("claude"),
            codex: on_path("codex"),
        }
    }
}

fn on_path(name: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| dir.join(name).is_file())
}

fn choice(label: &'static str, about: &'static str, command: &str) -> Choice {
    Choice {
        label,
        about,
        command: Some(command.to_string()),
    }
}

/// The steps for this machine: the ones about Claude Code and Codex only when they are there.
pub fn steps(installed: Installed) -> Vec<Step> {
    let mut steps = vec![Step {
        title: "What do you want to do with Neo?",
        about: "This decides Neo's tools. You can change it later in /settings.",
        choices: vec![
            choice(
                "Coding",
                "Write and change code. Neo works with a shell and language servers.",
                "/settings work coding",
            ),
            choice(
                "Computer work",
                "Files, documents, the web and your other apps. No shell, no language servers.",
                "/settings work computer",
            ),
        ],
    }];
    if installed.claude || installed.codex {
        let mut agents = vec![choice(
            "Neo",
            "Neo's own agent, with any model you have an account for.",
            "/harness neo",
        )];
        if installed.claude {
            agents.push(choice(
                "Claude Code",
                "Your Claude Code, with its own tools, in Neo's chat.",
                "/harness claude",
            ));
        }
        if installed.codex {
            agents.push(choice(
                "Codex",
                "Your Codex, with its own tools, in Neo's chat.",
                "/harness codex",
            ));
        }
        steps.push(Step {
            title: "Which agent should new chats start on?",
            about: "The others stay a /harness away, and every agent can hand work to the others.",
            choices: agents,
        });
    }
    if installed.claude {
        steps.push(Step {
            title: "Which file tools should Claude Code use?",
            about: "Neo's read, edit and write show every edit as a diff to approve.",
            choices: vec![
                choice(
                    "Its own",
                    "Claude Code's Read, Edit and Write, as you know them.",
                    "/settings claude-tools own",
                ),
                choice(
                    "Neo's",
                    "Neo's hashline read, edit and write, asking the way Neo does.",
                    "/settings claude-tools neo",
                ),
            ],
        });
    }
    if installed.claude || installed.codex {
        steps.push(Step {
            title: "Which subagents may the agents start?",
            about: "Subagents take on part of the work beside the agent you talk to.",
            choices: vec![
                choice(
                    "Both",
                    "Neo's delegate, and Claude Code's and Codex's own subagents.",
                    "/settings subagents both",
                ),
                choice(
                    "Neo's delegate",
                    "Every subagent goes through Neo, where you see and steer it.",
                    "/settings subagents delegate",
                ),
                choice(
                    "Their own",
                    "Claude Code and Codex use their own; Neo keeps delegate.",
                    "/settings subagents native",
                ),
            ],
        });
    }
    steps.push(Step {
        title: "How should answers look?",
        about: "Agents can answer with cards, tables, charts and forms drawn in the chat.",
        choices: vec![
            choice(
                "Show, don't tell",
                "Cards, tables and charts wherever they read better than text.",
                "/settings show-dont-tell on",
            ),
            choice(
                "Mostly text",
                "Plain answers; UI when it is asked for.",
                "/settings show-dont-tell off",
            ),
        ],
    });
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
        let picks = vec![0; steps.len()];
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
        let n = self.steps.get(self.at).map_or(0, |s| s.choices.len());
        if key.code == KeyCode::Enter && self.on_summary() {
            return Outcome::Done(self.commands());
        }
        let pick = self.picks.get_mut(self.at);
        match (key.code, pick) {
            (KeyCode::Esc, _) => return Outcome::Skipped,
            (KeyCode::Char('c'), _) if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return Outcome::Skipped;
            }
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
        let w = area.width.saturating_sub(4).min(96);
        let page = Rect {
            x: area.x + (area.width - w) / 2,
            y: area.y + area.height.saturating_sub(24) / 3,
            width: w,
            height: area.height.saturating_sub(area.height.saturating_sub(24) / 3),
        };
        let total = self.steps.len() + 1;
        // Welcome, then a dot per step, the one shown in the accent.
        let mut dots = Vec::new();
        for i in 0..total {
            let style = if i == self.at {
                styles.accent
            } else if i < self.at {
                styles.muted.add_modifier(Modifier::BOLD)
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
        let mut head = vec![
            Line::from(Span::styled(
                "Welcome to Neo",
                styles.accent.add_modifier(Modifier::BOLD),
            )),
            Line::from(dots),
            Line::default(),
        ];
        let (title, about) = match self.steps.get(self.at) {
            Some(step) => (step.title, step.about),
            None => (
                "You're set.",
                "Neo works with these settings. /setup runs through them again, /settings has them all, and /login adds an account for a model.",
            ),
        };
        head.push(Line::from(Span::styled(title, Modifier::BOLD)));
        head.push(Line::from(Span::styled(about, styles.muted)));
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
            Some(step) => self.draw_choices(frame, body, step, self.picks[self.at], styles),
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

    /// The choices as boxes side by side where they fit, else under each other; returns the
    /// row under them.
    fn draw_choices(
        &self,
        frame: &mut Frame,
        area: Rect,
        step: &Step,
        picked: usize,
        styles: &Styles,
    ) -> u16 {
        let n = step.choices.len().max(1) as u16;
        let gap = 2;
        let side = (area.width.saturating_sub(gap * (n - 1))) / n;
        let across = side >= 26;
        let box_w = if across { side } else { area.width };
        // As tall as the longest text wrapped: the borders, the name, a blank row, the text
        // (a row more than it takes unbroken, for the words that move down).
        let inner = usize::from(box_w.saturating_sub(4)).max(1);
        let text_rows = step
            .choices
            .iter()
            .map(|c| width(c.about).div_ceil(inner) + usize::from(width(c.about) > inner))
            .max()
            .unwrap_or(1);
        let box_h = u16::try_from(4 + text_rows).unwrap_or(u16::MAX);
        let mut bottom = area.y;
        for (i, choice) in step.choices.iter().enumerate() {
            let i16 = i as u16;
            let rect = if across {
                Rect {
                    x: area.x + i16 * (box_w + gap),
                    y: area.y,
                    width: box_w,
                    height: box_h.min(area.height),
                }
            } else {
                Rect {
                    x: area.x,
                    y: area.y + i16 * (box_h + 1),
                    width: box_w,
                    height: box_h,
                }
            };
            if rect.bottom() > area.bottom() {
                break;
            }
            let selected = i == picked;
            let edge = if selected { styles.accent } else { styles.border };
            let mark = if selected { "◉" } else { "○" };
            let block = Block::new()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(edge)
                .style(Style::new().bg(styles.raised));
            let name = if selected {
                styles.accent.add_modifier(Modifier::BOLD)
            } else {
                Style::new().add_modifier(Modifier::BOLD)
            };
            let lines = vec![
                Line::from(vec![
                    Span::styled(format!("{mark} {} ", i + 1), edge),
                    Span::styled(choice.label, name),
                ]),
                Line::default(),
                Line::from(Span::styled(choice.about, styles.muted)),
            ];
            frame.render_widget(
                Paragraph::new(lines)
                    .wrap(Wrap { trim: true })
                    .block(block.padding(ratatui::widgets::Padding::horizontal(1))),
                rect,
            );
            bottom = bottom.max(rect.bottom());
        }
        bottom + 1
    }

    /// What was picked in each step; returns the row under it.
    fn draw_summary(&self, frame: &mut Frame, area: Rect, styles: &Styles) -> u16 {
        let label_w = self
            .steps
            .iter()
            .map(|s| width(s.title))
            .max()
            .unwrap_or(0);
        let lines: Vec<Line> = self
            .steps
            .iter()
            .zip(&self.picks)
            .filter_map(|(step, &p)| {
                let choice = step.choices.get(p)?;
                Some(Line::from(vec![
                    Span::styled(
                        format!("{:<label_w$}  ", step.title),
                        styles.muted,
                    ),
                    Span::styled(choice.label, Modifier::BOLD),
                ]))
            })
            .collect();
        let h = (lines.len() as u16).min(area.height);
        frame.render_widget(
            Paragraph::new(lines),
            Rect {
                height: h,
                ..area
            },
        );
        area.y + h + 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn the_steps_about_claude_code_and_codex_are_there_only_when_they_are() {
        let none = steps(Installed::default());
        let both = steps(Installed {
            claude: true,
            codex: true,
        });
        let codex = steps(Installed {
            claude: false,
            codex: true,
        });
        assert_eq!(none.len(), 2);
        assert_eq!(both.len(), 5);
        // Codex alone: no step about Claude Code's tools, and no Claude Code to start on.
        assert_eq!(codex.len(), 4);
        assert!(codex[1].choices.iter().all(|c| c.label != "Claude Code"));
    }

    #[test]
    fn the_picks_become_commands_once_the_summary_is_confirmed() {
        let mut o = Onboarding::new(steps(Installed::default()));
        // Second choice of the first step, then on with Enter; back and forth keeps the picks.
        assert_eq!(o.key(key(KeyCode::Right)), Outcome::Stay);
        assert_eq!(o.key(key(KeyCode::Enter)), Outcome::Stay);
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
        let mut left = Onboarding::new(steps(Installed::default()));
        assert_eq!(left.key(key(KeyCode::Esc)), Outcome::Skipped);
    }
}
