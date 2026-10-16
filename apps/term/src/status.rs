//! The three status lines under the prompt.

use ratatui::text::{Line, Span};

use crate::style::Styles;
use crate::util::{fit_line, split_line};

/// What the daemon asks before the agent acts (`/approvals`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Approvals {
    /// Edits, commands, guests: all ask.
    #[default]
    Ask,
    /// File edits are written right away (and can be reverted after the turn); the rest asks.
    Edits,
    /// Nothing asks.
    Auto,
}

impl Approvals {
    pub fn parse(value: &str) -> Option<Approvals> {
        match value {
            "ask" => Some(Approvals::Ask),
            "edits" => Some(Approvals::Edits),
            "auto" => Some(Approvals::Auto),
            _ => None,
        }
    }

    /// The mode Shift-Tab goes to: ask, edits, auto, ask…
    pub fn next(self) -> Approvals {
        match self {
            Approvals::Ask => Approvals::Edits,
            Approvals::Edits => Approvals::Auto,
            Approvals::Auto => Approvals::Ask,
        }
    }

    pub fn word(self) -> &'static str {
        match self {
            Approvals::Ask => "ask",
            Approvals::Edits => "edits",
            Approvals::Auto => "auto",
        }
    }
}

/// One harness's settings from the session log, in the order they were first set.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct HarnessSettings {
    pub harness: String,
    pub values: Vec<(String, String)>,
}

/// Records `harness option = value`, keeping first-seen order.
pub fn record_setting(all: &mut Vec<HarnessSettings>, harness: &str, option: &str, value: &str) {
    let h = match all.iter().position(|h| h.harness == harness) {
        Some(i) => &mut all[i],
        None => {
            all.push(HarnessSettings {
                harness: harness.into(),
                values: Vec::new(),
            });
            all.last_mut().expect("just pushed")
        }
    };
    match h.values.iter_mut().find(|(o, _)| o == option) {
        Some((_, v)) => *v = value.into(),
        None => h.values.push((option.into(), value.into())),
    }
}

pub struct Status<'a> {
    /// The working directory, already shortened with `~`.
    pub cwd: &'a str,
    pub branch: Option<&'a str>,
    pub settings: &'a [HarnessSettings],
    /// The session's agent (`pi`, `claude`, `codex`).
    pub harness: &'a str,
    /// The app the agent is in, when that isn't this one.
    pub agent_in: Option<&'a str>,
    pub usage: Option<Line<'static>>,
    /// Seconds the running turn has taken; None when idle.
    pub working_secs: Option<u64>,
    pub spinner: char,
    pub queued: usize,
    pub connected: bool,
    pub approvals: Approvals,
    /// A transient message and whether it is an error.
    pub notice: Option<(&'a str, bool)>,
    pub styles: &'a Styles,
}

impl Status<'_> {
    pub fn lines(&self, width: usize) -> [Line<'static>; 3] {
        [self.line1(width), self.line2(width), self.line3(width)]
    }

    /// `~/site · main ↑1 · pi · model sonnet · in photo-editor`: the harness is always named,
    /// with only its own settings.
    fn line1(&self, width: usize) -> Line<'static> {
        let s = self.styles;
        let sep = || Span::styled(" · ", s.muted);
        let mut spans = vec![Span::raw(self.cwd.to_string())];
        if let Some(branch) = self.branch {
            spans.push(sep());
            spans.push(Span::styled(branch.to_string(), s.muted));
        }
        spans.push(sep());
        spans.push(Span::raw(self.harness.to_string()));
        for h in self.settings.iter().filter(|h| h.harness == self.harness) {
            for (option, value) in &h.values {
                spans.push(sep());
                spans.push(Span::styled(format!("{option} {value}"), s.muted));
            }
        }
        if let Some(app) = self.agent_in {
            spans.push(sep());
            spans.push(Span::styled(format!("in {app}"), s.accent));
        }
        fit_line(Line::from(spans), width)
    }

    /// `ctx 35% · $0.18 · ◐ working 12s · 2 queued`, or the lost connection.
    fn line2(&self, width: usize) -> Line<'static> {
        let s = self.styles;
        let mut spans: Vec<Span<'static>> = Vec::new();
        let push = |spans: &mut Vec<Span<'static>>, part: Vec<Span<'static>>| {
            if !spans.is_empty() {
                spans.push(Span::styled(" · ", s.muted));
            }
            spans.extend(part);
        };
        if let Some(usage) = &self.usage {
            push(&mut spans, usage.spans.clone());
        }
        if let Some(secs) = self.working_secs {
            push(
                &mut spans,
                vec![Span::styled(
                    format!("{} working {}", self.spinner, duration(secs)),
                    s.accent,
                )],
            );
        }
        if self.queued > 0 {
            push(
                &mut spans,
                vec![Span::styled(format!("{} queued", self.queued), s.muted)],
            );
        }
        if !self.connected {
            push(
                &mut spans,
                vec![Span::styled("disconnected — reconnecting", s.danger)],
            );
        }
        fit_line(Line::from(spans), width)
    }

    /// `⏵⏵ accept edits on (shift+tab to cycle)`, `⏵⏵ auto mode on …` or `? ask before acting …`,
    /// a notice on the right (the hint in parentheses goes first when they don't both fit).
    fn line3(&self, width: usize) -> Line<'static> {
        let s = self.styles;
        let (text, style) = match self.approvals {
            Approvals::Auto => ("⏵⏵ auto mode on", s.danger),
            Approvals::Edits => ("⏵⏵ accept edits on", s.accent),
            Approvals::Ask => ("? ask before acting", s.muted),
        };
        let right = self.notice.map(|(text, error)| {
            Line::from(Span::styled(
                text.to_string(),
                if error { s.danger } else { s.muted },
            ))
        });
        let hint = " (shift+tab to cycle)";
        let long = text.chars().count() + hint.chars().count();
        let room = right.as_ref().map_or(0, |r| r.width() + 2);
        let left = if long + room <= width {
            Line::from(vec![Span::styled(text, style), Span::styled(hint, s.muted)])
        } else {
            Line::from(Span::styled(text, style))
        };
        split_line(left, right, width)
    }
}

