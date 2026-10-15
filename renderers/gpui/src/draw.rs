//! The layout model as GPUI elements, one to one: boxes are flex divs, text runs are
//! `StyledText`, the rest are small compositions of divs. Nothing here decides what to show.

use std::cell::Cell;
use std::ops::Range;
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    Animation, AnimationExt, AnyElement, App, AppContext as _, Context, Font, FontStyle,
    FontWeight, Hsla, InteractiveElement, InteractiveText, IntoElement, ParentElement, Render,
    SharedString, StatefulInteractiveElement, StrikethroughStyle, Styled, StyledText, TextRun,
    UnderlineStyle, Window, div, img, px, relative, rgb,
};
use suite_protocol::DiffLineKind;

use crate::layout::{
    BoxNode, ButtonNode, CodeNode, DiffNode, Dir, FileNode, ImageNode, ImageSource, Insets, KvNode,
    Node, ProgressNode, TableNode, TextNode,
};
use crate::theme::{Paint, TextStyle};

/// Pressed button `n` of the item being drawn (click handler installed by the chat view).
pub type PressFn = Rc<dyn Fn(usize, &mut Window, &mut App)>;

/// The font families text is drawn in. The theme only says mono or not.
#[derive(Debug, Clone, PartialEq)]
pub struct ChatFonts {
    pub sans: SharedString,
    pub mono: SharedString,
}

impl Default for ChatFonts {
    fn default() -> ChatFonts {
        // GPUI on Linux and Windows only knows fonts by exact family name and falls back to its
        // own default when one is missing, so a wrong guess for mono is not fatal.
        let mono = if cfg!(target_os = "macos") {
            "Menlo"
        } else if cfg!(target_os = "windows") {
            "Consolas"
        } else {
            "DejaVu Sans Mono"
        };
        ChatFonts {
            sans: ".SystemUIFont".into(),
            mono: mono.into(),
        }
    }
}

const MAX_IMAGE_WIDTH: f32 = 480.0;

/// What drawing one item needs besides its layout.
pub struct DrawCx {
    pub fonts: ChatFonts,
    pub press: PressFn,
    /// The button that was just copied (shown as confirmed for a moment).
    pub copied: Option<usize>,
    /// Draw http(s) images (needs an HTTP client installed in the app); else show the fallback.
    pub remote_images: bool,
    /// Ids for the stateful elements (scrolling code, buttons), unique within the item.
    next_id: Cell<usize>,
}

impl DrawCx {
    pub fn new(
        fonts: ChatFonts,
        press: PressFn,
        copied: Option<usize>,
        remote_images: bool,
    ) -> DrawCx {
        DrawCx {
            fonts,
            press,
            copied,
            remote_images,
            next_id: Cell::new(0),
        }
    }

    fn id(&self, name: &'static str) -> (SharedString, usize) {
        let n = self.next_id.get();
        self.next_id.set(n + 1);
        (name.into(), n)
    }
}

pub fn color(paint: Paint) -> Hsla {
    rgb(paint.rgb).into()
}

fn pad_all<E: Styled>(el: E, pad: &Insets) -> E {
    el.pt(px(pad.top))
        .pr(px(pad.right))
        .pb(px(pad.bottom))
        .pl(px(pad.left))
}

pub fn node(node: &Node, cx: &DrawCx) -> AnyElement {
    match node {
        Node::Box(b) => boxed(b, cx),
        Node::Text(t) => text(t, cx),
        Node::Code(c) => code(c, cx),
        Node::Rule(paint) => div()
            .w_full()
            .h(px(1.))
            .flex_none()
            .bg(color(*paint))
            .into_any_element(),
        Node::Progress(p) => progress(p, cx),
        Node::Button(b) => button(b, cx),
        Node::Image(i) => image(i, cx),
        Node::File(f) => file(f, cx),
        Node::Diff(d) => diff(d, cx),
        Node::Table(t) => table(t, cx),
        Node::Kv(k) => kv(k, cx),
    }
}

