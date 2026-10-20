//! Forms: every input is a focusable field, `Label *` over `[ value or placeholder ]`.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use serde_json::Value;
use unicode_width::UnicodeWidthChar;

use super::{N, UiKind, UiOption, join, number, scalar};
use crate::render::{Cx, Lines, stack};
use crate::text::{clean, sline, truncate, width, wrap};

/// The widest a text field is drawn.
const FIELD_W: usize = 44;
/// Rows a text area has at least, and at most.
const AREA_MIN: usize = 3;
const AREA_MAX: usize = 8;

fn cell(c: char) -> usize {
    c.width().unwrap_or(0)
}

/// `chars` from `start` while they fit `room` cells.
fn fit_from(chars: &[char], start: usize, room: usize) -> usize {
    let mut used = 0;
    let mut end = start;
    while end < chars.len() && used + cell(chars[end]) <= room {
        used += cell(chars[end]);
        end += 1;
    }
    end
}

impl<'a> Cx<'a> {
    /// The value of a field: what was chosen here and not echoed yet, else the node's.
    fn field_value(&self, n: N<'_>) -> Value {
        n.key()
            .and_then(|k| self.ui_state().and_then(|s| s.values.get(k)))
            .cloned()
            .or_else(|| n.prop("value").cloned())
            .unwrap_or(Value::Null)
    }

    /// Registers the node as a target and says whether it has focus.
    fn field_target(&mut self, n: N<'_>, kind: UiKind, value: Value) -> bool {
        let Some(key) = n.key() else {
            return false;
        };
        let label = self
            .ui
            .control_label
            .clone()
            .or_else(|| n.line("label"))
            .or_else(|| n.line("name"))
            .unwrap_or_default();
        self.ui_target(key, kind, label, value);
        self.ui_focused(key)
    }

    /// The error under a field, in the danger tone.
    fn field_error(&self, n: N<'_>, w: usize) -> Lines {
        match n.line("error") {
            Some(e) => wrap(&[Span::styled(e, Style::new().fg(self.p.danger))], w),
            None => Vec::new(),
        }
    }

    fn bracket(&self, n: N<'_>, focused: bool) -> Style {
        if n.line("error").is_some() {
            Style::new().fg(self.p.danger)
        } else if focused {
            Style::new().fg(self.p.accent).add_modifier(Modifier::BOLD)
        } else {
            self.p.muted_style()
        }
    }

