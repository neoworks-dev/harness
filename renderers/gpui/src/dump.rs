//! A readable text dump of the layout model: what the snapshot tests compare
//! (`fixtures/<name>/gpui.txt`). Colors print as theme token names, lengths in pixels.

use std::fmt::Write;

use neo_protocol::DiffLineKind;

use crate::layout::{
    BoxNode, Dir, Frame, Header, ImageSource, Insets, ItemLayout, Node, Run, TextNode,
};
use crate::theme::{ChatTheme, Paint, TextStyle};

/// The whole chat as text: a theme line, then one block per item.
pub fn dump_items(items: &[ItemLayout], theme: &ChatTheme) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "theme {} spacing s={} m={} l={}",
        theme.name,
        num(theme.s),
        num(theme.m),
        num(theme.l)
    );
    for item in items {
        out.push('\n');
        dump_item(&mut out, item);
    }
    out
}

fn num(x: f32) -> String {
    let rounded = (x * 10.0).round() / 10.0;
    if rounded.fract() == 0.0 {
        format!("{rounded:.0}")
    } else {
        format!("{rounded:.1}")
    }
}

fn insets(i: &Insets) -> String {
    if i.top == i.bottom && i.left == i.right {
        if i.top == i.left {
            num(i.top)
        } else {
            format!("{}/{}", num(i.top), num(i.left))
        }
    } else {
        format!(
            "{}/{}/{}/{}",
            num(i.top),
            num(i.right),
            num(i.bottom),
            num(i.left)
        )
    }
}

fn dump_item(out: &mut String, item: &ItemLayout) {
    let _ = write!(
        out,
        "item {} kind={}",
        item.r#ref,
        format!("{:?}", item.kind).to_lowercase()
    );
    if let Some(state) = item.state {
        let _ = write!(out, " state={}", format!("{state:?}").to_lowercase());
    }
    if item.collapsible {
        out.push_str(" collapsed");
    }
    out.push('\n');
    let frame = &item.frame;
    if *frame != Frame::default() {
        out.push_str("  frame");
        if frame.align_end {
            out.push_str(" align=end");
        }
        if let Some(max) = frame.max_width {
            let _ = write!(out, " max-width={}%", num(max * 100.0));
        }
        if let Some(bg) = frame.bg {
            let _ = write!(out, " bg={bg}");
        }
        if let Some(border) = frame.border {
            let _ = write!(out, " border={border}");
        }
        if frame.radius > 0.0 {
            let _ = write!(out, " radius={}", num(frame.radius));
        }
        if frame.pad != Insets::default() {
            let _ = write!(out, " pad={}", insets(&frame.pad));
        }
        if frame.body_indent > 0.0 {
            let _ = write!(out, " body-indent={}", num(frame.body_indent));
        }
        out.push('\n');
    }
    if let Some(header) = &item.header {
        dump_header(out, header);
    }
    for (i, button) in item.buttons.iter().enumerate() {
        let _ = writeln!(
            out,
            "  button {} [{}] {:?} -> {:?}{}",
            i,
            button.hint,
            button.label,
            button.action,
            button
                .args
                .as_ref()
                .map(|args| format!(" args={args}"))
                .unwrap_or_default()
        );
    }
    out.push_str("  body\n");
    dump_node(out, &item.body, 2);
}

fn dump_header(out: &mut String, header: &Header) {
    out.push_str("  header\n");
    if let Some(icon) = &header.icon {
        dump_text(out, icon, 2, "icon ");
    }
    dump_text(out, &header.label, 2, "");
}

fn pad(out: &mut String, depth: usize) {
    for _ in 0..depth {
        out.push_str("  ");
    }
}

fn style(s: &TextStyle) -> String {
    format!(
        "{}/{}{}",
        num(s.size),
        s.weight,
        if s.mono { " mono" } else { "" }
    )
}