fn boxed(b: &BoxNode, cx: &DrawCx) -> AnyElement {
    let mut el = div().flex().min_w_0().gap(px(b.gap));
    el = match b.dir {
        Dir::Column => el.flex_col(),
        Dir::Row => el.flex_row().items_start(),
        Dir::Wrap => el.flex_row().flex_wrap().items_start(),
    };
    el = pad_all(el, &b.pad);
    if b.role == "item-body" {
        el = el.flex_1();
    }
    if let Some(bg) = b.bg {
        el = el.bg(color(bg));
    }
    if let Some(border) = b.border {
        el = el.border_1().border_color(color(border));
    }
    if let Some((paint, _)) = b.bar {
        el = el.border_l_3().border_color(color(paint));
    }
    if b.radius > 0.0 {
        el = el.rounded(px(b.radius));
    }
    el.children(b.children.iter().map(|child| node(child, cx)))
        .into_any_element()
}

fn font(cx: &DrawCx, style: &TextStyle, run: &crate::layout::Run) -> Font {
    let weight = if run.bold {
        FontWeight::BOLD.0.max(style.weight as f32 + 100.0)
    } else {
        style.weight as f32
    };
    Font {
        family: if style.mono || run.mono {
            cx.fonts.mono.clone()
        } else {
            cx.fonts.sans.clone()
        },
        features: Default::default(),
        fallbacks: None,
        weight: FontWeight(weight),
        style: if run.italic {
            FontStyle::Italic
        } else {
            FontStyle::Normal
        },
    }
}

/// The text of a node and its styled runs, plus the byte ranges and URLs of links.
type Links = Vec<(Range<usize>, String)>;

fn styled(t: &TextNode, cx: &DrawCx) -> (SharedString, Vec<TextRun>, Links) {
    let mut string = String::new();
    let mut runs = Vec::new();
    let mut links = Vec::new();
    for run in &t.runs {
        let fg = color(run.color);
        let start = string.len();
        string.push_str(&run.text);
        if let Some(href) = &run.href {
            links.push((start..string.len(), href.clone()));
        }
        runs.push(TextRun {
            len: run.text.len(),
            font: font(cx, &t.style, run),
            color: fg,
            background_color: run.bg.map(color),
            underline: run.underline.then_some(UnderlineStyle {
                thickness: px(1.),
                color: Some(fg),
                wavy: false,
            }),
            strikethrough: run.strike.then_some(StrikethroughStyle {
                thickness: px(1.),
                color: Some(fg),
            }),
        });
    }
    // An empty cell or label still takes a line, and a zero-length run confuses text layout.
    if string.is_empty()
        && let Some(first) = runs.first_mut()
    {
        string.push(' ');
        first.len = 1;
    }
    (string.into(), runs, links)
}

fn text(t: &TextNode, cx: &DrawCx) -> AnyElement {
    if t.runs.is_empty() {
        return div().into_any_element();
    }
    let (string, runs, links) = styled(t, cx);
    let mut el = div()
        .text_size(px(t.style.size))
        .line_height(relative(1.45))
        .text_color(color(t.color));
    if t.nowrap {
        el = el.whitespace_nowrap().flex_none();
    }
    if let Some(width) = t.min_width {
        el = el.min_w(px(width)).flex_none();
    }
    let text = StyledText::new(string).with_runs(runs);
    if links.is_empty() {
        return el.child(text).into_any_element();
    }
    let ranges = links.iter().map(|(range, _)| range.clone()).collect();
    let urls: Vec<String> = links.into_iter().map(|(_, url)| url).collect();
    let id = cx.id("link");
    el.child(
        InteractiveText::new(id, text).on_click(ranges, move |ix, _, cx| {
            if let Some(url) = urls.get(ix) {
                cx.open_url(url);
            }
        }),
    )
    .into_any_element()
}

