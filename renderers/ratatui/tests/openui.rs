//! OpenUI docs (`docs/neoui.md`): layout, charts, forms, focus and the visual state, one case
//! at a time (the snapshots cover the fixtures).

mod common;

use std::collections::HashMap;
use std::sync::Arc;

use common::*;
use neo_protocol::ViewItem;
use neo_renderer_ratatui::{
    ChartKind, ChartSpec, ImageOptions, ImageSlot, ImageStatus, Options, Palette, UiEdit, UiKind,
    UiState, draw_chart, image_sources, openui_docs, openui_targets, plain_text, render_item,
    render_item_images,
};
use ratatui::style::{Color, Modifier};
use serde_json::{Value, json};

fn pal() -> Palette {
    Palette::default()
}

fn node(component: &str, props: Value) -> Value {
    json!({"component": component, "props": props})
}

fn keyed(component: &str, key: &str, props: Value) -> Value {
    json!({"component": component, "key": key, "props": props})
}

fn doc(root: Value) -> ViewItem {
    doc_with(root, false)
}

fn doc_with(root: Value, streaming: bool) -> ViewItem {
    item_of(
        "agent",
        "",
        &json!({"type":"openui","ui":{"id":"0","root":root,"streaming":streaming}}).to_string(),
    )
}

fn text(s: &str) -> Value {
    node(
        "Text",
        json!({"text": s, "blocks": [{"type":"paragraph","spans":[{"text": s}]}]}),
    )
}

/// The text of `it`, drawn on a theme whose surface is the terminal's own: cards are outlines
/// there, so the text shows where each one is. [`filled`] draws them as they usually are.
fn lines(it: &ViewItem, width: u16) -> Vec<String> {
    lines_of(it, width, &outlined())
}

fn outlined() -> Options {
    Options {
        theme: Palette {
            surface: Color::Reset,
            ..Palette::default()
        },
        ..Options::default()
    }
}

fn filled(it: &ViewItem, width: u16) -> Vec<String> {
    lines_of(it, width, &Options::default())
}

fn state(f: impl FnOnce(&mut UiState)) -> Options {
    let mut s = UiState::default();
    f(&mut s);
    Options {
        openui: HashMap::from([("0".to_string(), s)]),
        ..outlined()
    }
}

fn focused(id: &str) -> Options {
    state(|s| s.focus = Some(id.to_string()))
}

fn fixture(name: &str) -> ViewItem {
    fixture_items(name).remove(0)
}

fn assert_fits(it: &ViewItem, opts: &Options) {
    for width in 1..=120u16 {
        for (n, line) in render_item(it, width, opts).iter().enumerate() {
            assert!(
                line.width() <= usize::from(width),
                "width {width} line {n} is {} wide: {line:?}",
                line.width()
            );
        }
    }
}

// ---- layout ----

fn metric(label: &str, value: &str) -> Value {
    node("Metric", json!({"label": label, "value": value}))
}

fn tag(label: &str) -> Value {
    node("Tag", json!({"label": label}))
}

fn card(title: &str, children: Vec<Value>) -> Value {
    node("Card", json!({"title": title, "children": children}))
}

fn column(children: Vec<Value>) -> Value {
    node("Stack", json!({"children": children}))
}

fn row(children: Vec<Value>, extra: Value) -> Value {
    let mut props = json!({"direction": "row", "children": children});
    if let (Some(p), Some(e)) = (props.as_object_mut(), extra.as_object()) {
        p.extend(e.clone());
    }
    node("Stack", props)
}

#[test]
fn a_row_puts_children_side_by_side_at_their_own_widths() {
    let it = doc(row(
        vec![tag("Revenue"), tag("Customers"), tag("Churn")],
        json!({}),
    ));
    assert_eq!(lines(&it, 80), ["▐Revenue▌   ▐Customers▌   ▐Churn▌"]);
}

#[test]
fn a_row_squeezes_text_before_it_stacks() {
    let it = doc(row(
        vec![text("one two three four five six seven"), tag("A")],
        json!({}),
    ));
    let out = lines(&it, 30);
    assert!(out.len() >= 2, "{out:?}");
    // The text gave way, the metric kept its width next to it.
    assert!(out[0].ends_with("▐A▌"), "{out:?}");
    assert!(out.iter().all(|l| l.chars().count() <= 30), "{out:?}");
}

#[test]
fn a_row_that_cannot_be_squeezed_stacks() {
    let it = doc(row(
        vec![tag("Revenue"), tag("Customers"), tag("Churn")],
        json!({}),
    ));
    assert_eq!(lines(&it, 12), ["▐Revenue▌", "▐Customers▌", "▐Churn▌"]);
}

#[test]
fn a_wrapping_row_breaks_into_lines_of_whole_children() {
    let it = doc(row(
        vec![
            node("Tag", json!({"label": "alpha"})),
            node("Tag", json!({"label": "beta"})),
            node("Tag", json!({"label": "gamma"})),
            node("Tag", json!({"label": "delta"})),
        ],
        json!({"wrap": true, "gap": "s"}),
    ));
    assert_eq!(lines(&it, 80), ["▐alpha▌  ▐beta▌  ▐gamma▌  ▐delta▌"]);
    assert_eq!(lines(&it, 20), ["▐alpha▌  ▐beta▌", "▐gamma▌  ▐delta▌"]);
    assert_eq!(lines(&it, 10), ["▐alpha▌", "▐beta▌", "▐gamma▌", "▐delta▌"]);
}

#[test]
fn a_row_justifies_between_and_aligns_children_to_the_bottom() {
    let it = doc(row(vec![tag("A"), tag("B")], json!({"justify": "between"})));
    assert_eq!(lines(&it, 10), ["▐A▌    ▐B▌"]);
    let it = doc(row(
        vec![
            node(
                "Callout",
                json!({"text": "x", "blocks": [{"type":"paragraph","spans":[{"text":"x"}]}], "title": "T"}),
            ),
            text("short"),
        ],
        json!({"align": "end"}),
    ));
    let out = lines(&it, 40);
    assert_eq!(out.len(), 2, "{out:?}");
    assert!(out[1].contains("short"), "{out:?}");
    assert!(!out[0].contains("short"), "{out:?}");
}

#[test]
fn stacks_align_their_children_and_gap_l_is_a_blank_line() {
    let it = doc(node(
        "Stack",
        json!({"children": [text("a"), text("b")], "gap": "l"}),
    ));
    assert_eq!(lines(&it, 20), ["a", "", "b"]);
    let it = doc(node(
        "Stack",
        json!({"children": [text("a"), text("bbbb")], "align": "end"}),
    ));
    assert_eq!(lines(&it, 8), ["       a", "    bbbb"]);
}

#[test]
fn card_variants() {
    let kids = json!([text("body")]);
    let it = doc(node("Card", json!({"children": kids, "title": "Hello"})));
    assert_eq!(
        lines(&it, 20),
        [
            "╭─ Hello ──────────╮",
            "│ body             │",
            "╰──────────────────╯"
        ]
    );
    let it = doc(node(
        "Card",
        json!({"children": kids, "title": "Hello", "variant": "clear"}),
    ));
    assert_eq!(lines(&it, 30), ["Hello", "body"]);
    let it = doc(node(
        "Card",
        json!({"children": kids, "title": "Hello", "variant": "sunk"}),
    ));
    assert_eq!(lines(&it, 12), [" Hello", " body"]);
    // Sunk is painted on the surface color across the whole width.
    let rendered = render_item(&it, 12, &Options::default());
    assert!(rendered.iter().all(|l| l.width() == 12));
    assert!(
        rendered
            .iter()
            .flat_map(|l| &l.spans)
            .all(|s| s.style.bg == Some(pal().surface))
    );
    // A tone colors the bar down the card's side and the title; outlined, the frame.
    let it = doc(node(
        "Card",
        json!({"children": kids, "title": "Hello", "tone": "danger"}),
    ));
    let rendered = render_item(&it, 30, &Options::default());
    assert_eq!(rendered[1].spans[0].content, "▎ ");
    assert_eq!(rendered[1].spans[0].style.fg, Some(pal().danger));
    assert_eq!(rendered[1].spans[1].style.fg, Some(pal().danger));
    assert_eq!(rendered[2].spans[0].style.fg, Some(pal().danger));
    let rendered = render_item(&it, 30, &outlined());
    assert_eq!(rendered[0].spans[0].style.fg, Some(pal().danger));
    assert_eq!(rendered[0].spans[1].style.fg, Some(pal().danger));
    assert_eq!(rendered[1].spans[0].style.fg, Some(pal().danger));
    assert_eq!(rendered[2].spans[0].style.fg, Some(pal().danger));
}

#[test]
fn a_card_is_a_fill_with_half_row_edges() {
    let it = doc(card("Hello", vec![text("body")]));
    assert_eq!(
        filled(&it, 20),
        [
            "▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄",
            "  Hello",
            "  body",
            "▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀"
        ]
    );
    let rendered = render_item(&it, 20, &Options::default());
    // The edges are the fill's color on the terminal's; the rows between are on the fill.
    for edge in [&rendered[0], &rendered[3]] {
        assert!(
            edge.spans
                .iter()
                .all(|s| s.style.fg == Some(pal().surface) && s.style.bg.is_none())
        );
    }
    for line in &rendered[1..3] {
        assert_eq!(line.width(), 20);
        assert!(
            line.spans.iter().all(|s| s.style.bg == Some(pal().surface)),
            "{line:?}"
        );
    }
    assert!(
        rendered[1]
            .spans
            .iter()
            .any(|s| s.content == "Hello" && s.style.add_modifier.contains(Modifier::BOLD))
    );
    // A card in a card is a shade lighter.
    let it = doc(card("Outer", vec![card("Inner", vec![text("deep")])]));
    let rendered = render_item(&it, 24, &Options::default());
    let deep = rendered
        .iter()
        .flat_map(|l| &l.spans)
        .find(|s| s.content.contains("deep"))
        .unwrap();
    assert_ne!(deep.style.bg, Some(pal().surface));
    assert!(matches!(deep.style.bg, Some(Color::Rgb(..))));
}

#[test]
fn without_a_colored_surface_a_card_is_an_outline() {
    let it = doc(card("Hello", vec![text("body")]));
    let rendered = render_item(&it, 20, &outlined());
    let surface = outlined().theme.surface;
    assert_eq!(rendered.len(), 3);
    // Every cell of the box, the outline included, is on the surface; the outline in the
    // border color, the title bold.
    for line in &rendered {
        assert_eq!(line.width(), 20);
        assert!(
            line.spans.iter().all(|s| s.style.bg == Some(surface)),
            "{line:?}"
        );
    }
    assert_eq!(rendered[0].spans[0].style.fg, Some(pal().border));
    assert!(
        rendered[0]
            .spans
            .iter()
            .any(|s| s.content == "Hello" && s.style.add_modifier.contains(Modifier::BOLD))
    );
    // A card in a card is on the same surface; its outline tells it apart.
    let it = doc(card("Outer", vec![card("Inner", vec![text("deep")])]));
    assert_eq!(
        lines(&it, 24),
        [
            "╭─ Outer ──────────────╮",
            "│ ╭─ Inner ──────────╮ │",
            "│ │ deep             │ │",
            "│ ╰──────────────────╯ │",
            "╰──────────────────────╯"
        ]
    );
}

#[test]
fn header_separator_and_steps() {
    let it = doc(node(
        "Stack",
        json!({"children": [
            node("Header", json!({"title": "Deploy", "subtitle": "to prod"})),
            node("Separator", json!({})),
            node("Steps", json!({"steps": [
                node("Step", json!({"title": "Build", "detail": "2m", "status": "done"})),
                node("Step", json!({"title": "Canary", "status": "current"})),
                node("Step", json!({"title": "Rollout", "status": "pending"})),
                node("Step", json!({"title": "Verify", "status": "failed"})),
            ]})),
        ]}),
    ));
    assert_eq!(
        lines(&it, 20),
        [
            "Deploy  to prod",
            "────────────────────",
            "✓ Build  2m",
            "● Canary",
            "○ Rollout",
            "✗ Verify"
        ]
    );
    // Too narrow for the subtitle beside the title: under it.
    assert_eq!(lines(&it, 10)[..2], ["Deploy", "to prod"]);
    let rendered = render_item(&it, 20, &Options::default());
    let glyph = |row: usize| rendered[row].spans[0].style.fg;
    assert_eq!(glyph(2), Some(pal().success));
    assert_eq!(glyph(3), Some(pal().accent));
    assert_eq!(glyph(4), Some(pal().muted));
    assert_eq!(glyph(5), Some(pal().danger));
}

fn tabs() -> Value {
    node(
        "Tabs",
        json!({"tabs": [
            node("Tab", json!({"label": "One", "children": [text("first")]})),
            node("Tab", json!({"label": "Two", "children": [keyed("Input", "name", json!({"name": "name"}))]})),
            node("Tab", json!({"label": "Three", "children": [text("third")]})),
        ]}),
    )
}

