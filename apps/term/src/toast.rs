//! Event cards in the top right of the main pane: they fly in from the right edge, stay a few
//! seconds and fly back out. For what happens beside the chat (a background command finished,
//! a subagent started) that would otherwise only show in a list under the input.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph, Widget};

use crate::style::Styles;
use crate::util::truncate;

/// How long a card takes to fly in (and out).
const FLY_MS: i64 = 260;
/// How long a card is on screen, flights included.
pub const LIFE_MS: i64 = 5000;
/// The widest a card gets.
const MAX_WIDTH: u16 = 46;
/// Rows a card takes: its border, the title and the detail.
const HEIGHT: u16 = 4;
/// Cards on screen at once; the oldest give way.
const MAX_SHOWN: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Accent,
    Success,
    Danger,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Toast {
    pub title: String,
    pub detail: String,
    pub tone: Tone,
    born_ms: i64,
}

#[derive(Debug, Default)]
pub struct Toasts {
    list: Vec<Toast>,
}

impl Toasts {
    pub fn push(
        &mut self,
        title: impl Into<String>,
        detail: impl Into<String>,
        tone: Tone,
        now_ms: i64,
    ) {
        self.list.push(Toast {
            title: title.into(),
            detail: detail.into(),
            tone,
            born_ms: now_ms,
        });
        if self.list.len() > MAX_SHOWN {
            self.list.remove(0);
        }
    }

    pub fn list(&self) -> &[Toast] {
        &self.list
    }

    /// Drops the cards whose time is up; whether anything on screen moves or went.
    pub fn tick(&mut self, now_ms: i64) -> bool {
        let before = self.list.len();
        self.list.retain(|t| now_ms - t.born_ms < LIFE_MS);
        self.list.len() != before || !self.list.is_empty()
    }

    /// The cards, stacked down from the top right of `area`, each as far in as its flight got.
    pub fn draw(&self, buf: &mut Buffer, area: Rect, now_ms: i64, styles: &Styles) {
        let width = MAX_WIDTH.min(area.width.saturating_sub(4));
        if width < 16 {
            return;
        }
        let rest_x = area.right() - 1 - width;
        for (i, toast) in self.list.iter().enumerate() {
            let y = area.y + 1 + i as u16 * HEIGHT;
            if y + HEIGHT > area.bottom() {
                break;
            }
            // Where the card is: off the right edge, then in place, then off again.
            let out = 1.0 - flight(now_ms - toast.born_ms);
            let x = rest_x + ((area.right() - rest_x) as f32 * out).round() as u16;
            if x >= area.right() {
                continue;
            }
            let full = Rect::new(0, 0, width, HEIGHT);
            let mut card = Buffer::empty(full);
            toast.render(full, &mut card, styles);
            // Only the part that has come in from the edge.
            for cy in 0..HEIGHT {
                for cx in 0..width.min(area.right() - x) {
                    buf[(x + cx, y + cy)] = card[(cx, cy)].clone();
                }
            }
        }
    }
}

/// How far in a card `age` ms old is, 0 (off screen) to 1 (in place), eased.
fn flight(age: i64) -> f32 {
    let t = if age < FLY_MS {
        age as f32 / FLY_MS as f32
    } else if age > LIFE_MS - FLY_MS {
        (LIFE_MS - age) as f32 / FLY_MS as f32
    } else {
        1.0
    };
    // Fast in, settling: an ease-out cubic.
    1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(3)
}

impl Toast {
    fn render(&self, area: Rect, buf: &mut Buffer, styles: &Styles) {
        let tone = match self.tone {
            Tone::Accent => styles.accent,
            Tone::Success => styles.success,
            Tone::Danger => styles.danger,
        };
        let inner = usize::from(area.width.saturating_sub(4));
        let block = Block::new()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(tone)
            .style(Style::new().bg(styles.raised));
        let lines = vec![
            Line::from(Span::styled(
                truncate(&format!("● {}", self.title), inner),
                tone.add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(truncate(&self.detail, inner), styles.muted)),
        ];
        Clear.render(area, buf);
        Paragraph::new(lines)
            .block(block.padding(ratatui::widgets::Padding::horizontal(1)))
            .render(area, buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_card_stays_and_leaves() {
        let mut toasts = Toasts::default();
        toasts.push("Command finished", "cargo test · exit 0", Tone::Success, 0);
        // Gone once its time is up (a last redraw to clear it), then nothing to draw.
        assert!(toasts.tick(LIFE_MS - 1));
        assert!(toasts.tick(LIFE_MS));
        assert!(toasts.list().is_empty());
        assert!(!toasts.tick(LIFE_MS + 50));
    }

    #[test]
    fn only_a_few_cards_show_the_oldest_give_way() {
        let mut toasts = Toasts::default();
        for i in 0..5 {
            toasts.push(format!("t{i}"), "", Tone::Accent, 0);
        }
        let titles: Vec<_> = toasts.list().iter().map(|t| t.title.as_str()).collect();
        assert_eq!(titles, ["t2", "t3", "t4"]);
    }
}
