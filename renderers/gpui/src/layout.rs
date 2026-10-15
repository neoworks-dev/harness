//! The layout model: a resolved view item plus a theme becomes a tree of boxes, text runs and a
//! few leaf shapes, with colors as theme tokens and spacing in pixels. No GPUI in here, so it can
//! be snapshot-tested (`dump.rs`); `draw.rs` turns the tree into elements one to one.

use std::fmt;
use std::sync::Arc;

use suite_protocol::{
    Block, ButtonComponent, CardComponent, CodeComponent, Component, DiffComponent, DiffLineKind,
    FileComponent, ImageComponent, ItemKind, ItemState, KvComponent, LinkComponent, ListBlock,
    ProgressComponent, Span, TableComponent, TextComponent, TextRole, ViewItem,
};

use crate::theme::{ChatTheme, Paint, TextStyle};

/// Tool item states, drawn the same way in every renderer.
pub fn state_icon(state: ItemState) -> &'static str {
    match state {
        ItemState::Pending => "○",
        ItemState::Running => "◐",
        ItemState::Done => "✓",
        ItemState::Failed => "✗",
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Insets {
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub left: f32,
}

impl Insets {
    pub fn xy(vertical: f32, horizontal: f32) -> Insets {
        Insets {
            top: vertical,
            right: horizontal,
            bottom: vertical,
            left: horizontal,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    Column,
    Row,
    /// A row that continues on the next line when it runs out of width.
    Wrap,
}

/// One item as drawn: an optional header line, the body, and the frame around them.
#[derive(Debug, Clone, PartialEq)]
pub struct ItemLayout {
    pub r#ref: String,
    pub kind: ItemKind,
    pub state: Option<ItemState>,
    pub header: Option<Header>,
    pub body: Node,
    /// Thoughts show only their header until the user opens them.
    pub collapsible: bool,
    pub frame: Frame,
    /// The item's buttons in document order; `Node::Button` points into this by index.
    pub buttons: Vec<ButtonSpec>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Header {
    /// The tool state icon, drawn separately so a running one can be animated.
    pub icon: Option<TextNode>,
    pub label: TextNode,
}

/// What surrounds an item: the user's right-leaning bubble, the tool body's indent.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Frame {
    pub align_end: bool,
    /// Fraction of the chat's width.
    pub max_width: Option<f32>,
    pub bg: Option<Paint>,
    pub border: Option<Paint>,
    pub radius: f32,
    pub pad: Insets,
    pub body_indent: f32,
}

impl ItemLayout {
    /// Whether the keyboard can stop at this item.
    pub fn has_buttons(&self) -> bool {
        !self.buttons.is_empty()
    }

    /// The button that `key` triggers: a button's own `key`, else its 1-based position among the
    /// item's buttons (the key shown on it, as in the TUI). The first match wins.
    pub fn button_for_key(&self, key: &str) -> Option<usize> {
        self.buttons.iter().position(|b| b.hint == key)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ButtonSpec {
    pub label: String,
    pub action: String,
    pub args: Option<serde_json::Value>,
    /// The key shown on the button and bound while the item is selected.
    pub hint: String,
}

/// What pressing a button does.
#[derive(Debug, Clone, PartialEq)]
pub enum ButtonEffect {
    /// `copy:<text>`: put `<text>` on the clipboard, nothing goes to the bus.
    Copy(String),
    /// Anything else becomes `ui.action {session, ref, action, args}`.
    Action {
        action: String,
        args: Option<serde_json::Value>,
    },
}

impl ButtonSpec {
    pub fn effect(&self) -> ButtonEffect {
        match self.action.strip_prefix("copy:") {
            Some(text) => ButtonEffect::Copy(text.to_string()),
            None => ButtonEffect::Action {
                action: self.action.clone(),
                args: self.args.clone(),
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    Box(BoxNode),
    Text(TextNode),
    Code(CodeNode),
    Rule(Paint),
    Progress(ProgressNode),
    Button(ButtonNode),
    Image(ImageNode),
    File(FileNode),
    Diff(DiffNode),
    Table(TableNode),
    Kv(KvNode),
}

#[derive(Debug, Clone, PartialEq)]
pub struct BoxNode {
    /// What produced it (`card`, `stack`, `row`, `text`, `quote`, `list`, `list-item`).
    pub role: &'static str,
    pub dir: Dir,
    pub gap: f32,
    pub pad: Insets,
    pub bg: Option<Paint>,
    pub border: Option<Paint>,
    /// A bar down the left edge (quotes): color and width.
    pub bar: Option<(Paint, f32)>,
    pub radius: f32,
    pub children: Vec<Node>,
}

impl BoxNode {
    fn new(role: &'static str, dir: Dir, gap: f32, children: Vec<Node>) -> BoxNode {
        BoxNode {
            role,
            dir,
            gap,
            pad: Insets::default(),
            bg: None,
            border: None,
            bar: None,
            radius: 0.0,
            children,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TextNode {
    pub style: TextStyle,
    pub color: Paint,
    pub runs: Vec<Run>,
    pub nowrap: bool,
    pub min_width: Option<f32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Run {
    pub text: String,
    pub color: Paint,
    pub bold: bool,
    pub italic: bool,
    pub strike: bool,
    pub underline: bool,
    pub mono: bool,
    pub bg: Option<Paint>,
    /// Set for links that are safe to open (http, https, mailto).
    pub href: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CodeNode {
    pub text: String,
    pub language: Option<String>,
    pub style: TextStyle,
    pub color: Paint,
    pub bg: Paint,
    pub pad: f32,
    pub radius: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProgressNode {
    /// 0..=1.
    pub fraction: f32,
    pub percent: TextNode,
    pub label: Option<TextNode>,
    pub track: Paint,
    pub fill: Paint,
    pub height: f32,
    pub gap: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ButtonNode {
    /// Index into [`ItemLayout::buttons`].
    pub index: usize,
    pub label: TextNode,
    /// The key hint, drawn after the label.
    pub hint: TextNode,
    pub bg: Paint,
    pub border: Paint,
    /// Border while hovered.
    pub accent: Paint,
    /// Label and border for a moment after a `copy:` press.
    pub success: Paint,
    pub radius: f32,
    pub pad: Insets,
    pub gap: f32,
}

/// What an image can be drawn from. Everything else shows the fallback; nothing is fetched.
#[derive(Debug, Clone, PartialEq)]
pub enum ImageSource {
    /// A `data:image/…;base64,` URL, decoded.
    Data(ImageData),
    /// An http(s) URL; drawn only when the chat view is allowed to fetch.
    Remote(String),
    /// `asset://` handles (Phase 5), paths, unknown schemes, undecodable data.
    None,
}

/// Decoded bytes of a `data:` image, ready for GPUI.
#[derive(Clone)]
pub struct ImageData(pub Arc<gpui::Image>);

impl PartialEq for ImageData {
    fn eq(&self, other: &ImageData) -> bool {
        self.0.id() == other.0.id()
    }
}

impl fmt::Debug for ImageData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "ImageData({:?}, {} bytes)",
            self.0.format,
            self.0.bytes.len()
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ImageNode {
    pub src: String,
    pub alt: Option<String>,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub source: ImageSource,
    /// `[image W×H · src]` (`[image · src]` without a size): what the other sources show.
    pub fallback: TextNode,
    pub fallback_border: Paint,
    pub fallback_pad: Insets,
    pub caption: Option<TextNode>,
    pub radius: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FileNode {
    /// The full path, shown as a tooltip.
    pub path: String,
    pub tip_fg: Paint,
    pub tip_bg: Paint,
    pub label: TextNode,
    pub bg: Paint,
    pub border: Paint,
    pub radius: f32,
    pub pad: Insets,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DiffNode {
    pub path: Option<TextNode>,
    pub rows: Vec<DiffRow>,
    pub style: TextStyle,
    pub border: Paint,
    pub header_bg: Paint,
    pub hunk_bg: Paint,
    /// Width of each line-number column.
    pub num_width: f32,
    pub radius: f32,
    pub pad: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DiffRow {
    pub kind: DiffLineKind,
    pub old: String,
    pub new: String,
    pub sign: &'static str,
    pub text: String,
    pub color: Paint,
    pub num_color: Paint,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TableNode {
    pub header: Vec<TextNode>,
    pub rows: Vec<Vec<TextNode>>,
    pub border: Paint,
    pub pad: Insets,
}

#[derive(Debug, Clone, PartialEq)]
pub struct KvNode {
    pub rows: Vec<(TextNode, TextNode)>,
    pub gap: f32,
}

/// Inherited while walking down: the text color, whether we sit on a surface already (cards and
/// bubbles, where code uses the page background to stay visible), italics, and a size factor.
#[derive(Clone, Copy)]
struct Ctx {
    fg: Paint,
    on_surface: bool,
    italic: bool,
    scale: f32,
}

struct Layout<'a> {
    theme: &'a ChatTheme,
    buttons: Vec<ButtonSpec>,
}

pub fn layout_item(item: &ViewItem, theme: &ChatTheme) -> ItemLayout {
    let mut layout = Layout {
        theme,
        buttons: Vec::new(),
    };
    let mut ctx = Ctx {
        fg: theme.fg,
        on_surface: false,
        italic: false,
        scale: 1.0,
    };
    let mut frame = Frame::default();
    let mut header = None;
    let mut collapsible = false;
    let label = item.label.as_deref().filter(|l| !l.is_empty());
    match item.kind {
        ItemKind::User => {
            frame = Frame {
                align_end: true,
                max_width: Some(0.85),
                bg: Some(theme.surface),
                border: Some(theme.border),
                radius: 12.0,
                pad: Insets::xy(theme.m, theme.l),
                body_indent: 0.0,
            };
            ctx.on_surface = true;
            header = Some(layout.header(
                None,
                label.map_or_else(|| "»".to_string(), |l| format!("» {l}")),
            ));
        }
        ItemKind::Tool => {
            frame.body_indent = theme.l;
            let icon = item.state.map(|state| layout.state_icon(state));
            header = (icon.is_some() || label.is_some())
                .then(|| layout.header(icon, label.unwrap_or_default().to_string()));
        }
        ItemKind::Thought => {
            collapsible = true;
            ctx.fg = theme.muted;
            ctx.italic = true;
            header = Some(layout.header(None, "… thinking".into()));
        }
        ItemKind::Notice => {
            ctx.fg = theme.muted;
            ctx.scale = 0.9;
        }
        ItemKind::Agent => {}
    }
    let body = layout.component(&item.component, ctx);
    ItemLayout {
        r#ref: item.r#ref.clone(),
        kind: item.kind,
        state: item.state,
        header,
        body,
        collapsible,
        frame,
        buttons: layout.buttons,
    }
}

pub fn layout_items(items: &[ViewItem], theme: &ChatTheme) -> Vec<ItemLayout> {
    items.iter().map(|item| layout_item(item, theme)).collect()
}

/// Only links that can't run anything: the daemon passes hrefs through as the agent wrote them.
pub fn safe_url(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    ["http://", "https://", "mailto:"]
        .iter()
        .any(|scheme| lower.starts_with(scheme))
}

/// Where an image's pixels come from: `data:image/` (decoded here), http(s), or nowhere.
fn image_source(src: &str) -> ImageSource {
    let lower = src.get(..11).map(str::to_ascii_lowercase);
    if lower.as_deref() == Some("data:image/") {
        return decode_data_url(src).map_or(ImageSource::None, ImageSource::Data);
    }
    if safe_url(src) && !src.to_ascii_lowercase().starts_with("mailto:") {
        return ImageSource::Remote(src.to_string());
    }
    ImageSource::None
}

fn decode_data_url(url: &str) -> Option<ImageData> {
    let (meta, payload) = url.split_once(',')?;
    let mut parts = meta["data:image/".len()..].split(';');
    let mime = parts.next()?.to_ascii_lowercase();
    if !parts.any(|p| p.eq_ignore_ascii_case("base64")) {
        return None;
    }
    let format = match mime.as_str() {
        "png" => gpui::ImageFormat::Png,
        "jpeg" | "jpg" => gpui::ImageFormat::Jpeg,
        "webp" => gpui::ImageFormat::Webp,
        "gif" => gpui::ImageFormat::Gif,
        "svg+xml" => gpui::ImageFormat::Svg,
        "bmp" => gpui::ImageFormat::Bmp,
        "tiff" => gpui::ImageFormat::Tiff,
        _ => return None,
    };
    let bytes = base64_decode(payload)?;
    Some(ImageData(Arc::new(gpui::Image::from_bytes(format, bytes))))
}

/// Standard base64, padding optional, whitespace ignored.
fn base64_decode(input: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(input.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in input.trim_end_matches('=').chars() {
        let v = match c {
            'A'..='Z' => c as u32 - 'A' as u32,
            'a'..='z' => c as u32 - 'a' as u32 + 26,
            '0'..='9' => c as u32 - '0' as u32 + 52,
            '+' => 62,
            '/' => 63,
            c if c.is_ascii_whitespace() => continue,
            _ => return None,
        };
        acc = acc << 6 | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    (!out.is_empty()).then_some(out)
}

fn base_name(path: &str) -> &str {
    let trimmed = path.trim_end_matches(['/', '\\']);
    trimmed.rsplit(['/', '\\']).next().unwrap_or(trimmed)
}

impl Layout<'_> {
    fn text_style(&self, role: Option<TextRole>, scale: f32) -> TextStyle {
        let base = match role {
            Some(TextRole::Title) => self.theme.title,
            Some(TextRole::Code) => self.theme.code,
            Some(TextRole::Body) | None => self.theme.body,
        };
        TextStyle {
            size: base.size * scale,
            ..base
        }
    }

    fn plain(&self, text: &str, style: TextStyle, color: Paint) -> TextNode {
        TextNode {
            style,
            color,
            runs: vec![Run {
                text: text.to_string(),
                color,
                bold: false,
                italic: false,
                strike: false,
                underline: false,
                mono: false,
                bg: None,
                href: None,
            }],
            nowrap: false,
            min_width: None,
        }
    }

    fn header(&self, icon: Option<TextNode>, label: String) -> Header {
        let style = self.text_style(None, 0.9);
        Header {
            icon,
            label: self.plain(&label, style, self.theme.muted),
        }
    }

    fn state_icon(&self, state: ItemState) -> TextNode {
        let color = match state {
            ItemState::Pending => self.theme.muted,
            ItemState::Running => self.theme.accent,
            ItemState::Done => self.theme.success,
            ItemState::Failed => self.theme.danger,
        };
        self.plain(state_icon(state), self.text_style(None, 0.9), color)
    }

    fn component(&mut self, component: &Component, ctx: Ctx) -> Node {
        let theme = self.theme;
        match component {
            Component::CardComponent(card) => self.card(card, ctx),
            Component::RowComponent(row) => {
                let gap = row.gap.map_or(theme.s, |g| theme.space(g));
                let children = self.children(&row.children, ctx);
                Node::Box(BoxNode::new("row", Dir::Wrap, gap, children))
            }
            Component::StackComponent(stack) => {
                let gap = stack.gap.map_or(theme.s, |g| theme.space(g));
                let children = self.children(&stack.children, ctx);
                Node::Box(BoxNode::new("stack", Dir::Column, gap, children))
            }
            Component::TextComponent(text) => self.text(text, ctx),
            Component::ImageComponent(image) => self.image(image, ctx),
            Component::FileComponent(file) => self.file(file, ctx),
            Component::CodeComponent(code) => self.code(code, ctx),
            Component::DiffComponent(diff) => self.diff(diff),
            Component::TableComponent(table) => self.table(table, ctx),
            Component::KvComponent(kv) => self.kv(kv, ctx),
            Component::ProgressComponent(progress) => self.progress(progress, ctx),
            Component::ButtonComponent(button) => self.button(button, ctx),
            Component::LinkComponent(link) => self.link(link, ctx),
            Component::Unknown(value) => {
                let kind = value.get("type").and_then(|t| t.as_str()).unwrap_or("?");
                let style = self.text_style(None, ctx.scale);
                Node::Text(self.plain(
                    &format!("[unsupported component: {kind}]"),
                    style,
                    theme.muted,
                ))
            }
        }
    }

    fn children(&mut self, children: &[Component], ctx: Ctx) -> Vec<Node> {
        children.iter().map(|c| self.component(c, ctx)).collect()
    }

    fn card(&mut self, card: &CardComponent, ctx: Ctx) -> Node {
        let theme = self.theme;
        let tone = card.tone.map(|tone| theme.tone(tone));
        let inner = Ctx {
            on_surface: true,
            ..ctx
        };
        let mut children = Vec::new();
        if let Some(title) = &card.title {
            let mut node = self.plain(
                title,
                self.text_style(Some(TextRole::Title), ctx.scale),
                tone.unwrap_or(ctx.fg),
            );
            node.runs[0].italic = ctx.italic;
            children.push(Node::Text(node));
        }
        children.extend(self.children(&card.children, inner));
        let mut node = BoxNode::new("card", Dir::Column, theme.s, children);
        node.pad = Insets::xy(theme.m, theme.l);
        node.bg = Some(theme.surface);
        node.border = Some(tone.unwrap_or(theme.border));
        node.radius = 8.0;
        Node::Box(node)
    }

    // --- text ---

    fn text(&mut self, text: &TextComponent, ctx: Ctx) -> Node {
        let ctx = Ctx {
            fg: text.tone.map_or(ctx.fg, |tone| self.theme.tone(tone)),
            ..ctx
        };
        let blocks = self.blocks(&text.blocks, ctx, text.role);
        Node::Box(BoxNode::new("text", Dir::Column, self.theme.s, blocks))
    }

    fn blocks(&self, blocks: &[Block], ctx: Ctx, role: Option<TextRole>) -> Vec<Node> {
        blocks
            .iter()
            .filter_map(|block| self.block(block, ctx, role))
            .collect()
    }

    fn block(&self, block: &Block, ctx: Ctx, role: Option<TextRole>) -> Option<Node> {
        let theme = self.theme;
        Some(match block {
            Block::ParagraphBlock(p) => {
                Node::Text(self.spans(&p.spans, self.text_style(role, ctx.scale), ctx))
            }
            Block::HeadingBlock(h) => {
                let mut style = if h.level <= 2 {
                    self.text_style(Some(TextRole::Title), ctx.scale)
                } else {
                    self.text_style(role, ctx.scale)
                };
                style.weight = theme.title.weight;
                Node::Text(self.spans(&h.spans, style, ctx))
            }
            Block::ListBlock(list) => self.list(list, ctx, role),
            Block::CodeBlock(code) => {
                Node::Code(self.code_node(&code.code, code.language.as_deref(), ctx))
            }
            Block::QuoteBlock(quote) => {
                let inner = Ctx {
                    fg: theme.muted,
                    ..ctx
                };
                let children = self.blocks(&quote.blocks, inner, role);
                let mut node = BoxNode::new("quote", Dir::Column, theme.s, children);
                node.pad.left = theme.m;
                node.bar = Some((theme.border, 3.0));
                Node::Box(node)
            }
            Block::RuleBlock(_) => Node::Rule(theme.border),
            Block::Unknown(_) => return None,
        })
    }

    fn list(&self, list: &ListBlock, ctx: Ctx, role: Option<TextRole>) -> Node {
        let theme = self.theme;
        let style = self.text_style(role, ctx.scale);
        let start = list.start.unwrap_or(1);
        let markers: Vec<String> = list
            .items
            .iter()
            .enumerate()
            .map(|(i, item)| match item.checked {
                Some(true) => "[x]".to_string(),
                Some(false) => "[ ]".to_string(),
                None if list.ordered => format!("{}.", start + i as i64),
                None => "•".to_string(),
            })
            .collect();
        let chars = markers.iter().map(|m| m.chars().count()).max().unwrap_or(1);
        // Markers share a column so item text lines up; nested lists indent through it.
        let width = (chars as f32 * style.size * 0.6).max(style.size * 1.2);
        let rows = list
            .items
            .iter()
            .zip(markers)
            .map(|(item, marker)| {
                let mut marker = self.plain(&marker, style, ctx.fg);
                marker.min_width = Some(width);
                let blocks = self.blocks(&item.blocks, ctx, role);
                let body = Node::Box(BoxNode::new("item-body", Dir::Column, theme.s, blocks));
                Node::Box(BoxNode::new(
                    "list-item",
                    Dir::Row,
                    theme.s,
                    vec![Node::Text(marker), body],
                ))
            })
            .collect();
        Node::Box(BoxNode::new("list", Dir::Column, theme.s / 2.0, rows))
    }

    fn spans(&self, spans: &[Span], style: TextStyle, ctx: Ctx) -> TextNode {
        let theme = self.theme;
        let runs = spans
            .iter()
            .filter(|span| !span.text.is_empty())
            .map(|span| {
                let link = span.href.as_deref().filter(|href| safe_url(href));
                let code = span.code == Some(true);
                Run {
                    text: span.text.clone(),
                    color: if link.is_some() { theme.accent } else { ctx.fg },
                    bold: span.bold == Some(true),
                    italic: span.italic == Some(true) || ctx.italic,
                    strike: span.strike == Some(true),
                    underline: link.is_some(),
                    mono: code,
                    bg: code.then_some(if ctx.on_surface {
                        theme.bg
                    } else {
                        theme.surface
                    }),
                    href: link.map(String::from),
                }
            })
            .collect();
        TextNode {
            style,
            color: ctx.fg,
            runs,
            nowrap: false,
            min_width: None,
        }
    }

    fn code_node(&self, code: &str, language: Option<&str>, ctx: Ctx) -> CodeNode {
        let theme = self.theme;
        CodeNode {
            text: code.to_string(),
            language: language.map(String::from),
            style: TextStyle {
                size: theme.code.size * ctx.scale,
                ..theme.code
            },
            color: ctx.fg,
            bg: if ctx.on_surface {
                theme.bg
            } else {
                theme.surface
            },
            pad: theme.m,
            radius: 6.0,
        }
    }

    fn code(&self, code: &CodeComponent, ctx: Ctx) -> Node {
        Node::Code(self.code_node(&code.code, code.language.as_deref(), ctx))
    }

    // --- leaves ---

    fn image(&self, image: &ImageComponent, ctx: Ctx) -> Node {
        let theme = self.theme;
        let style = TextStyle {
            size: theme.code.size * ctx.scale,
            ..theme.code
        };
        let size = match (image.width, image.height) {
            (Some(w), Some(h)) => format!(" {w}×{h}"),
            _ => String::new(),
        };
        let fallback = self.plain(
            &format!("[image{size} · {}]", image.src),
            style,
            theme.muted,
        );
        let source = image_source(&image.src);
        Node::Image(ImageNode {
            src: image.src.clone(),
            alt: image.alt.clone(),
            width: image.width,
            height: image.height,
            source,
            fallback,
            fallback_border: theme.border,
            fallback_pad: Insets::xy(theme.s, theme.m),
            caption: image.caption.as_ref().map(|caption| {
                self.plain(caption, self.text_style(None, 0.9 * ctx.scale), theme.muted)
            }),
            radius: 6.0,
        })
    }

    fn file(&self, file: &FileComponent, ctx: Ctx) -> Node {
        let theme = self.theme;
        let name = file
            .name
            .as_deref()
            .unwrap_or_else(|| base_name(&file.path));
        Node::File(FileNode {
            path: file.path.clone(),
            tip_fg: theme.fg,
            tip_bg: theme.bg,
            label: self.plain(
                &format!("📄 {name}"),
                self.text_style(None, ctx.scale),
                ctx.fg,
            ),
            bg: if ctx.on_surface {
                theme.bg
            } else {
                theme.surface
            },
            border: theme.border,
            radius: 12.0,
            pad: Insets::xy(0.0, theme.m),
        })
    }

    fn diff(&self, diff: &DiffComponent) -> Node {
        let theme = self.theme;
        let style = theme.code;
        let digits = diff
            .lines
            .iter()
            .flat_map(|line| [line.old, line.new])
            .flatten()
            .map(|n| n.to_string().len())
            .max()
            .unwrap_or(1)
            .max(3);
        let rows = diff
            .lines
            .iter()
            .map(|line| {
                let (sign, color) = match line.kind {
                    DiffLineKind::Add => ("+", theme.success),
                    DiffLineKind::Remove => ("-", theme.danger),
                    DiffLineKind::Context => (" ", theme.fg),
                    DiffLineKind::Hunk => ("", theme.muted),
                };
                let number = |n: Option<i64>| n.map(|n| n.to_string()).unwrap_or_default();
                DiffRow {
                    kind: line.kind,
                    old: number(line.old),
                    new: number(line.new),
                    sign,
                    text: line.text.clone(),
                    color,
                    num_color: theme.muted,
                }
            })
            .collect();
        Node::Diff(DiffNode {
            path: diff.path.as_deref().map(|path| {
                let mut node = self.plain(path, style, theme.muted);
                node.nowrap = true;
                node
            }),
            rows,
            style,
            border: theme.border,
            header_bg: theme.surface,
            hunk_bg: theme.surface,
            num_width: digits as f32 * style.size * 0.62 + theme.s * 2.0,
            radius: 6.0,
            pad: theme.m,
        })
    }

    fn table(&self, table: &TableComponent, ctx: Ctx) -> Node {
        let theme = self.theme;
        let style = self.text_style(None, ctx.scale);
        let cell = |text: &str, color: Paint, weight: Option<u16>| {
            let mut node = self.plain(text, style, color);
            node.nowrap = true;
            if let Some(weight) = weight {
                node.style.weight = weight;
            }
            node
        };
        let columns = table
            .columns
            .len()
            .max(table.rows.iter().map(Vec::len).max().unwrap_or(0));
        let header = (0..columns)
            .map(|i| {
                let text = table.columns.get(i).map_or("", String::as_str);
                cell(text, theme.muted, Some(theme.title.weight))
            })
            .collect();
        let rows = table
            .rows
            .iter()
            .map(|row| {
                (0..columns)
                    .map(|i| cell(row.get(i).map_or("", String::as_str), ctx.fg, None))
                    .collect()
            })
            .collect();
        Node::Table(TableNode {
            header,
            rows,
            border: theme.border,
            pad: Insets::xy(theme.s, theme.m),
        })
    }

    fn kv(&self, kv: &KvComponent, ctx: Ctx) -> Node {
        let style = self.text_style(None, ctx.scale);
        Node::Kv(KvNode {
            rows: kv
                .items
                .iter()
                .map(|item| {
                    (
                        self.plain(&item.key, style, self.theme.muted),
                        self.plain(&item.value, style, ctx.fg),
                    )
                })
                .collect(),
            gap: self.theme.l,
        })
    }

    fn progress(&self, progress: &ProgressComponent, ctx: Ctx) -> Node {
        let theme = self.theme;
        let style = self.text_style(None, ctx.scale);
        let fraction = if progress.value.is_finite() {
            progress.value.clamp(0.0, 1.0) as f32
        } else {
            0.0
        };
        let percent = (fraction * 100.0).round() as u32;
        let mut percent_text = self.plain(&format!("{percent}%"), style, ctx.fg);
        percent_text.nowrap = true;
        Node::Progress(ProgressNode {
            fraction,
            percent: percent_text,
            label: progress
                .label
                .as_deref()
                .map(|label| self.plain(label, style, theme.muted)),
            track: theme.border,
            fill: theme.accent,
            height: 8.0,
            gap: theme.m,
        })
    }

    fn button(&mut self, button: &ButtonComponent, ctx: Ctx) -> Node {
        let theme = self.theme;
        let index = self.buttons.len();
        // A button without a `key` answers to its 1-based position, as in the TUI.
        let hint = button
            .key
            .clone()
            .filter(|key| !key.is_empty())
            .unwrap_or_else(|| (index + 1).to_string());
        self.buttons.push(ButtonSpec {
            label: button.label.clone(),
            action: button.action.clone(),
            args: button.args.clone(),
            hint: hint.clone(),
        });
        let style = self.text_style(None, ctx.scale);
        let mut label = self.plain(&button.label, style, theme.fg);
        label.nowrap = true;
        let mut hint_node = self.plain(
            &hint,
            TextStyle {
                size: theme.code.size * 0.85 * ctx.scale,
                ..theme.code
            },
            theme.muted,
        );
        hint_node.nowrap = true;
        Node::Button(ButtonNode {
            index,
            label,
            hint: hint_node,
            bg: if ctx.on_surface {
                theme.bg
            } else {
                theme.surface
            },
            border: theme.border,
            accent: theme.accent,
            success: theme.success,
            radius: 6.0,
            pad: Insets::xy(theme.s, theme.m),
            gap: theme.s,
        })
    }

    fn link(&self, link: &LinkComponent, ctx: Ctx) -> Node {
        let theme = self.theme;
        let text = link.text.as_deref().unwrap_or(&link.url);
        let mut node = self.plain(text, self.text_style(None, ctx.scale), theme.accent);
        node.runs[0].underline = true;
        node.runs[0].href = safe_url(&link.url).then(|| link.url.clone());
        Node::Text(node)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn item(value: serde_json::Value) -> ViewItem {
        serde_json::from_value(value).unwrap()
    }

    fn buttons_item() -> ItemLayout {
        layout_item(
            &item(
                json!({"ref": "r", "kind": "tool", "state": "done", "component": {
                "type": "stack", "children": [
                    {"type": "button", "label": "A", "action": "app.a"},
                    {"type": "card", "children": [
                        {"type": "button", "label": "B", "action": "copy:hello", "key": "c"}
                    ]},
                    {"type": "button", "label": "C", "action": "app.c", "args": {"x": 1}}
                ]}}),
            ),
            &ChatTheme::default(),
        )
    }

    #[test]
    fn button_hints_are_key_or_position_in_the_item() {
        let layout = buttons_item();
        let hints: Vec<&str> = layout.buttons.iter().map(|b| b.hint.as_str()).collect();
        assert_eq!(hints, ["1", "c", "3"]);
        assert_eq!(layout.button_for_key("1"), Some(0));
        assert_eq!(layout.button_for_key("c"), Some(1));
        // Button B has its own key, so its position doesn't also trigger it.
        assert_eq!(layout.button_for_key("2"), None);
        assert_eq!(layout.button_for_key("3"), Some(2));
        assert_eq!(layout.button_for_key("x"), None);
    }

    #[test]
    fn copy_is_local_everything_else_is_an_action() {
        let layout = buttons_item();
        assert_eq!(
            layout.buttons[1].effect(),
            ButtonEffect::Copy("hello".into())
        );
        assert_eq!(
            layout.buttons[2].effect(),
            ButtonEffect::Action {
                action: "app.c".into(),
                args: Some(json!({"x": 1}))
            }
        );
    }

    #[test]
    fn base64_and_image_sources() {
        assert_eq!(base64_decode("aGk="), Some(b"hi".to_vec()));
        assert_eq!(base64_decode("aGk"), Some(b"hi".to_vec()));
        assert_eq!(base64_decode("aG\nk="), Some(b"hi".to_vec()));
        assert_eq!(base64_decode("a$k="), None);
        let png = image_source("data:image/png;base64,iVBORw0KGgo=");
        assert!(matches!(png, ImageSource::Data(_)), "{png:?}");
        assert!(matches!(
            image_source("DATA:image/svg+xml;base64,PHN2Zz4="),
            ImageSource::Data(_)
        ));
        assert_eq!(image_source("data:image/png,raw"), ImageSource::None);
        assert_eq!(
            image_source("data:text/html;base64,aGk="),
            ImageSource::None
        );
        assert_eq!(
            image_source("https://example.com/a.png"),
            ImageSource::Remote("https://example.com/a.png".into())
        );
        for src in [
            "asset://img-1",
            "/tmp/shot.png",
            "file:///tmp/a.png",
            "ftp://x/y.png",
            "mailto:a@b.c",
        ] {
            assert_eq!(image_source(src), ImageSource::None, "{src}");
        }
    }

    #[test]
    fn urls_are_filtered() {
        assert!(safe_url("https://example.com"));
        assert!(safe_url("MAILTO:a@b.c"));
        assert!(!safe_url("javascript:alert(1)"));
        assert!(!safe_url("/etc/passwd"));
    }

    #[test]
    fn user_items_get_a_bubble_and_a_header() {
        let theme = ChatTheme::default();
        let layout = layout_item(
            &item(json!({"ref": "u", "kind": "user", "label": "overlay",
                "component": {"type": "text", "blocks": []}})),
            &theme,
        );
        assert!(layout.frame.align_end);
        let header = layout.header.unwrap();
        assert_eq!(header.label.runs[0].text, "» overlay");

        let bare = layout_item(
            &item(json!({"ref": "u", "kind": "user", "component": {"type": "text", "blocks": []}})),
            &theme,
        );
        assert_eq!(bare.header.unwrap().label.runs[0].text, "»");
    }

    #[test]
    fn unknown_components_leave_a_marker() {
        let layout = layout_item(
            &item(json!({"ref": "u", "kind": "agent", "component": {"type": "hologram"}})),
            &ChatTheme::default(),
        );
        let Node::Text(text) = layout.body else {
            panic!()
        };
        assert_eq!(text.runs[0].text, "[unsupported component: hologram]");
    }
}