#[test]
fn tabs_draw_the_selected_tab_reversed_over_its_children() {
    let it = doc(tabs());
    assert_eq!(lines(&it, 40), ["│ One │ Two │ Three │", "first"]);
    let rendered = render_item(&it, 40, &Options::default());
    let selected = &rendered[0].spans[1];
    assert_eq!(selected.content, " One ");
    assert!(selected.style.add_modifier.contains(Modifier::REVERSED));
    assert!(
        !rendered[0].spans[3]
            .style
            .add_modifier
            .contains(Modifier::REVERSED)
    );
}

#[test]
fn switching_tabs_changes_the_content_and_the_targets() {
    let it = doc(tabs());
    let opts = state(|s| {
        s.tabs.insert("/".into(), 2);
    });
    assert_eq!(lines_of(&it, 40, &opts), ["│ One │ Two │ Three │", "third"]);
    let on_two = state(|s| {
        s.tabs.insert("/".into(), 1);
    });
    let ids =
        |o: &Options| -> Vec<String> { openui_targets(&it, o).into_iter().map(|t| t.id).collect() };
    assert_eq!(ids(&Options::default()), ["/"]);
    assert_eq!(ids(&on_two), ["/", "name"]);
    // An index past the end shows the last tab.
    let past = state(|s| {
        s.tabs.insert("/".into(), 9);
    });
    assert_eq!(lines_of(&it, 40, &past)[1], "third");
}

#[test]
fn a_tab_strip_wider_than_the_view_scrolls_to_the_selected_tab() {
    let it = doc(tabs());
    let opts = state(|s| {
        s.tabs.insert("/".into(), 2);
    });
    let out = lines_of(&it, 12, &opts);
    assert!(out[0].contains("Three"), "{out:?}");
    assert!(out[0].starts_with('‹'), "{out:?}");
    assert_fits(&it, &opts);
}

#[test]
fn accordion_items_toggle_and_are_focus_targets_by_path() {
    let it = doc(node(
        "Accordion",
        json!({"items": [
            node("AccordionItem", json!({"title": "Closed", "children": [text("hidden")]})),
            node("AccordionItem", json!({"title": "Open", "open": true, "children": [text("shown")]})),
        ]}),
    ));
    assert_eq!(lines(&it, 30), ["▸ Closed", "▾ Open", "  shown"]);
    let targets = openui_targets(&it, &Options::default());
    let ids: Vec<&str> = targets.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(ids, ["/items.0", "/items.1"]);
    assert!(targets.iter().all(|t| t.kind == UiKind::Accordion));
    assert_eq!(targets[0].value, json!(false));
    assert_eq!(targets[1].value, json!(true));
    let flipped = state(|s| {
        s.open.insert("/items.0".into(), true);
        s.open.insert("/items.1".into(), false);
    });
    assert_eq!(
        lines_of(&it, 30, &flipped),
        ["▾ Closed", "  hidden", "▸ Open"]
    );
    // The focused item is reversed in the accent color.
    let rendered = render_item(&it, 30, &focused("/items.1"));
    assert!(
        rendered[1].spans[0]
            .style
            .add_modifier
            .contains(Modifier::REVERSED)
    );
    assert_eq!(rendered[1].spans[0].style.fg, Some(pal().accent));
}

// ---- content ----

#[test]
fn text_and_callout_draw_the_daemons_blocks() {
    let it = doc(node(
        "Stack",
        json!({"children": [
            node("Text", json!({"text": "raw **x**", "blocks": [{"type":"paragraph","spans":[{"text":"p99 rose "},{"text":"18%","bold":true}]}], "tone": "danger"})),
            node("Callout", json!({"text": "t", "title": "Latency", "tone": "success", "blocks": [{"type":"paragraph","spans":[{"text":"all good"}]}]})),
        ]}),
    ));
    assert_eq!(lines(&it, 40), ["p99 rose 18%", "┃ Latency", "┃ all good"]);
    let rendered = render_item(&it, 40, &Options::default());
    assert_eq!(rendered[0].spans[0].style.fg, Some(pal().danger));
    assert!(
        rendered[0].spans[1]
            .style
            .add_modifier
            .contains(Modifier::BOLD)
    );
    assert_eq!(rendered[1].spans[0].style.fg, Some(pal().success));
    // The gutter and title carry the tone, the text does not.
    assert_eq!(rendered[2].spans[1].style.fg, None);
}

#[test]
fn tables_take_node_cells_and_align_numbers_right() {
    let it = doc(node(
        "Table",
        json!({"columns": ["Account", "Plan", "MRR"], "rows": [
            ["Acme", node("Tag", json!({"label": "Pro", "tone": "accent"})), 1200],
            ["Globex", node("Link", json!({"text": "docs", "url": "https://x.dev"})), 480],
        ]}),
    ));
    assert_eq!(
        lines(&it, 40),
        [
            "Account │ Plan  │  MRR",
            "────────┼───────┼─────",
            "Acme    │ ▐Pro▌ │ 1200",
            "Globex  │ docs  │  480",
        ]
    );
    let rendered = render_item(&it, 40, &Options::default());
    let tag = rendered[2]
        .spans
        .iter()
        .find(|s| s.content == "Pro")
        .unwrap();
    assert_eq!(tag.style.fg, Some(pal().accent));
    assert!(tag.style.bg.is_some(), "a tag sits on a tint");
    // Its ends are half blocks in the tint: a pill.
    let cap = rendered[2].spans.iter().find(|s| s.content == "▐").unwrap();
    assert_eq!(cap.style.fg, tag.style.bg);
    assert_fits(&it, &Options::default());
}

#[test]
fn lists_key_values_metrics_and_progress() {
    let it = doc(node(
        "Stack",
        json!({"children": [
            node("List", json!({"items": ["web-1", node("Tag", json!({"label": "x"}))], "ordered": true})),
            node("KeyValue", json!({"pairs": [
                node("KV", json!({"key": "Region", "value": "eu"})),
                node("KV", json!({"key": "Errors", "value": 3, "tone": "danger"})),
            ]})),
            node("Metric", json!({"label": "Churn", "value": "2.1%", "delta": "+0.4pt", "tone": "danger"})),
            node("Progress", json!({"value": 0.5, "label": "half"})),
        ]}),
    ));
    assert_eq!(
        lines(&it, 40),
        [
            "1. web-1",
            "2. ▐x▌",
            "Region  eu",
            "Errors  3",
            "2.1% +0.4pt",
            "Churn",
            &format!("{}{} 50% half", "▰".repeat(16), "▱".repeat(15)),
        ]
    );
    let rendered = render_item(&it, 40, &Options::default());
    let delta = rendered[4]
        .spans
        .iter()
        .find(|s| s.content.contains("+0.4pt"))
        .unwrap();
    assert_eq!(delta.style.fg, Some(pal().danger));
}

#[test]
fn code_images_and_links_reuse_the_existing_drawing() {
    let it = doc(node(
        "Stack",
        json!({"children": [
            node("Code", json!({"code": "let x = 1;", "language": "rust"})),
            node("Image", json!({"src": "/tmp/a.png", "alt": "chart"})),
            node("Link", json!({"text": "Docs", "url": "https://example.com"})),
        ]}),
    ));
    assert_eq!(
        lines(&it, 40),
        [
            "  let x = 1;                            ",
            "[image chart]",
            "Docs (https://example.com)"
        ]
        .map(|l| l.trim_end().to_string())
    );
}

// ---- charts ----

fn series(name: &str, values: &[f64]) -> Value {
    node("Series", json!({"name": name, "values": values}))
}

/// Charts as pictures, in 10x20 pixel cells.
fn chart_pictures() -> Options {
    Options {
        images: Some(ImageOptions {
            max_rows: 20,
            cell: (10, 20),
            status: Arc::new(HashMap::new()),
            charts: true,
        }),
        ..Options::default()
    }
}

#[test]
fn a_line_chart_as_a_picture_keeps_its_axis_labels_and_legend_as_text() {
    let it = doc(node(
        "LineChart",
        json!({"labels": ["Jan", "Feb", "Mar"], "series": [series("a", &[1.0, 4.0, 2.0]), series("b", &[3.0, 2.0, 1.0])]}),
    ));
    let out = render_item_images(&it, 40, &chart_pictures());
    let text = plain_text(&out.lines);
    // The axis beside 8 rows of picture, the labels and the legend under it.
    assert_eq!(text.len(), 10, "{text:?}");
    assert_eq!(text[0].trim_end(), "4");
    assert_eq!(text[7].trim_end(), "1");
    assert!(
        text[8].contains("Jan") && text[8].contains("Mar"),
        "{text:?}"
    );
    assert!(
        text[9].contains("━ a") && text[9].contains("━ b"),
        "{text:?}"
    );
    let [slot] = &out.images[..] else {
        panic!("{:?}", out.images)
    };
    assert_eq!((slot.line, slot.x, slot.rows), (0, 2, 8));
    let spec = ChartSpec::from_src(&slot.src).unwrap();
    assert_eq!(spec.kind, ChartKind::Line);
    assert_eq!((spec.cols, spec.lo, spec.hi), (slot.cols, 1.0, 4.0));
    assert_eq!(spec.series.len(), 2);
    // The picture draws.
    let px = draw_chart(&spec, u32::from(spec.cols) * 10, 8 * 20).unwrap();
    assert!(px.chunks(4).any(|p| p[3] > 0));
}

#[test]
fn columns_and_a_pie_as_pictures_line_up_with_their_labels() {
    let bars = doc(node(
        "BarChart",
        json!({"labels": ["a", "b"], "series": [series("n", &[2.0, 4.0])]}),
    ));
    let out = render_item_images(&bars, 40, &chart_pictures());
    let text = plain_text(&out.lines);
    let spec = ChartSpec::from_src(&out.images[0].src).unwrap();
    assert_eq!((spec.kind, spec.hi, spec.bar), (ChartKind::Columns, 4.0, 3));
    // The labels sit under the middle of each group of `group` cells, past the axis.
    let x = usize::from(out.images[0].x);
    assert_eq!(x, 2);
    let label = &text[8];
    let group = usize::from(spec.group);
    assert_eq!(
        label.find('a'),
        Some(x + group / 2 - usize::from(group % 2 == 0))
    );
    assert_eq!(usize::from(spec.cols), 2 * group + 1);

    let pie = doc(node(
        "PieChart",
        json!({"slices": [
            node("Slice", json!({"label": "Rent", "value": 3})),
            node("Slice", json!({"label": "Food", "value": 1})),
        ], "donut": true}),
    ));
    let out = render_item_images(&pie, 60, &chart_pictures());
    let text = plain_text(&out.lines);
    let slot = &out.images[0];
    let spec = ChartSpec::from_src(&slot.src).unwrap();
    assert_eq!(
        (spec.kind, spec.donut, slot.rows),
        (ChartKind::Pie, true, 5)
    );
    // Round in 10x20 cells: twice as many columns as rows.
    assert_eq!(slot.cols, 10);
    // The legend beside it, in the middle rows.
    assert!(
        text[1].contains("■ Rent") && text[1].contains("75%"),
        "{text:?}"
    );
    assert!(
        text[2].contains("■ Food") && text[2].contains("25%"),
        "{text:?}"
    );
}

#[test]
fn without_true_colors_or_while_streaming_charts_stay_characters() {
    let chart = node(
        "LineChart",
        json!({"labels": ["a", "b"], "series": [series("a", &[1.0, 2.0])]}),
    );
    let mut plain = chart_pictures();
    plain.theme.accent = Color::Indexed(2);
    assert!(
        render_item_images(&doc(chart.clone()), 40, &plain)
            .images
            .is_empty()
    );
    let streaming = doc_with(chart, true);
    assert!(
        render_item_images(&streaming, 40, &chart_pictures())
            .images
            .is_empty()
    );
}

#[test]
fn horizontal_bars_are_half_height_with_the_value_at_the_end() {
    let it = doc(node(
        "BarChart",
        json!({"labels": ["a", "bb"], "series": [series("n", &[8.0, 4.5])], "horizontal": true}),
    ));
    let out = lines(&it, 30);
    assert_eq!(out.len(), 2);
    assert!(out[0].starts_with("a  "), "{out:?}");
    assert!(out[0].ends_with(" 8"), "{out:?}");
    assert!(out[1].ends_with(" 4.5"), "{out:?}");
    let bar = |l: &str| l.chars().filter(|c| "▄▖".contains(*c)).collect::<String>();
    // Room for 23 cells after the label and before the value.
    assert_eq!(bar(&out[0]).chars().count(), 23);
    // 4.5 of 8 is 12.9 cells: 13, rounded to halves. The upper half of every line stays empty,
    // so neighbouring bars never merge.
    assert_eq!(bar(&out[1]), "▄".repeat(13));
    assert!(!out.iter().any(|l| l.contains('█')), "{out:?}");
}

#[test]
fn vertical_bars_group_series_and_say_what_they_are() {
    let it = doc(node(
        "BarChart",
        json!({"labels": ["Jan", "Feb"], "series": [series("Revenue", &[12.0, 24.0]), series("Costs", &[6.0, 3.0])]}),
    ));
    let out = lines(&it, 40);
    // A value axis, six rows of bars, the labels, the legend.
    assert_eq!(out.len(), 8, "{out:?}");
    assert!(out[0].starts_with("24"), "{out:?}");
    assert!(out[5].starts_with(" 0"), "{out:?}");
    assert!(out[6].contains("Jan") && out[6].contains("Feb"), "{out:?}");
    assert_eq!(out[7], "■ Revenue  ■ Costs");
    let rendered = render_item(&it, 40, &Options::default());
    let fg = |row: usize| -> Vec<Option<Color>> {
        rendered[row]
            .spans
            .iter()
            .filter(|s| s.content.contains('█'))
            .map(|s| s.style.fg)
            .collect()
    };
    // The two series have two colors.
    let colors = fg(5);
    assert!(
        colors.contains(&Some(pal().accent)) && colors.contains(&Some(pal().success)),
        "{colors:?}"
    );
}