fn dump_text(out: &mut String, text: &TextNode, depth: usize, prefix: &str) {
    pad(out, depth);
    let _ = write!(out, "{prefix}text {} fg={}", style(&text.style), text.color);
    if text.nowrap {
        out.push_str(" nowrap");
    }
    if let Some(width) = text.min_width {
        let _ = write!(out, " min-width={}", num(width));
    }
    out.push('\n');
    for run in &text.runs {
        pad(out, depth + 1);
        let _ = writeln!(out, "{:?}{}", run.text, run_flags(run, text.color));
    }
}

fn run_flags(run: &Run, base: Paint) -> String {
    let mut out = String::new();
    for (on, name) in [
        (run.bold, "bold"),
        (run.italic, "italic"),
        (run.strike, "strike"),
        (run.underline, "underline"),
        (run.mono, "mono"),
    ] {
        if on {
            out.push(' ');
            out.push_str(name);
        }
    }
    if run.color != base {
        let _ = write!(out, " fg={}", run.color);
    }
    if let Some(bg) = run.bg {
        let _ = write!(out, " bg={bg}");
    }
    if let Some(href) = &run.href {
        let _ = write!(out, " href={href}");
    }
    out
}

fn lines(out: &mut String, depth: usize, text: &str) {
    for line in text.split('\n') {
        pad(out, depth);
        let _ = writeln!(out, "| {line}");
    }
}