fn duration(secs: u64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else {
        format!("{}m{:02}s", secs / 60, secs % 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use suite_renderer_ratatui::Palette;

    fn status<'a>(styles: &'a Styles, settings: &'a [HarnessSettings]) -> Status<'a> {
        Status {
            cwd: "~/site",
            branch: Some("main ↑1"),
            settings,
            harness: "pi",
            agent_in: None,
            usage: None,
            working_secs: None,
            spinner: '◐',
            queued: 0,
            connected: true,
            approvals: Approvals::Ask,
            notice: None,
            styles,
        }
    }

    #[test]
    fn line_one_names_the_harness_and_shows_only_its_settings() {
        let styles = Styles::new(&Palette::default(), false);
        let mut settings = Vec::new();
        let s = status(&styles, &settings);
        assert_eq!(s.lines(80)[0].to_string(), "~/site · main ↑1 · pi");
        record_setting(&mut settings, "pi", "model", "sonnet");
        record_setting(&mut settings, "suite", "approvals", "auto");
        let mut s = status(&styles, &settings);
        assert_eq!(
            s.lines(80)[0].to_string(),
            "~/site · main ↑1 · pi · model sonnet"
        );
        s.agent_in = Some("photo-editor");
        s.branch = None;
        assert_eq!(
            s.lines(80)[0].to_string(),
            "~/site · pi · model sonnet · in photo-editor"
        );
        record_setting(&mut settings, "pi", "effort", "high");
        record_setting(&mut settings, "pi", "model", "opus");
        assert_eq!(
            status(&styles, &settings).lines(80)[0].to_string(),
            "~/site · main ↑1 · pi · model opus · effort high"
        );
        assert_eq!(status(&styles, &settings).lines(20)[0].width(), 20);
        // Another harness's settings stay out of sight until it is the session's.
        record_setting(&mut settings, "claude", "model", "opus-4");
        let mut s = status(&styles, &settings);
        assert!(!s.lines(80)[0].to_string().contains("opus-4"));
        s.harness = "claude";
        assert_eq!(
            s.lines(80)[0].to_string(),
            "~/site · main ↑1 · claude · model opus-4"
        );
    }

    #[test]
    fn line_two_has_usage_turn_state_queue_and_connection() {
        let styles = Styles::new(&Palette::default(), false);
        let none: [HarnessSettings; 0] = [];
        let mut s = status(&styles, &none);
        assert_eq!(s.lines(80)[1].to_string(), "");
        s.usage = Some(Line::from("ctx 35% · $0.18"));
        s.working_secs = Some(12);
        s.queued = 2;
        assert_eq!(
            s.lines(80)[1].to_string(),
            "ctx 35% · $0.18 · ◐ working 12s · 2 queued"
        );
        s.working_secs = Some(75);
        s.connected = false;
        let line = &s.lines(80)[1];
        assert!(
            line.to_string()
                .ends_with("1m15s · 2 queued · disconnected — reconnecting")
        );
        let last = line.spans.last().unwrap();
        assert_eq!(last.style, styles.danger);
    }

    #[test]
    fn line_three_is_the_approvals_mode() {
        let styles = Styles::new(&Palette::default(), false);
        let none: [HarnessSettings; 0] = [];
        let mut s = status(&styles, &none);
        let l = &s.lines(60)[2];
        assert_eq!(l.to_string(), "? ask before acting (shift+tab to cycle)");
        assert_eq!(l.spans[0].style, styles.muted);
        assert_eq!(l.spans[1].style, styles.muted);
        s.approvals = Approvals::Edits;
        let l = &s.lines(60)[2];
        assert_eq!(l.to_string(), "⏵⏵ accept edits on (shift+tab to cycle)");
        assert_eq!(l.spans[0].style, styles.accent);
        s.approvals = Approvals::Auto;
        let l = &s.lines(60)[2];
        assert_eq!(l.to_string(), "⏵⏵ auto mode on (shift+tab to cycle)");
        assert_eq!(l.spans[0].style, styles.danger);
        s.notice = Some(("press ctrl-c again to quit", false));
        let l = &s.lines(70)[2];
        assert!(l.to_string().starts_with("⏵⏵ auto mode on (shift+tab"));
        assert!(l.to_string().ends_with("press ctrl-c again to quit"));
        assert_eq!(l.width(), 70);
        // Narrower: the notice wins over the hint.
        let l = &s.lines(48)[2];
        assert_eq!(
            l.to_string(),
            "⏵⏵ auto mode on       press ctrl-c again to quit"
        );
    }

    #[test]
    fn approvals_cycle_ask_edits_auto() {
        let modes = [Approvals::Ask, Approvals::Edits, Approvals::Auto];
        for (i, mode) in modes.iter().enumerate() {
            assert_eq!(mode.next(), modes[(i + 1) % 3]);
            assert_eq!(Approvals::parse(mode.word()), Some(*mode));
        }
        assert_eq!(Approvals::parse("yolo"), None);
    }
}