#[test]
fn stacked_bars_add_up_and_labels_that_do_not_fit_lay_the_bars_down() {
    let it = doc(node(
        "BarChart",
        json!({"labels": ["Mon", "Tue"], "series": [series("a", &[2.0, 4.0]), series("b", &[2.0, 4.0])],
               "horizontal": true, "stacked": true}),
    ));
    let out = lines(&it, 30);
    assert_eq!(out.len(), 3, "{out:?}");
    assert!(out[0].ends_with(" 4"), "{out:?}");
    assert!(out[1].ends_with(" 8"), "{out:?}");
    // Two series in one bar: two colors on a row.
    let rendered = render_item(&it, 30, &Options::default());
    let colors: Vec<_> = rendered[1]
        .spans
        .iter()
        .filter_map(|s| s.style.fg)
        .collect();
    assert!(
        colors.contains(&pal().accent) && colors.contains(&pal().success),
        "{colors:?}"
    );
    // Many long labels in few columns: lying down although `horizontal` is not set.
    let labels: Vec<String> = (0..12).map(|i| format!("category {i}")).collect();
    let it = doc(node(
        "BarChart",
        json!({"labels": labels, "series": [series("n", &[1.0; 12])]}),
    ));
    let out = lines(&it, 40);
    assert_eq!(out.len(), 12, "{out:?}");
    assert!(out[0].starts_with("category 0"));
}

#[test]
fn line_and_area_charts_use_braille_with_an_axis_and_a_legend() {
    let it = doc(node(
        "LineChart",
        json!({"labels": ["a", "b", "c"], "series": [series("p50", &[1.0, 5.0, 3.0]), series("p99", &[2.0, 9.0, 4.0])]}),
    ));
    let out = lines(&it, 40);
    assert_eq!(out.len(), 8, "{out:?}");
    assert!(out[0].starts_with("9 "), "{out:?}");
    assert!(out[5].starts_with("1 "), "{out:?}");
    assert!(
        out[6].trim().starts_with('a') && out[6].trim().ends_with('c'),
        "{out:?}"
    );
    assert_eq!(out[7], "━ p50  ━ p99");
    assert!(
        out[..6]
            .iter()
            .any(|l| l.chars().any(|c| ('\u{2801}'..='\u{28ff}').contains(&c)))
    );
    let area = doc(node(
        "AreaChart",
        json!({"labels": ["a", "b"], "series": [series("x", &[1.0, 4.0])]}),
    ));
    // The area is filled under the line down to zero.
    let out = lines(&area, 40);
    assert!(out[5].chars().filter(|c| *c != ' ').count() > 10, "{out:?}");
    assert!(out[5].starts_with("0 "), "{out:?}");
}

#[test]
fn pie_charts_are_a_stacked_bar_and_a_legend_with_percentages() {
    let slice = |l: &str, v: f64| node("Slice", json!({"label": l, "value": v}));
    let it = doc(node(
        "PieChart",
        json!({"slices": [slice("Pro", 62.0), slice("Team", 28.0), slice("Free", 10.0)], "donut": true}),
    ));
    let out = lines(&it, 40);
    assert_eq!(out[1..], ["■ Pro    62%", "■ Team   28%", "■ Free   10%"]);
    assert_eq!(out[0].chars().count(), 40);
    let rendered = render_item(&it, 40, &Options::default());
    let cells: Vec<usize> = rendered[0]
        .spans
        .iter()
        .map(|s| s.content.chars().count())
        .collect();
    assert_eq!(cells.iter().sum::<usize>(), 40);
    assert_eq!(cells.len(), 3);
    // Nothing to show: nothing.
    let none = doc(node("PieChart", json!({"slices": []})));
    assert!(lines(&none, 40).is_empty());
}

#[test]
fn sparklines_show_the_shape_and_the_last_value() {
    let it = doc(node(
        "Sparkline",
        json!({"values": [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0], "label": "load"}),
    ));
    assert_eq!(lines(&it, 40), ["load ▁▂▃▄▅▆▇█ 8"]);
    // More values than room: resampled.
    let long: Vec<f64> = (0..100).map(f64::from).collect();
    let it = doc(node("Sparkline", json!({"values": long})));
    assert!(lines(&it, 20)[0].chars().count() <= 20);
}

#[test]
fn charts_never_overflow() {
    let it = doc(node(
        "Stack",
        json!({"children": [
            node("BarChart", json!({"labels": ["Jan","Feb","Mar"], "series": [series("a", &[1.0,2.0,3.0]), series("b", &[3.0,2.0,1.0])]})),
            node("BarChart", json!({"labels": ["Jan","Feb"], "series": [series("a", &[1.0,2.0])], "stacked": true})),
            node("LineChart", json!({"labels": ["a","b","c"], "series": [series("a", &[1.0,5.0,2.0])]})),
            node("AreaChart", json!({"labels": ["a","b","c"], "series": [series("a", &[1.0,5.0,2.0])]})),
            node("PieChart", json!({"slices": [node("Slice", json!({"label": "Alpha", "value": 1})), node("Slice", json!({"label": "Beta", "value": 3}))]})),
            node("Sparkline", json!({"values": [1,2,3], "label": "x"})),
        ]}),
    ));
    assert_fits(&it, &Options::default());
}

// ---- forms ----

fn field(label: &str, input: Value) -> Value {
    node("FormControl", json!({"label": label, "input": input}))
}

fn form(children: Vec<Value>, buttons: Option<Value>) -> Value {
    let mut props = json!({"name": "f", "children": children});
    if let Some(b) = buttons {
        props["buttons"] = b;
    }
    node("Form", props)
}

#[test]
fn an_input_has_a_label_a_required_marker_and_a_placeholder() {
    let it = doc(form(
        vec![field(
            "Email",
            keyed(
                "Input",
                "email",
                json!({"name": "email", "placeholder": "you@x.com", "required": true}),
            ),
        )],
        None,
    ));
    let out = lines(&it, 30);
    assert_eq!(out[0], "Email *");
    assert_eq!(out[1], "[ you@x.com                  ]");
    let rendered = render_item(&it, 30, &Options::default());
    assert_eq!(rendered[0].spans[1].style.fg, Some(pal().danger));
    // The placeholder is muted.
    assert_eq!(rendered[1].spans[1].style.fg, Some(pal().muted));
}

#[test]
fn the_error_is_drawn_under_the_field_in_the_danger_tone() {
    let it = doc(field(
        "Email",
        keyed(
            "Input",
            "email",
            json!({"name": "email", "value": "ops@", "error": "Enter a valid email"}),
        ),
    ));
    let out = lines(&it, 30);
    assert_eq!(
        out[1..],
        ["[ ops@                       ]", "Enter a valid email"]
    );
    let rendered = render_item(&it, 30, &Options::default());
    assert_eq!(rendered[2].spans[0].style.fg, Some(pal().danger));
    assert_eq!(rendered[1].spans[0].style.fg, Some(pal().danger));
}

#[test]
fn the_focused_field_shows_a_cursor_and_its_edit_buffer() {
    let it = doc(field(
        "Name",
        keyed(
            "Input",
            "name",
            json!({"name": "name", "value": "daemon value"}),
        ),
    ));
    // Not focused: the daemon's value.
    assert!(lines(&it, 30)[1].contains("daemon value"));
    // Focused with an edit buffer: the buffer, the cursor reversed on the character at it.
    let opts = state(|s| {
        s.focus = Some("name".into());
        s.edits.insert(
            "name".into(),
            UiEdit {
                text: "abc".into(),
                cursor: 1,
            },
        );
    });
    assert!(lines_of(&it, 30, &opts)[1].starts_with("[ abc "));
    let rendered = render_item(&it, 30, &opts);
    let reversed: Vec<&str> = rendered[1]
        .spans
        .iter()
        .filter(|s| s.style.add_modifier.contains(Modifier::REVERSED))
        .map(|s| s.content.as_ref())
        .collect();
    assert_eq!(reversed, ["b"]);
    // At the end the cursor is a reversed space; the label turns accent.
    let at_end = state(|s| {
        s.focus = Some("name".into());
        s.edits.insert(
            "name".into(),
            UiEdit {
                text: "abc".into(),
                cursor: 3,
            },
        );
    });
    let rendered = render_item(&it, 30, &at_end);
    assert!(
        rendered[1]
            .spans
            .iter()
            .any(|s| s.content == " " && s.style.add_modifier.contains(Modifier::REVERSED))
    );
    assert_eq!(rendered[0].spans[0].style.fg, Some(pal().accent));
    // The buffer is drawn without a cursor when the field has lost focus.
    let blurred = state(|s| {
        s.edits.insert(
            "name".into(),
            UiEdit {
                text: "abc".into(),
                cursor: 3,
            },
        );
    });
    let rendered = render_item(&it, 30, &blurred);
    assert!(
        rendered[1]
            .spans
            .iter()
            .all(|s| !s.style.add_modifier.contains(Modifier::REVERSED))
    );
}

#[test]
fn a_long_value_scrolls_to_keep_the_cursor_in_view() {
    let it = doc(keyed("Input", "n", json!({"name": "n"})));
    let opts = state(|s| {
        s.focus = Some("n".into());
        s.edits.insert(
            "n".into(),
            UiEdit {
                text: "0123456789abcdefghijklmnopqrstuvwxyz".into(),
                cursor: 36,
            },
        );
    });
    let out = lines_of(&it, 16, &opts);
    assert_eq!(out[0].chars().count(), 16);
    assert!(out[0].contains("xyz"), "{out:?}");
    assert!(!out[0].contains("0123"), "{out:?}");
    assert_fits(&it, &opts);
}

#[test]
fn passwords_show_dots_and_dates_are_checked() {
    let it = doc(keyed(
        "Input",
        "pw",
        json!({"name": "pw", "type": "password", "value": "hunter2"}),
    ));
    assert!(lines(&it, 30)[0].contains("•••••••"));
    assert!(!lines(&it, 30)[0].contains("hunter"));
    let good = doc(keyed(
        "DatePicker",
        "d",
        json!({"name": "d", "value": "2026-10-21"}),
    ));
    let bad = doc(keyed(
        "DatePicker",
        "d",
        json!({"name": "d", "value": "2026-13-40"}),
    ));
    let color = |it: &ViewItem| {
        render_item(it, 30, &Options::default())[0].spans[1]
            .style
            .fg
    };
    assert_eq!(color(&good), None);
    assert_eq!(color(&bad), Some(pal().danger));
    let empty = doc(keyed("DatePicker", "d", json!({"name": "d"})));
    assert!(lines(&empty, 30)[0].contains("YYYY-MM-DD"));
}

#[test]
fn a_text_area_has_rows_and_wraps_what_is_typed() {
    let it = doc(keyed(
        "TextArea",
        "t",
        json!({"name": "t", "placeholder": "Notes"}),
    ));
    assert_eq!(lines(&it, 20).len(), 3);
    let opts = state(|s| {
        s.focus = Some("t".into());
        s.edits.insert(
            "t".into(),
            UiEdit {
                text: "one\ntwo\nthree\nfour\nfive".into(),
                cursor: 23,
            },
        );
    });
    let out = lines_of(&it, 20, &opts);
    // Three rows, scrolled to the cursor's row.
    assert_eq!(out.len(), 5);
    assert!(out.last().unwrap().contains("five"), "{out:?}");
    assert_fits(&it, &opts);
}

#[test]
fn select_shows_the_choice_and_an_open_list() {
    let select = keyed(
        "Select",
        "svc",
        json!({"name": "svc", "placeholder": "Pick one", "value": "web", "options": [
            node("Option", json!({"value": "api", "label": "API"})),
            node("Option", json!({"value": "web", "label": "Web app"})),
        ]}),
    );
    let it = doc(select.clone());
    assert_eq!(lines(&it, 30), ["[ Web app                  ▾ ]"]);
    let opts = state(|s| {
        s.focus = Some("svc".into());
        s.select = Some(("svc".into(), 0));
    });
    let out = lines_of(&it, 30, &opts);
    assert_eq!(out.len(), 3);
    assert_eq!(out[1].trim_end(), "› API");
    assert_eq!(out[2].trim_end(), "  Web app ✓");
    // No value: the placeholder in muted.
    let none = doc(keyed(
        "Select",
        "svc",
        json!({"name": "svc", "placeholder": "Pick one", "options": []}),
    ));
    assert!(lines(&none, 30)[0].contains("Pick one"));
    // A value chosen here but not echoed yet wins over the daemon's.
    let local = state(|s| {
        s.values.insert("svc".into(), json!("api"));
    });
    assert!(lines_of(&it, 30, &local)[0].contains("API"));
}