fn dump_node(out: &mut String, node: &Node, depth: usize) {
    match node {
        Node::Box(b) => dump_box(out, b, depth),
        Node::Text(t) => dump_text(out, t, depth, ""),
        Node::Code(code) => {
            pad(out, depth);
            let _ = write!(
                out,
                "code {} fg={} bg={} pad={} radius={} scroll-x",
                style(&code.style),
                code.color,
                code.bg,
                num(code.pad),
                num(code.radius)
            );
            if let Some(language) = &code.language {
                let _ = write!(out, " lang={language}");
            }
            out.push('\n');
            lines(out, depth + 1, &code.text);
        }
        Node::Rule(paint) => {
            pad(out, depth);
            let _ = writeln!(out, "rule {paint}");
        }
        Node::Progress(p) => {
            pad(out, depth);
            let _ = writeln!(
                out,
                "progress {} track={} fill={} height={} gap={}",
                num(p.fraction),
                p.track,
                p.fill,
                num(p.height),
                num(p.gap)
            );
            dump_text(out, &p.percent, depth + 1, "percent ");
            if let Some(label) = &p.label {
                dump_text(out, label, depth + 1, "label ");
            }
        }
        Node::Button(b) => {
            pad(out, depth);
            let _ = writeln!(
                out,
                "button #{} bg={} border={} hover={} copied={} radius={} pad={} gap={}",
                b.index,
                b.bg,
                b.border,
                b.accent,
                b.success,
                num(b.radius),
                insets(&b.pad),
                num(b.gap)
            );
            dump_text(out, &b.label, depth + 1, "label ");
            dump_text(out, &b.hint, depth + 1, "hint ");
        }
        Node::Image(image) => {
            pad(out, depth);
            let _ = write!(out, "image {:?}", image.src);
            match &image.source {
                ImageSource::Data(data) => {
                    let _ = write!(out, " source=data {data:?}");
                }
                ImageSource::Remote(_) => out.push_str(" source=remote"),
                ImageSource::None => out.push_str(" source=fallback"),
            }
            if let (Some(w), Some(h)) = (image.width, image.height) {
                let _ = write!(out, " size={w}x{h}");
            }
            if let Some(alt) = &image.alt {
                let _ = write!(out, " alt={alt:?}");
            }
            let _ = writeln!(
                out,
                " border={} dashed radius={} pad={}",
                image.fallback_border,
                num(image.radius),
                insets(&image.fallback_pad)
            );
            dump_text(out, &image.fallback, depth + 1, "fallback ");
            if let Some(caption) = &image.caption {
                dump_text(out, caption, depth + 1, "caption ");
            }
        }
        Node::File(file) => {
            pad(out, depth);
            let _ = writeln!(
                out,
                "file {:?} bg={} border={} radius={} pad={}",
                file.path,
                file.bg,
                file.border,
                num(file.radius),
                insets(&file.pad)
            );
            dump_text(out, &file.label, depth + 1, "");
        }
        Node::Diff(diff) => {
            pad(out, depth);
            let _ = writeln!(
                out,
                "diff {} border={} radius={} num-width={} scroll-x",
                style(&diff.style),
                diff.border,
                num(diff.radius),
                num(diff.num_width)
            );
            if let Some(path) = &diff.path {
                dump_text(out, path, depth + 1, "path ");
            }
            for row in &diff.rows {
                pad(out, depth + 1);
                let kind = match row.kind {
                    DiffLineKind::Context => "context",
                    DiffLineKind::Add => "add",
                    DiffLineKind::Remove => "remove",
                    DiffLineKind::Hunk => "hunk",
                };
                let bg = if row.kind == DiffLineKind::Hunk {
                    format!(" bg={}", diff.hunk_bg)
                } else {
                    String::new()
                };
                if row.kind == DiffLineKind::Hunk {
                    let _ = writeln!(out, "{kind} {:?} fg={}{bg}", row.text, row.color);
                } else {
                    let _ = writeln!(
                        out,
                        "{kind} old={:?} new={:?} sign={:?} {:?} fg={} num={}",
                        row.old, row.new, row.sign, row.text, row.color, row.num_color
                    );
                }
            }
        }
        Node::Table(table) => {
            pad(out, depth);
            let _ = writeln!(
                out,
                "table border={} cell-pad={} scroll-x",
                table.border,
                insets(&table.pad)
            );
            pad(out, depth + 1);
            let _ = writeln!(out, "header {}", cells(&table.header));
            for row in &table.rows {
                pad(out, depth + 1);
                let _ = writeln!(out, "row {}", cells(row));
            }
            if let Some(cell) = table.header.first() {
                pad(out, depth + 1);
                let _ = writeln!(out, "header style {} fg={}", style(&cell.style), cell.color);
            }
            if let Some(cell) = table.rows.first().and_then(|row| row.first()) {
                pad(out, depth + 1);
                let _ = writeln!(out, "cell style {} fg={}", style(&cell.style), cell.color);
            }
        }
        Node::Kv(kv) => {
            pad(out, depth);
            let _ = writeln!(out, "kv gap={}", num(kv.gap));
            for (key, value) in &kv.rows {
                pad(out, depth + 1);
                let _ = writeln!(
                    out,
                    "{:?} fg={} = {:?} fg={}",
                    key.runs
                        .first()
                        .map(|r| r.text.as_str())
                        .unwrap_or_default(),
                    key.color,
                    value
                        .runs
                        .first()
                        .map(|r| r.text.as_str())
                        .unwrap_or_default(),
                    value.color
                );
            }
            if let Some((key, _)) = kv.rows.first() {
                pad(out, depth + 1);
                let _ = writeln!(out, "style {}", style(&key.style));
            }
        }
    }
}

fn cells(cells: &[TextNode]) -> String {
    cells
        .iter()
        .map(|cell| {
            format!(
                "{:?}",
                cell.runs
                    .first()
                    .map(|r| r.text.as_str())
                    .unwrap_or_default()
            )
        })
        .collect::<Vec<_>>()
        .join(" | ")
}

fn dump_box(out: &mut String, b: &BoxNode, depth: usize) {
    pad(out, depth);
    let dir = match b.dir {
        Dir::Column => "column",
        Dir::Row => "row",
        Dir::Wrap => "row-wrap",
    };
    let _ = write!(out, "{} {dir} gap={}", b.role, num(b.gap));
    if b.pad != Insets::default() {
        let _ = write!(out, " pad={}", insets(&b.pad));
    }
    if let Some(bg) = b.bg {
        let _ = write!(out, " bg={bg}");
    }
    if let Some(border) = b.border {
        let _ = write!(out, " border={border}");
    }
    if let Some((paint, width)) = b.bar {
        let _ = write!(out, " bar={paint}:{}", num(width));
    }
    if b.radius > 0.0 {
        let _ = write!(out, " radius={}", num(b.radius));
    }
    out.push('\n');
    for child in &b.children {
        dump_node(out, child, depth + 1);
    }
}
