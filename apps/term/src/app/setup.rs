use super::*;

/// What the user can do with neo, as the first start asks it: the setting's value, its name
/// and what it means.
pub(super) const WORK_CHOICES: [(&str, &str, &str); 2] = [
    (
        "coding",
        "Coding",
        "Write and change code, with a shell and language servers.",
    ),
    (
        "computer",
        "Computer work",
        "Files, documents, the web and your other apps. No shell.",
    ),
];

impl App {
    /// The daemon's settings came: with nothing chosen yet, the first start asks what the user
    /// does with neo, once.
    pub(super) fn maybe_ask_work(&mut self, chosen: bool) {
        if chosen || self.setup_asked || self.overlay != Overlay::None {
            return;
        }
        self.setup_asked = true;
        self.setup_sel = 0;
        self.overlay = Overlay::Setup;
    }

    /// Keys in the setup dialog: Up/Down (or j/k) move, Enter or a digit picks, Esc keeps the
    /// first, coding.
    pub(super) fn setup_key(&mut self, key: KeyEvent, fx: &mut Vec<Effect>) {
        let n = WORK_CHOICES.len();
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.setup_sel = (self.setup_sel + n - 1) % n,
            KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab => {
                self.setup_sel = (self.setup_sel + 1) % n;
            }
            KeyCode::Enter => self.choose_work(self.setup_sel, fx),
            KeyCode::Esc => self.choose_work(0, fx),
            KeyCode::Char(c) => {
                if let Some(at) = c.to_digit(10).map(|d| d as usize).filter(|d| (1..=n).contains(d)) {
                    self.choose_work(at - 1, fx);
                }
            }
            _ => {}
        }
    }

    fn choose_work(&mut self, at: usize, fx: &mut Vec<Effect>) {
        self.overlay = Overlay::None;
        let Some((value, _, _)) = WORK_CHOICES.get(at) else {
            return;
        };
        fx.push(Effect::Input {
            session: self.session.clone(),
            text: format!("/settings work {value}"),
            images: Vec::new(),
            steer: false,
        });
    }

    /// The first start's question, over the chat.
    pub(super) fn draw_setup(&self, frame: &mut Frame, area: Rect) {
        use ratatui::widgets::{Block, BorderType, Borders, Padding};
        // The question's two rows and a blank one, two rows a choice and a blank one after
        // each, the keys.
        let rows = u16::try_from(3 + WORK_CHOICES.len() * 3 + 1).unwrap_or(u16::MAX);
        let dialog = Rect {
            width: area.width.saturating_sub(4).min(72),
            height: (rows + 3).min(area.height),
            ..area
        };
        let dialog = Rect {
            x: area.x + (area.width - dialog.width) / 2,
            y: area.y + (area.height - dialog.height) / 3,
            ..dialog
        };
        let w = usize::from(dialog.width.saturating_sub(4));
        let block = Block::new()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(self.styles.border)
            .title(Line::from(Span::styled(
                " Welcome to Neo ",
                self.styles.accent.add_modifier(Modifier::BOLD),
            )))
            .style(Style::new().bg(self.styles.raised))
            .padding(Padding::new(1, 1, 1, 0));
        let band = chat::mix(self.styles.raised, self.palette.accent, 0.16);
        let mut lines = vec![
            fit_line(
                Line::from(Span::styled(
                    "What do you want to do with Neo?",
                    Modifier::BOLD,
                )),
                w,
            ),
            fit_line(
                Line::from(Span::styled(
                    "You can change it later in /settings.",
                    self.styles.muted,
                )),
                w,
            ),
            Line::default(),
        ];
        for (i, (_, name, what)) in WORK_CHOICES.iter().enumerate() {
            let selected = i == self.setup_sel;
            let mut rows = vec![
                Line::from(vec![
                    Span::styled(
                        format!("{} ", i + 1),
                        if selected {
                            self.styles.accent.add_modifier(Modifier::BOLD)
                        } else {
                            self.styles.muted
                        },
                    ),
                    Span::styled(name.to_string(), Modifier::BOLD),
                ]),
                Line::from(Span::styled(format!("  {what}"), self.styles.muted)),
            ];
            for row in &mut rows {
                let used = row.width();
                row.spans.push(Span::raw(" ".repeat(w.saturating_sub(used))));
                *row = fit_line(row.clone(), w);
                if selected && let Some(bg) = band {
                    *row = row.clone().style(Style::new().bg(bg));
                } else if selected {
                    *row = row.clone().style(Style::new().add_modifier(Modifier::REVERSED));
                }
            }
            lines.extend(rows);
            lines.push(Line::default());
        }
        let key = |k: &'static str| Span::styled(k, self.styles.accent);
        let what = |d: &'static str| Span::styled(d, self.styles.muted);
        lines.push(fit_line(
            Line::from(vec![
                key("Enter"),
                what(" Choose  "),
                key("↑/↓"),
                what(" Move  "),
                key("Esc"),
                what(" Coding"),
            ]),
            w,
        ));
        frame.render_widget(Clear, dialog);
        frame.render_widget(Paragraph::new(lines).block(block), dialog);
    }
}