#[test]
fn radio_check_switch_and_slider() {
    let it = doc(node(
        "Stack",
        json!({"children": [
            keyed("RadioGroup", "env", json!({"name": "env", "value": "prod", "options": [
                node("Option", json!({"value": "staging", "label": "Staging"})),
                node("Option", json!({"value": "prod", "label": "Production"})),
            ]})),
            keyed("CheckBox", "ok", json!({"name": "ok", "label": "Checked", "value": true})),
            keyed("CheckBox", "no", json!({"name": "no", "label": "Agreed", "required": true, "value": false, "error": "Required"})),
            keyed("Switch", "on", json!({"name": "on", "label": "Notify", "value": true})),
            keyed("Switch", "off", json!({"name": "off", "label": "Mute"})),
            keyed("Slider", "n", json!({"name": "n", "min": 0, "max": 10, "step": 1, "value": 5})),
        ]}),
    ));
    assert_eq!(
        lines(&it, 40),
        [
            "( ) Staging  (•) Production",
            "[x] Checked",
            "[ ] Agreed *",
            "Required",
            "● on  Notify",
            "○ off Mute",
            "━━━━━━━━━━━━━━━●────────────── 5  (0–10)",
        ][..]
            .iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>()
    );
    // The radio group goes vertical when the options do not fit one line.
    assert_eq!(lines(&it, 20)[..2], ["( ) Staging", "(•) Production"]);
}

#[test]
fn buttons_sit_on_a_background_by_variant_and_a_focused_one_on_the_text_color() {
    let it = doc(node(
        "Buttons",
        json!({"buttons": [
            keyed("Button", "go", json!({"label": "Deploy", "variant": "primary"})),
            keyed("Button", "x", json!({"label": "Delete", "variant": "danger"})),
            keyed("Button", "link", json!({"label": "Docs", "url": "https://x.dev"})),
            keyed("Button", "cp", json!({"label": "Copy", "copy": "text"})),
            keyed("Button", "off", json!({"label": "Wait", "disabled": true})),
        ]}),
    ));
    assert_eq!(
        lines(&it, 80),
        [" Deploy    Delete    Docs ↗    Copy ⧉    Wait"]
    );
    let rendered = render_item(&it, 80, &focused("x"));
    let by = |text: &str| {
        rendered[0]
            .spans
            .iter()
            .find(|s| s.content.contains(text))
            .unwrap()
            .style
    };
    // No brackets, so a button doesn't read as a field: each has a background of its own.
    assert_eq!(by("Deploy").bg, Some(pal().accent));
    assert_eq!(by("Deploy").fg, Some(pal().bg));
    assert_eq!(by("Docs").bg, Some(pal().band()));
    assert_eq!(by("Wait").fg, Some(pal().muted));
    assert_eq!(by("Wait").bg, Some(pal().band()));
    // The focused one (Delete, danger) is on the text color, the others keep theirs.
    assert_eq!(by("Delete").bg, Some(pal().fg));
    assert_eq!(by("Delete").fg, Some(pal().bg));
    let unfocused = render_item(&it, 80, &Options::default());
    let delete = unfocused[0]
        .spans
        .iter()
        .find(|s| s.content.contains("Delete"))
        .unwrap()
        .style;
    assert_eq!(delete.bg, Some(pal().danger));
    // Wraps when they do not fit.
    assert_eq!(lines(&it, 30).len(), 2);
}

fn follow_ups() -> Value {
    node(
        "FollowUpBlock",
        json!({"items": [
            keyed("FollowUpItem", "f.0", json!({"text": "Why is it slow?"})),
            keyed("FollowUpItem", "f.1", json!({"text": "Show the queries"})),
            keyed("FollowUpItem", "f.2", json!({"text": "Fix it"})),
        ]}),
    )
}

#[test]
fn follow_ups_are_pills_in_a_row_that_wraps() {
    let it = doc(follow_ups());
    assert_eq!(
        lines(&it, 80),
        ["( Why is it slow? )  ( Show the queries )  ( Fix it )"]
    );
    assert_eq!(
        lines(&it, 40),
        ["( Why is it slow? )", "( Show the queries )  ( Fix it )"]
    );
    // A pill that is wider than the view is cut.
    assert_eq!(lines(&it, 10)[0], "( Why is …");
    assert_fits(&it, &focused("f.1"));
}

#[test]
fn a_focused_follow_up_is_reversed_and_a_target_like_a_button() {
    let it = doc(follow_ups());
    let rendered = render_item(&it, 80, &focused("f.1"));
    let reversed: Vec<&str> = rendered[0]
        .spans
        .iter()
        .filter(|s| s.style.add_modifier.contains(Modifier::REVERSED))
        .map(|s| s.content.as_ref())
        .collect();
    assert_eq!(reversed, ["( Show the queries )"]);
    let targets = openui_targets(&it, &Options::default());
    let got: Vec<(&str, &str)> = targets
        .iter()
        .map(|t| (t.id.as_str(), t.label.as_str()))
        .collect();
    assert_eq!(
        got,
        [
            ("f.0", "Why is it slow?"),
            ("f.1", "Show the queries"),
            ("f.2", "Fix it")
        ]
    );
    assert!(targets.iter().all(|t| matches!(
        t.kind,
        UiKind::Button {
            disabled: false,
            url: None,
            copy: None
        }
    )));
    // The first line of the focused pill is reported, wherever it wrapped to.
    let rendered = render_item_images(&it, 40, &focused("f.2"));
    assert_eq!(rendered.focus_line, Some(1));
}

#[test]
fn follow_ups_can_be_focused_but_do_not_ask_for_the_keyboard() {
    let it = doc(follow_ups());
    let docs = openui_docs(&it);
    assert!(docs[0].focusable && !docs[0].interactive);
    // Next to a form's field they are part of the form.
    let both = doc(column(vec![
        follow_ups(),
        keyed("Input", "name", json!({"name": "name"})),
    ]));
    assert!(openui_docs(&both)[0].interactive);
}

// ---- image galleries ----

fn picture(src: &str, alt: &str) -> Value {
    node("Image", json!({"src": src, "alt": alt}))
}

fn gallery(n: usize) -> Value {
    let names = [
        "Harbour",
        "Market",
        "Lighthouse",
        "Dunes",
        "Cliffs",
        "Bridge",
    ];
    let images: Vec<Value> = (0..n)
        .map(|i| picture(&format!("/p/{i}.png"), names[i % names.len()]))
        .collect();
    node("ImageGallery", json!({"images": images}))
}

/// Pictures of 400x300 pixels in 10x20 pixel cells, `ready` of them known.
fn pictures(ready: &[usize], rows: u16) -> Options {
    let status: HashMap<String, ImageStatus> = ready
        .iter()
        .map(|i| {
            (
                format!("/p/{i}.png"),
                ImageStatus::Ready {
                    width: 400,
                    height: 300,
                },
            )
        })
        .collect();
    Options {
        images: Some(ImageOptions {
            max_rows: rows,
            cell: (10, 20),
            status: Arc::new(status),
            charts: false,
        }),
        ..Options::default()
    }
}

fn slot(line: usize, x: u16, cols: u16, rows: u16, src: &str) -> ImageSlot {
    ImageSlot {
        line,
        x,
        cols,
        rows,
        src: src.into(),
    }
}

#[test]
fn a_gallery_is_one_large_picture_with_small_ones_stacked_beside_it() {
    let it = doc(gallery(4));
    let out = render_item_images(&it, 80, &pictures(&[0, 1, 2, 3], 12));
    assert_eq!(
        out.images,
        [
            slot(0, 0, 32, 12, "/p/0.png"),
            slot(0, 34, 8, 3, "/p/1.png"),
            slot(4, 34, 8, 3, "/p/2.png"),
            slot(8, 34, 8, 3, "/p/3.png"),
        ]
    );
    let text = plain_text(&out.lines);
    assert_eq!(text.len(), 12 + 1);
    assert_eq!(text[12], "Harbour · 400×300");
    assert!(text[..12].iter().all(String::is_empty));
    // Nothing is left of the marks and nothing is wider than the view.
    assert!(out.lines.iter().all(|l| l.width() <= 80));
    assert!(
        out.lines
            .iter()
            .flat_map(|l| &l.spans)
            .all(|s| s.content != "\u{2060}")
    );
}

#[test]
fn a_gallery_shows_four_pictures_and_counts_the_rest() {
    let it = doc(gallery(6));
    let out = render_item_images(&it, 80, &pictures(&[0, 1, 2, 3, 4, 5], 12));
    assert_eq!(out.images.len(), 4);
    assert_eq!(
        plain_text(&out.lines).last().unwrap(),
        "Harbour · 400×300  +2 more"
    );
    // Two pictures: the large one and one beside it.
    let out = render_item_images(&doc(gallery(2)), 80, &pictures(&[0, 1], 12));
    assert_eq!(out.images.len(), 2);
    assert_eq!((out.images[1].x, out.images[1].line), (34, 0));
    assert_eq!((out.images[1].cols, out.images[1].rows), (16, 6));
}

#[test]
fn a_narrow_gallery_keeps_the_large_picture_and_says_how_many_more() {
    let out = render_item_images(&doc(gallery(4)), 30, &pictures(&[0, 1, 2, 3], 12));
    assert_eq!(out.images.len(), 1);
    assert_eq!(out.images[0].cols, 30);
    assert_eq!(
        plain_text(&out.lines).last().unwrap(),
        "Harbour · 400×300  +3 more"
    );
}

#[test]
fn pictures_that_are_not_there_yet_keep_their_text() {
    let it = doc(gallery(3));
    // One is ready: the images one after the other, as `Image` draws them.
    let out = render_item_images(&it, 80, &pictures(&[1], 12));
    let text = plain_text(&out.lines);
    assert_eq!(out.images, [slot(1, 0, 32, 12, "/p/1.png")]);
    assert_eq!(text[0], "[image Harbour] loading image…");
    assert!(text.contains(&"[image Lighthouse] loading image…".to_string()));
    // Two ready, one still loading: the gallery, then the text of the third.
    let out = render_item_images(&it, 80, &pictures(&[0, 2], 12));
    assert_eq!(out.images.len(), 2);
    assert_eq!(
        plain_text(&out.lines).last().unwrap(),
        "[image Market] loading image…"
    );
    // Without room for pictures they are placeholders.
    assert_eq!(
        lines(&it, 80),
        ["[image Harbour]", "[image Market]", "[image Lighthouse]"]
    );
}

#[test]
fn a_picture_in_a_row_or_grid_keeps_its_room() {
    // Measuring the row draws its children once without keeping them.
    let it = doc(row(
        vec![picture("/p/0.png", "Harbour"), tag("new")],
        json!({}),
    ));
    let out = render_item_images(&it, 80, &pictures(&[0], 12));
    assert_eq!(out.images.len(), 1);
    assert_eq!((out.images[0].line, out.images[0].x), (0, 0));
    let it = doc(node(
        "Grid",
        json!({"children": [
            card("A", vec![picture("/p/0.png", "Harbour")]),
            card("B", vec![picture("/p/1.png", "Market")]),
        ]}),
    ));
    let out = render_item_images(&it, 100, &pictures(&[0, 1], 12));
    assert_eq!(out.images.len(), 2);
    assert!(out.images[0].x < out.images[1].x);
}

#[test]
fn galleries_fit_at_every_width() {
    for n in [2, 4, 6] {
        let ready: Vec<usize> = (0..n).collect();
        let opts = pictures(&ready, 12);
        let it = doc(card("Trip", vec![gallery(n)]));
        for width in 1..=120u16 {
            let out = render_item_images(&it, width, &opts);
            for (i, line) in out.lines.iter().enumerate() {
                assert!(line.width() <= usize::from(width), "{width} line {i}");
            }
            for s in &out.images {
                assert!(
                    usize::from(s.x + s.cols) <= usize::from(width),
                    "{width}: {s:?}"
                );
                assert!(
                    s.line + usize::from(s.rows) <= out.lines.len(),
                    "{width}: {s:?}"
                );
            }
        }
    }
}

// ---- carousels ----

fn slides(n: usize, extra: Value) -> Value {
    let kids: Vec<Value> = (1..=n)
        .map(|i| card(&format!("Slide {i}"), vec![text(&format!("body {i}"))]))
        .collect();
    let mut props = json!({"children": kids});
    if let (Some(p), Some(e)) = (props.as_object_mut(), extra.as_object()) {
        p.extend(e.clone());
    }
    node("Carousel", props)
}

#[test]
fn a_carousel_shows_as_many_slides_as_fit_with_where_you_are_under_them() {
    let it = doc(slides(5, json!({})));
    let out = lines(&it, 80);
    assert!(out[0].matches('╭').count() == 2, "{out:?}");
    assert!(
        out[0].contains("Slide 1") && out[0].contains("Slide 2"),
        "{out:?}"
    );
    assert!(!out.join("\n").contains("Slide 3"), "{out:?}");
    assert_eq!(out.last().unwrap().trim(), "‹ 1–2/5 ›");
    // Centered under the slides.
    assert_eq!(out.last().unwrap().find('‹'), Some((80 - 9) / 2));
    // One slide on a narrow view, and the indicator says so.
    let narrow = lines(&it, 40);
    assert!(
        narrow[0].contains("Slide 1") && !narrow[0].contains("Slide 2"),
        "{narrow:?}"
    );
    assert_eq!(narrow.last().unwrap().trim(), "‹ 1/5 ›");
    // `visible` is a maximum.
    let one = lines(&doc(slides(5, json!({"visible": 1}))), 80);
    assert!(!one.join("\n").contains("Slide 2"), "{one:?}");
}

