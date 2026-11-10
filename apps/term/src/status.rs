//! The two status lines under the prompt.

use ratatui::text::{Line, Span};

use crate::style::Styles;
use crate::util::{fit_line, split_line, width};

/// What the daemon asks before the agent acts (`/approvals`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Approvals {
    /// Edits, commands, guests: all ask.
    #[default]
    Ask,
    /// Each file edit is written right away and then kept or reverted by itself; the rest asks.
    Review,
    /// File edits are written right away (and can be reverted after the turn); the rest asks.
    Edits,
    /// Nothing asks.
    Auto,
}

impl Approvals {
    pub fn parse(value: &str) -> Option<Approvals> {
        match value {
            "ask" => Some(Approvals::Ask),
            "review" => Some(Approvals::Review),
            "edits" => Some(Approvals::Edits),
            "auto" => Some(Approvals::Auto),
            _ => None,
        }
    }

    /// The mode Shift-Tab goes to: ask, review, edits, auto, ask…
    pub fn next(self) -> Approvals {
        match self {
            Approvals::Ask => Approvals::Review,
            Approvals::Review => Approvals::Edits,
            Approvals::Edits => Approvals::Auto,
            Approvals::Auto => Approvals::Ask,
        }
    }

    pub fn word(self) -> &'static str {
        match self {
            Approvals::Ask => "ask",
            Approvals::Review => "review",
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

/// The names to show for model values, by (harness, value): `Sonnet 5.5` for `sonnet`.
pub type ModelNames = std::collections::HashMap<(String, String), String>;

/// The harness's name for the status line: a glyph for it and its name capitalized.
fn harness_label(harness: &str) -> String {
    let glyph = match harness {
        "neo" => "◆",
        "claude" => "✻",
        "codex" => "⬡",
        _ => "◈",
    };
    format!("{glyph} {}", capitalized(harness))
}

/// The first letter in capitals: `high` is `High`.
fn capitalized(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

pub struct Status<'a> {
    /// The working directory, as its last segment (the project).
    pub cwd: &'a str,
    pub branch: Option<&'a str>,
    pub settings: &'a [HarnessSettings],
    /// What the daemon calls the models, for the ones it knows.
    pub names: &'a ModelNames,
    /// The session's agent (`neo`, `claude`, `codex`).
    pub harness: &'a str,
    /// The app the agent is in, when that isn't this one.
    pub agent_in: Option<&'a str>,
    pub usage: Option<Line<'static>>,
    /// Seconds the running turn has taken; None when idle.
    pub working_secs: Option<u64>,
    pub spinner: char,
    pub queued: usize,
    /// A draft is put aside (Ctrl-S).
    pub stashed: bool,
    /// The last turn was over an hour ago: the provider's prompt cache is gone.
    pub cache_cold: bool,
    pub connected: bool,
    pub approvals: Approvals,
    /// A transient message and whether it is an error.
    pub notice: Option<(&'a str, bool)>,
    /// The chat has a doc with buttons or fields that no key reaches now: say how to get there.
    pub ui_hint: bool,
    pub styles: &'a Styles,
}

impl Status<'_> {
    pub fn lines(&self, width: usize) -> [Line<'static>; 2] {
        [self.line1(width), self.line2(width)]
    }

    /// `site · main ↑1 · neo · model sonnet · in photo-editor`: the harness is always named,
    /// with only its own settings; it and their values stand out, the rest is muted.
    fn line1(&self, width: usize) -> Line<'static> {
        let s = self.styles;
        let sep = || Span::styled(" · ", s.muted);
        let mut spans = vec![Span::raw(self.cwd.to_string())];
        if let Some(branch) = self.branch {
            spans.push(sep());
            spans.push(Span::styled(branch.to_string(), s.muted));
        }
        spans.push(sep());
        spans.push(Span::raw(harness_label(self.harness)));
        for h in self.settings.iter().filter(|h| h.harness == self.harness) {
            for (option, value) in &h.values {
                spans.push(sep());
                spans.push(Span::styled(format!("{option} "), s.muted));
                let shown = match option.as_str() {
                    "model" => self
                        .names
                        .get(&(h.harness.clone(), value.clone()))
                        .cloned()
                        .unwrap_or_else(|| value.clone()),
                    "effort" => capitalized(value),
                    _ => value.clone(),
                };
                spans.push(Span::raw(shown));
            }
        }
        if let Some(app) = self.agent_in {
            spans.push(sep());
            spans.push(Span::styled(format!("in {app}"), s.accent));
        }
        fit_line(Line::from(spans), width)
    }

    /// The approvals mode, then `ctx 35% · $0.18 · ◐ working 12s · 2 queued` or the lost
    /// connection; a notice on the right, else `ctrl-p use the buttons above` when there is a doc
    /// to use. The mode is `? ask before acting`, `⏵ review each edit`, `⏵⏵ accept edits on` or
    /// `⏵⏵ auto mode on`, with `(shift+tab to cycle)` when nothing else is said and it fits.
    fn line2(&self, cols: usize) -> Line<'static> {
        let s = self.styles;
        let (mode, mode_style) = match self.approvals {
            Approvals::Auto => ("⏵⏵ auto mode on", s.danger),
            Approvals::Review => ("⏵ review each edit", s.accent),
            Approvals::Edits => ("⏵⏵ accept edits on", s.accent),
            Approvals::Ask => ("? ask before acting", s.muted),
        };
        let mut rest: Vec<Span<'static>> = Vec::new();
        let push = |rest: &mut Vec<Span<'static>>, part: Vec<Span<'static>>| {
            rest.push(Span::styled(" · ", s.muted));
            rest.extend(part);
        };
        if let Some(usage) = &self.usage {
            push(&mut rest, usage.spans.clone());
        }
        if let Some(secs) = self.working_secs {
            push(
                &mut rest,
                vec![Span::styled(
                    format!("{} working {}", self.spinner, duration(secs)),
                    s.accent,
                )],
            );
        }
        if self.queued > 0 {
            push(
                &mut rest,
                vec![Span::styled(format!("{} queued", self.queued), s.muted)],
            );
        }
        if self.stashed {
            push(
                &mut rest,
                vec![Span::styled("draft put aside (ctrl-s)", s.muted)],
            );
        }
        if self.cache_cold {
            push(&mut rest, vec![Span::styled("cache expired", s.muted)]);
        }
        if !self.connected {
            push(
                &mut rest,
                vec![Span::styled("disconnected — reconnecting", s.danger)],
            );
        }
        let right = match self.notice {
            Some((text, error)) => Some(Line::from(Span::styled(
                text.to_string(),
                if error { s.danger } else { s.muted },
            ))),
            None if self.ui_hint => Some(Line::from(vec![
                Span::styled("ctrl-p", s.accent),
                Span::styled(" use the buttons above", s.muted),
            ])),
            None => None,
        };
        let hint = " (shift+tab to cycle)";
        let room = right.as_ref().map_or(0, |r| r.width() + 2);
        let mut left = vec![Span::styled(mode, mode_style)];
        if rest.is_empty() && width(mode) + width(hint) + room <= cols {
            left.push(Span::styled(hint, s.muted));
        }
        left.extend(rest);
        // A notice is said now: what is left of it gives way.
        let mut left = Line::from(left);
        if let Some(r) = &right
            && left.width() + 2 + r.width() > cols
            && cols >= width(mode) + 2 + r.width()
        {
            left = fit_line(left, cols - 2 - r.width());
        }
        split_line(left, right, cols)
    }
}

