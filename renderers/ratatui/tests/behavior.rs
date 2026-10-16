//! What the renderer promises, one case at a time (the snapshots cover the fixtures).

mod common;

use common::*;
use ratatui::style::{Color, Modifier};
use ratatui::text::Line;
use serde_json::json;
use suite_renderer_ratatui::{Options, Palette, buttons, render_item};

fn pal() -> Palette {
    Palette::default()
}

fn text(s: &str) -> String {
    format!(
        r#"{{"type":"text","blocks":[{{"type":"paragraph","spans":[{{"text":{}}}]}}]}}"#,
        json!(s)
    )
}

fn lines(it: &suite_protocol::ViewItem, width: u16) -> Vec<String> {
    lines_of(it, width, &Options::default())
}

fn tool(state: &str, component: &str) -> suite_protocol::ViewItem {
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
fn user_items_get_a_prompt_and_the_app_dimmed() {
    let it = item_of("user", r#""label":"browser","#, &text("make it smaller"));
    assert_eq!(lines(&it, 40), ["> make it smaller  · browser"]);
    let rendered = render_item(&it, 40, &Options::default());
    let spans = &rendered[0].spans;
    assert_eq!(spans[0].content, ">");
    assert_eq!(spans[0].style.fg, Some(pal().accent));
    assert_eq!(spans.last().unwrap().style.fg, Some(pal().muted));
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
        ["> one two", "  three four", "  five six", "  · browser"]
    );
    let bare = item_of("user", "", &text("hi"));
    assert_eq!(lines(&bare, 14), ["> hi"]);
}

#[test]
fn agent_items_are_plain_text_with_a_header_for_guests() {
    let main = item_of("agent", "", &text("hello there"));
    assert_eq!(lines(&main, 40), ["hello there"]);
    let guest = item_of("agent", r#""label":"claude","#, &text("hello there"));
    assert_eq!(lines(&guest, 40), ["claude", "hello there"]);
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
fn tool_bullets_show_the_state() {
    for (state, color) in [
        ("pending", pal().muted),
        ("running", pal().accent),
        ("done", pal().success),
        ("failed", pal().danger),
    ] {
        let it = tool(state, &code("x"));
        let head = &render_item(&it, 40, &Options::default())[0];
        assert_eq!(head.spans[0].content, "● ");
        assert_eq!(head.spans[0].style.fg, Some(color), "{state}");
        assert_eq!(head.spans[1].content, "bash: ls");
    }
}

#[test]
fn compact_code_shows_the_first_lines_and_how_many_are_cut() {
    let out = tool("done", &code("a\nb\nc\nd\ne\n"));
    assert_eq!(
        lines(&out, 40),
        ["● bash: ls", "  ⎿ a", "    b", "    c", "    … +2 lines"]
    );
    let short = tool("done", &code("a\nb"));
    assert_eq!(lines(&short, 40), ["● bash: ls", "  ⎿ a", "    b"]);
    let long_line = tool("done", &code("0123456789abcdef"));
    assert_eq!(lines(&long_line, 12), ["● bash: ls", "  ⎿ 0123456…"]);
    let empty = tool("done", &code("\n"));
    assert_eq!(lines(&empty, 40), ["● bash: ls"]);
}

#[test]
fn compact_results_keep_the_last_leaf_visible() {
    let stack = format!(
        r#"{{"type":"stack","children":[{},{}]}}"#,
        code("1\n2\n3\n4\n5\n6"),
        r#"{"type":"text","tone":"danger","blocks":[{"type":"paragraph","spans":[{"text":"Exit code 1"}]}]}"#
    );
    assert_eq!(
        lines(&tool("failed", &stack), 40),
        [
            "● bash: ls",
            "  ⎿ 1",
            "    2",
            "    … +4 lines",
            "    Exit code 1"
        ]
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
    assert_eq!(lines(&it, 40), ["● bash: ls", "  ⎿ src/a.ts  +3 −1"]);
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
        ["● bash: ls", "  ⎿ Cropped", "    http://x"]
    );

    let table = r#"{"type":"table","columns":["a","b"],"rows":[["1","2"],["3","4"]]}"#;
    assert_eq!(lines(&tool("done", table), 40)[1], "  ⎿ 2 rows · a, b");

    let long = tool(
        "done",
        &text("one two three four five six seven eight nine ten"),
    );
    assert_eq!(
        lines(&long, 14),
        [
            "● bash: ls",
            "  ⎿ one two",
            "    three four",
            "    five six",
            "    … +3 lines"
        ]
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
    assert_eq!(got[0], "● bash: ls");
    assert_eq!(&got[1..], ["    a", "    b", "    c", "    d", "    e"]);
}

#[test]
fn tools_with_buttons_are_always_drawn_in_full() {
    let row = r#"{"type":"row","gap":"s","children":[
        {"type":"button","label":"Allow","action":"suite.permit","args":{"id":"1","option":"a"},"key":"y"},
        {"type":"button","label":"Deny","action":"suite.permit","args":{"id":"1","option":"b"}}]}"#;
    let it = item_of(
        "tool",
        r#""label":"claude asks: run rm","state":"pending","#,
        row,
    );
    assert_eq!(
        lines(&it, 60),
        ["● claude asks: run rm", "  [y Allow]  [2 Deny]"]
    );
    // Too narrow for one line: stacked.
    assert_eq!(
        lines(&it, 14),
        ["● claude asks:", "  run rm", "  [y Allow]", "  [2 Deny]"]
    );
}

// ---- permission previews and review cards ----

/// What the daemon's session view builds for a permission request that previews an edit: the
/// diff, then one button per option.
fn permission(diff_lines: usize) -> suite_protocol::ViewItem {
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
            json!({"type":"button","label":name,"action":"suite.permit",
                   "args":{"id":"7","option":id},"key":(i + 1).to_string()})
        })
        .collect();
    item(
        &json!({"ref":"p-7","kind":"tool","label":"pi asks: Edit src/app.ts","state":"pending",
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
            "● pi asks: Edit src/app.ts",
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
        json!({"type":"button","label":name,"action":"suite.choose",
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
            "● The agent changed 2 files: keep them?",
            "  [1 Keep]",
            "  [2 Revert all]",
            "  [3 Revert a.ts]",
            "  [4 Revert b.ts]",
        ]
    );
    let b = buttons(&it);
    assert_eq!(b[3].args, Some(json!({"id":"r1","option":"revert:b.ts"})));
    assert_eq!(b[3].action, "suite.choose");
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

fn two_buttons() -> suite_protocol::ViewItem {
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
        ["● t", "  ╭ C", "  │ [1 Open]  [c Copy]", "  │ [3 Three]"]
    );
}

// ---- components ----

fn one(component: &str, width: u16) -> Vec<String> {
    lines(&item_of("agent", "", component), width)
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
    suite_renderer_ratatui::plain_text(lines)
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
    assert_eq!(lines(&it, 40), ["● x[2Jy", "  ⎿ a]0;titleb  c"]);
}

#[test]
fn zero_width_draws_nothing() {
    let it = item_of("agent", "", &text("x"));
    assert!(render_item(&it, 0, &Options::default()).is_empty());
}

#[test]
fn cjk_and_emoji_wrap_by_display_width() {
    let it = item_of("agent", "", &text("日本語のテキストです 👨‍👩‍👧👨‍👩‍👧👨‍👩‍👧"));
    let got = lines(&it, 8);
    assert_eq!(got, ["日本語の", "テキスト", "です", "👨‍👩‍👧👨‍👩‍👧👨‍👩‍👧"]);
    // Narrower than the three: broken between families, never inside one.
    let narrow = lines(&it, 4);
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
    let mut theme: suite_protocol::Theme =
        serde_json::from_str(include_str!("../../../protocol/themes/default.json")).unwrap();
    theme.colors.success = "#00ff00".into();
    let opts = Options {
        theme: Palette::from_theme(&theme),
        ..Options::default()
    };
    let head = &render_item(&tool("done", &code("x")), 40, &opts)[0];
    assert_eq!(head.spans[0].style.fg, Some(Color::Rgb(0, 255, 0)));
}