#[test]
fn scrolling_moves_the_first_slide_and_lights_the_arrows() {
    let it = doc(slides(5, json!({})));
    let at = |first: usize| {
        state(|s| {
            s.slides.insert("/".into(), first);
        })
    };
    let out = lines_of(&it, 80, &at(2));
    assert!(
        out[0].contains("Slide 3") && out[0].contains("Slide 4"),
        "{out:?}"
    );
    assert_eq!(out.last().unwrap().trim(), "‹ 3–4/5 ›");
    // The last slide can lead; a number past the end stays on it.
    for first in [4, 9] {
        let out = lines_of(&it, 80, &at(first));
        assert!(out[0].contains("Slide 5"), "{out:?}");
        assert_eq!(out.last().unwrap().trim(), "‹ 5/5 ›");
    }
    let lit = |first: usize| -> (bool, bool) {
        let rendered = render_item(&it, 80, &at(first));
        let bar = rendered.last().unwrap();
        let fg = |glyph: &str| {
            bar.spans
                .iter()
                .find(|s| s.content == glyph)
                .unwrap()
                .style
                .fg
        };
        (fg("‹") == Some(pal().accent), fg("›") == Some(pal().accent))
    };
    assert_eq!(lit(0), (false, true));
    assert_eq!(lit(2), (true, true));
    assert_eq!(lit(4), (true, false));
}

#[test]
fn a_carousel_is_a_target_and_its_slides_keep_their_own() {
    let buttoned = node(
        "Carousel",
        json!({"children": [
            card("A", vec![keyed("Button", "a", json!({"label": "Open A"}))]),
            card("B", vec![keyed("Button", "b", json!({"label": "Open B"}))]),
            card("C", vec![keyed("Button", "c", json!({"label": "Open C"}))]),
        ]}),
    );
    let it = doc(buttoned);
    let targets = openui_targets(&it, &Options::default());
    let ids: Vec<&str> = targets.iter().map(|t| t.id.as_str()).collect();
    // The strip first, then the buttons of the slides in view; the third is not drawn.
    assert_eq!(ids, ["/", "a", "b"]);
    assert!(matches!(targets[0].kind, UiKind::Carousel { count: 3 }));
    assert_eq!(targets[0].value, json!(0));
    let scrolled = state(|s| {
        s.slides.insert("/".into(), 1);
    });
    let ids: Vec<String> = openui_targets(&it, &scrolled)
        .into_iter()
        .map(|t| t.id)
        .collect();
    assert_eq!(ids, ["/", "b", "c"]);
    // The strip is reversed while it has focus, and a lone slide has no strip.
    let rendered = render_item(&it, 80, &focused("/"));
    assert!(
        rendered
            .last()
            .unwrap()
            .spans
            .iter()
            .any(|s| s.style.add_modifier.contains(Modifier::REVERSED))
    );
    assert!(openui_targets(&doc(slides(1, json!({}))), &Options::default()).is_empty());
    assert!(openui_docs(&it)[0].focusable);
    assert_eq!(
        render_item_images(&it, 80, &focused("/")).focus_line,
        Some(rendered.len() - 1)
    );
}

#[test]
fn carousels_fit_at_every_width() {
    let it = doc(slides(4, json!({})));
    assert_fits(&it, &Options::default());
    assert_fits(&it, &focused("/"));
    assert_fits(
        &doc(card("In a card", vec![slides(3, json!({}))])),
        &Options::default(),
    );
}

// ---- metric indicators ----

fn indicator(props: Value) -> Value {
    node("MetricIndicator", props)
}

#[test]
fn a_metric_indicator_is_the_value_an_arrow_with_the_change_and_a_note() {
    let it = doc(column(vec![
        indicator(
            json!({"value": 1284, "delta": "12%", "direction": "up", "subtext": "vs last week"}),
        ),
        indicator(json!({"value": "3.2s", "delta": "0.4s", "direction": "down"})),
        indicator(json!({"value": "99.9%", "delta": "stable"})),
        indicator(json!({"value": "17", "direction": "up"})),
        indicator(json!({"value": "42"})),
    ]));
    assert_eq!(
        lines(&it, 80),
        [
            "1284 ▲ 12% vs last week",
            "3.2s ▼ 0.4s",
            "99.9% stable",
            "17 ▲",
            "42",
        ]
    );
}

#[test]
fn a_metric_indicator_colors_its_change_by_direction_unless_told_otherwise() {
    let color = |props: Value| {
        let rendered = render_item(&doc(indicator(props)), 80, &Options::default());
        let span = rendered[0]
            .spans
            .iter()
            .find(|s| s.content.contains("5%"))
            .unwrap();
        span.style.fg
    };
    let p = pal();
    assert_eq!(
        color(json!({"value": 1, "delta": "5%", "direction": "up"})),
        Some(p.success)
    );
    assert_eq!(
        color(json!({"value": 1, "delta": "5%", "direction": "down"})),
        Some(p.danger)
    );
    // Costs going down are good news.
    assert_eq!(
        color(json!({"value": 1, "delta": "5%", "direction": "down", "tone": "success"})),
        Some(p.success)
    );
    assert_eq!(color(json!({"value": 1, "delta": "5%"})), Some(p.muted));
    let rendered = render_item(
        &doc(indicator(json!({"value": 7}))),
        80,
        &Options::default(),
    );
    assert!(
        rendered[0].spans[0]
            .style
            .add_modifier
            .contains(Modifier::BOLD)
    );
}

#[test]
fn metric_indicators_sit_in_rows_and_table_cells_and_never_overflow() {
    let up = || indicator(json!({"value": "812", "delta": "3%", "direction": "up"}));
    let it = doc(row(vec![up(), tag("live"), up()], json!({})));
    assert_eq!(lines(&it, 80), ["812 ▲ 3%   ▐live▌   812 ▲ 3%"]);
    let table = doc(node(
        "Table",
        json!({"columns": ["Region", "Orders"], "rows": [["EU", up()], ["US", up()]]}),
    ));
    assert_eq!(lines(&table, 80)[2], "EU     │ 812 ▲ 3%");
    assert_fits(&it, &Options::default());
    assert_fits(&doc(up()), &Options::default());
}

// ---- option cards ----

fn plans(value: Option<&str>) -> Value {
    let mut props = json!({"name": "plan", "options": [
        node("OptionCard", json!({"value": "free", "title": "Free", "description": "Up to three users, community support"})),
        node("OptionCard", json!({"value": "pro", "title": "Pro", "description": "Unlimited users"})),
        node("OptionCard", json!({"value": "team", "title": "Team"})),
    ]});
    if let Some(v) = value {
        props["value"] = json!(v);
    }
    keyed("OptionCards", "plan", props)
}

#[test]
fn option_cards_sit_side_by_side_as_framed_cards() {
    let it = doc(plans(Some("pro")));
    assert_eq!(
        lines(&it, 80),
        [
            "╭────────────────────────╮ ╭────────────────────────╮ ╭────────────────────────╮",
            "│ ( ) Free               │ │ (•) Pro                │ │ ( ) Team               │",
            "│     Up to three users, │ │     Unlimited users    │ │                        │",
            "│     community support  │ │                        │ │                        │",
            "╰────────────────────────╯ ╰────────────────────────╯ ╰────────────────────────╯",
        ]
    );
    // Fewer fit on a narrower view, the rest wrap to a second row of cards.
    let out = lines(&it, 50);
    assert_eq!(out.len(), 9, "{out:?}");
    assert!(
        out[1].contains("( ) Free") && out[1].contains("(•) Pro"),
        "{out:?}"
    );
    assert!(out[7].contains("( ) Team"), "{out:?}");
    // One column on a small screen, cards no wider than they need to be.
    let out = lines(&it, 30);
    assert!(out.iter().all(|l| l.chars().count() <= 30), "{out:?}");
    assert_eq!(out[0].chars().count(), 30);
}

#[test]
fn the_chosen_card_has_an_accent_frame_and_a_focused_one_a_reversed_title() {
    let it = doc(plans(Some("pro")));
    let rendered = render_item(&it, 80, &Options::default());
    let edge = |col: usize| {
        let mut x = 0;
        for s in &rendered[0].spans {
            let w = s.content.chars().count();
            if col < x + w {
                return s.style.fg;
            }
            x += w;
        }
        None
    };
    // The first card keeps the border color, the chosen one the accent.
    assert_eq!(edge(0), Some(pal().border));
    assert_eq!(edge(27), Some(pal().accent));
    let rendered = render_item(&it, 80, &focused("plan"));
    let title = rendered[1]
        .spans
        .iter()
        .find(|s| s.content.contains("Pro"))
        .unwrap();
    assert!(title.style.add_modifier.contains(Modifier::REVERSED));
    assert!(
        !rendered[1].spans.iter().any(
            |s| s.content.contains("Free") && s.style.add_modifier.contains(Modifier::REVERSED)
        )
    );
}

#[test]
fn option_cards_are_a_radio_target_with_the_titles_as_labels() {
    let it = doc(plans(None));
    let targets = openui_targets(&it, &Options::default());
    assert_eq!(targets.len(), 1);
    let UiKind::Radio { options } = &targets[0].kind else {
        panic!("{:?}", targets[0].kind);
    };
    let got: Vec<(&str, &str)> = options
        .iter()
        .map(|o| (o.value.as_str(), o.label.as_str()))
        .collect();
    assert_eq!(got, [("free", "Free"), ("pro", "Pro"), ("team", "Team")]);
    // A value chosen here and not echoed yet wins; the daemon's value shows otherwise.
    let local = state(|s| {
        s.values.insert("plan".into(), json!("team"));
    });
    assert!(lines_of(&it, 80, &local)[1].contains("(•) Team"));
    // Nothing chosen and focused: the first card lights up, and the focus line is its title.
    let rendered = render_item_images(&it, 80, &focused("plan"));
    assert_eq!(rendered.focus_line, Some(1));
}

#[test]
fn option_cards_in_a_form_control_have_their_label_error_and_fit_everywhere() {
    let mut cards = plans(Some("free"));
    cards["props"]["error"] = json!("Pick a plan");
    cards["props"]["required"] = json!(true);
    let it = doc(field("Plan", cards));
    let out = lines(&it, 80);
    assert_eq!(out[0], "Plan *");
    assert_eq!(out.last().unwrap(), "Pick a plan");
    assert_fits(&it, &focused("plan"));
    assert_fits(&it, &Options::default());
}

// ---- checkbox and switch groups ----

fn item_of_group(component: &str, name: &str, label: &str, description: Option<&str>) -> Value {
    let mut props = json!({"name": name, "label": label});
    if let Some(d) = description {
        props["description"] = json!(d);
    }
    node(component, props)
}

fn toppings(value: Value) -> Value {
    keyed(
        "CheckBoxGroup",
        "toppings",
        json!({"name": "toppings", "value": value, "items": [
            item_of_group("CheckBoxItem", "cheese", "Cheese", Some("Extra mozzarella")),
            item_of_group("CheckBoxItem", "olives", "Olives", None),
            item_of_group("CheckBoxItem", "basil", "Basil", Some("Fresh, added after baking")),
        ]}),
    )
}

fn alerts(value: Value) -> Value {
    keyed(
        "SwitchGroup",
        "alerts",
        json!({"name": "alerts", "value": value, "items": [
            item_of_group("SwitchItem", "mail", "Email", Some("A message on every failure")),
            item_of_group("SwitchItem", "sms", "Text message", None),
        ]}),
    )
}

#[test]
fn a_checkbox_group_lists_its_boxes_with_the_checked_ones_on() {
    let it = doc(toppings(json!(["cheese", "basil"])));
    assert_eq!(
        lines(&it, 80),
        [
            "[x] Cheese",
            "    Extra mozzarella",
            "[ ] Olives",
            "[x] Basil",
            "    Fresh, added after baking",
        ]
    );
    // A description wraps under its label.
    let out = lines(&it, 20);
    assert_eq!(out[4..], ["    Fresh, added", "    after baking"]);
}

#[test]
fn a_switch_group_reads_its_object() {
    let it = doc(alerts(json!({"mail": true, "sms": false})));
    assert_eq!(
        lines(&it, 80),
        [
            "● on  Email",
            "      A message on every failure",
            "○ off Text message",
        ]
    );
    // A switch the object does not mention is off.
    assert_eq!(lines(&doc(alerts(json!({}))), 80)[0], "○ off Email");
}

