//! What the renderer promises, one case at a time (the snapshots cover the fixtures).

mod common;

use common::*;
use neo_renderer_ratatui::syntax::Class;
use ratatui::style::{Color, Modifier};
use ratatui::text::Line;
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;

use neo_renderer_ratatui::{
    ImageOptions, ImageSlot, ImageStatus, Options, Palette, buttons, image_cells, image_sources,
    keep_buttons, plain_text, render_item, render_item_images,
};

fn pal() -> Palette {
    Palette::default()
}

fn text(s: &str) -> String {
    format!(
        r#"{{"type":"text","blocks":[{{"type":"paragraph","spans":[{{"text":{}}}]}}]}}"#,
        json!(s)
    )
}

fn lines(it: &neo_protocol::ViewItem, width: u16) -> Vec<String> {
    lines_of(it, width, &Options::default())
}

fn tool(state: &str, component: &str) -> neo_protocol::ViewItem {
    item_of(
        "tool",
        &format!(r#""label":"bash: ls","state":"{state}","#),
        component,
    )
}

fn code(s: &str) -> String {
    format!(r#"{{"type":"code","code":{}}}"#, json!(s))
}

fn assert_fits(lines: &[Line<'_>], width: u16) {
    for l in lines {
        assert!(l.width() <= usize::from(width), "{width}: {l:?}");
    }
}

// ---- items ----

#[test]
fn the_apps_own_messages_leave_out_their_tag() {
    let it = |app: &str| item_of("user", &format!(r#""label":"{app}","#), &text("hi"));
    let opts = Options {
        app: Some("terminal".into()),
        ..Options::default()
    };
    assert_eq!(lines_of(&it("terminal"), 40, &opts), ["› hi"]);
    assert_eq!(lines_of(&it("terminal-2"), 40, &opts), ["› hi"]);
    assert_eq!(lines_of(&it("gui"), 40, &opts), ["› hi  · gui"]);
}

#[test]
fn user_items_get_a_prompt_and_the_app_dimmed_on_a_band() {
    let it = item_of("user", r#""label":"browser","#, &text("make it smaller"));
    assert_eq!(lines(&it, 40), ["› make it smaller  · browser"]);
    let rendered = render_item(&it, 40, &Options::default());
    let spans = &rendered[0].spans;
    assert_eq!(spans[0].content, "›");
    assert_eq!(spans[0].style.fg, Some(pal().accent));
    let tag = spans
        .iter()
        .find(|s| s.content.contains("browser"))
        .unwrap();
    assert_eq!(tag.style.fg, Some(pal().muted));
    // The band spans the whole width, so the user's turns stand out from the agent's.
    assert_eq!(rendered[0].width(), 40);
    assert!(spans.iter().all(|s| s.style.bg == Some(pal().band())));
    assert_eq!(pal().band(), ratatui::style::Color::Rgb(0x33, 0x34, 0x39));
}

#[test]
fn user_text_hangs_under_the_prompt_and_the_label_moves_when_it_does_not_fit() {
    let it = item_of(
        "user",
        r#""label":"browser","#,
        &text("one two three four five six"),
    );
    assert_eq!(
        lines(&it, 14),
        ["› one two", "  three four", "  five six", "  · browser"]
    );
    let bare = item_of("user", "", &text("hi"));
    assert_eq!(lines(&bare, 14), ["› hi"]);
}

#[test]
fn agent_text_is_drawn_as_it_is_with_a_header_for_guests() {
    let main = item_of("agent", "", &text("hello there"));
    assert_eq!(lines(&main, 40), ["hello there"]);
    assert_eq!(lines(&main, 9), ["hello", "there"]);
    let guest = item_of("agent", r#""label":"claude","#, &text("hello there"));
    assert_eq!(lines(&guest, 40), ["claude", "hello there"]);
    // Other components are drawn as they are.
    let card = r#"{"type":"card","title":"T","children":[]}"#;
    assert!(lines(&item_of("agent", "", card), 40)[0].starts_with('╭'));
    let head = &render_item(&guest, 40, &Options::default())[0];
    assert_eq!(head.spans[0].style.fg, Some(pal().accent));
}

#[test]
fn thoughts_collapse_to_one_line() {
    let it = item_of("thought", "", &text("let me think about this at length"));
    assert_eq!(lines(&it, 20), ["∴ thinking"]);
    let open = Options {
        show_thoughts: true,
        ..Options::default()
    };
    assert_eq!(
        lines_of(&it, 20, &open),
        ["∴ let me think about", "  this at length"]
    );
    let line = &render_item(&it, 20, &open)[0];
    assert_eq!(line.spans[1].style.fg, Some(pal().muted));
    assert!(line.spans[1].style.add_modifier.contains(Modifier::ITALIC));
    let guest = item_of("thought", r#""label":"codex","#, &text("x"));
    assert_eq!(lines(&guest, 40), ["∴ codex thinking"]);
}

#[test]
fn notices_are_one_dim_line_and_red_when_failed() {
    let note = item_of("notice", "", &text("now in overlay"));
    let l = &render_item(&note, 40, &Options::default())[0];
    assert_eq!(l.spans[0].style.fg, Some(pal().muted));
    let failed = item_of("notice", r#""state":"failed","#, &text("boom"));
    let l = &render_item(&failed, 40, &Options::default())[0];
    assert_eq!(l.spans[0].style.fg, Some(pal().danger));
}

// ---- tool items ----

#[test]
fn only_a_failed_call_is_colored() {
    for state in ["pending", "running", "done", "failed"] {
        let it = tool(state, &code("x"));
        let head = &render_item(&it, 40, &Options::default())[0];
        let label: String = head.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(label, "bash: ls");
        let red = head.spans.iter().all(|s| s.style.fg == Some(pal().danger));
        assert_eq!(red, state == "failed", "{state}");
    }
}

#[test]
fn the_verb_is_bold_and_the_summary_muted() {
    let bold = |s: &ratatui::text::Span<'_>| s.style.add_modifier.contains(Modifier::BOLD);
    for (label, verb, rest) in [
        ("fetch https://a.dev", "fetch ", "https://a.dev"),
        ("search the web: a b: c", "search the web: ", "a b: c"),
        ("update", "update", ""),
    ] {
        let it = item_of(
            "tool",
            &format!(r#""label":{},"state":"done","#, json!(label)),
            &code("out"),
        );
        let lines = render_item(&it, 60, &Options::default());
        let head = &lines[0].spans[..];
        assert_eq!((head[0].content.as_ref(), bold(&head[0])), (verb, true));
        if !rest.is_empty() {
            assert_eq!((head[1].content.as_ref(), bold(&head[1])), (rest, false));
        }
        let out = lines[1]
            .spans
            .iter()
            .find(|s| s.content.contains("out"))
            .unwrap();
        assert_eq!(out.style.fg, Some(pal().muted));
    }
}

#[test]
fn prose_wraps_at_the_prose_width_but_code_does_not() {
    let words = "word ".repeat(40);
    let c = json!({"type": "text", "blocks": [
        {"type": "paragraph", "spans": [{"text": words}]},
        {"type": "code_block", "code": "x".repeat(150)}
    ]});
    let opts = Options {
        prose_width: Some(50),
        ..Options::default()
    };
    let lines = render_item(&item_of("agent", "", &c.to_string()), 160, &opts);
    let widest = |text: bool| {
        lines
            .iter()
            .filter(|l| text == !l.to_string().contains('x'))
            .map(Line::width)
            .max()
            .unwrap()
    };
    assert!(widest(true) <= 52, "bullet + 50");
    assert_eq!(widest(false), 160);
}

#[test]
fn a_command_to_approve_is_highlighted_and_broken_at_its_operators() {
    let c = json!({"type": "stack", "gap": "s", "children": [
        {"type": "code", "language": "bash", "code": "cd /tmp/some/dir && cargo test --workspace 2>&1 | tail -20"},
        {"type": "button", "label": "Allow", "action": "neo.permit", "key": "1"}
    ]});
    let it = item_of(
        "tool",
        r#""label":"Run this command?","state":"pending","#,
        &c.to_string(),
    );
    let lines = render_item(&it, 40, &Options::default());
    let text: Vec<String> = lines
        .iter()
        .map(|l| l.to_string().trim_end().to_string())
        .collect();
    assert_eq!(
        text[1..4],
        [
            "    cd /tmp/some/dir &&",
            "      cargo test --workspace 2>&1 |",
            "      tail -20",
        ]
    );
    let cargo = lines[2]
        .spans
        .iter()
        .find(|s| s.content == "cargo")
        .unwrap();
    assert_eq!(cargo.style.fg, code_fg(Class::Function), "the command name");
}

/// The color code of a class is drawn in (true colors).
fn code_fg(class: Class) -> Option<ratatui::style::Color> {
    neo_renderer_ratatui::syntax::code_colors(&pal()).unwrap()[class as usize].fg
}

#[test]
fn compact_code_shows_the_first_line_and_how_many_are_cut() {
    let out = tool("done", &code("a\nb\nc\nd\ne\n"));
    assert_eq!(lines(&out, 40), ["bash: ls", "  ⎿ a  … +4 lines"]);
    let one = tool("done", &code("a"));
    assert_eq!(lines(&one, 40), ["bash: ls", "  ⎿ a"]);
    let long_line = tool("done", &code("0123456789abcdef"));
    assert_eq!(lines(&long_line, 12), ["bash: ls", "  ⎿ 0123456…"]);
    // The count keeps its room: the line is cut for it.
    let long = tool("done", &code("0123456789abcdefghij\nx"));
    assert_eq!(lines(&long, 24), ["bash: ls", "  ⎿ 0123456…  … +1 lines"]);
    let empty = tool("done", &code("\n"));
    assert_eq!(lines(&empty, 40), ["bash: ls"]);
}

#[test]
fn compact_results_count_the_leaves_left_out() {
    let stack = format!(
        r#"{{"type":"stack","children":[{},{}]}}"#,
        code("1\n2\n3\n4\n5\n6"),
        r#"{"type":"text","tone":"danger","blocks":[{"type":"paragraph","spans":[{"text":"Exit code 1"}]}]}"#
    );
    assert_eq!(
        lines(&tool("failed", &stack), 40),
        ["bash: ls", "  ⎿ 1  … +6 lines"]
    );
}

#[test]
fn a_command_is_highlighted_and_broken_at_its_operators() {
    let command = "cd /work/site && cargo build --release 2>&1 | grep -E 'error|warning' | head -40; echo done";
    let item = |label: Option<&str>, state: &str| {
        let label = label.map_or(String::new(), |l| format!(r#""label":{},"#, json!(l)));
        item_of(
            "tool",
            &format!(r#"{label}"command":{},"state":"{state}","#, json!(command)),
            &code("ok"),
        )
    };
    // Without a label the command is the header, broken after `&&` and `;`; a pipeline stays
    // whole where it fits, else it breaks after its `|` too.
    assert_eq!(
        lines(&item(None, "running"), 80),
        [
            "cd /work/site &&",
            "  cargo build --release 2>&1 | grep -E 'error|warning' | head -40;",
            "  echo done",
            "  ⎿ ok"
        ]
    );
    assert_eq!(
        lines(&item(None, "running"), 40),
        [
            "cd /work/site &&",
            "  cargo build --release 2>&1 |",
            "  grep -E 'error|warning' |",
            "  head -40;…",
            "  ⎿ ok"
        ]
    );
    let rendered = render_item(&item(None, "running"), 40, &Options::default());
    let span = |text: &str| {
        rendered
            .iter()
            .flat_map(|l| l.spans.iter())
            .find(|s| s.content.contains(text))
            .unwrap_or_else(|| panic!("no {text}"))
            .style
    };
    assert_ne!(
        span("cargo").fg,
        span("--release").fg,
        "commands and flags differ"
    );
    assert_eq!(span("'error|warning'").fg, code_fg(Class::String));

    // Once it has run it is muted.
    let done = render_item(&item(None, "done"), 40, &Options::default());
    assert!(
        done[0]
            .spans
            .iter()
            .skip(1)
            .all(|s| s.style.fg == Some(pal().muted)
                && !s.style.add_modifier.contains(Modifier::BOLD)),
        "{:?}",
        done[0]
    );
    // Once it has run, two rows; under a label (a description, or whose call it is) too.
    let done = lines(&item(None, "done"), 60);
    assert_eq!(
        done,
        [
            "cd /work/site &&",
            "  cargo build --release 2>&1 |…",
            "  ⎿ ok"
        ]
    );
    let described = lines(&item(Some("Build the site"), "done"), 60);
    assert_eq!(
        described,
        [
            "Build the site",
            "cd /work/site &&",
            "  cargo build --release 2>&1 |…",
            "  ⎿ ok"
        ]
    );
}

#[test]
fn a_cd_into_the_sessions_directory_goes_without_saying() {
    let in_site = Options {
        cwd: Some("/work/site/".into()),
        ..Options::default()
    };
    let item = |extra: &str| item_of("tool", extra, &code("ok"));
    for command in [
        "cd /work/site && cargo build",
        "cd \"/work/site\"; cargo build",
        "  cd /work/site/ ;\ncargo build",
    ] {
        let it = item(&format!(r#""command":{},"state":"done","#, json!(command)));
        assert_eq!(
            lines_of(&it, 60, &in_site),
            ["cargo build", "  ⎿ ok"],
            "{command}"
        );
    }
    // neo's label carries its command.
    let it = item(r#""label":"bash: cd /work/site && ls","state":"done","#);
    assert_eq!(lines_of(&it, 60, &in_site), ["bash: ls", "  ⎿ ok"]);
    // Somewhere else, or nothing after it, it stays.
    for command in [
        "cd /work/site2 && ls",
        "cd /work/site/src && ls",
        "cd /work/site",
    ] {
        let it = item(&format!(r#""command":{},"state":"done","#, json!(command)));
        assert_eq!(lines_of(&it, 60, &in_site)[0], command);
    }
}

#[test]
fn a_running_command_takes_four_rows_unless_asked_about_or_expanded() {
    let command = (1..=8)
        .map(|i| format!("step-{i}"))
        .collect::<Vec<_>>()
        .join(" && ");
    let item = item_of(
        "tool",
        &format!(
            r#""label":"Run the steps","command":{},"state":"running","#,
            json!(command)
        ),
        r#"{"type":"stack","gap":"s","children":[]}"#,
    );
    let shown = lines(&item, 30);
    assert_eq!(
        shown,
        [
            "Run the steps",
            "step-1 &&",
            "  step-2 &&",
            "  step-3 &&",
            "  step-4 &&…"
        ],
        "and no result line while there is no output"
    );
    let expanded = Options {
        expanded: true,
        ..Options::default()
    };
    assert_eq!(lines_of(&item, 30, &expanded).len(), 9);
}

#[test]
fn a_long_label_is_cut_once_the_call_has_run() {
    let command = "cd /very/long/path && ".repeat(6) + "cargo test";
    let item = |state: &str| {
        item_of(
            "tool",
            &format!(r#""label":{},"state":"{state}","#, json!(command)),
            &code("ok"),
        )
    };
    let done = lines(&item("done"), 40);
    assert_eq!(
        &done[..2],
        [
            "cd /very/long/path && cd /very/long/path",
            "&& cd /very/long/path && cd…"
        ]
    );
    assert_eq!(done[2], "  ⎿ ok");
    assert_eq!(lines(&item("failed"), 40).len(), 3);
    // Running, or with everything expanded, the whole command shows.
    assert!(lines(&item("running"), 40).len() > 3);
    let expanded = Options {
        expanded: true,
        ..Options::default()
    };
    assert!(lines_of(&item("done"), 40, &expanded).len() > 3);
    // A label that fits is left alone.
    assert_eq!(
        lines(&tool("done", &code("ok")), 40),
        ["bash: ls", "  ⎿ ok"]
    );
}

#[test]
fn compact_summaries_by_component() {
    let diff = r#"{"type":"diff","path":"src/a.ts","lines":[
        {"kind":"hunk","text":"@@ -1,3 +1,4 @@"},
        {"kind":"context","text":"x"},
        {"kind":"add","text":"y"},{"kind":"add","text":"z"},{"kind":"add","text":"w"},
        {"kind":"remove","text":"q"}]}"#;
    let it = tool("done", diff);
    assert_eq!(lines(&it, 40), ["bash: ls", "  ⎿ src/a.ts  +3 −1"]);
    let l = &render_item(&it, 40, &Options::default())[1];
    let plus = l.spans.iter().find(|s| s.content == "+3").unwrap();
    let minus = l.spans.iter().find(|s| s.content == "−1").unwrap();
    assert_eq!(plus.style.fg, Some(pal().success));
    assert_eq!(minus.style.fg, Some(pal().danger));

    let kv =
        r#"{"type":"kv","items":[{"key":"Format","value":"PNG"},{"key":"Size","value":"512"}]}"#;
    assert_eq!(
        lines(&tool("done", kv), 40)[1],
        "  ⎿ Format: PNG · Size: 512"
    );
    assert_eq!(lines(&tool("done", kv), 16)[1], "  ⎿ Format: PNG…");

    let image = r#"{"type":"image","src":"asset://i","alt":"a cat","width":4,"height":3}"#;
    assert_eq!(lines(&tool("done", image), 40)[1], "  ⎿ [image a cat]");

    let card = r#"{"type":"card","title":"Cropped","children":[{"type":"link","url":"http://x"}]}"#;
    assert_eq!(
        lines(&tool("done", card), 40),
        ["bash: ls", "  ⎿ Cropped  … +1 lines"]
    );

    let table = r#"{"type":"table","columns":["a","b"],"rows":[["1","2"],["3","4"]]}"#;
    assert_eq!(lines(&tool("done", table), 40)[1], "  ⎿ 2 rows · a, b");

    let long = tool(
        "done",
        &text("one two three four five six seven eight nine ten"),
    );
    assert_eq!(
        lines(&long, 14),
        // Too narrow for the count beside it.
        ["bash: ls", "  ⎿ one two", "    … +5 lines"]
    );
}

#[test]
fn expanded_tools_draw_the_whole_component_indented() {
    let it = tool("done", &code("a\nb\nc\nd\ne"));
    let opts = Options {
        expanded: true,
        ..Options::default()
    };
    let got = lines_of(&it, 20, &opts);
    assert_eq!(got[0], "bash: ls");
    assert_eq!(&got[1..], ["    a", "    b", "    c", "    d", "    e"]);
}

#[test]
fn tools_with_buttons_are_always_drawn_in_full() {
    let row = r#"{"type":"row","gap":"s","children":[
        {"type":"button","label":"Allow","action":"neo.permit","args":{"id":"1","option":"a"},"key":"y"},
        {"type":"button","label":"Deny","action":"neo.permit","args":{"id":"1","option":"b"}}]}"#;
    let it = item_of(
        "tool",
        r#""label":"claude: run rm","state":"pending","#,
        row,
    );
    assert_eq!(lines(&it, 60), ["claude: run rm", "  [y Allow]  [2 Deny]"]);
    // Too narrow for one line: stacked.
    assert_eq!(
        lines(&it, 14),
        ["claude: run rm", "  [y Allow]", "  [2 Deny]"]
    );
}

// ---- permission previews and review cards ----

/// What the daemon's session view builds for a permission request that previews an edit: the
/// diff, then one button per option.
fn permission(diff_lines: usize) -> neo_protocol::ViewItem {
    let mut lines = vec![json!({"kind":"hunk","text":"@@ -1,3 +1,3 @@"})];
    for i in 0..diff_lines {
        lines.push(
            json!({"kind": if i % 2 == 0 {"remove"} else {"add"}, "text": format!("line {i}")}),
        );
    }
    let options = [
        ("allow", "Allow once"),
        ("always", "Always allow"),
        ("reject", "Reject"),
    ];
    let buttons: Vec<_> = options
        .iter()
        .enumerate()
        .map(|(i, (id, name))| {
            json!({"type":"button","label":name,"action":"neo.permit",
                   "args":{"id":"7","option":id},"key":(i + 1).to_string()})
        })
        .collect();
    item(
        &json!({"ref":"p-7","kind":"tool","label":"Edit src/app.ts","state":"pending",
            "component":{"type":"stack","gap":"s","children":[
                {"type":"diff","path":"src/app.ts","lines":lines},
                {"type":"row","gap":"s","children":buttons}]}})
        .to_string(),
    )
}

#[test]
fn a_permission_shows_its_diff_colored_and_the_buttons_below_a_gap() {
    let it = permission(2);
    assert_eq!(
        lines(&it, 60),
        [
            "Edit src/app.ts",
            "  src/app.ts",
            "  @@ -1,3 +1,3 @@",
            "  -line 0",
            "  +line 1",
            "",
            "  [1 Allow once]  [2 Always allow]  [3 Reject]",
        ]
    );
    let rendered = render_item(&it, 60, &Options::default());
    assert_eq!(
        rendered[3].spans.last().unwrap().style.fg,
        Some(pal().danger)
    );
    assert_eq!(
        rendered[4].spans.last().unwrap().style.fg,
        Some(pal().success)
    );
    assert_eq!(
        rendered[2].spans.last().unwrap().style.fg,
        Some(pal().muted)
    );
    let keys: Vec<_> = buttons(&it).iter().map(|b| b.key.clone()).collect();
    assert_eq!(keys, ["1", "2", "3"]);
    // Narrow: long diff lines wrap under their mark and the buttons stack, nothing overflows.
    let narrow = render_item(&it, 14, &Options::default());
    assert_fits(&narrow, 14);
    assert_eq!(plain_lines(&narrow).last().unwrap(), "  [3 Reject]");
}

#[test]
fn diffs_are_highlighted_by_their_grammar_on_the_change_tint() {
    let diff = r#"{"type":"diff","path":"src/m.rs","lines":[
        {"kind":"hunk","text":"@@ -1,2 +1,2 @@"},
        {"kind":"context","text":"fn keep() {}"},
        {"kind":"remove","text":"let a = \"x\";"},
        {"kind":"add","text":"let a = \"y\";"}]}"#;
    let it = tool("done", diff);
    let opts = Options {
        expanded: true,
        ..Options::default()
    };
    let rendered = render_item(&it, 40, &opts);
    let row = |prefix: &str| {
        rendered
            .iter()
            .find(|l| plain_lines(std::slice::from_ref(l))[0].contains(prefix))
            .unwrap_or_else(|| panic!("no {prefix} in {:?}", plain_lines(&rendered)))
    };
    let span = |line: &Line<'static>, text: &str| {
        line.spans
            .iter()
            .find(|s| s.content.contains(text))
            .unwrap_or_else(|| panic!("no {text} in {line:?}"))
            .style
    };
    let colors = neo_renderer_ratatui::syntax::code_colors(&pal()).unwrap();
    let keyword = colors[neo_renderer_ratatui::syntax::Class::Keyword as usize].fg;
    let string = colors[neo_renderer_ratatui::syntax::Class::String as usize].fg;

    let context = row("fn keep");
    assert_eq!(span(context, "fn").fg, keyword);
    assert_eq!(span(context, "fn").bg, None);

    let removed = row("-let");
    assert_eq!(span(removed, "-").fg, Some(pal().danger));
    assert_eq!(span(removed, "let").fg, keyword);
    let tint = span(removed, "let").bg;
    assert!(matches!(tint, Some(Color::Rgb(..))), "{tint:?}");
    // The tint runs to the edge of the item.
    assert_eq!(removed.width(), 40);
    assert_eq!(removed.spans.last().unwrap().style.bg, tint);

    let added = row("+let");
    assert_eq!(span(added, "\"y\"").fg, string);
    assert_ne!(
        span(added, "let").bg,
        tint,
        "additions are tinted green, not red"
    );

    // Without true colors a change is just its color, and code stays plain.
    let plain = Palette {
        fg: Color::Reset,
        bg: Color::Reset,
        ..pal()
    };
    let rendered = render_item(
        &it,
        40,
        &Options {
            theme: plain,
            ..opts
        },
    );
    let added = rendered
        .iter()
        .find(|l| plain_lines(std::slice::from_ref(l))[0].contains("+let"))
        .unwrap();
    assert!(
        added
            .spans
            .iter()
            .filter(|s| !s.content.trim().is_empty())
            .all(|s| s.style.fg == Some(pal().success) && s.style.bg.is_none())
    );
}

#[test]
fn a_long_diff_is_cut_before_the_buttons_unless_expanded() {
    let it = permission(40);
    let opts = Options {
        preview_lines: Some(6),
        ..Options::default()
    };
    let got = lines_of(&it, 60, &opts);
    // The limit counts the path line: title, 6 lines of diff, the count, a gap, the buttons.
    assert_eq!(got.len(), 1 + 6 + 1 + 1 + 1, "{got:#?}");
    assert_eq!(got[2], "  @@ -1,3 +1,3 @@");
    assert_eq!(got[6], "  +line 3");
    assert_eq!(got[7], "  … 36 more lines (expand to see all)");
    assert_eq!(got[8], "");
    assert!(got[9].starts_with("  [1 Allow once]"));
    // Expanded, or without a limit, it is all there.
    let all = |opts: &Options| lines_of(&it, 60, opts).len();
    let full = 1 + 42 + 1 + 1;
    let expanded = Options {
        expanded: true,
        ..opts.clone()
    };
    assert_eq!(all(&expanded), full);
    assert_eq!(all(&Options::default()), full);
    // One line over the limit is shown rather than replaced by a hint that takes the same room.
    assert_eq!(lines_of(&permission(5), 60, &opts).len(), 1 + 7 + 1 + 1);
    assert_eq!(lines_of(&permission(6), 60, &opts).len(), 1 + 6 + 1 + 1 + 1);
}

#[test]
fn the_line_limit_is_shared_by_all_previews_of_an_item() {
    let it = item(
        &json!({"ref":"p","kind":"tool","label":"asks","state":"pending",
            "component":{"type":"stack","children":[
                {"type":"diff","path":"a","lines":[
                    {"kind":"add","text":"1"},{"kind":"add","text":"2"},{"kind":"add","text":"3"}]},
                {"type":"code","code":"a\nb\nc\nd\ne"},
                {"type":"row","children":[{"type":"button","label":"Go","action":"x.y"}]}]}})
        .to_string(),
    );
    let opts = Options {
        preview_lines: Some(5),
        ..Options::default()
    };
    let got = lines_of(&it, 40, &opts);
    // The diff takes 4 (path and 3 lines), the code gets 1 and a count.
    assert_eq!(&got[1..5], ["  a", "  +1", "  +2", "  +3"]);
    assert_eq!(got[5].trim_end(), "    a");
    assert_eq!(got[6], "  … 4 more lines (expand to see all)");
}

#[test]
fn a_review_card_offers_keep_and_reverts_as_buttons() {
    // The daemon's choice item after a turn in edits mode.
    let option = |i: usize, id: &str, name: &str| {
        json!({"type":"button","label":name,"action":"neo.choose",
               "args":{"id":"r1","option":id},"key":(i + 1).to_string()})
    };
    let it = item(
        &json!({"ref":"q-r1","kind":"tool","label":"The agent changed 2 files: keep them?",
            "state":"pending","component":{"type":"stack","gap":"s","children":[
                option(0, "keep", "Keep"),
                option(1, "revert", "Revert all"),
                option(2, "revert:a.ts", "Revert a.ts"),
                option(3, "revert:b.ts", "Revert b.ts")]}})
        .to_string(),
    );
    assert_eq!(
        lines(&it, 50),
        [
            "The agent changed 2 files: keep them?",
            "  [1 Keep]",
            "  [2 Revert all]",
            "  [3 Revert a.ts]",
            "  [4 Revert b.ts]",
        ]
    );
    let b = buttons(&it);
    assert_eq!(b[3].args, Some(json!({"id":"r1","option":"revert:b.ts"})));
    assert_eq!(b[3].action, "neo.choose");
    // The third button is the focused one.
    let focused = render_item(
        &it,
        50,
        &Options {
            focused_button: Some(2),
            ..Options::default()
        },
    );
    let reversed = |l: &Line<'_>| {
        l.spans
            .iter()
            .any(|s| s.style.add_modifier.contains(Modifier::REVERSED))
    };
    assert!(reversed(&focused[3]));
    assert!(!reversed(&focused[2]) && !reversed(&focused[4]));
}

// ---- buttons ----

fn two_buttons() -> neo_protocol::ViewItem {
    item_of(
        "tool",
        r#""label":"t","state":"done","#,
        r#"{"type":"card","title":"C","children":[
            {"type":"text","blocks":[]},
            {"type":"row","children":[
              {"type":"button","label":"Open","action":"photo.open","args":{"id":7}},
              {"type":"button","label":"Copy","action":"copy:asset://x","key":"c"}]},
            {"type":"stack","children":[{"type":"button","label":"Three","action":"a.b"}]}]}"#,
    )
}

#[test]
fn buttons_lists_them_in_drawing_order_with_their_keys() {
    let b = buttons(&two_buttons());
    let summary: Vec<_> = b.iter().map(|b| (&*b.label, &*b.key, &*b.action)).collect();
    assert_eq!(
        summary,
        [
            ("Open", "1", "photo.open"),
            ("Copy", "c", "copy:asset://x"),
            ("Three", "3", "a.b")
        ]
    );
    assert_eq!(b[0].args, Some(json!({"id": 7})));
    assert_eq!(b[1].args, None);
    assert_eq!(b[1].copy_text(), Some("asset://x"));
    assert_eq!(b[0].copy_text(), None);
    assert!(buttons(&item_of("agent", "", &text("hi"))).is_empty());
}

#[test]
fn the_focused_button_is_reversed_and_only_that_one() {
    let it = two_buttons();
    for focus in 0..3 {
        let opts = Options {
            focused_button: Some(focus),
            ..Options::default()
        };
        let rendered = render_item(&it, 60, &opts);
        let reversed: Vec<String> = rendered
            .iter()
            .flat_map(|l| &l.spans)
            .filter(|s| s.style.add_modifier.contains(Modifier::REVERSED))
            .map(|s| s.content.to_string())
            .collect();
        let label = buttons(&it)[focus].label.clone();
        assert!(reversed.concat().contains(&label), "{focus}: {reversed:?}");
        let other: Vec<_> = buttons(&it)
            .iter()
            .filter(|b| b.label != label)
            .map(|b| b.label.clone())
            .collect();
        for o in other {
            assert!(!reversed.concat().contains(&o), "{focus}: {reversed:?}");
        }
        // Focus changes the style, not the text.
        assert_eq!(
            plain(&rendered),
            plain(&render_item(&it, 60, &Options::default()))
        );
    }
    let none = Options {
        focused_button: Some(9),
        ..Options::default()
    };
    assert!(
        render_item(&it, 60, &none)
            .iter()
            .flat_map(|l| &l.spans)
            .all(|s| !s.style.add_modifier.contains(Modifier::REVERSED))
    );
}

#[test]
fn button_text_is_key_and_label() {
    let it = two_buttons();
    assert_eq!(
        lines(&it, 60),
        ["t", "  ╭ C", "  │ [1 Open]  [c Copy]", "  │ [3 Three]"]
    );
}

// ---- components ----

/// A component as an agent item draws it; wrapped in a stack so text gets no bullet.
fn one(component: &str, width: u16) -> Vec<String> {
    let stack = format!(r#"{{"type":"stack","children":[{component}]}}"#);
    lines(&item_of("agent", "", &stack), width)
}

#[test]
fn rows_sit_side_by_side_only_when_they_fit() {
    let row = r#"{"type":"row","children":[{"type":"link","url":"http://a"},{"type":"file","path":"b.txt"}]}"#;
    assert_eq!(one(row, 40), ["http://a  📄 b.txt"]);
    assert_eq!(one(row, 15), ["http://a", "📄 b.txt"]);
    let tall = format!(
        r#"{{"type":"row","children":[{},{}]}}"#,
        text("one two three"),
        text("x")
    );
    assert_eq!(one(&tall, 9), ["one two", "three", "x"]);
}

#[test]
fn stack_gap_l_is_one_blank_line() {
    let stack = format!(
        r#"{{"type":"stack","gap":"l","children":[{},{}]}}"#,
        text("a"),
        text("b")
    );
    assert_eq!(one(&stack, 10), ["a", "", "b"]);
    assert_eq!(one(&stack.replace("\"l\"", "\"m\""), 10), ["a", "b"]);
}

#[test]
fn text_blocks() {
    let blocks = r##"{"type":"text","blocks":[
      {"type":"heading","level":1,"spans":[{"text":"Title"}]},
      {"type":"paragraph","spans":[{"text":"a "},{"text":"b","bold":true},{"text":" "},{"text":"c","code":true},{"text":" "},{"text":"link","href":"http://x.y"}]},
      {"type":"list","ordered":true,"start":9,"items":[
        {"blocks":[{"type":"paragraph","spans":[{"text":"nine"}]}]},
        {"checked":true,"blocks":[{"type":"paragraph","spans":[{"text":"ten"}]},
          {"type":"list","ordered":false,"items":[{"blocks":[{"type":"paragraph","spans":[{"text":"sub"}]}]}]}]}]},
      {"type":"quote","blocks":[{"type":"paragraph","spans":[{"text":"quoted"}]}]},
      {"type":"code_block","code":"x = 1"},
      {"type":"rule"}]}"##;
    assert_eq!(
        one(blocks, 24),
        [
            "Title",
            "",
            "a b c link (http://x.y)",
            "",
            " 9. nine",
            "10. [x] ten",
            "        • sub",
            "",
            "│ quoted",
            "",
            "  x = 1",
            "",
            "────────────────────────"
        ]
    );
}

#[test]
fn code_has_a_background_band_the_width_of_the_view() {
    let rendered = render_item(
        &item_of("agent", "", &code("ab\ncdefghij")),
        8,
        &Options::default(),
    );
    assert_eq!(plain_lines(&rendered), ["  ab", "  cdefgh", "  ij"]);
    for l in &rendered {
        assert_eq!(l.width(), 8);
        assert!(l.spans.iter().all(|s| s.style.bg == Some(pal().surface)));
    }
}

fn plain_lines(lines: &[Line<'_>]) -> Vec<String> {
    neo_renderer_ratatui::plain_text(lines)
}

#[test]
fn diffs_color_additions_and_removals() {
    let diff = r#"{"type":"diff","path":"a.ts","lines":[
        {"kind":"hunk","text":"@@ -1 +1 @@"},{"kind":"remove","text":"old"},
        {"kind":"add","text":"new"},{"kind":"context","text":"same"}]}"#;
    assert_eq!(
        one(diff, 20),
        ["a.ts", "@@ -1 +1 @@", "-old", "+new", " same"]
    );
    let rendered = render_item(&item_of("agent", "", diff), 20, &Options::default());
    assert_eq!(rendered[2].spans[0].style.fg, Some(pal().danger));
    assert_eq!(rendered[3].spans[0].style.fg, Some(pal().success));
    assert_eq!(rendered[1].spans[0].style.fg, Some(pal().muted));
    // A wrapped diff line keeps the mark's column.
    let long = r#"{"type":"diff","lines":[{"kind":"add","text":"abcdefgh"}]}"#;
    assert_eq!(one(long, 5), ["+abcd", " efgh"]);
}

#[test]
fn tables_cut_the_widest_column_first() {
    let table = r#"{"type":"table","columns":["name","size"],
        "rows":[["a-very-long-file-name.png","1 KB"],["b","2 KB"]]}"#;
    assert_eq!(
        one(table, 80),
        [
            "name                      │ size",
            "──────────────────────────┼─────",
            "a-very-long-file-name.png │ 1 KB",
            "b                         │ 2 KB"
        ]
    );
    assert_eq!(
        one(table, 20),
        [
            "name          │ size",
            "──────────────┼─────",
            "a-very-long-… │ 1 KB",
            "b             │ 2 KB"
        ]
    );
    // More columns than cells: clipped, never wider.
    let wide = r#"{"type":"table","columns":["a","b","c","d"],"rows":[["1","2","3","4"]]}"#;
    assert_fits(
        &render_item(&item_of("agent", "", wide), 5, &Options::default()),
        5,
    );
}

#[test]
fn kv_aligns_values_and_wraps_them() {
    let kv = r#"{"type":"kv","items":[{"key":"Format","value":"PNG"},{"key":"Description","value":"a long value here"}]}"#;
    assert_eq!(
        one(kv, 24),
        [
            "Format       PNG",
            "Description  a long",
            "             value here"
        ]
    );
}

#[test]
fn a_kv_value_with_a_tone_is_highlighted_in_the_full_and_the_compact_form() {
    let kv = r#"{"type":"kv","items":[{"key":"Harness","value":"neo","tone":"accent"},{"key":"Model","value":"default"}]}"#;
    let accent = pal().accent;
    let full = render_item(&item_of("agent", "", kv), 40, &Options::default());
    let value = |line: usize, text: &str| {
        full[line]
            .spans
            .iter()
            .find(|s| s.content == text)
            .map(|s| s.style)
            .expect(text)
    };
    assert_eq!(value(0, "neo").fg, Some(accent));
    assert!(value(0, "neo").add_modifier.contains(Modifier::BOLD));
    assert_eq!(value(1, "default").fg, None, "no tone, no highlight");

    let compact = render_item(&tool("done", kv), 60, &Options::default());
    let neo = compact[1]
        .spans
        .iter()
        .find(|s| s.content == "neo")
        .unwrap();
    assert_eq!(neo.style.fg, Some(accent));
    assert_eq!(compact[1].to_string(), "  ⎿ Harness: neo · Model: default");
}

#[test]
fn small_components() {
    let p = |v: &str| format!(r#"{{"type":"progress","value":{v},"label":"Compressing"}}"#);
    assert_eq!(one(&p("0.6"), 60), ["▓▓▓▓▓▓▓▓▓▓▓▓░░░░░░░░ 60% Compressing"]);
    assert_eq!(
        one(&p("0.6"), 30),
        ["▓▓▓▓▓▓▓▓▓▓▓▓░░░░░░░░ 60%", "Compressing"]
    );
    assert_eq!(one(&p("7"), 12)[0], "▓▓▓▓▓▓▓ 100%");
    assert_eq!(
        one(r#"{"type":"progress","value":0}"#, 30),
        ["░░░░░░░░░░░░░░░░░░░░ 0%"]
    );
    assert_eq!(
        one(
            r#"{"type":"image","src":"asset://i","alt":"a cat","width":4,"height":3,"caption":"Cute"}"#,
            40
        ),
        ["[image a cat · 4×3]", "Cute"]
    );
    assert_eq!(
        one(r#"{"type":"image","src":"/tmp/x.png"}"#, 40),
        ["[image /tmp/x.png]"]
    );
    assert_eq!(
        one(r#"{"type":"file","path":"/a/b.rs","name":"b.rs"}"#, 40),
        ["📄 b.rs (/a/b.rs)"]
    );
    assert_eq!(
        one(r#"{"type":"file","path":"/a/b.rs"}"#, 40),
        ["📄 /a/b.rs"]
    );
    assert_eq!(
        one(r#"{"type":"link","url":"http://x","text":"site"}"#, 40),
        ["site (http://x)"]
    );
    assert_eq!(
        one(r#"{"type":"link","url":"http://x","text":"http://x"}"#, 40),
        ["http://x"]
    );
}

#[test]
fn cards_have_a_gutter_and_a_bold_title() {
    let card = format!(
        r#"{{"type":"card","title":"Pinned","tone":"danger","children":[{}]}}"#,
        text("body text")
    );
    assert_eq!(one(&card, 20), ["╭ Pinned", "│ body text"]);
    let rendered = render_item(&item_of("agent", "", &card), 20, &Options::default());
    assert_eq!(rendered[0].spans[0].style.fg, Some(pal().danger));
    assert!(
        rendered[0].spans[1]
            .style
            .add_modifier
            .contains(Modifier::BOLD)
    );
    // Too narrow for a gutter: just the text.
    assert_eq!(one(&card, 5), ["Pinn…", "body", "text"]);
}

// ---- unknown things, hostile text, narrow widths ----

#[test]
fn unknown_components_and_blocks_get_a_dim_placeholder() {
    let it = item_of("agent", "", r#"{"type":"hologram","depth":3}"#);
    assert_eq!(lines(&it, 40), ["[unsupported: hologram]"]);
    let l = &render_item(&it, 40, &Options::default())[0];
    assert_eq!(l.spans[0].style.fg, Some(pal().muted));

    let nested = r#"{"type":"stack","children":[{"type":"hologram"},{"type":"text","blocks":[{"type":"mermaid","src":"x"}]}]}"#;
    assert_eq!(
        one(nested, 40),
        ["[unsupported: hologram]", "[unsupported block]"]
    );
    // In every item kind, compact or not.
    for kind in ["user", "agent", "thought", "tool", "notice"] {
        let it = item_of(kind, "", r#"{"type":"hologram"}"#);
        for expanded in [false, true] {
            let opts = Options {
                expanded,
                show_thoughts: true,
                ..Options::default()
            };
            assert!(!lines_of(&it, 30, &opts).is_empty(), "{kind}");
        }
    }
    assert!(buttons(&it).is_empty());
}

#[test]
fn terminal_escapes_in_text_never_reach_the_output() {
    let it = item_of(
        "tool",
        r#""label":"x\u001b[2Jy","state":"done","#,
        &code("a\u{1b}]0;title\u{7}b\tc"),
    );
    for line in lines(&it, 40) {
        assert!(!line.chars().any(char::is_control), "{line:?}");
    }
    assert_eq!(lines(&it, 40), ["x[2Jy", "  ⎿ a]0;titleb  c"]);
}

#[test]
fn zero_width_draws_nothing() {
    let it = item_of("agent", "", &text("x"));
    assert!(render_item(&it, 0, &Options::default()).is_empty());
}

#[test]
fn cjk_and_emoji_wrap_by_display_width() {
    let cjk = text("日本語のテキストです 👨‍👩‍👧👨‍👩‍👧👨‍👩‍👧");
    let it = item_of("agent", "", &cjk);
    let got = one(&cjk, 8);
    assert_eq!(got, ["日本語の", "テキスト", "です", "👨‍👩‍👧👨‍👩‍👧👨‍👩‍👧"]);
    // Narrower than the three: broken between families, never inside one.
    let narrow = one(&cjk, 4);
    assert_eq!(narrow[narrow.len() - 2..], ["👨‍👩‍👧👨‍👩‍👧", "👨‍👩‍👧"]);
    let rendered = render_item(&it, 8, &Options::default());
    assert_fits(&rendered, 8);
}

#[test]
fn ten_columns_never_overflow() {
    let wide = format!(
        r#"{{"type":"stack","gap":"l","children":[{text},{code},
        {{"type":"card","title":"A very long card title","tone":"accent","children":[{text}]}},
        {{"type":"row","children":[{{"type":"button","label":"A long button label","action":"a.b"}},{{"type":"button","label":"B","action":"a.c"}}]}},
        {{"type":"table","columns":["name","size","modified"],"rows":[["a-long-name.png","1 KB","2026-10-14"]]}},
        {{"type":"kv","items":[{{"key":"a-long-key-here","value":"a-long-value-here-too"}}]}},
        {{"type":"progress","value":0.5,"label":"a long progress label"}},
        {{"type":"diff","path":"src/a/very/long/path.ts","lines":[{{"kind":"add","text":"日本語日本語日本語"}}]}},
        {{"type":"image","src":"asset://some-long-handle","alt":"alt text that is long","width":100,"height":100}},
        {{"type":"file","path":"/some/long/path/name.rs"}},
        {{"type":"link","url":"https://example.com/a/very/long/url","text":"text"}},
        {{"type":"hologram"}}]}}"#,
        text = text("supercalifragilisticexpialidocious 日本語のテキスト 👍🏽👍🏽👍🏽 words"),
        code = code("fn main() { println!(\"hello, world\"); }\n\twith tab"),
    );
    for kind in ["user", "agent", "thought", "tool", "notice"] {
        let it = item_of(
            kind,
            r#""label":"a very long label indeed","state":"done","#,
            &wide,
        );
        for width in [1u16, 2, 3, 6, 10, 11] {
            for opts in [
                Options::default(),
                Options {
                    expanded: true,
                    show_thoughts: true,
                    focused_button: Some(0),
                    ..Options::default()
                },
            ] {
                assert_fits(&render_item(&it, width, &opts), width);
            }
        }
    }
}

#[test]
fn colors_come_from_the_palette() {
    let mut theme: neo_protocol::Theme =
        serde_json::from_str(include_str!("../../../protocol/themes/default.json")).unwrap();
    theme.colors.danger = "#00ff00".into();
    let opts = Options {
        theme: Palette::from_theme(&theme),
        ..Options::default()
    };
    let head = &render_item(&tool("failed", &code("x")), 40, &opts)[0];
    assert_eq!(head.spans[0].style.fg, Some(Color::Rgb(0, 255, 0)));
}

// ---- images ----

fn with_images(rows: u16, status: &[(&str, ImageStatus)]) -> Options {
    Options {
        images: Some(ImageOptions {
            max_rows: rows,
            cell: (10, 20),
            status: Arc::new(
                status
                    .iter()
                    .map(|(s, st)| (s.to_string(), st.clone()))
                    .collect::<HashMap<_, _>>(),
            ),
        }),
        ..Options::default()
    }
}

fn ready(width: u32, height: u32) -> ImageStatus {
    ImageStatus::Ready { width, height }
}

const PNG: &str = r#"{"type":"image","src":"/tmp/cat.png","alt":"a cat"}"#;

#[test]
fn images_off_keep_the_placeholder() {
    let it = item_of("agent", "", PNG);
    assert_eq!(lines(&it, 40), ["[image a cat]"]);
    let out = render_item_images(&it, 40, &Options::default());
    assert!(out.images.is_empty());
    // Status that is passed but not asked for changes nothing.
    assert_eq!(
        lines_of(&it, 40, &with_images(12, &[])),
        ["[image a cat] loading image…"]
    );
}

#[test]
fn cells_keep_the_aspect_ratio_and_never_grow() {
    // 10x20 pixel cells: a 400x400 picture is 40 cols x 20 rows (cells are twice as tall).
    assert_eq!(image_cells(400, 400, (10, 20), 80, 30), (40, 20));
    assert_eq!(image_cells(400, 400, (10, 20), 80, 10), (20, 10));
    assert_eq!(image_cells(400, 400, (10, 20), 16, 30), (16, 8));
    assert_eq!(image_cells(1024, 768, (10, 20), 80, 12), (32, 12));
    assert_eq!(image_cells(10, 10, (10, 20), 80, 12), (1, 1));
    assert_eq!(image_cells(0, 0, (10, 20), 80, 12), (1, 1));
    assert_eq!(image_cells(1000, 10, (10, 20), 40, 12), (40, 1));
}

#[test]
fn a_known_image_reserves_rows_and_a_caption() {
    let it = item_of("agent", "", PNG);
    let opts = with_images(12, &[("/tmp/cat.png", ready(1024, 768))]);
    let out = render_item_images(&it, 80, &opts);
    let text = plain_text(&out.lines);
    assert_eq!(text.len(), 12 + 1);
    assert!(text[..12].iter().all(String::is_empty));
    assert_eq!(text[12], "a cat · 1024×768");
    assert_eq!(
        out.images,
        [ImageSlot {
            line: 0,
            x: 0,
            cols: 32,
            rows: 12,
            src: "/tmp/cat.png".into()
        }]
    );
    // No mark is left in the lines.
    assert!(out.lines.iter().all(|l| l.width() <= 80));
    assert!(
        out.lines
            .iter()
            .flat_map(|l| &l.spans)
            .all(|s| s.content != "\u{2060}")
    );
    // Narrower than the picture: scaled down to the width, fewer rows.
    let out = render_item_images(&it, 16, &opts);
    assert_eq!((out.images[0].cols, out.images[0].rows), (16, 6));
    assert_eq!(plain_text(&out.lines).len(), 6 + 1);
}

#[test]
fn slots_follow_the_indent_of_where_the_image_sits() {
    let it = tool("done", PNG);
    let opts = with_images(30, &[("/tmp/cat.png", ready(200, 100))]);
    // Collapsed: under the elbow.
    let out = render_item_images(&it, 40, &opts);
    let rows = plain_text(&out.lines);
    assert_eq!(rows[0], "bash: ls");
    assert_eq!((out.images[0].line, out.images[0].x), (1, 4));
    assert_eq!(rows[1], "  ⎿");
    assert_eq!(rows.last().unwrap(), "    a cat · 200×100");
    // Expanded: indented 2.
    let open = Options {
        expanded: true,
        ..opts
    };
    let out = render_item_images(&it, 40, &open);
    assert_eq!((out.images[0].line, out.images[0].x), (1, 2));
    // Inside a card the gutter is the indent, and later lines keep their place.
    let card = item_of(
        "agent",
        "",
        &format!(
            r#"{{"type":"card","title":"T","children":[{PNG},{}]}}"#,
            text("after")
        ),
    );
    let out = render_item_images(&card, 40, &open);
    assert_eq!((out.images[0].line, out.images[0].x), (1, 2));
    let text = plain_text(&out.lines);
    assert_eq!(text.last().unwrap(), "│ after");
    assert_eq!(
        text[out.images[0].line + usize::from(out.images[0].rows)],
        "│ a cat · 200×100"
    );
}

#[test]
fn several_images_each_get_a_slot_in_order() {
    let two = format!(
        r#"{{"type":"stack","children":[{PNG},{}, {{"type":"image","src":"asset://b"}}]}}"#,
        text("between")
    );
    let it = item_of("agent", "", &two);
    let opts = with_images(
        12,
        &[
            ("/tmp/cat.png", ready(100, 100)),
            ("asset://b", ready(300, 300)),
        ],
    );
    let out = render_item_images(&it, 40, &opts);
    assert_eq!(out.images.len(), 2);
    assert_eq!(out.images[0].src, "/tmp/cat.png");
    assert_eq!(out.images[1].src, "asset://b");
    assert!(out.images[1].line > out.images[0].line + 5);
    assert_eq!(image_sources(&it), ["/tmp/cat.png", "asset://b"]);
}

#[test]
fn unready_images_are_a_line_of_text() {
    let it = item_of("agent", "", PNG);
    let loading = with_images(12, &[("/tmp/cat.png", ImageStatus::Loading)]);
    assert_eq!(
        lines_of(&it, 40, &loading),
        ["[image a cat] loading image…"]
    );
    let failed = with_images(
        12,
        &[("/tmp/cat.png", ImageStatus::Failed("not found".into()))],
    );
    let out = render_item_images(&it, 40, &failed);
    assert_eq!(plain_text(&out.lines), ["[image a cat · not found]"]);
    assert!(out.images.is_empty());
}

#[test]
fn a_collapsed_tool_result_keeps_the_text_and_the_picture() {
    let both = format!(
        r#"{{"type":"stack","children":[{},{PNG}]}}"#,
        code("Read image cat.png (image/png, 4 bytes).")
    );
    let it = tool("done", &both);
    let opts = with_images(4, &[("/tmp/cat.png", ready(400, 400))]);
    let out = render_item_images(&it, 60, &opts);
    let text = plain_text(&out.lines);
    assert_eq!(text[1], "  ⎿ Read image cat.png (image/png, 4 bytes).");
    assert_eq!(out.images.len(), 1);
    assert_eq!(out.images[0].line, 2);
    assert_eq!(out.images[0].rows, 4);
    for width in [1u16, 2, 5, 12] {
        assert_fits(&render_item(&it, width, &opts), width);
    }
}

fn choice() -> neo_protocol::ViewItem {
    let button = |label: &str, id: &str| {
        format!(
            r#"{{"type":"button","label":"{label}","action":"neo.choose","args":{{"id":"c","option":"{id}"}}}}"#
        )
    };
    let row = |label: &str, id: &str, about: &str| {
        format!(
            r#"{{"type":"row","children":[{},{}]}}"#,
            button(label, id),
            text(about)
        )
    };
    let body = format!(
        r#"{{"type":"stack","children":[{},{},{}]}}"#,
        button("One", "a/1"),
        row("Two", "a/2", "the second"),
        row("Three", "b/3", "the third"),
    );
    tool("pending", &body)
}

#[test]
fn a_buttons_description_is_the_text_after_it_in_its_row() {
    let got: Vec<_> = buttons(&choice())
        .into_iter()
        .map(|b| b.description)
        .collect();
    assert_eq!(
        got,
        [None, Some("the second".into()), Some("the third".into())]
    );
}

#[test]
fn kept_buttons_drag_their_description_along_and_empty_rows_go() {
    let kept = keep_buttons(&choice(), &[2]);
    let got = buttons(&kept);
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].label, "Three");
    assert_eq!(got[0].description.as_deref(), Some("the third"));
    let shown = lines(&kept, 40).join("\n");
    assert!(
        !shown.contains("One") && !shown.contains("second"),
        "{shown}"
    );
    assert!(buttons(&keep_buttons(&choice(), &[])).is_empty());
}

#[test]
fn hidden_keys_leave_the_label_alone() {
    let opts = Options {
        hide_keys: true,
        ..Options::default()
    };
    let got = lines_of(&choice(), 40, &opts).join("\n");
    assert!(got.contains("[One]") && got.contains("[Two]"), "{got}");
    assert!(!got.contains("[1 One]"));
}

// ---- markdown tables and headings ----

fn table_block(header: &[&str], rows: &[&[&str]]) -> String {
    let cells = |row: &[&str]| {
        row.iter()
            .map(|t| json!({"spans": [{"text": t}]}))
            .collect::<Vec<_>>()
    };
    let block = json!({
        "type": "table_block",
        "header": cells(header),
        "rows": rows.iter().map(|r| cells(r)).collect::<Vec<_>>(),
    });
    json!({"type": "text", "blocks": [block]}).to_string()
}

#[test]
fn table_cells_wrap_in_their_column_and_rows_get_rules_then() {
    let it = item_of(
        "notice",
        "",
        &table_block(
            &["Engine", "Status"],
            &[
                &["bing", "ok"],
                &["duckduckgo", "blocked by a captcha page"],
            ],
        ),
    );
    let out = lines(&it, 30);
    assert_eq!(
        out,
        [
            "┌────────────┬───────────────┐",
            "│ Engine     │ Status        │",
            "├────────────┼───────────────┤",
            "│ bing       │ ok            │",
            "├────────────┼───────────────┤",
            "│ duckduckgo │ blocked by a  │",
            "│            │ captcha page  │",
            "└────────────┴───────────────┘",
        ]
    );
}

#[test]
fn a_table_too_narrow_for_its_box_lists_each_row() {
    let it = item_of(
        "notice",
        "",
        &table_block(&["a", "b", "c"], &[&["1", "2", "3"], &["4", "5", "6"]]),
    );
    assert_eq!(
        lines(&it, 9),
        ["a: 1", "b: 2", "c: 3", "", "a: 4", "b: 5", "c: 6"]
    );
    for w in 1..60 {
        assert_fits(&render_item(&it, w, &Options::default()), w);
    }
}

#[test]
fn headings_stand_out_by_level() {
    let heading = |level: u8| {
        let c = json!({"type": "text", "blocks": [
            {"type": "heading", "level": level, "spans": [{"text": "Title"}]}
        ]});
        let rendered = render_item(
            &item_of("agent", "", &c.to_string()),
            40,
            &Options::default(),
        );
        let title = rendered[0].spans.iter().find(|s| s.content == "Title");
        title.expect("the heading's text").style
    };
    let h1 = heading(1);
    assert_eq!(h1.fg, Some(pal().accent));
    // Not underlined: that is how links look.
    assert!(h1.add_modifier.contains(Modifier::BOLD));
    assert!(!h1.add_modifier.contains(Modifier::UNDERLINED));
    assert_eq!(heading(2).fg, Some(pal().accent));
    assert!(!heading(2).add_modifier.contains(Modifier::UNDERLINED));
    assert!(heading(3).add_modifier.contains(Modifier::BOLD));
    assert_eq!(heading(3).fg, None);
}

// ---- shell highlighting ----

#[test]
fn bash_rows_and_shell_code_are_highlighted() {
    let it = tool("done", &text("ok"));
    let rendered = render_item(&it, 60, &Options::default());
    let ls = rendered[0]
        .spans
        .iter()
        .find(|s| s.content == "ls")
        .unwrap();
    assert_eq!(ls.style.fg, code_fg(Class::Function));

    let block = json!({"type": "code", "language": "bash", "code": "echo \"hi there\" | wc -c"});
    let it = item_of("notice", "", &block.to_string());
    let rendered = render_item(&it, 12, &Options::default());
    let all: String = rendered
        .iter()
        .flat_map(|l| l.spans.iter().map(|s| s.content.to_string()))
        .collect();
    // Wrapped to the width, nothing lost, every cell on the code background.
    assert_eq!(all.replace(' ', ""), "echo\"hithere\"|wc-c");
    assert_fits(&rendered, 12);
    let string = rendered
        .iter()
        .flat_map(|l| l.spans.iter())
        .find(|s| s.content.contains("\"hi"))
        .unwrap();
    assert_eq!(string.style.fg, code_fg(Class::String));
    assert_eq!(string.style.bg, Some(pal().surface));

    // Any language its grammar knows, not only shell.
    let block = json!({"type": "code", "language": "rust", "code": "fn main() {}"});
    let it = item_of("notice", "", &block.to_string());
    let rendered = render_item(&it, 40, &Options::default());
    let main = rendered[0]
        .spans
        .iter()
        .find(|s| s.content == "main")
        .unwrap();
    assert_eq!(main.style.fg, code_fg(Class::Function));
}
