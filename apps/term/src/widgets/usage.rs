//! The usage line under the prompt: context fill and cost of the agent and its guests.

use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use suite_protocol::Usage;
use suite_renderer_ratatui::Palette;

#[derive(Debug, Clone, Copy, PartialEq, Default)]
struct Numbers {
    /// contextUsed / contextSize, when both are known and the size is not zero.
    ctx: Option<f64>,
    cost: Option<f64>,
}

/// The newest `usage` event per agent, in the order the agents first reported.
#[derive(Debug, Default)]
pub(crate) struct UsageState {
    agents: Vec<(Option<String>, Numbers)>,
}

impl UsageState {
    pub(crate) fn apply(&mut self, u: &Usage) {
        let ctx = match (u.context_used, u.context_size) {
            (Some(used), Some(size)) if size > 0 => Some(used.max(0) as f64 / size as f64),
            _ => None,
        };
        let numbers = Numbers { ctx, cost: u.cost };
        match self.agents.iter_mut().find(|(a, _)| *a == u.agent) {
            Some((_, n)) => *n = numbers,
            None => self.agents.push((u.agent.clone(), numbers)),
        }
    }

    pub(crate) fn line(&self, palette: &Palette) -> Option<Line<'static>> {
        let muted = Style::new().fg(palette.muted);
        let mut spans: Vec<Span<'static>> = Vec::new();
        let main = self.agents.iter().find(|(a, _)| a.is_none());
        let guests = self.agents.iter().filter(|(a, _)| a.is_some());
        for (agent, n) in main.into_iter().chain(guests) {
            if n.ctx.is_none() && n.cost.is_none() {
                continue;
            }
            match agent {
                None => {
                    if let Some(f) = n.ctx {
                        spans.push(ctx_span(f, palette));
                    }
                    if let Some(c) = n.cost {
                        if !spans.is_empty() {
                            spans.push(Span::styled(" · ", muted));
                        }
                        spans.push(Span::styled(format_cost(c), muted));
                    }
                }
                Some(name) => {
                    if !spans.is_empty() {
                        spans.push(Span::styled(" · ", muted));
                    }
                    spans.push(Span::styled(name.clone(), muted));
                    if let Some(f) = n.ctx {
                        spans.push(Span::styled(" ", muted));
                        spans.push(ctx_span(f, palette));
                    }
                    if let Some(c) = n.cost {
                        spans.push(Span::styled(format!(" {}", format_cost(c)), muted));
                    }
                }
            }
        }
        (!spans.is_empty()).then(|| Line::from(spans))
    }
}

fn ctx_color(fraction: f64, palette: &Palette) -> Color {
    if fraction >= 0.85 {
        palette.danger
    } else if fraction >= 0.6 {
        palette.accent
    } else {
        palette.muted
    }
}

fn ctx_span(fraction: f64, palette: &Palette) -> Span<'static> {
    Span::styled(
        format!("ctx {}%", (fraction * 100.0).round() as i64),
        Style::new().fg(ctx_color(fraction, palette)),
    )
}

/// Two decimals; `<$0.01` for anything below a cent that isn't zero.
pub(crate) fn format_cost(cost: f64) -> String {
    if cost > 0.0 && cost < 0.01 {
        "<$0.01".to_string()
    } else {
        format!("${:.2}", cost.max(0.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usage(
        agent: Option<&str>,
        used: Option<i64>,
        size: Option<i64>,
        cost: Option<f64>,
    ) -> Usage {
        Usage {
            agent: agent.map(String::from),
            context_used: used,
            context_size: size,
            cost,
            input_tokens: None,
            output_tokens: None,
        }
    }

    fn text(line: &Line) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn nothing_before_any_event() {
        assert!(UsageState::default().line(&Palette::default()).is_none());
    }

    #[test]
    fn main_then_guests_and_last_wins() {
        let p = Palette::default();
        let mut s = UsageState::default();
        s.apply(&usage(Some("claude"), Some(12), Some(100), Some(0.42)));
        s.apply(&usage(None, Some(10), Some(100), Some(0.01)));
        s.apply(&usage(None, Some(35), Some(100), Some(0.184)));
        assert_eq!(
            text(&s.line(&p).unwrap()),
            "ctx 35% · $0.18 · claude ctx 12% $0.42"
        );
    }

    #[test]
    fn missing_parts_are_omitted() {
        let p = Palette::default();
        let mut s = UsageState::default();
        s.apply(&usage(None, None, Some(100), Some(0.003)));
        assert_eq!(text(&s.line(&p).unwrap()), "<$0.01");
        s.apply(&usage(None, Some(5), Some(0), None));
        assert!(s.line(&p).is_none());
        s.apply(&usage(Some("codex"), Some(50), Some(100), None));
        assert_eq!(text(&s.line(&p).unwrap()), "codex ctx 50%");
        s.apply(&usage(None, Some(1), Some(1000), None));
        assert_eq!(text(&s.line(&p).unwrap()), "ctx 0% · codex ctx 50%");
    }

    #[test]
    fn context_colors_by_fill() {
        let p = Palette::default();
        let color = |pct: i64| {
            let mut s = UsageState::default();
            s.apply(&usage(None, Some(pct), Some(100), None));
            s.line(&p).unwrap().spans[0].style.fg.unwrap()
        };
        assert_eq!(color(59), p.muted);
        assert_eq!(color(60), p.accent);
        assert_eq!(color(84), p.accent);
        assert_eq!(color(85), p.danger);
        assert_eq!(color(120), p.danger);
    }

    #[test]
    fn cost_format() {
        assert_eq!(format_cost(0.0), "$0.00");
        assert_eq!(format_cost(0.0099), "<$0.01");
        assert_eq!(format_cost(0.01), "$0.01");
        assert_eq!(format_cost(12.5), "$12.50");
    }
}