fn code(c: &CodeNode, cx: &DrawCx) -> AnyElement {
    let family = cx.fonts.mono.clone();
    div()
        .id(cx.id("code"))
        .w_full()
        .min_w_0()
        .flex_none()
        .overflow_x_scroll()
        .p(px(c.pad))
        .rounded(px(c.radius))
        .bg(color(c.bg))
        .child(
            div()
                .font_family(family)
                .text_size(px(c.style.size))
                .line_height(relative(1.4))
                .text_color(color(c.color))
                .whitespace_nowrap()
                .child(c.text.trim_end_matches('\n').to_string()),
        )
        .into_any_element()
}

fn progress(p: &ProgressNode, cx: &DrawCx) -> AnyElement {
    let bar = div()
        .flex_1()
        .min_w(px(96.))
        .h(px(p.height))
        .rounded(px(p.height / 2.0))
        .overflow_hidden()
        .bg(color(p.track))
        .child(div().h_full().w(relative(p.fraction)).bg(color(p.fill)));
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(p.gap))
        .child(bar)
        .child(text(&p.percent, cx))
        .children(p.label.as_ref().map(|label| text(label, cx)))
        .into_any_element()
}

fn button(b: &ButtonNode, cx: &DrawCx) -> AnyElement {
    let copied = cx.copied == Some(b.index);
    let press = cx.press.clone();
    let index = b.index;
    let hover = color(b.accent);
    let mut label = b.label.clone();
    if copied {
        label.color = b.success;
        for run in &mut label.runs {
            run.color = b.success;
        }
    }
    let el = div()
        .id(cx.id("button"))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(b.gap))
        .border_1()
        .border_color(color(if copied { b.success } else { b.border }))
        .rounded(px(b.radius))
        .bg(color(b.bg))
        .cursor_pointer()
        .hover(move |style| style.border_color(hover))
        .on_click(move |_, window, app| press(index, window, app));
    pad_all(el, &b.pad)
        .child(text(&label, cx))
        .child(
            div()
                .px(px(4.))
                .border_1()
                .border_color(color(b.border))
                .rounded(px(4.))
                .child(text(&b.hint, cx)),
        )
        .into_any_element()
}

fn image(i: &ImageNode, cx: &DrawCx) -> AnyElement {
    let sized = |el: gpui::Img| {
        let el = el.rounded(px(i.radius));
        match (i.width, i.height) {
            (Some(w), Some(h)) if w > 0 && h > 0 => {
                let width = (w as f32).min(MAX_IMAGE_WIDTH);
                el.w(px(width)).h(px(width * h as f32 / w as f32))
            }
            _ => el.max_w(px(MAX_IMAGE_WIDTH)).max_h(px(360.)),
        }
        .into_any_element()
    };
    let picture = match &i.source {
        ImageSource::Data(data) => Some(sized(img(data.0.clone()))),
        ImageSource::Remote(url) if cx.remote_images => Some(sized(img(url.clone()))),
        _ => None,
    };
    let picture = picture.unwrap_or_else(|| {
        pad_all(
            div()
                .flex_none()
                .border_1()
                .border_dashed()
                .border_color(color(i.fallback_border))
                .rounded(px(i.radius)),
            &i.fallback_pad,
        )
        .child(text(&i.fallback, cx))
        .into_any_element()
    });
    div()
        .flex()
        .flex_col()
        .items_start()
        .gap(px(2.))
        .child(picture)
        .children(i.caption.as_ref().map(|caption| text(caption, cx)))
        .into_any_element()
}

/// The full path of a file chip.
struct PathTip {
    path: SharedString,
    fg: Paint,
    bg: Paint,
    border: Paint,
}

impl Render for PathTip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .px(px(8.))
            .py(px(4.))
            .rounded(px(4.))
            .border_1()
            .border_color(color(self.border))
            .bg(color(self.bg))
            .text_color(color(self.fg))
            .text_size(px(12.))
            .child(self.path.clone())
    }
}

