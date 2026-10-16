//! The three status lines under the prompt.

use ratatui::text::{Line, Span};

use crate::style::Styles;
use crate::util::{fit_line, split_line};

/// Whether the daemon answers guests' permission requests itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Approvals {
    #[default]
    Ask,
    Auto,
}

impl Approvals {
    pub fn parse(value: &str) -> Option<Approvals> {
        match value {
            "ask" => Some(Approvals::Ask),
            "auto" => Some(Approvals::Auto),
            _ => None,
        }
    }

    pub fn other(self) -> Approvals {
        match self {
            Approvals::Ask => Approvals::Auto,
            Approvals::Auto => Approvals::Ask,
        }
    }

    pub fn word(self) -> &'static str {
        match self {
            Approvals::Ask => "ask",
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

    /// `~/site · main ↑1 · pi · model sonnet · in photo-editor`
    fn line1(&self, width: usize) -> Line<'static> {
        let s = self.styles;
        let sep = || Span::styled(" · ", s.muted);
        let mut spans = vec![Span::raw(self.cwd.to_string())];
        if let Some(branch) = self.branch {
            spans.push(sep());
            spans.push(Span::styled(branch.to_string(), s.muted));
        }
        for h in self.settings.iter().filter(|h| h.harness != "suite") {
            if h.values.is_empty() {
                continue;
            }
            spans.push(sep());
            spans.push(Span::raw(h.harness.clone()));
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

    /// `⏵⏵ auto (shift+tab to cycle)` / `ask (shift+tab to cycle)`, a notice on the right.
    fn line3(&self, width: usize) -> Line<'static> {
        let s = self.styles;
        let left = match self.approvals {
            Approvals::Auto => Span::styled("⏵⏵ auto (shift+tab to cycle)", s.accent),
            Approvals::Ask => Span::styled("ask (shift+tab to cycle)", s.muted),
        };
        let right = self.notice.map(|(text, error)| {
            Line::from(Span::styled(
                text.to_string(),
                if error { s.danger } else { s.muted },
            ))
        });
        split_line(Line::from(left), right, width)
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
    fn line_one_shows_only_what_is_set() {
        let styles = Styles::new(&Palette::default(), false);
        let mut settings = Vec::new();
        let s = status(&styles, &settings);
        assert_eq!(s.lines(80)[0].to_string(), "~/site · main ↑1");
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
        assert_eq!(l.to_string(), "ask (shift+tab to cycle)");
        assert_eq!(l.spans[0].style, styles.muted);
        s.approvals = Approvals::Auto;
        let l = &s.lines(60)[2];
        assert_eq!(l.to_string(), "⏵⏵ auto (shift+tab to cycle)");
        assert_eq!(l.spans[0].style, styles.accent);
        s.notice = Some(("press ctrl-c again to quit", false));
        let l = &s.lines(60)[2];
        assert!(l.to_string().starts_with("⏵⏵ auto"));
        assert!(l.to_string().ends_with("press ctrl-c again to quit"));
        assert_eq!(l.width(), 60);
    }
}
