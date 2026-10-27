//! A masked input for a secret the daemon asks for (`secret.request`): an API key, a login code.
//!
//! The typed value lives only in [`Secret`] (wiped when it goes) and travels once, inside
//! [`Effect::Secret`](crate::app::Effect::Secret). It never reaches the editor, the input history,
//! the chat or a rendered frame: the field draws one `•` per character.

use std::fmt;

use ratatui::crossterm::event::KeyCode;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use zeroize::Zeroizing;

use crate::chat;
use crate::util::{fit_line, truncate};

/// The text of a secret on its way to the daemon. Its `Debug` hides it, so a logged effect or a
/// failed assertion cannot print it; the memory is wiped when it is dropped.
#[derive(Clone, PartialEq, Eq)]
pub struct SecretValue(Zeroizing<String>);

impl SecretValue {
    pub fn new(text: String) -> Self {
        SecretValue(Zeroizing::new(text))
    }

    /// Takes the text out, to put it on the wire. What stays here is empty.
    pub fn take(mut self) -> String {
        std::mem::take(&mut *self.0)
    }

    /// For tests: compare with what was typed.
    #[cfg(test)]
    pub fn reveal(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SecretValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretValue(..)")
    }
}

/// What a key did to the field.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Still typing (or the key meant nothing).
    Typing,
    /// Enter: send this.
    Submit(SecretValue),
    /// Esc: send no value.
    Cancel,
}

/// One open question: its id, what is asked, and what has been typed so far.
pub struct Secret {
    pub id: String,
    title: String,
    placeholder: Option<String>,
    value: Zeroizing<String>,
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Secret")
            .field("id", &self.id)
            .field("title", &self.title)
            .field("typed", &self.value.chars().count())
            .finish()
    }
}

impl Secret {
    pub fn new(id: String, title: String, placeholder: Option<String>) -> Self {
        Secret {
            id,
            title,
            placeholder,
            value: Zeroizing::new(String::new()),
        }
    }

    /// How many characters were typed.
    pub fn len(&self) -> usize {
        self.value.chars().count()
    }

    pub fn is_empty(&self) -> bool {
        self.value.is_empty()
    }

    /// Pasted text: control characters (a trailing newline) are dropped.
    pub fn paste(&mut self, text: &str) {
        self.value.extend(text.chars().filter(|c| !c.is_control()));
    }

    pub fn key(&mut self, code: KeyCode, ctrl: bool, alt: bool) -> Outcome {
        match code {
            KeyCode::Esc => Outcome::Cancel,
            KeyCode::Char('c' | 'd') if ctrl => Outcome::Cancel,
            KeyCode::Enter => {
                let typed = self.value.trim();
                if typed.is_empty() {
                    return Outcome::Typing;
                }
                Outcome::Submit(SecretValue::new(typed.to_string()))
            }
            // A masked word has no edges to find: Ctrl-W and Ctrl-U both start over.
            KeyCode::Char('u' | 'w') if ctrl => {
                self.value.clear();
                Outcome::Typing
            }
            KeyCode::Char(c) if !ctrl && !alt && !c.is_control() => {
                self.value.push(c);
                Outcome::Typing
            }
            KeyCode::Backspace => {
                self.value.pop();
                Outcome::Typing
            }
            _ => Outcome::Typing,
        }
    }

    /// The field as it takes the editor's place: the title, one `•` per typed character (or the
    /// placeholder while empty) and a line of keys, each with the bar in the margin (`bar`, none
    /// in the input box).
    pub fn lines(
        &self,
        width: usize,
        marker: Style,
        bar: Option<Style>,
        muted: Style,
    ) -> Vec<Line<'static>> {
        let title = Line::from(Span::styled(
            truncate(&self.title, width.saturating_sub(2)),
            Style::new().add_modifier(Modifier::BOLD),
        ));
        let lead = "› ";
        let cursor = Span::styled(" ", Style::new().add_modifier(Modifier::REVERSED));
        let room = width.saturating_sub(lead.chars().count() + 3).max(1);
        let mut field = vec![Span::styled(lead, marker)];
        if self.is_empty() {
            field.push(cursor);
            if let Some(hint) = &self.placeholder {
                field.push(Span::styled(truncate(hint, room), muted));
            }
        } else {
            field.push(Span::raw("•".repeat(self.len().min(room))));
            field.push(cursor);
        }
        let keys = Line::from(Span::styled(
            "enter submit · esc cancel · hidden, never shown or logged",
            muted,
        ));
        [title, Line::from(field), keys]
            .into_iter()
            .map(|line| fit_line(chat::barred(line, bar), width))
            .collect()
    }
}