fn file(f: &FileNode, cx: &DrawCx) -> AnyElement {
    let tip = (
        SharedString::from(f.path.clone()),
        f.tip_fg,
        f.tip_bg,
        f.border,
    );
    let chip = pad_all(
        div()
            .id(cx.id("file"))
            .border_1()
            .border_color(color(f.border))
            .rounded(px(f.radius))
            .bg(color(f.bg)),
        &f.pad,
    )
    .tooltip(move |_, app| {
        let (path, fg, bg, border) = tip.clone();
        app.new(|_| PathTip {
            path,
            fg,
            bg,
            border,
        })
        .into()
    })
    .child(text(&f.label, cx));
    div().flex().flex_row().child(chip).into_any_element()
}

fn diff(d: &DiffNode, cx: &DrawCx) -> AnyElement {
    let mono = |el: gpui::Div| {
        el.font_family(cx.fonts.mono.clone())
            .text_size(px(d.style.size))
            .line_height(relative(1.4))
    };
    let rows = d.rows.iter().map(|row| {
        let mut el = mono(div()).flex().flex_row().pr(px(d.pad)).w_full();
        if row.kind == DiffLineKind::Hunk {
            el = el.bg(color(d.hunk_bg)).pl(px(d.pad));
        } else {
            let num = |n: &str| {
                div()
                    .w(px(d.num_width))
                    .flex_none()
                    .flex()
                    .justify_end()
                    .px(px(4.))
                    .text_color(color(row.num_color))
                    .child(n.to_string())
            };
            el = el.child(num(&row.old)).child(num(&row.new)).child(
                div()
                    .w(px(d.style.size * 1.4))
                    .flex_none()
                    .flex()
                    .justify_center()
                    .child(row.sign),
            );
        }
        el.text_color(color(row.color))
            .whitespace_nowrap()
            .child(row.text.clone())
    });
    let header = d.path.as_ref().map(|path| {
        div()
            .px(px(d.pad))
            .py(px(4.))
            .bg(color(d.header_bg))
            .border_b_1()
            .border_color(color(d.border))
            .child(text(path, cx))
    });
    div()
        .flex()
        .flex_col()
        .w_full()
        .min_w_0()
        .flex_none()
        .border_1()
        .border_color(color(d.border))
        .rounded(px(d.radius))
        .overflow_hidden()
        .children(header)
        .child(
            div()
                .id(cx.id("diff"))
                .overflow_x_scroll()
                .flex()
                .flex_col()
                .children(rows),
        )
        .into_any_element()
}

fn table(t: &TableNode, cx: &DrawCx) -> AnyElement {
    let columns = t.header.len();
    let cell = |node: &TextNode| {
        pad_all(div().border_b_1().border_color(color(t.border)), &t.pad).child(text(node, cx))
    };
    let cols = (0..columns).map(|c| {
        div()
            .flex()
            .flex_col()
            .flex_none()
            .child(cell(&t.header[c]))
            .children(t.rows.iter().map(|row| cell(&row[c])))
    });
    div()
        .id(cx.id("table"))
        .w_full()
        .min_w_0()
        .flex_none()
        .overflow_x_scroll()
        .child(div().flex().flex_row().children(cols))
        .into_any_element()
}

fn kv(k: &KvNode, cx: &DrawCx) -> AnyElement {
    div()
        .flex()
        .flex_row()
        .gap(px(k.gap))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_none()
                .children(k.rows.iter().map(|(key, _)| text(key, cx))),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .min_w_0()
                .children(k.rows.iter().map(|(_, value)| text(value, cx))),
        )
        .into_any_element()
}

/// The running tool icon fades in and out; the others stay as they are.
pub fn pulse(el: AnyElement) -> AnyElement {
    div()
        .child(el)
        .with_animation(
            "running",
            Animation::new(Duration::from_millis(1400)).repeat(),
            |el, t| el.opacity(0.35 + 0.65 * (t * std::f32::consts::TAU).sin().abs()),
        )
        .into_any_element()
}
