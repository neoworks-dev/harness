//! Drawing the Pad window: tabs, the editor body, problems, the chat pane, the open prompt.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, ClickEvent, Context, Entity, HighlightStyle, InteractiveElement, IntoElement,
    MouseButton, MouseDownEvent, MouseMoveEvent, ParentElement, Pixels, Render,
    StatefulInteractiveElement, Styled, StyledText, UnderlineStyle, Window, canvas, div, hsla, px,
    rgb, uniform_list,
};
use suite_renderer_gpui::ChatFonts;

use crate::buffer::display_line;
use crate::editor::{DiagState, Mark, Severity, segments};
use crate::field::Field;
use crate::pad::{EDITOR_CONTEXT, FONT, INPUT_CONTEXT, PROMPT_CONTEXT, Pad, TITLE};

pub const LINE_HEIGHT: f32 = 20.0;
const FONT_SIZE: f32 = 13.0;
const GUTTER: f32 = 64.0;
const TEXT_PAD: f32 = 8.0;
const CHAT_WIDTH: f32 = 440.0;
const PROBLEMS_HEIGHT: f32 = 120.0;

const BG: u32 = 0x1e1e1e;
const BORDER: u32 = 0x3c3c3c;
const MUTED: u32 = 0x808080;
const ACCENT: u32 = 0x569cd6;
const ERROR: u32 = 0xf48771;
const WARNING: u32 = 0xe5a000;
const INFO: u32 = 0x75beff;

/// Where the text starts, from the left edge of the editor body.
pub fn text_left() -> Pixels {
    px(GUTTER + TEXT_PAD)
}

fn severity_color(severity: Severity) -> u32 {
    match severity {
        Severity::Error => ERROR,
        Severity::Warning => WARNING,
        Severity::Info | Severity::Hint => INFO,
    }
}

fn style_of(marks: &[Mark], focused: bool) -> HighlightStyle {
    let mut style = HighlightStyle::default();
    let wavy = |color: u32| UnderlineStyle {
        thickness: px(1.),
        color: Some(rgb(color).into()),
        wavy: true,
    };
    for mark in marks {
        match mark {
            Mark::Selected => style.background_color = Some(hsla(0.58, 0.55, 0.4, 0.55)),
            Mark::Error => style.underline = Some(wavy(ERROR)),
            Mark::Warning => style.underline = Some(wavy(WARNING)),
            Mark::Info => style.underline = Some(wavy(INFO)),
            Mark::Cursor => {}
        }
    }
    if marks.contains(&Mark::Cursor) {
        style.background_color = Some(hsla(0.58, 0.6, 0.6, if focused { 0.7 } else { 0.3 }));
    }
    style
}

pub fn button(id: &'static str, label: &'static str) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .px_3()
        .py_1()
        .border_1()
        .border_color(rgb(BORDER))
        .rounded_md()
        .cursor_pointer()
        .hover(|style| style.bg(rgb(0x2a2d2e)))
        .child(label)
}

fn mono() -> String {
    ChatFonts::default().mono.to_string()
}

impl Render for Pad {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let status = if self.connected {
            String::new()
        } else {
            "daemon connection lost, retrying".to_string()
        };
        let editor = self.editor_column(window, cx);
        let chat = self.chat_column(window, cx);
        let root = div()
            .size_full()
            .flex()
            .flex_col()
            .relative()
            .bg(rgb(BG))
            .text_color(rgb(0xd4d4d4))
            .font_family(FONT)
            .text_size(px(14.))
            .child(
                div()
                    .flex()
                    .justify_between()
                    .px_3()
                    .py_2()
                    .border_b_1()
                    .border_color(rgb(BORDER))
                    .child(TITLE)
                    .child(div().text_color(rgb(WARNING)).child(status)),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_row()
                    .child(editor)
                    .child(chat),
            )
            .children(self.prompt.is_some().then(|| self.prompt_box(window, cx)));
        Pad::listeners(cx, root)
    }
}

impl Pad {
    // --- the editor ---

    fn editor_column(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let focused = self.editor_focus.is_focused(window);
        div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .child(self.tabs(cx))
            .child(self.editor_body(window, cx))
            .child(self.problems(cx))
            .child(self.status_line(focused))
            .into_any_element()
    }