#[test]
fn the_group_cursor_item_is_highlighted_while_the_group_has_focus() {
    let it = doc(toppings(json!(["cheese"])));
    let reversed = |opts: &Options| -> Vec<String> {
        render_item(&it, 80, opts)
            .iter()
            .flat_map(|l| &l.spans)
            .filter(|s| s.style.add_modifier.contains(Modifier::REVERSED))
            .map(|s| s.content.to_string())
            .collect()
    };
    assert!(reversed(&Options::default()).is_empty());
    assert_eq!(reversed(&focused("toppings")), ["[x]"]);
    let on_basil = state(|s| {
        s.focus = Some("toppings".into());
        s.cursors.insert("toppings".into(), 2);
    });
    assert_eq!(reversed(&on_basil), ["[ ]"]);
    let rendered = render_item(&it, 80, &on_basil);
    let label = rendered[3]
        .spans
        .iter()
        .find(|s| s.content.contains("Basil"))
        .unwrap();
    assert_eq!(label.style.fg, Some(pal().accent));
    // The focus line is the cursor item's, and a cursor past the end stays on the last item.
    assert_eq!(render_item_images(&it, 80, &on_basil).focus_line, Some(3));
    let past = state(|s| {
        s.focus = Some("toppings".into());
        s.cursors.insert("toppings".into(), 9);
    });
    assert_eq!(render_item_images(&it, 80, &past).focus_line, Some(3));
}

#[test]
fn a_group_is_one_target_with_its_items_and_a_local_value_wins() {
    let it = doc(column(vec![
        toppings(json!(["cheese"])),
        alerts(json!({"mail": true})),
    ]));
    let targets = openui_targets(&it, &Options::default());
    assert_eq!(targets.len(), 2);
    let UiKind::CheckBoxGroup { items } = &targets[0].kind else {
        panic!("{:?}", targets[0].kind);
    };
    let got: Vec<(&str, &str)> = items
        .iter()
        .map(|i| (i.value.as_str(), i.label.as_str()))
        .collect();
    assert_eq!(
        got,
        [
            ("cheese", "Cheese"),
            ("olives", "Olives"),
            ("basil", "Basil")
        ]
    );
    assert_eq!(targets[0].value, json!(["cheese"]));
    assert!(matches!(&targets[1].kind, UiKind::SwitchGroup { items } if items.len() == 2));
    assert_eq!(targets[1].value, json!({"mail": true}));
    let local = state(|s| {
        s.values.insert("toppings".into(), json!(["olives"]));
        s.values
            .insert("alerts".into(), json!({"mail": false, "sms": true}));
    });
    let out = lines_of(&it, 80, &local);
    assert!(out.contains(&"[x] Olives".to_string()), "{out:?}");
    assert!(out.contains(&"[ ] Cheese".to_string()), "{out:?}");
    assert!(out.contains(&"● on  Text message".to_string()), "{out:?}");
}

#[test]
fn a_group_in_a_form_control_has_a_label_a_marker_and_an_error() {
    let mut group = toppings(json!([]));
    group["props"]["required"] = json!(true);
    group["props"]["error"] = json!("This field is required");
    let it = doc(field("Toppings", group));
    let out = lines(&it, 80);
    assert_eq!(out[0], "Toppings *");
    assert_eq!(out.last().unwrap(), "This field is required");
    for focus in ["toppings", "alerts"] {
        assert_fits(&it, &focused(focus));
    }
    assert_fits(&doc(alerts(json!({"mail": true}))), &focused("alerts"));
}

// ---- entity lists ----

fn entity(title: &str, subtitle: Option<&str>, trailing: Option<Value>) -> Value {
    let mut props = json!({"title": title});
    if let Some(s) = subtitle {
        props["subtitle"] = json!(s);
    }
    if let Some(t) = trailing {
        props["trailing"] = t;
    }
    node("ListItem", props)
}

fn people() -> Value {
    node(
        "EntityList",
        json!({"items": [
            entity("Ada Lovelace", Some("analyst"), Some(tag("admin"))),
            entity("Grace Hopper", Some("rear admiral, compiler pioneer"), Some(tag("owner"))),
            entity("Linus", None, None),
        ]}),
    )
}

#[test]
fn an_entity_list_is_columns_of_title_subtitle_and_trailing() {
    let it = doc(people());
    assert_eq!(
        lines(&it, 80),
        [
            "Ada Lovelace  analyst                         ▐admin▌",
            "Grace Hopper  rear admiral, compiler pioneer  ▐owner▌",
            "Linus",
        ]
    );
    let rendered = render_item(&it, 80, &Options::default());
    let span = |line: usize, text: &str| {
        rendered[line]
            .spans
            .iter()
            .find(|s| s.content.contains(text))
            .unwrap()
            .style
    };
    assert!(span(0, "Ada").add_modifier.contains(Modifier::BOLD));
    assert_eq!(span(0, "analyst").fg, Some(pal().muted));
}

#[test]
fn one_long_title_stacks_the_list_rather_than_squeeze_the_subtitles() {
    let it = doc(node(
        "EntityList",
        json!({"items": [
            entity("Background subagents and session answers", Some("stopped"), None),
            entity("Sessions", Some("kept"), None),
        ]}),
    ));
    assert_eq!(
        lines(&it, 80),
        [
            "Background subagents and session answers",
            "  stopped",
            "Sessions",
            "  kept",
        ]
    );
}

#[test]
fn a_subtitle_that_would_wrap_stacks_the_list() {
    let it = doc(people());
    assert_eq!(
        lines(&it, 40),
        [
            "Ada Lovelace                     ▐admin▌",
            "  analyst",
            "Grace Hopper                     ▐owner▌",
            "  rear admiral, compiler pioneer",
            "Linus",
        ]
    );
}

#[test]
fn a_narrow_entity_list_puts_the_subtitle_under_the_title() {
    let it = doc(people());
    assert_eq!(
        lines(&it, 24),
        [
            "Ada Lovelace     ▐admin▌",
            "  analyst",
            "Grace Hopper     ▐owner▌",
            "  rear admiral, compiler",
            "  pioneer",
            "Linus",
        ]
    );
}

#[test]
fn entity_values_are_aligned_right_and_may_be_numbers() {
    let it = doc(node(
        "EntityList",
        json!({"items": [
            entity("api", Some("12 pods"), Some(json!(1204))),
            entity("web", None, Some(json!("37"))),
            entity("db", Some("primary"), Some(json!(5))),
        ]}),
    ));
    assert_eq!(
        lines(&it, 80),
        [
            "api  12 pods  1204",
            "web             37",
            "db   primary     5"
        ]
    );
}

#[test]
fn entity_lists_fit_at_every_width() {
    assert_fits(&doc(people()), &Options::default());
    assert_fits(&doc(card("People", vec![people()])), &Options::default());
}

// ---- focus targets ----

#[test]
fn the_form_fixture_lists_its_targets_in_drawing_order() {
    let it = fixture("openui-form");
    let targets = openui_targets(&it, &Options::default());
    let got: Vec<(&str, &str)> = targets
        .iter()
        .map(|t| {
            let kind = match t.kind {
                UiKind::Select { .. } => "select",
                UiKind::Input { .. } => "input",
                UiKind::Radio { .. } => "radio",
                UiKind::TextArea => "textarea",
                UiKind::Date => "date",
                UiKind::Slider { .. } => "slider",
                UiKind::Switch => "switch",
                UiKind::CheckBox => "checkbox",
                UiKind::Button { .. } => "button",
                _ => "other",
            };
            (t.id.as_str(), kind)
        })
        .collect();
    assert_eq!(
        got,
        [
            ("service", "select"),
            ("email", "input"),
            ("env", "radio"),
            ("notes", "textarea"),
            ("when", "date"),
            ("canary", "slider"),
            ("notify", "switch"),
            ("agree", "checkbox"),
            ("deploy", "button"),
            ("docs", "button"),
        ]
    );
    assert!(targets.iter().all(|t| t.doc == "0" && t.form.is_some()));
    assert_eq!(targets[0].label, "Service");
    assert_eq!(targets[0].value, json!("api"));
    let UiKind::Select { options } = &targets[0].kind else {
        panic!()
    };
    assert_eq!(options.len(), 3);
    assert_eq!(options[1].label, "Web app");
    let UiKind::Button { url, .. } = &targets[9].kind else {
        panic!()
    };
    assert_eq!(url.as_deref(), Some("https://example.com/runbook"));
    let UiKind::Slider { min, max, step } = targets[5].kind else {
        panic!()
    };
    assert_eq!((min, max, step), (0.0, 50.0, 5.0));
}

#[test]
fn a_streaming_doc_has_no_targets_and_no_focus_marker() {
    let root = form(
        vec![field("A", keyed("Input", "a", json!({"name": "a"})))],
        None,
    );
    let streaming = doc_with(root.clone(), true);
    assert!(openui_targets(&streaming, &Options::default()).is_empty());
    let opts = focused("a");
    let rendered = render_item_images(&streaming, 30, &opts);
    assert_eq!(rendered.focus_line, None);
    assert!(
        rendered.lines[1]
            .spans
            .iter()
            .all(|s| !s.style.add_modifier.contains(Modifier::REVERSED))
    );
    let info = openui_docs(&streaming);
    assert!(info[0].streaming && info[0].interactive);
}

#[test]
fn docs_say_what_they_hold() {
    let it = fixture("openui-chat");
    let docs = openui_docs(&it);
    assert_eq!(docs.len(), 1);
    // Accordions can take focus, but nothing here has a key.
    assert!(docs[0].focusable && !docs[0].interactive);
    let it = fixture("openui-dashboard");
    assert!(openui_docs(&it)[0].focusable);
    assert!(!openui_docs(&it)[0].interactive);
    let it = fixture("openui-form");
    assert!(openui_docs(&it)[0].interactive);
    let plain = item_of(
        "agent",
        "",
        r#"{"type":"text","blocks":[{"type":"paragraph","spans":[{"text":"hi"}]}]}"#,
    );
    assert!(openui_docs(&plain).is_empty());
}

#[test]
fn the_focused_line_is_reported_within_the_item() {
    let it = fixture("openui-form");
    let rendered = render_item_images(&it, 80, &focused("env"));
    let line = rendered.focus_line.expect("focus line");
    assert!(
        rendered.lines[line]
            .spans
            .iter()
            .any(|s| s.content.contains("Staging")),
        "{line}"
    );
    // The marker is never left in the text.
    assert!(
        rendered
            .lines
            .iter()
            .flat_map(|l| &l.spans)
            .all(|s| !s.content.contains('\u{2063}'))
    );
    assert_eq!(
        render_item_images(&it, 80, &Options::default()).focus_line,
        None
    );
    // The first button of the second form field, behind a form's other targets.
    let rendered = render_item_images(&it, 80, &focused("deploy"));
    assert_eq!(rendered.focus_line, Some(rendered.lines.len() - 1));
}

#[test]
fn the_targets_of_a_form_know_their_form() {
    let it = doc(node(
        "Stack",
        json!({"children": [
            form(vec![field("A", keyed("Input", "a", json!({"name": "a"})))], Some(node("Buttons", json!({"buttons": [keyed("Button", "ok", json!({"label": "OK"}))]})))),
            keyed("Button", "loose", json!({"label": "Loose"})),
        ]}),
    ));
    let targets = openui_targets(&it, &Options::default());
    let forms: Vec<(&str, Option<&str>)> = targets
        .iter()
        .map(|t| (t.id.as_str(), t.form.as_deref()))
        .collect();
    assert_eq!(
        forms,
        [
            ("a", Some("/children.0")),
            ("ok", Some("/children.0")),
            ("loose", None),
        ]
    );
}

// ---- docs without a tree, unknown components, partial nodes ----