    /// `[ content ]`, `content` filling `inner` cells.
    fn boxed(&self, content: Vec<Span<'static>>, inner: usize, bracket: Style) -> Line<'static> {
        let used: usize = content.iter().map(|s| width(&s.content)).sum();
        let mut spans = vec![Span::styled("[ ", bracket)];
        spans.extend(content);
        spans.push(Span::raw(" ".repeat(inner.saturating_sub(used))));
        spans.push(Span::styled(" ]", bracket));
        Line::from(spans)
    }

    /// The characters of a field in `inner` cells, the one at `cursor` reversed (a space past
    /// the end), scrolled so the cursor shows.
    fn field_text(
        &self,
        chars: &[char],
        cursor: Option<usize>,
        inner: usize,
        style: Style,
    ) -> Vec<Span<'static>> {
        let cursor_style = style.add_modifier(Modifier::REVERSED);
        let mut spans: Vec<Span<'static>> = Vec::new();
        let push = |spans: &mut Vec<Span<'static>>, s: &[char], st: Style| {
            if !s.is_empty() {
                spans.push(Span::styled(s.iter().collect::<String>(), st));
            }
        };
        match cursor {
            None => {
                let end = fit_from(chars, 0, inner);
                if end < chars.len() && inner > 0 {
                    let end = fit_from(chars, 0, inner - 1);
                    push(&mut spans, &chars[..end], style);
                    spans.push(Span::styled("…", style));
                } else {
                    push(&mut spans, chars, style);
                }
            }
            Some(c) => {
                let c = c.min(chars.len());
                // Scroll right until the cursor cell is inside.
                let mut start = 0;
                let behind =
                    |start: usize| -> usize { chars[start..c].iter().map(|&x| cell(x)).sum() };
                while start < c
                    && behind(start) + cell(chars.get(c).copied().unwrap_or(' ')).max(1) > inner
                {
                    start += 1;
                }
                let at_end = c >= chars.len();
                let end = fit_from(chars, start, inner.saturating_sub(usize::from(at_end)));
                push(&mut spans, &chars[start..c.min(end)], style);
                if at_end {
                    spans.push(Span::styled(" ", cursor_style));
                } else if c < end {
                    push(&mut spans, &chars[c..=c], cursor_style);
                    push(&mut spans, &chars[c + 1..end], style);
                }
            }
        }
        spans
    }

    /// `Input`, `TextArea` and `DatePicker`.
    pub(super) fn ui_text_field(&mut self, n: N<'_>, w: usize) -> Lines {
        let area = n.name() == "TextArea";
        let date = n.name() == "DatePicker";
        let secret = n.str("type") == Some("password");
        let kind = if area {
            UiKind::TextArea
        } else if date {
            UiKind::Date
        } else {
            UiKind::Input { secret }
        };
        let value = self.field_value(n);
        let focused = self.field_target(n, kind, value.clone());
        let edit = n
            .key()
            .and_then(|k| self.ui_state().and_then(|s| s.edits.get(k)));
        let (text, cursor) = match edit {
            Some(e) => (e.text.clone(), Some(e.cursor)),
            None => (scalar(&value), None),
        };
        // Passwords show dots; a single line shows no newlines.
        let text = if area {
            clean(&text).into_owned()
        } else {
            text.replace('\n', " ")
        };
        let chars: Vec<char> = if secret {
            text.chars().map(|_| '•').collect()
        } else {
            text.chars().collect()
        };
        let cursor = cursor.filter(|_| focused);
        let box_w = w.min(FIELD_W);
        let inner = box_w.saturating_sub(4).max(1);
        let placeholder = n
            .line("placeholder")
            .or_else(|| date.then(|| "YYYY-MM-DD".to_string()))
            .unwrap_or_default();
        let bracket = self.bracket(n, focused);
        let bad_date = date && !chars.is_empty() && !focused && !is_date(&text);
        let text_style = if bad_date {
            Style::new().fg(self.p.danger)
        } else {
            Style::new()
        };
        let mut out = if area {
            self.text_area(&chars, cursor, &placeholder, inner, bracket)
        } else if chars.is_empty() {
            let mut content = Vec::new();
            let mut hint = truncate(&placeholder, inner.saturating_sub(usize::from(focused)));
            if focused {
                content.push(Span::styled(
                    " ",
                    Style::new().add_modifier(Modifier::REVERSED),
                ));
            }
            if hint.is_empty() && !focused {
                hint = String::new();
            }
            content.push(Span::styled(hint, self.p.muted_style()));
            vec![self.boxed(content, inner, bracket)]
        } else {
            let content = self.field_text(&chars, cursor, inner, text_style);
            vec![self.boxed(content, inner, bracket)]
        };
        if focused {
            self.mark_focus(&mut out[0]);
        }
        out.extend(self.field_error(n, w));
        out
    }

    fn text_area(
        &self,
        chars: &[char],
        cursor: Option<usize>,
        placeholder: &str,
        inner: usize,
        bracket: Style,
    ) -> Lines {
        // Visual rows as ranges of chars; a newline ends its row and is not drawn.
        let mut rows: Vec<(usize, usize)> = Vec::new();
        let mut start = 0;
        let mut used = 0;
        for (i, &c) in chars.iter().enumerate() {
            if c == '\n' {
                rows.push((start, i));
                start = i + 1;
                used = 0;
                continue;
            }
            if used + cell(c) > inner && i > start {
                rows.push((start, i));
                start = i;
                used = 0;
            }
            used += cell(c);
        }
        rows.push((start, chars.len()));
        let height = rows.len().clamp(AREA_MIN, AREA_MAX);
        let cursor_row = cursor.map(|c| {
            let c = c.min(chars.len());
            rows.iter().rposition(|&(s, _)| s <= c).unwrap_or(0)
        });
        let first = cursor_row.map_or(0, |r| (r + 1).saturating_sub(height));
        let mut out = Vec::new();
        for r in first..first + height {
            let Some(&(s, e)) = rows.get(r) else {
                out.push(self.boxed(Vec::new(), inner, bracket));
                continue;
            };
            let line = &chars[s..e];
            if chars.is_empty() && r == 0 {
                let mut content = Vec::new();
                if cursor.is_some() {
                    content.push(Span::styled(
                        " ",
                        Style::new().add_modifier(Modifier::REVERSED),
                    ));
                }
                content.push(Span::styled(
                    truncate(
                        placeholder,
                        inner.saturating_sub(usize::from(cursor.is_some())),
                    ),
                    self.p.muted_style(),
                ));
                out.push(self.boxed(content, inner, bracket));
                continue;
            }
            let here = (cursor_row == Some(r)).then(|| cursor.unwrap_or(0).min(chars.len()) - s);
            let content = self.field_text(line, here, inner, Style::new());
            out.push(self.boxed(content, inner, bracket));
        }
        out
    }

    fn options(&self, n: N<'_>) -> Vec<UiOption> {
        n.nodes("options")
            .into_iter()
            .map(|o| {
                let value = o
                    .str("value")
                    .map(crate::text::one_line)
                    .unwrap_or_default();
                let label = o.line("label").unwrap_or_else(|| value.clone());
                UiOption { value, label }
            })
            .collect()
    }

    /// `[ Chosen option                ▾ ]`, and the list under it while it is open.
    pub(super) fn ui_select(&mut self, n: N<'_>, w: usize) -> Lines {
        let options = self.options(n);
        let value = self.field_value(n);
        let focused = self.field_target(
            n,
            UiKind::Select {
                options: options.clone(),
            },
            value.clone(),
        );
        let current = scalar(&value);
        let chosen = options.iter().position(|o| o.value == current);
        let box_w = w.min(FIELD_W);
        let inner = box_w.saturating_sub(4).max(1);
        let text_room = inner.saturating_sub(2);
        let content = match chosen {
            Some(i) => vec![Span::raw(truncate(&options[i].label, text_room))],
            None if !current.is_empty() => vec![Span::raw(truncate(&current, text_room))],
            None => vec![Span::styled(
                truncate(&n.line("placeholder").unwrap_or_default(), text_room),
                self.p.muted_style(),
            )],
        };
        let used: usize = content.iter().map(|s| width(&s.content)).sum();
        let mut content = content;
        content.push(Span::raw(" ".repeat(inner.saturating_sub(used + 1))));
        content.push(Span::styled("▾", self.p.muted_style()));
        let bracket = self.bracket(n, focused);
        let mut head = self.boxed(content, inner, bracket);
        if focused {
            self.mark_focus(&mut head);
        }
        let mut out = vec![head];
        let open = n
            .key()
            .and_then(|k| {
                self.ui_state()
                    .and_then(|s| s.select.as_ref().filter(|(open, _)| open == k))
            })
            .map(|(_, at)| *at);
        if let (Some(at), true) = (open, focused) {
            let at = at.min(options.len().saturating_sub(1));
            for (i, o) in options.iter().enumerate() {
                let on = i == at;
                let marker = if on { "› " } else { "  " };
                let mark = if Some(i) == chosen { " ✓" } else { "" };
                let text = truncate(&format!("{marker}{}{mark}", o.label), box_w);
                let style = if on {
                    Style::new()
                        .fg(self.p.accent)
                        .add_modifier(Modifier::REVERSED | Modifier::BOLD)
                } else {
                    Style::new()
                };
                let pad = " ".repeat(box_w.saturating_sub(width(&text)));
                out.push(sline(format!("{text}{pad}"), style));
            }
        }
        out.extend(self.field_error(n, w));
        out
    }

    /// `(•) Staging  ( ) Production`, one per line when they do not fit.
    pub(super) fn ui_radio(&mut self, n: N<'_>, w: usize) -> Lines {
        let options = self.options(n);
        let value = self.field_value(n);
        let focused = self.field_target(
            n,
            UiKind::Radio {
                options: options.clone(),
            },
            value.clone(),
        );
        let current = scalar(&value);
        let item = |o: &UiOption| -> Vec<Span<'static>> {
            let on = o.value == current;
            let (dot, label) = if on {
                let st = if focused {
                    Style::new()
                        .fg(self.p.accent)
                        .add_modifier(Modifier::REVERSED | Modifier::BOLD)
                } else {
                    Style::new().fg(self.p.accent)
                };
                (
                    Span::styled("(•)", st),
                    Span::styled(
                        format!(" {}", o.label),
                        if focused { st } else { Style::new() },
                    ),
                )
            } else {
                (
                    Span::styled("( )", self.p.muted_style()),
                    Span::raw(format!(" {}", o.label)),
                )
            };
            vec![dot, label]
        };
        let total: usize = options.iter().map(|o| 4 + width(&o.label)).sum::<usize>()
            + 2 * options.len().saturating_sub(1);
        let mut out: Lines = if total <= w {
            let mut spans = Vec::new();
            for (i, o) in options.iter().enumerate() {
                if i > 0 {
                    spans.push(Span::raw("  "));
                }
                spans.extend(item(o));
            }
            vec![Line::from(spans)]
        } else {
            options
                .iter()
                .map(|o| Line::from(crate::text::truncate_spans(item(o), w)))
                .collect()
        };
        if focused && let Some(first) = out.first_mut() {
            self.mark_focus(first);
        }
        out.extend(self.field_error(n, w));
        out
    }

    /// `[x] label` for a checkbox, `● on  label` for a switch.
    pub(super) fn ui_toggle(&mut self, n: N<'_>, w: usize) -> Lines {
        let switch = n.name() == "Switch";
        let value = self.field_value(n);
        let on = value.as_bool().unwrap_or(false);
        let kind = if switch {
            UiKind::Switch
        } else {
            UiKind::CheckBox
        };
        let focused = self.field_target(n, kind, value);
        let label = n.line("label").unwrap_or_default();
        let (marker, color) = match (switch, on) {
            (false, true) => ("[x]", self.p.success),
            (false, false) => ("[ ]", self.p.muted),
            (true, true) => ("● on ", self.p.success),
            (true, false) => ("○ off", self.p.muted),
        };
        let marker_style = if focused {
            Style::new()
                .fg(color)
                .add_modifier(Modifier::REVERSED | Modifier::BOLD)
        } else {
            Style::new().fg(color)
        };
        let label_style = if focused {
            Style::new().fg(self.p.accent).add_modifier(Modifier::BOLD)
        } else {
            Style::new()
        };
        let mut spans = vec![
            Span::styled(marker, marker_style),
            Span::styled(format!(" {label}"), label_style),
        ];
        if n.bool("required") == Some(true) {
            spans.push(Span::styled(" *", Style::new().fg(self.p.danger)));
        }
        let mut line = Line::from(crate::text::truncate_spans(spans, w));
        if focused {
            self.mark_focus(&mut line);
        }
        let mut out = vec![line];
        out.extend(self.field_error(n, w));
        out
    }

    /// `━━●────── 10  (0–50)`
    pub(super) fn ui_slider(&mut self, n: N<'_>, w: usize) -> Lines {
        let min = n.num("min").unwrap_or(0.0);
        let max = n.num("max").unwrap_or(100.0).max(min);
        let step = n.num("step").filter(|s| *s > 0.0).unwrap_or(1.0);
        let value = self.field_value(n);
        let focused = self.field_target(n, UiKind::Slider { min, max, step }, value.clone());
        let v = value.as_f64().unwrap_or(min).clamp(min, max);
        let shown = number(v);
        let range = format!("  ({}–{})", number(min), number(max));
        let with_range = width(&shown) + width(&range) + 1 + 8 <= w;
        let side = width(&shown) + 1 + if with_range { width(&range) } else { 0 };
        let track = w.saturating_sub(side).clamp(1, 30);
        let at = if max > min {
            (((v - min) / (max - min)) * (track - 1) as f64).round() as usize
        } else {
            0
        };
        let accent = Style::new().fg(self.p.accent);
        let handle = if focused {
            accent.add_modifier(Modifier::REVERSED | Modifier::BOLD)
        } else {
            accent.add_modifier(Modifier::BOLD)
        };
        let mut spans = vec![
            Span::styled("━".repeat(at), accent),
            Span::styled("●", handle),
            Span::styled("─".repeat(track - 1 - at), self.p.muted_style()),
            Span::styled(
                format!(" {shown}"),
                Style::new().add_modifier(Modifier::BOLD),
            ),
        ];
        if with_range {
            spans.push(Span::styled(range, self.p.muted_style()));
        }
        let mut line = Line::from(spans);
        if focused {
            self.mark_focus(&mut line);
        }
        let mut out = vec![line];
        out.extend(self.field_error(n, w));
        out
    }

    /// `[ Label ]`: primary in the accent color, danger in red; reversed while it has focus.
    fn button_spans(&mut self, n: N<'_>) -> Vec<Span<'static>> {
        let label = n.line("label").unwrap_or_default();
        let url = n.str("url").filter(|u| !u.is_empty()).map(str::to_string);
        let copy = n.str("copy").map(str::to_string);
        let disabled = n.bool("disabled") == Some(true);
        let mut focused = false;
        if let Some(key) = n.key() {
            self.ui_target(
                key,
                UiKind::Button {
                    url: url.clone(),
                    copy: copy.clone(),
                    disabled,
                },
                label.clone(),
                Value::Null,
            );
            focused = self.ui_focused(key);
        }
        let mut style = match n.str("variant") {
            Some("primary") => Style::new().fg(self.p.accent).add_modifier(Modifier::BOLD),
            Some("danger") => Style::new().fg(self.p.danger),
            _ => Style::new(),
        };
        if disabled {
            style = self.p.muted_style().add_modifier(Modifier::DIM);
        }
        if focused {
            style = style.add_modifier(Modifier::REVERSED);
        }
        let tail = if url.is_some() {
            " ↗"
        } else if copy.is_some() {
            " ⧉"
        } else {
            ""
        };
        vec![Span::styled(format!("[ {label}{tail} ]"), style)]
    }

    pub(super) fn ui_button_line(&mut self, n: N<'_>, w: usize) -> Lines {
        let focused = n.key().is_some_and(|k| self.ui_focused(k));
        let spans = self.button_spans(n);
        let mut line = Line::from(crate::text::truncate_spans(spans, w));
        if focused {
            self.mark_focus(&mut line);
        }
        vec![line]
    }

    /// Buttons side by side, wrapping.
    pub(super) fn ui_buttons(&mut self, n: N<'_>, w: usize) -> Lines {
        let mut lines: Lines = Vec::new();
        let mut cur: Vec<Span<'static>> = Vec::new();
        let mut mark = false;
        for button in n.nodes("buttons") {
            let focused = button.key().is_some_and(|k| self.ui_focused(k));
            let spans = self.button_spans(button);
            let need: usize = spans.iter().map(|s| width(&s.content)).sum();
            let used: usize = cur.iter().map(|s| width(&s.content)).sum();
            if !cur.is_empty() && used + 2 + need > w {
                lines.push(Line::from(std::mem::take(&mut cur)));
            }
            if !cur.is_empty() {
                cur.push(Span::raw("  "));
            }
            cur.extend(spans);
            if focused && !mark {
                mark = true;
                // The mark goes on the line the button ends up on.
                let mut line = Line::from(std::mem::take(&mut cur));
                self.mark_focus(&mut line);
                cur = line.spans;
            }
        }
        if !cur.is_empty() {
            lines.push(Line::from(cur));
        }
        lines
    }

    /// The label (with `*` when required) over the input, the hint under it.
    pub(super) fn ui_form_control(&mut self, n: N<'_>, path: &str, w: usize) -> Lines {
        let Some(input) = n.node("input") else {
            return Vec::new();
        };
        let label = n.line("label");
        let required = input.bool("required") == Some(true);
        let focused = input.key().is_some_and(|k| self.ui_focused(k));
        let inline = matches!(input.name(), "CheckBox" | "Switch");
        let mut out: Lines = Vec::new();
        if let (Some(label), false) = (&label, inline) {
            let style = if focused {
                Style::new().fg(self.p.accent).add_modifier(Modifier::BOLD)
            } else {
                Style::new().add_modifier(Modifier::BOLD)
            };
            let mut spans = vec![Span::styled(truncate(label, w.saturating_sub(2)), style)];
            if required {
                spans.push(Span::styled(" *", Style::new().fg(self.p.danger)));
            }
            out.push(Line::from(spans));
        }
        let before = (self.ui.in_control, self.ui.control_label.take());
        self.ui.in_control = true;
        self.ui.control_label = label;
        let body = self.ui_node(input, &join(path, "input"), w);
        self.ui.in_control = before.0;
        self.ui.control_label = before.1;
        out.extend(body);
        if let Some(hint) = n.line("hint") {
            out.extend(wrap(&[Span::styled(hint, self.p.muted_style())], w));
        }
        out
    }

    /// The controls, then a gap and the form's buttons.
    pub(super) fn ui_form(&mut self, n: N<'_>, path: &str, w: usize) -> Lines {
        let before = self.ui.form.replace(path.to_string());
        let mut parts = Vec::new();
        let body = self.ui_children(n, "children", path, w, None);
        if !body.is_empty() {
            parts.push(body);
        }
        if let Some(buttons) = n.node("buttons") {
            let lines = self.ui_node(buttons, &join(path, "buttons"), w);
            if !lines.is_empty() {
                if !parts.is_empty() {
                    parts.push(vec![Line::default()]);
                }
                parts.push(lines);
            }
        }
        self.ui.form = before;
        stack(parts, None)
    }
}

/// `YYYY-MM-DD` with a month and a day that exist.
pub fn is_date(s: &str) -> bool {
    let parts: Vec<&str> = s.split('-').collect();
    let [y, m, d] = parts[..] else {
        return false;
    };
    if y.len() != 4 || m.len() != 2 || d.len() != 2 {
        return false;
    }
    let (Ok(y), Ok(m), Ok(d)) = (y.parse::<u32>(), m.parse::<u32>(), d.parse::<u32>()) else {
        return false;
    };
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let days = match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    (1..=days).contains(&d)
}