    fn tabs(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let current = self.editor.current_index();
        div()
            .flex()
            .flex_row()
            .overflow_hidden()
            .border_b_1()
            .border_color(rgb(BORDER))
            .children(self.editor.docs().iter().enumerate().map(|(i, doc)| {
                let mut label = doc.name();
                if doc.dirty() {
                    label.push_str(" ●");
                }
                if doc.changed_on_disk {
                    label.push_str(" !");
                }
                div()
                    .id(("tab", i))
                    .px_3()
                    .py_1()
                    .cursor_pointer()
                    .border_r_1()
                    .border_color(rgb(BORDER))
                    .when(current == Some(i), |tab| {
                        tab.bg(rgb(0x2a2d2e)).text_color(rgb(0xffffff))
                    })
                    .when(current != Some(i), |tab| tab.text_color(rgb(MUTED)))
                    .child(label)
                    .on_click(cx.listener(move |pad, _: &ClickEvent, _, cx| pad.switch_tab(i, cx)))
            }))
    }

    fn editor_body(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let focused = self.editor_focus.is_focused(window);
        let entity: Entity<Pad> = cx.entity();
        let focus = self.editor_focus.clone();
        let bounds_cell = self.body.clone();
        let count = self.editor.current().map(|doc| doc.buffer.line_count());

        let frame = div()
            .key_context(EDITOR_CONTEXT)
            .track_focus(&self.editor_focus)
            .flex_1()
            .min_h_0()
            .relative()
            .font_family(mono())
            .text_size(px(FONT_SIZE))
            .border_1()
            .border_color(if focused { rgb(ACCENT) } else { rgb(BG) });

        let Some(count) = count else {
            return frame
                .flex()
                .items_center()
                .justify_center()
                .text_color(rgb(MUTED))
                .font_family(FONT)
                .child("No file open. ctrl-o opens one; the agent can open files too.")
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|pad, _: &MouseDownEvent, window, _| {
                        window.focus(&pad.editor_focus)
                    }),
                )
                .into_any_element();
        };

        // The width of one character, for mapping clicks to columns.
        let font = gpui::font(mono());
        let text_system = window.text_system();
        let char_width = text_system
            .advance(text_system.resolve_font(&font), px(FONT_SIZE), 'm')
            .map_or(px(FONT_SIZE * 0.6), |size| size.width);

        let list = uniform_list(
            "editor-lines",
            count,
            cx.processor(move |pad, range, window, cx| pad.lines(range, char_width, window, cx)),
        )
        .track_scroll(self.scroll.clone())
        .size_full();

        frame
            .child(list)
            .child(
                // Remembers where the body is drawn, and registers this view as the window's
                // input handler while the editor has focus.
                canvas(
                    move |bounds, _, _| bounds_cell.set(bounds),
                    move |bounds, _, window, cx| {
                        window.handle_input(
                            &focus,
                            gpui::ElementInputHandler::new(bounds, entity.clone()),
                            cx,
                        );
                    },
                )
                .absolute()
                .size_full(),
            )
            .into_any_element()
    }

    /// The rows of the visible range.
    fn lines(
        &mut self,
        range: std::ops::Range<usize>,
        char_width: Pixels,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let focused = self.editor_focus.is_focused(window);
        let Some(doc) = self.editor.current() else {
            return Vec::new();
        };
        let cursor_line = doc.buffer.cursor().line;
        let has_selection = doc.buffer.selection().is_some();
        range
            .map(|ix| {
                let text = doc.buffer.line(ix);
                let mut shown = display_line(text);
                let marks = doc.marks(ix);
                // The caret past the end of the line (or a selection running on) needs a cell.
                if marks.iter().any(|(r, _)| r.end > shown.len()) {
                    shown.push(' ');
                }
                let highlights: Vec<_> = segments(shown.len(), &marks)
                    .into_iter()
                    .map(|(r, ms)| (r, style_of(&ms, focused)))
                    .collect();
                let worst = doc.worst_on(ix);
                let revealed = doc.highlight == Some(ix);
                let number_color = match worst {
                    Some(severity) => rgb(severity_color(severity)),
                    None if ix == cursor_line => rgb(0xc6c6c6),
                    None => rgb(0x6e7681),
                };
                div()
                    .id(("line", ix))
                    .h(px(LINE_HEIGHT))
                    .w_full()
                    .flex()
                    .flex_row()
                    .items_center()
                    .when(revealed, |row| row.bg(hsla(0.13, 0.8, 0.5, 0.25)))
                    .when(!revealed && ix == cursor_line && !has_selection, |row| {
                        row.bg(rgb(0x262626))
                    })
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |pad, event: &MouseDownEvent, window, cx| {
                            window.focus(&pad.editor_focus);
                            pad.point_at(
                                ix,
                                event.position.x,
                                event.modifiers.shift,
                                cx,
                                char_width,
                            );
                        }),
                    )
                    .on_mouse_move(cx.listener(move |pad, event: &MouseMoveEvent, _, cx| {
                        if event.dragging() {
                            pad.point_at(ix, event.position.x, true, cx, char_width);
                        }
                    }))
                    .child(
                        div()
                            .w(px(GUTTER))
                            .flex_none()
                            .flex()
                            .flex_row()
                            .justify_end()
                            .gap(px(6.))
                            .pr(px(8.))
                            .text_color(number_color)
                            .child(div().child(if worst.is_some() { "●" } else { "" }))
                            .child(
                                div()
                                    .w(px(f32::from(char_width) * 4.0))
                                    .flex()
                                    .justify_end()
                                    .child((ix + 1).to_string()),
                            ),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .pl(px(TEXT_PAD))
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .child(StyledText::new(shown).with_highlights(highlights)),
                    )
                    .into_any_element()
            })
            .collect()
    }

    fn problems(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(doc) = self.editor.current() else {
            return div().into_any_element();
        };
        let errors = doc
            .diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Error)
            .count();
        let warnings = doc
            .diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Warning)
            .count();
        let summary = match &doc.diag_state {
            DiagState::Idle => "Problems".to_string(),
            DiagState::Checking => "Problems (checking…)".to_string(),
            DiagState::Done if doc.diagnostics.is_empty() => "No problems".to_string(),
            DiagState::Done => format!("Problems: {errors} errors, {warnings} warnings"),
            DiagState::Stale => format!(
                "Problems from before your edits: {errors} errors, {warnings} warnings (ctrl-s checks again)"
            ),
            DiagState::Unavailable(why) => format!("Diagnostics unavailable: {why}"),
        };
        div()
            .h(px(PROBLEMS_HEIGHT))
            .flex_none()
            .flex()
            .flex_col()
            .border_t_1()
            .border_color(rgb(BORDER))
            .child(
                div()
                    .px_3()
                    .py_1()
                    .text_size(px(12.))
                    .text_color(rgb(MUTED))
                    .child(summary),
            )
            .child(
                div()
                    .id("problems")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .font_family(mono())
                    .text_size(px(12.))
                    .children(doc.diagnostics.iter().enumerate().map(|(i, d)| {
                        let (line, column) = (d.line.saturating_sub(1), d.column.saturating_sub(1));
                        div()
                            .id(("problem", i))
                            .px_3()
                            .flex()
                            .flex_row()
                            .gap_3()
                            .cursor_pointer()
                            .hover(|row| row.bg(rgb(0x2a2d2e)))
                            .on_click(cx.listener(move |pad, _: &ClickEvent, window, cx| {
                                window.focus(&pad.editor_focus);
                                pad.go_to(line, column, cx);
                            }))
                            .child(
                                div()
                                    .w(px(64.))
                                    .flex_none()
                                    .text_color(rgb(MUTED))
                                    .child(format!("{}:{}", d.line, d.column)),
                            )
                            .child(
                                div()
                                    .w(px(56.))
                                    .flex_none()
                                    .text_color(rgb(severity_color(d.severity)))
                                    .child(d.severity.label()),
                            )
                            .child(div().flex_1().min_w_0().child(d.message.clone()))
                            .child(div().text_color(rgb(MUTED)).child(d.source.clone()))
                    })),
            )
            .into_any_element()
    }

    fn status_line(&self, _focused: bool) -> impl IntoElement {
        let place = match self.editor.current() {
            Some(doc) => {
                let at = doc.buffer.cursor();
                format!(
                    "{}   Ln {}, Col {}   {}   {}",
                    doc.path().display(),
                    at.line + 1,
                    at.col + 1,
                    doc.language(),
                    doc.state()
                )
            }
            None => "ctrl-o open · ctrl-1/2/3 or F6 move between editor, chat, input".to_string(),
        };
        div()
            .flex_none()
            .flex()
            .flex_row()
            .justify_between()
            .gap_3()
            .px_3()
            .py_1()
            .border_t_1()
            .border_color(rgb(BORDER))
            .text_size(px(12.))
            .child(
                div()
                    .min_w_0()
                    .overflow_hidden()
                    .text_color(rgb(MUTED))
                    .child(place),
            )
            .child(
                div()
                    .text_color(rgb(WARNING))
                    .child(self.notice.clone().unwrap_or_default()),
            )
    }

    // --- the open prompt ---

    fn prompt_box(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let entity: Entity<Pad> = cx.entity();
        let focus = self.prompt_focus.clone();
        let focused = self.prompt_focus.is_focused(window);
        let field = self.prompt.as_ref();
        div()
            .absolute()
            .top(px(48.))
            .left(px(48.))
            .w(px(560.))
            .p_3()
            .bg(rgb(0x252526))
            .border_1()
            .border_color(rgb(ACCENT))
            .rounded_md()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .text_color(rgb(MUTED))
                    .child("Open file (path, or path:line) — enter opens, escape cancels"),
            )
            .child(
                div()
                    .key_context(PROMPT_CONTEXT)
                    .track_focus(&self.prompt_focus)
                    .relative()
                    .child(field.map_or_else(
                        || div().into_any_element(),
                        |field| input_text(field, focused, "").into_any_element(),
                    ))
                    .child(
                        canvas(
                            |bounds, _, _| bounds,
                            move |bounds, _, window, cx| {
                                window.handle_input(
                                    &focus,
                                    gpui::ElementInputHandler::new(bounds, entity.clone()),
                                    cx,
                                );
                            },
                        )
                        .absolute()
                        .size_full(),
                    ),
            )
            .into_any_element()
    }

    // --- the chat ---

    fn chat_column(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let body = if self.follower.pane().is_some() {
            self.pane(window, cx).into_any_element()
        } else {
            self.collapsed(cx).into_any_element()
        };
        div()
            .w(px(CHAT_WIDTH))
            .flex_none()
            .min_h_0()
            .flex()
            .flex_col()
            .border_l_1()
            .border_color(rgb(BORDER))
            .child(body)
            .into_any_element()
    }

    /// Shown while the agent is in another app.
    fn collapsed(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let place = match &self.agent_at {
            Some(app) => format!("The agent is in {app}."),
            None => "The agent isn't in an app.".to_string(),
        };
        div()
            .flex_1()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_3()
            .child(place)
            .child(
                button("bring-here", "Bring the agent here")
                    .on_click(cx.listener(|pad, _: &ClickEvent, _, _| pad.bring_here())),
            )
    }

    /// The chat: transcript on top, the input line below.
    fn pane(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let working = if self.chat.read(cx).busy() {
            "working…"
        } else {
            ""
        };
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .justify_between()
                    .items_center()
                    .px_3()
                    .py_1()
                    .child(div().text_color(rgb(MUTED)).child(working))
                    .child(
                        button("send-home", "Send home")
                            .on_click(cx.listener(|pad, _: &ClickEvent, _, _| pad.send_home())),
                    ),
            )
            .child(div().flex_1().min_h_0().child(self.chat.clone()))
            .child(self.input_line(window, cx))
    }

    fn input_line(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let entity: Entity<Pad> = cx.entity();
        let focus = self.input_focus.clone();
        let focused = self.input_focus.is_focused(window);
        div()
            .key_context(INPUT_CONTEXT)
            .track_focus(&self.input_focus)
            // Clicking the line focuses it, in case focus was elsewhere when the pane opened.
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|pad, _: &MouseDownEvent, window, _| window.focus(&pad.input_focus)),
            )
            .relative()
            .m_3()
            .px_3()
            .py_2()
            .border_1()
            .border_color(if focused { rgb(ACCENT) } else { rgb(BORDER) })
            .rounded_md()
            .child(input_text(&self.field, focused, "Message the agent"))
            .child(
                // Registers this view as the window's input handler while it has focus, so
                // typed and composed text reaches `EntityInputHandler`.
                canvas(
                    |bounds, _, _| bounds,
                    move |bounds, _, window, cx| {
                        window.handle_input(
                            &focus,
                            gpui::ElementInputHandler::new(bounds, entity.clone()),
                            cx,
                        );
                    },
                )
                .absolute()
                .size_full(),
            )
    }
}

/// The typed text with a block caret on the character under it (a space at the end).
fn input_text(field: &Field, focused: bool, placeholder: &str) -> impl IntoElement {
    let caret = HighlightStyle {
        background_color: Some(hsla(0.58, 0.6, 0.6, 0.6)),
        ..Default::default()
    };
    if field.text().is_empty() {
        return div()
            .flex()
            .child(StyledText::new(" ").with_highlights(focused.then_some((0..1, caret))))
            .child(div().text_color(rgb(MUTED)).child(placeholder.to_string()))
            .into_any_element();
    }
    let text = field.text();
    let at = field.cursor();
    let width = text[at..].chars().next().map_or(1, char::len_utf8);
    let shown = if at == text.len() {
        format!("{text} ")
    } else {
        text.to_string()
    };
    StyledText::new(shown)
        .with_highlights(focused.then_some((at..at + width, caret)))
        .into_any_element()
}