pub fn duration(secs: u64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else {
        format!("{}m{:02}s", secs / 60, secs % 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use neo_renderer_ratatui::Palette;

    fn status<'a>(styles: &'a Styles, settings: &'a [HarnessSettings]) -> Status<'a> {
        Status {
            cwd: "site",
            branch: Some("main ↑1"),
            settings,
            names: Box::leak(Box::default()),
            harness: "neo",
            agent_in: None,
            usage: None,
            working_secs: None,
            spinner: '◐',
            queued: 0,
            stashed: false,
            cache_cold: false,
            connected: true,
            approvals: Approvals::Ask,
            notice: None,
            ui_hint: false,
            styles,
        }
    }

    #[test]
    fn line_one_names_the_harness_and_shows_only_its_settings() {
        let styles = Styles::new(&Palette::default(), false);
        let mut settings = Vec::new();
        let s = status(&styles, &settings);
        assert_eq!(s.lines(80)[0].to_string(), "site · main ↑1 · ◆ Neo");
        record_setting(&mut settings, "neo", "model", "sonnet");
        record_setting(&mut settings, "daemon", "approvals", "auto");
        let mut s = status(&styles, &settings);
        assert_eq!(
            s.lines(80)[0].to_string(),
            "site · main ↑1 · ◆ Neo · model sonnet"
        );
        s.agent_in = Some("photo-editor");
        s.branch = None;
        assert_eq!(
            s.lines(80)[0].to_string(),
            "site · ◆ Neo · model sonnet · in photo-editor"
        );
        record_setting(&mut settings, "neo", "effort", "high");
        record_setting(&mut settings, "neo", "model", "opus");
        assert_eq!(
            status(&styles, &settings).lines(80)[0].to_string(),
            "site · main ↑1 · ◆ Neo · model opus · effort High"
        );
        // Another harness's settings stay out of sight until it is the session's.
        record_setting(&mut settings, "claude", "model", "opus-4");
        let mut s = status(&styles, &settings);
        assert!(!s.lines(80)[0].to_string().contains("opus-4"));
        s.harness = "claude";
        assert_eq!(
            s.lines(80)[0].to_string(),
            "site · main ↑1 · ✻ Claude · model opus-4"
        );
    }

    #[test]
    fn a_model_shows_under_the_name_the_daemon_gives_it() {
        let styles = Styles::new(&Palette::default(), false);
        let mut settings = Vec::new();
        record_setting(&mut settings, "claude", "model", "sonnet");
        let mut names = ModelNames::new();
        names.insert(("claude".into(), "sonnet".into()), "Sonnet 5.5".into());
        let mut s = status(&styles, &settings);
        s.harness = "claude";
        s.names = &names;
        assert!(s.lines(80)[0].to_string().ends_with("model Sonnet 5.5"));
    }

    #[test]
    fn line_two_has_the_mode_usage_turn_state_queue_and_connection() {
        let styles = Styles::new(&Palette::default(), false);
        let none: [HarnessSettings; 0] = [];
        let mut s = status(&styles, &none);
        assert_eq!(
            s.lines(80)[1].to_string(),
            "? ask before acting (shift+tab to cycle)"
        );
        s.usage = Some(Line::from("ctx 35% · $0.18"));
        s.working_secs = Some(12);
        s.queued = 2;
        assert_eq!(
            s.lines(80)[1].to_string(),
            "? ask before acting · ctx 35% · $0.18 · ◐ working 12s · 2 queued"
        );
        s.working_secs = Some(75);
        s.connected = false;
        let line = &s.lines(120)[1];
        assert!(
            line.to_string()
                .ends_with("1m15s · 2 queued · disconnected — reconnecting")
        );
        s.working_secs = None;
        s.queued = 0;
        s.connected = true;
        s.cache_cold = true;
        assert_eq!(
            s.lines(80)[1].to_string(),
            "? ask before acting · ctx 35% · $0.18 · cache expired"
        );
    }

    #[test]
    fn the_mode_leads_line_two_and_a_notice_wins() {
        let styles = Styles::new(&Palette::default(), false);
        let none: [HarnessSettings; 0] = [];
        let mut s = status(&styles, &none);
        s.approvals = Approvals::Review;
        let l = &s.lines(60)[1];
        assert_eq!(l.to_string(), "⏵ review each edit (shift+tab to cycle)");
        s.approvals = Approvals::Edits;
        let l = &s.lines(60)[1];
        assert_eq!(l.to_string(), "⏵⏵ accept edits on (shift+tab to cycle)");
        s.approvals = Approvals::Auto;
        let l = &s.lines(60)[1];
        assert_eq!(l.to_string(), "⏵⏵ auto mode on (shift+tab to cycle)");
        s.notice = Some(("press ctrl-c again to quit", false));
        let l = &s.lines(70)[1];
        assert!(l.to_string().starts_with("⏵⏵ auto mode on (shift+tab"));
        assert!(l.to_string().ends_with("press ctrl-c again to quit"));
        // Narrower: the notice wins over the hint.
        let l = &s.lines(48)[1];
        assert!(l.to_string().starts_with("⏵⏵ auto mode on"));
        assert!(!l.to_string().contains("shift+tab"));
        assert!(l.to_string().ends_with("press ctrl-c again to quit"));
        // And over the usage, which is cut for it.
        s.usage = Some(Line::from("ctx 35% · $0.18 · 120k tokens"));
        let l = &s.lines(60)[1];
        assert!(l.to_string().starts_with("⏵⏵ auto mode on · ctx"));
        assert!(l.to_string().ends_with("press ctrl-c again to quit"));
    }

    #[test]
    fn line_two_says_how_to_reach_the_buttons_above_unless_a_notice_is_up() {
        let styles = Styles::new(&Palette::default(), false);
        let none: [HarnessSettings; 0] = [];
        let mut s = status(&styles, &none);
        s.ui_hint = true;
        let l = &s.lines(70)[1];
        assert!(l.to_string().starts_with("? ask before acting (shift+tab"));
        assert!(l.to_string().ends_with("ctrl-p use the buttons above"));
        s.notice = Some(("copied", false));
        let l = &s.lines(70)[1];
        assert!(l.to_string().ends_with("copied"));
        assert!(!l.to_string().contains("ctrl-p"));
    }

    #[test]
    fn approvals_cycle_ask_review_edits_auto() {
        let modes = [
            Approvals::Ask,
            Approvals::Review,
            Approvals::Edits,
            Approvals::Auto,
        ];
        for (i, mode) in modes.iter().enumerate() {
            assert_eq!(mode.next(), modes[(i + 1) % 4]);
            assert_eq!(Approvals::parse(mode.word()), Some(*mode));
        }
        assert_eq!(Approvals::parse("yolo"), None);
    }
}