#[test]
fn a_doc_without_a_root_is_its_first_error_or_nothing() {
    let with_errors = item_of(
        "agent",
        "",
        r#"{"type":"openui","ui":{"id":"0","errors":[{"code":"parse","message":"unexpected token"},{"code":"x","message":"second"}]}}"#,
    );
    assert_eq!(lines(&with_errors, 40), ["[ui: unexpected token]"]);
    let rendered = render_item(&with_errors, 40, &Options::default());
    assert_eq!(rendered[0].spans[0].style.fg, Some(pal().muted));
    let nothing = item_of("agent", "", r#"{"type":"openui","ui":{"id":"0"}}"#);
    assert!(lines(&nothing, 40).is_empty());
}

#[test]
fn unknown_components_are_a_muted_placeholder() {
    let it = doc(node(
        "Stack",
        json!({"children": [node("Hologram", json!({})), text("after")]}),
    ));
    assert_eq!(lines(&it, 40), ["[unsupported: Hologram]", "after"]);
    let rendered = render_item(&it, 40, &Options::default());
    assert_eq!(rendered[0].spans[0].style.fg, Some(pal().muted));
}

#[test]
fn partial_nodes_draw_what_is_there() {
    let it = doc(node(
        "Stack",
        json!({"children": [
            node("Table", json!({"columns": ["A", "B"], "rows": [["x"], ["y", 2]]})),
            node("BarChart", json!({})),
            node("Card", json!({})),
            node("Tabs", json!({})),
            node("Metric", json!({})),
            node("List", json!({})),
            node("Select", json!({})),
        ]}),
    ));
    let out = lines(&it, 30);
    assert!(out[0].starts_with("A"), "{out:?}");
    assert_fits(&it, &Options::default());
}

#[test]
fn hostile_text_is_cleaned() {
    let it = doc(node(
        "Stack",
        json!({"children": [
            node("Header", json!({"title": "a\u{1b}[31mb", "subtitle": "\u{202e}evil"})),
            node("Tag", json!({"label": "x\u{7}y"})),
            keyed("Input", "i", json!({"name": "i", "value": "p\u{1b}[0mq", "error": "bad\u{1b}[1m"})),
        ]}),
    ));
    for l in lines(&it, 40) {
        assert!(
            !l.contains('\u{1b}') && !l.contains('\u{202e}') && !l.contains('\u{7}'),
            "{l:?}"
        );
    }
}

// ---- items ----

#[test]
fn openui_blocks_sit_in_the_text_of_an_item() {
    let it = fixture("openui-chat");
    let out = lines(&it, 60);
    assert_eq!(out[0], "The deploy is halfway:");
    assert!(out.iter().any(|l| l.contains("✓ Build")));
    assert_eq!(
        out.last().unwrap(),
        "I'll continue once the canary is clean."
    );
}

#[test]
fn a_tool_result_is_one_line_unless_it_holds_fields_or_buttons() {
    let dashboard = fixture("openui-dashboard");
    let out = lines(&dashboard, 80);
    assert_eq!(out.len(), 2);
    assert!(out[1].starts_with("  ⎿ Revenue"), "{out:?}");
    assert!(out[1].contains("… +"), "{out:?}");
    let expanded = Options {
        expanded: true,
        ..Options::default()
    };
    assert!(lines_of(&dashboard, 80, &expanded).len() > 10);
    // A doc with a field is drawn in full whatever the options say.
    let tool = item_of(
        "tool",
        r#""label":"app.ask","state":"done","#,
        &json!({"type":"openui","ui":{"id":"0","root": field("A", keyed("Input", "a", json!({"name": "a"})))}}).to_string(),
    );
    let out = lines(&tool, 40);
    assert!(out.len() >= 3 && out[1].trim() == "A", "{out:?}");
    // A focused doc is in use: shown in full too.
    let focus = state(|s| s.focus = Some("/".into()));
    assert!(lines_of(&dashboard, 80, &focus).len() > 10);
}

#[test]
fn every_state_fits_at_every_width() {
    let it = fixture("openui-form");
    let ids = [
        "service", "email", "env", "notes", "when", "canary", "notify", "agree", "deploy", "docs",
    ];
    for id in ids {
        let opts = state(|s| {
            s.focus = Some(id.into());
            s.select = Some(("service".into(), 1));
            s.edits.insert(
                "email".into(),
                UiEdit {
                    text: "a long address@example.com".into(),
                    cursor: 5,
                },
            );
            s.edits.insert(
                "notes".into(),
                UiEdit {
                    text: "line one\nline two is a bit longer than the rest".into(),
                    cursor: 12,
                },
            );
        });
        assert_fits(&it, &opts);
    }
    for name in ["openui-dashboard", "openui-chat", "openui-streaming"] {
        let it = fixture(name);
        let tabs = state(|s| {
            s.focus = Some("/children.2".into());
            for id in ["/children.2", "/"] {
                s.tabs.insert(id.into(), 1);
            }
            s.open.insert("/items.0".into(), true);
        });
        assert_fits(&it, &tabs);
        assert_fits(
            &it,
            &Options {
                expanded: true,
                ..Options::default()
            },
        );
    }
}

#[test]
fn colors_come_from_the_palette() {
    let mut opts = Options::default();
    opts.theme.accent = Color::Rgb(1, 2, 3);
    opts.theme.danger = Color::Rgb(9, 9, 9);
    let it = fixture("openui-form");
    let rendered = render_item(&it, 80, &opts);
    let has = |c: Color| {
        rendered
            .iter()
            .flat_map(|l| &l.spans)
            .any(|s| s.style.fg == Some(c))
    };
    assert!(has(Color::Rgb(1, 2, 3)) && has(Color::Rgb(9, 9, 9)));
}

// ---- grids, strips and the rest of the dashboard look ----

/// The column where the left card of a side-by-side pair ends and the right one starts.
fn card_starts(line: &str) -> Vec<usize> {
    line.chars()
        .enumerate()
        .filter(|(_, c)| *c == '╭')
        .map(|(i, _)| i)
        .collect()
}

#[test]
fn sibling_cards_are_a_grid_that_stacks_when_narrow() {
    let it = doc(column(vec![
        card("One", vec![text("short")]),
        card("Two", vec![text("a"), text("b"), text("c")]),
    ]));
    let wide = lines(&it, 80);
    // Side by side with a one cell gap, columns of the same width.
    assert_eq!(card_starts(&wide[0]), [0, 41], "{wide:?}");
    assert!(
        wide.iter()
            .all(|l| l.chars().count() == 79 + 1 || l.chars().count() == 80)
    );
    // Both outlines end on the tallest card's line: five lines, bottoms aligned.
    assert_eq!(wide.len(), 5, "{wide:?}");
    assert!(
        wide[4].starts_with('╰') && wide[4].ends_with('╯'),
        "{wide:?}"
    );
    assert_eq!(wide[4].matches('╰').count(), 2, "{wide:?}");
    assert!(wide[1].starts_with("│ short"), "{wide:?}");
    // Too narrow for two columns of 36: one under the other.
    let narrow = lines(&it, 40);
    assert_eq!(narrow.len(), 3 + 5, "{narrow:?}");
    assert_eq!(card_starts(&narrow[0]), [0]);
    assert_eq!(card_starts(&narrow[3]), [0]);
    // Three cards at 80: two columns, then the third under the first.
    let it = doc(column(vec![
        card("A", vec![text("a")]),
        card("B", vec![text("b")]),
        card("C", vec![text("c")]),
    ]));
    let out = lines(&it, 80);
    assert_eq!(out.len(), 6, "{out:?}");
    assert_eq!(card_starts(&out[3]), [0], "{out:?}");
    // A single card, or cards split by something else, stay as they are.
    let it = doc(column(vec![
        card("A", vec![text("a")]),
        text("between"),
        card("B", vec![text("b")]),
    ]));
    assert!(lines(&it, 80).iter().all(|l| card_starts(l).len() <= 1));
}

#[test]
fn cards_in_a_card_or_a_tab_are_a_grid_too() {
    let two = vec![card("A", vec![text("a")]), card("B", vec![text("b")])];
    let it = doc(node("Card", json!({"children": two, "title": "Outer"})));
    let out = lines(&it, 90);
    assert_eq!(out.len(), 5, "{out:?}");
    assert_eq!(card_starts(&out[1]).len(), 2, "{out:?}");
    let tabs = node(
        "Tabs",
        json!({"tabs": [node("Tab", json!({"label": "x", "children": two}))]}),
    );
    let out = lines(&doc(tabs), 80);
    assert_eq!(card_starts(&out[1]).len(), 2, "{out:?}");
}

#[test]
fn a_grid_has_at_most_columns_columns_and_as_many_as_fit() {
    let cards: Vec<Value> = (0..4)
        .map(|i| card(&format!("C{i}"), vec![text("x")]))
        .collect();
    let grid = |columns: Option<u32>| {
        let mut props = json!({"children": cards});
        if let Some(c) = columns {
            props["columns"] = json!(c);
        }
        doc(node("Grid", props))
    };
    // 120 cells hold three columns of 36; `columns` can only lower that.
    assert_eq!(card_starts(&lines(&grid(None), 120)[0]).len(), 3);
    assert_eq!(card_starts(&lines(&grid(Some(2)), 120)[0]).len(), 2);
    assert_eq!(card_starts(&lines(&grid(Some(1)), 120)[0]).len(), 1);
    assert_eq!(card_starts(&lines(&grid(Some(4)), 80)[0]).len(), 2);
    // Narrow: one column whatever was asked for.
    assert_eq!(card_starts(&lines(&grid(Some(4)), 40)[0]).len(), 1);
    // The widths share the room: 118 = 40 + 39 + 39 (the spare cell goes to the first).
    let first = &lines(&grid(None), 120)[0];
    assert_eq!(card_starts(first), [0, 41, 81], "{first:?}");
    // Not only cards: a chart next to a table, each at its column.
    let it = doc(node(
        "Grid",
        json!({"children": [
            node("BarChart", json!({"labels": ["a"], "series": [series("n", &[1.0])], "horizontal": true})),
            text("beside"),
        ]}),
    ));
    let out = lines(&it, 80);
    assert!(
        out[0].starts_with("a ") && out[0].contains("beside"),
        "{out:?}"
    );
    assert_fits(&it, &Options::default());
}

#[test]
fn a_grid_row_is_as_tall_as_its_tallest_cell_in_every_cell() {
    let it = doc(node(
        "Grid",
        json!({"children": [
            card("Tall", vec![text("1"), text("2"), text("3"), text("4")]),
            card("Short", vec![text("1")]),
            card("Next", vec![text("n")]),
        ]}),
    ));
    let out = lines(&it, 80);
    // 6 lines for the first row, 3 for the lone card under it.
    assert_eq!(out.len(), 6 + 3, "{out:?}");
    assert!(out[5].matches('╯').count() == 2, "{out:?}");
    assert!(out[6].starts_with("╭─ Next"), "{out:?}");
}

#[test]
fn metrics_in_a_column_or_a_row_are_a_strip_divided_by_bars() {
    let ms = || {
        vec![
            node(
                "Metric",
                json!({"label": "Used", "value": "820 GB", "delta": "of 896 GB", "tone": "danger"}),
            ),
            metric("Free", "72 GB"),
        ]
    };
    let expect = ["820 GB of 896 GB  │  72 GB", "Used              │  Free"];
    assert_eq!(lines(&doc(column(ms())), 80), expect);
    assert_eq!(lines(&doc(row(ms(), json!({}))), 80), expect);
    // Narrower: tighter dividers.
    assert_eq!(
        lines(&doc(column(ms())), 25),
        ["820 GB of 896 GB │ 72 GB", "Used             │ Free"]
    );
    // Value bold, delta in its tone, label muted.
    let rendered = render_item(&doc(column(ms())), 80, &Options::default());
    let span = |row: usize, text: &str| {
        rendered[row]
            .spans
            .iter()
            .find(|s| s.content.contains(text))
            .unwrap()
            .style
    };
    assert!(span(0, "820 GB").add_modifier.contains(Modifier::BOLD));
    assert_eq!(span(0, "of 896").fg, Some(pal().danger));
    assert_eq!(span(1, "Used").fg, Some(pal().muted));
    assert_eq!(span(0, "│").fg, Some(pal().border));
    // More than fit wrap into further strips.
    let many: Vec<Value> = (0..4)
        .map(|i| metric(&format!("Label {i}"), "1234567"))
        .collect();
    let out = lines(&doc(column(many)), 30);
    assert_eq!(out.len(), 5, "{out:?}");
    assert_eq!(out[2], "");
    // A single Metric is the same column on its own.
    assert_eq!(
        lines(&doc(column(vec![metric("Free", "72 GB")])), 30),
        ["72 GB", "Free"]
    );
}

#[test]
fn bars_never_touch_and_the_labels_stay_left() {
    let it = doc(node(
        "BarChart",
        json!({"labels": ["Documents", ".cache", "Other"], "series": [series("GB", &[355.0, 65.0, 103.0])], "horizontal": true}),
    ));
    let out = lines(&it, 40);
    assert_eq!(out.len(), 3, "{out:?}");
    // No full block anywhere: the upper half of each line is empty, a gap between bars.
    assert!(out.iter().all(|l| !l.contains('█')), "{out:?}");
    assert!(out[0].starts_with("Documents "), "{out:?}");
    assert!(out[1].starts_with(".cache    "), "{out:?}");
    assert!(out[1].ends_with(" 65"), "{out:?}");
    let rendered = render_item(&it, 40, &Options::default());
    assert_eq!(rendered[1].spans[0].style.fg, Some(pal().muted));
}

#[test]
fn a_progress_label_that_says_the_percent_is_not_said_twice() {
    let progress = |value: f64, label: Option<&str>| {
        let mut props = json!({"value": value});
        if let Some(l) = label {
            props["label"] = json!(l);
        }
        lines(&doc(node("Progress", props)), 60).remove(0)
    };
    let tail = |l: &str| l.trim_start_matches(['▰', '▱']).to_string();
    assert_eq!(tail(&progress(0.92, Some("92% used"))), " 92% used");
    assert_eq!(tail(&progress(0.92, Some("used: 92%"))), " used: 92%");
    // A different number, or none: the bar's percent comes first.
    assert_eq!(tail(&progress(0.92, Some("of 896 GB"))), " 92% of 896 GB");
    assert_eq!(tail(&progress(0.5, Some("92% used"))), " 50% 92% used");
    assert_eq!(tail(&progress(0.6, None)), " 60%");
    let bar = progress(0.5, None);
    assert_eq!(bar.matches('▰').count(), bar.matches('▱').count(), "{bar}");
}

#[test]
fn a_table_has_no_blank_line_under_its_header() {
    let table = node(
        "Table",
        json!({"columns": ["Path", "Size"], "rows": [["a", 1], ["b", 22]]}),
    );
    let it = doc(table.clone());
    let out = lines(&it, 40);
    assert_eq!(out.len(), 4, "{out:?}");
    assert_eq!(out[1], "─────┼─────");
    assert!(out.iter().all(|l| !l.trim().is_empty()));
    // In a card, next to another, narrow or wide.
    let boxed = doc(column(vec![
        card("A", vec![table.clone()]),
        card("B", vec![table.clone()]),
    ]));
    for w in [40, 80] {
        let out = lines(&boxed, w);
        assert_eq!(out.len(), if w == 40 { 12 } else { 6 }, "{out:?}");
        assert!(out[2].contains('┼'), "{out:?}");
        assert!(out[3].contains("│ a"), "{out:?}");
    }
}

#[test]
fn a_form_in_a_card_keeps_its_focus_line() {
    let form = node(
        "Form",
        json!({"name": "f", "children": [
            node("FormControl", json!({"label": "Name", "input": keyed("Input", "name", json!({"name": "name"}))})),
        ]}),
    );
    let it = doc(column(vec![
        card("First", vec![text("a"), text("b")]),
        card("Second", vec![form]),
    ]));
    let opts = focused("name");
    let r = render_item_images(&it, 80, &opts);
    let line = r.focus_line.expect("focus line");
    let out = plain_text(&r.lines);
    // The field, not its label: the first line of the focused target.
    assert!(out[line].contains("│ [ "), "{line} {out:?}");
    assert_eq!(openui_targets(&it, &opts).len(), 1);
}

#[test]
fn the_images_of_a_doc_are_asked_for_also_in_cards_and_galleries() {
    let image = |src: &str| node("Image", json!({"src": src, "alt": "room"}));
    let it = doc(node(
        "Grid",
        json!({"children": [
            node("Card", json!({"children": [image("https://x.test/a.jpg"), text("Hôtel A")]})),
            node("Card", json!({"children": [
                node("ImageGallery", json!({"images": [image("https://x.test/b.jpg"), image("asset://img-c")]})),
            ]})),
        ]}),
    ));
    assert_eq!(
        image_sources(&it),
        [
            "https://x.test/a.jpg",
            "https://x.test/b.jpg",
            "asset://img-c"
        ]
    );
    let in_text = item_of(
        "agent",
        "",
        &json!({"type": "text", "blocks": [
            {"type": "paragraph", "spans": [{"text": "Here:"}]},
            {"type": "openui", "ui": {"id": "0", "root": image("/tmp/d.png")}},
        ]})
        .to_string(),
    );
    assert_eq!(image_sources(&in_text), ["/tmp/d.png"]);
}

// ─── Box and Shape ──────────────────────────────────────────────────────────

fn boxed(children: Vec<Value>, extra: Value) -> Value {
    let mut props = json!({"children": children});
    if let (Some(p), Some(e)) = (props.as_object_mut(), extra.as_object()) {
        p.extend(e.clone());
    }
    node("Box", props)
}

fn shape(kind: &str, points: Value, extra: Value) -> Value {
    let mut props = json!({"kind": kind, "points": points});
    if let (Some(p), Some(e)) = (props.as_object_mut(), extra.as_object()) {
        p.extend(e.clone());
    }
    node("Shape", props)
}

fn widths(it: &ViewItem, width: u16) -> Vec<usize> {
    lines(it, width).iter().map(|l| l.chars().count()).collect()
}

#[test]
fn a_box_in_a_row_is_a_share_of_what_the_gaps_leave() {
    let half = |label: &str| boxed(vec![text(label)], json!({"width": 0.5, "border": "muted"}));
    let it = doc(boxed(
        vec![half("left"), half("right")],
        json!({"dir": "row", "gap": "s"}),
    ));
    // 40 cells less the gap of 2, in two halves.
    let rows = lines(&it, 40);
    assert_eq!(rows.len(), 3, "{rows:?}");
    assert_eq!(
        rows[0],
        format!("┌{}┐  ┌{}┐", "─".repeat(17), "─".repeat(17))
    );
    assert!(
        rows[1].starts_with("│left") && rows[1].contains("│  │right"),
        "{rows:?}"
    );
    assert_eq!(widths(&it, 40), [40, 40, 40]);
}

#[test]
fn a_box_in_a_row_without_a_width_takes_its_content_and_a_fill_box_what_is_left() {
    let it = doc(boxed(
        vec![
            boxed(vec![text("ab")], json!({"border": "muted"})),
            boxed(
                vec![text("rest")],
                json!({"width": "fill", "border": "accent"}),
            ),
        ],
        json!({"dir": "row", "gap": "none"}),
    ));
    assert_eq!(widths(&it, 20), [20, 20, 20]);
    assert_eq!(lines(&it, 20)[1], format!("│ab││rest{}│", " ".repeat(10)));
}

#[test]
fn a_box_fills_a_column_unless_it_asks_for_less() {
    let framed = |extra: Value| {
        let mut props = json!({"border": "accent"});
        if let Some(p) = props.as_object_mut() {
            p.extend(extra.as_object().cloned().unwrap_or_default());
        }
        doc(column(vec![boxed(vec![text("hi")], props)]))
    };
    assert_eq!(widths(&framed(json!({})), 30), [30, 30, 30]);
    assert_eq!(widths(&framed(json!({"width": "fill"})), 30), [30, 30, 30]);
    assert_eq!(widths(&framed(json!({"width": "fit"})), 30), [4, 4, 4]);
    assert_eq!(widths(&framed(json!({"width": 0.5})), 30), [15, 15, 15]);
    // More than the whole is the whole.
    assert_eq!(widths(&framed(json!({"width": 3})), 30), [30, 30, 30]);
}

#[test]
fn a_box_frames_pads_and_washes_in_tones() {
    let it = doc(boxed(
        vec![text("hi")],
        json!({"border": "danger", "fill": "success", "pad": "s", "rounded": true, "width": "fit"}),
    ));
    assert_eq!(lines(&it, 20), ["╭────╮", "│ hi │", "╰────╯"]);
    let drawn = render_item(&it, 20, &Options::default());
    let edge = &drawn[0].spans[0];
    assert_eq!(edge.style.fg, Some(pal().danger));
    assert!(edge.style.bg.is_some(), "the fill reaches the frame");
    // Every cell of the middle line has the wash behind it.
    assert!(drawn[1].spans.iter().all(|s| s.style.bg == edge.style.bg));
    // Square corners unless rounded.
    let square = doc(boxed(
        vec![text("hi")],
        json!({"border": "muted", "width": "fit"}),
    ));
    assert_eq!(lines(&square, 20), ["┌──┐", "│hi│", "└──┘"]);
}

#[test]
fn a_box_without_a_frame_pad_or_fill_is_just_its_children() {
    let plain = doc(boxed(vec![text("a"), text("b")], json!({})));
    let stack = doc(column(vec![text("a"), text("b")]));
    assert_eq!(lines(&plain, 20), lines(&stack, 20));
    assert!(lines(&doc(boxed(vec![], json!({}))), 20).is_empty());
}

#[test]
fn a_box_row_wraps_and_a_column_gap_l_is_a_blank_line() {
    let tags = vec![tag("one"), tag("two"), tag("three")];
    let wrapped = doc(boxed(tags.clone(), json!({"dir": "row", "wrap": true})));
    assert_eq!(lines(&wrapped, 12).len(), 2);
    let spaced = doc(boxed(vec![text("a"), text("b")], json!({"gap": "l"})));
    assert_eq!(lines(&spaced, 20), ["a", "", "b"]);
}

#[test]
fn a_filled_rect_is_blocks_and_an_outline_leaves_its_inside_empty() {
    // 4 cells across at aspect 2: 8 dots by 4 dots, one row.
    let whole = doc(shape("rect", json!([0, 0, 1, 1]), json!({"fill": true})));
    assert_eq!(lines(&whole, 4), ["████"]);
    let half = doc(shape("rect", json!([0, 0, 1, 0.5]), json!({"fill": true})));
    assert_eq!(lines(&half, 4), ["▀▀▀▀"]);
    let left = doc(shape("rect", json!([0, 0, 0.5, 1]), json!({"fill": true})));
    assert_eq!(lines(&left, 4), ["██"]);
    // 12 dots square: three rows, the middle one hollow.
    let outline = doc(shape("rect", json!([0, 0, 1, 1]), json!({"aspect": 1})));
    let rows = lines(&outline, 6);
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[1], "▌    ▐");
}

#[test]
fn a_shape_is_drawn_in_its_tone() {
    let it = doc(shape(
        "rect",
        json!([0, 0, 1, 1]),
        json!({"tone": "danger", "fill": true}),
    ));
    let drawn = render_item(&it, 4, &Options::default());
    assert!(
        drawn[0]
            .spans
            .iter()
            .all(|s| s.style.fg == Some(pal().danger))
    );
    let plain = doc(shape("rect", json!([0, 0, 1, 1]), json!({"fill": true})));
    let drawn = render_item(&plain, 4, &Options::default());
    assert!(drawn[0].spans.iter().all(|s| s.style.fg == Some(pal().fg)));
}

#[test]
fn the_aspect_sets_the_height_and_the_height_is_held() {
    let tall = |aspect: f64| {
        let it = doc(shape(
            "rect",
            json!([0, 0, 1, 1]),
            json!({"fill": true, "aspect": aspect}),
        ));
        lines(&it, 20).len()
    };
    assert_eq!(tall(1.0), 10);
    assert_eq!(tall(2.0), 5);
    assert_eq!(tall(10.0), 1);
    // A very narrow box is not a very tall one.
    assert_eq!(tall(0.01), 16);
    // Nonsense falls back to the default.
    assert_eq!(tall(-3.0), 5);
}

#[test]
fn lines_arcs_and_paths_are_drawn_and_what_is_not_a_shape_is_not() {
    let ink = |it: &ViewItem, w: u16| lines(it, w).join("").chars().filter(|c| *c != ' ').count();
    let diagonal = doc(shape("line", json!([0, 0, 1, 1]), json!({})));
    assert!(ink(&diagonal, 10) >= 10);
    let ring = doc(shape(
        "arc",
        json!([0.5, 0.5, 0.5, 0, 1]),
        json!({"aspect": 1}),
    ));
    let pie = doc(shape(
        "arc",
        json!([0.5, 0.5, 0.5, 0, 1]),
        json!({"aspect": 1, "fill": true}),
    ));
    assert!(ink(&pie, 10) > ink(&ring, 10));
    // A quarter turn from the top is the upper right: nothing on the left.
    let quarter = doc(shape(
        "arc",
        json!([0.5, 0.5, 0.5, 0, 0.25]),
        json!({"aspect": 1, "fill": true}),
    ));
    for row in lines(&quarter, 10) {
        assert!(row.chars().take(5).all(|c| c == ' '), "{row:?}");
    }
    let triangle = doc(shape(
        "path",
        json!([0.5, 0, 1, 1, 0, 1]),
        json!({"fill": true, "aspect": 1}),
    ));
    let drawn = lines(&triangle, 10);
    assert!(drawn[0].trim().chars().count() < drawn.last().map_or(0, |l| l.trim().chars().count()));
    // Too few points and an unknown kind draw nothing.
    for bad in [
        shape("rect", json!([0, 0]), json!({})),
        shape("blob", json!([0, 0, 1, 1]), json!({})),
    ] {
        assert_eq!(ink(&doc(bad), 10), 0);
    }
}

#[test]
fn shapes_that_follow_each_other_share_one_box() {
    let a = shape(
        "rect",
        json!([0, 0, 0.5, 1]),
        json!({"fill": true, "tone": "accent"}),
    );
    let b = shape(
        "rect",
        json!([0.5, 0, 0.5, 1]),
        json!({"fill": true, "tone": "danger"}),
    );
    let together = doc(column(vec![a.clone(), b.clone()]));
    assert_eq!(lines(&together, 4), ["████"]);
    let drawn = render_item(&together, 4, &Options::default());
    assert_eq!(drawn[0].spans.len(), 2);
    assert_eq!(drawn[0].spans[0].style.fg, Some(pal().accent));
    assert_eq!(drawn[0].spans[1].style.fg, Some(pal().danger));
    // Something between them ends the run.
    let apart = doc(column(vec![a, text("x"), b]));
    assert_eq!(lines(&apart, 4), ["██", "x", "  ██"]);
}

#[test]
fn box_and_shape_never_overflow() {
    let it = doc(boxed(
        vec![
            boxed(
                vec![
                    text("a few words"),
                    shape("arc", json!([0.5, 0.5, 0.5, 0, 0.7]), json!({"fill": true})),
                ],
                json!({"border": "accent", "pad": "l", "fill": "muted", "width": 0.6, "rounded": true}),
            ),
            boxed(
                vec![tag("tag")],
                json!({"width": "fit", "border": "danger"}),
            ),
        ],
        json!({"dir": "row", "pad": "m", "border": "muted"}),
    ));
    assert_fits(&it, &Options::default());
    assert_fits(&fixture("openui-boxes"), &Options::default());
}

/// The fixtures use every component of the library (a daemon test checks that), so one this
/// renderer leaves out shows up here as the placeholder.
#[test]
fn nothing_in_the_openui_fixtures_is_left_undrawn() {
    for name in fixture_names().iter().filter(|n| n.starts_with("openui-")) {
        for it in fixture_items(name) {
            for width in [40u16, 80] {
                let drawn = lines(&it, width).join("\n");
                assert!(
                    !drawn.contains("[unsupported"),
                    "{name} at {width}: {drawn}"
                );
            }
        }
    }
}
