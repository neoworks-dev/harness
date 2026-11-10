use super::*;
use neo_renderer_ratatui::plain_text;
use serde_json::json;

pub fn item(r: &str, kind: &str, text: &str) -> ViewItem {
    serde_json::from_value(json!({
        "ref": r, "kind": kind,
        "component": {"type":"text","blocks":[{"type":"paragraph","spans":[{"text": text}]}]}
    }))
    .unwrap()
}

pub fn prompt(r: &str, state: &str, labels: &[&str]) -> ViewItem {
    let children: Vec<_> = labels
        .iter()
        .map(
            |l| json!({"type":"button","label":l,"action":"neo.permit","args":{"id":r,"option":l}}),
        )
        .collect();
    serde_json::from_value(json!({
        "ref": r, "kind": "tool", "label": "asks", "state": state,
        "component": {"type":"row","children": children}
    }))
    .unwrap()
}

fn snapshot(items: Vec<ViewItem>) -> ViewSnapshot {
    ViewSnapshot {
        before: None,
        agent: None,
        session: "s".into(),
        seq: 1,
        items,
    }
}

fn opts() -> ChatOpts {
    ChatOpts {
        expanded: false,
        show_thoughts: false,
        palette: Palette::default(),
        active: None,
        in_panel: false,
        marker: Style::new(),
        ui: Default::default(),
        icons: false,
        running: false,
        split_diffs: false,
        hide_edits: false,
        dismissed: Vec::new(),
        linked: None,
    }
}

fn many(n: usize) -> Chat {
    let mut c = Chat::new();
    c.apply_snapshot(snapshot(
        (0..n)
            .map(|i| item(&format!("a{i}"), "agent", &format!("line {i}")))
            .collect(),
    ));
    c.layout(40, 10, &opts());
    c
}

fn texts(c: &Chat) -> Vec<String> {
    plain_text(&c.visible())
}

#[test]
fn allowed_questions_and_finished_thoughts_are_left_out() {
    let mut c = Chat::new();
    c.apply_snapshot(snapshot(vec![
        item("u", "user", "go"),
        item("t1", "thought", "hmm"),
        prompt("p-1", "done", &[]),
        prompt("p-2", "failed", &[]),
        item("a", "agent", "ok"),
        item("t2", "thought", "next"),
    ]));
    c.layout(40, 20, &opts());
    let shown = texts(&c);
    // The refused question stays; the allowed one and the earlier thought go; the agent is
    // thinking again at the end.
    assert_eq!(shown.iter().filter(|l| l.contains("asks")).count(), 1);
    assert_eq!(shown.iter().filter(|l| l.contains("thinking")).count(), 1);
    assert_eq!(shown.last().map(String::as_str), Some("∴ thinking"));
}

#[test]
fn the_view_follows_the_tail() {
    let c = many(10);
    assert!(c.following());
    assert_eq!(texts(&c).last().unwrap(), "line 9");
}

fn call(r: &str, label: &str, state: &str) -> ViewItem {
    serde_json::from_value(json!({
        "ref": r, "kind": "tool", "label": label, "state": state,
        "component": {"type":"code","code":"out"}
    }))
    .unwrap()
}

#[test]
fn commands_never_fold_and_say_what_they_were_for() {
    let command = |r: &str, description: &str| -> ViewItem {
        serde_json::from_value(json!({
            "ref": r, "kind": "tool", "command": "cargo test", "description": description,
            "state": "done", "component": {"type":"code","code":"out"}
        }))
        .unwrap()
    };
    let mut c = Chat::new();
    let mut items = vec![
        command("c1", "Run the tests"),
        command("c2", "Lint"),
        command("c3", "Format"),
        item("a1", "agent", "Done."),
    ];
    c.apply_snapshot(snapshot(items.clone()));
    c.layout(80, 20, &opts());
    assert_eq!(texts(&c)[..5], ["Run the tests", "", "Lint", "", "Format"]);

    // Other calls still fold.
    items[..3].clone_from_slice(&[
        call("t1", "read a", "done"),
        call("t2", "read b", "done"),
        call("t3", "search x", "done"),
    ]);
    c.apply_snapshot(snapshot(items.clone()));
    c.layout(80, 20, &opts());
    assert_eq!(texts(&c)[0], "3 tool calls · 2 read, 1 search");
    items[2] = call("t3", "search x", "failed");
    c.apply_snapshot(snapshot(items.clone()));
    c.layout(80, 20, &opts());
    assert_eq!(texts(&c)[0], "3 tool calls · 2 read, 1 search · 1 failed");

    // Calls of one tool only say how many.
    items[2] = call("t3", "read c", "failed");
    c.apply_snapshot(snapshot(items));
    c.layout(80, 20, &opts());
    assert_eq!(texts(&c)[0], "3 read · 1 failed");
}

#[test]
fn a_finished_run_of_calls_folds_into_one_line() {
    let mut c = Chat::new();
    let mut items = vec![
        call("t1", "fetch https://a", "done"),
        call("t2", "bash: ls", "done"),
        call("t3", "fetch https://b", "failed"),
        call("t4", "search the web: x", "done"),
    ];
    c.apply_snapshot(snapshot(items.clone()));
    c.layout(80, 20, &opts());
    assert!(
        texts(&c).contains(&"bash: ls".to_string()),
        "the tail run is still going: shown in full"
    );

    items.push(item("a1", "agent", "Done."));
    c.apply_snapshot(snapshot(items.clone()));
    c.layout(80, 20, &opts());
    assert_eq!(
        texts(&c),
        [
            "4 tool calls · 2 fetch, 1 bash, 1 search · 1 failed",
            "",
            "Done."
        ]
    );
    // Ctrl-O shows every call.
    c.layout(
        80,
        20,
        &ChatOpts {
            expanded: true,
            ..opts()
        },
    );
    assert!(texts(&c).contains(&"bash: ls".to_string()));

    // Two are not worth folding.
    c.apply_snapshot(snapshot(vec![
        items[0].clone(),
        items[1].clone(),
        items[4].clone(),
    ]));
    c.layout(80, 20, &opts());
    assert_eq!(texts(&c)[0], "fetch https://a");
}

fn edit(r: &str, path: &str, added: usize, removed: usize) -> ViewItem {
    let mut lines: Vec<serde_json::Value> = Vec::new();
    lines.extend((0..removed).map(|_| json!({"kind":"remove","text":"old"})));
    lines.extend((0..added).map(|_| json!({"kind":"add","text":"new"})));
    serde_json::from_value(json!({
        "ref": r, "kind": "tool", "label": format!("edit {path}"), "state": "done",
        "component": {"type":"diff","path": path,"lines": lines}
    }))
    .unwrap()
}

#[test]
fn edits_stay_in_sight_and_a_finished_turn_sums_up_its_files() {
    let mut c = Chat::new();
    let items = vec![
        item("u1", "user", "go"),
        call("t1", "read a", "done"),
        edit("e1", "src/a.rs", 3, 1),
        call("t2", "read b", "done"),
        call("t3", "read c", "done"),
        call("t4", "read d", "done"),
        edit("e2", "src/b.rs", 2, 0),
        edit("e3", "src/a.rs", 1, 1),
        item("a1", "agent", "Done."),
    ];
    c.apply_snapshot(snapshot(items.clone()));
    let running = ChatOpts {
        running: true,
        ..opts()
    };
    c.layout(80, 40, &running);
    let shown = texts(&c);
    // The reads around the edits fold; the edits don't.
    assert!(shown.contains(&"3 read".to_string()), "{shown:#?}");
    assert!(shown.contains(&"edit src/a.rs  +3 −1".to_string()));
    assert!(shown.contains(&"edit src/b.rs  +2 −0".to_string()));
    assert!(
        !shown.iter().any(|l| l.contains("changed")),
        "not while it runs"
    );

    c.layout(80, 40, &opts());
    let shown = texts(&c);
    let at = shown
        .iter()
        .position(|l| l == "2 files changed  +6 −2")
        .unwrap();
    assert_eq!(shown[at - 2], "Done.");
    assert_eq!(shown[at + 1], "  src/a.rs  +4 −2");
    assert_eq!(shown[at + 2], "  src/b.rs  +2 −0");

    // A turn the next message ended is summed up even while a later one runs.
    let mut more = items.clone();
    more.push(item("u2", "user", "again"));
    c.apply_snapshot(snapshot(more));
    c.layout(80, 40, &running);
    let shown = texts(&c);
    let at = shown
        .iter()
        .position(|l| l == "2 files changed  +6 −2")
        .unwrap();
    assert_eq!(shown[at + 4], "› again");
}

/// A long session: prose with code, commands with output, edits with diffs.
fn long_session(turns: usize) -> Vec<ViewItem> {
    let code: String = (0..40)
        .map(|i| format!("fn f{i}(x: u32) -> u32 {{ x * {i} }} // note {i}\n"))
        .collect();
    let mut items = Vec::new();
    for t in 0..turns {
        items.push(item(&format!("u{t}"), "user", "do the next thing"));
        items.push(
            serde_json::from_value(json!({
                "ref": format!("a{t}"), "kind": "agent",
                "component": {"type":"text","blocks":[
                    {"type":"paragraph","spans":[{"text":"Here is the plan, with some code."}]},
                    {"type":"code","language":"rust","code": code}]}
            }))
            .unwrap(),
        );
        items.push(
            serde_json::from_value(json!({
                "ref": format!("c{t}"), "kind": "tool", "label": "bash: cargo test",
                "command": "cargo test -p neo-term 2>&1 | tail -40", "state": "done",
                "component": {"type":"code","code": code}
            }))
            .unwrap(),
        );
        let lines: Vec<serde_json::Value> = (0..80)
            .map(|i| {
                json!({"kind": if i % 3 == 0 {"add"} else {"context"},
                                "text": format!("    let x{i} = \"s\".len() + {i};")})
            })
            .collect();
        items.push(
            serde_json::from_value(json!({
                "ref": format!("e{t}"), "kind": "tool", "label": "edit src/a.rs", "state": "done",
                "component": {"type":"diff","path":"src/a.rs","lines": lines}
            }))
            .unwrap(),
        );
    }
    items
}

#[test]
fn code_gets_its_colors_as_it_comes_into_view() {
    let mut c = Chat::new();
    c.apply_snapshot(snapshot(long_session(30)));
    let mut o = opts();
    o.palette.fg = ratatui::style::Color::Rgb(220, 220, 220);
    o.palette.bg = ratatui::style::Color::Rgb(20, 20, 20);
    o.expanded = true;
    c.layout(120, 40, &o);
    // The tail is in view and lit; the top of the conversation isn't yet.
    let lit = |c: &Chat, r: &str| c.entries.iter().find(|e| e.r#ref == r).unwrap().lit;
    assert!(lit(&c, "e29"));
    assert!(!lit(&c, "e0"));
    let plain_lines = c
        .entries
        .iter()
        .find(|e| e.r#ref == "e0")
        .unwrap()
        .lines
        .len();
    c.to_top();
    c.layout(120, 40, &o);
    assert!(lit(&c, "e0"));
    assert_eq!(
        c.entries
            .iter()
            .find(|e| e.r#ref == "e0")
            .unwrap()
            .lines
            .len(),
        plain_lines,
        "the same lines, colored"
    );
}

#[test]
#[ignore = "a timing: cargo test --release -p neo-term ctrl_o_timing -- --ignored --nocapture"]
fn ctrl_o_timing() {
    let mut c = Chat::new();
    c.apply_snapshot(snapshot(long_session(100)));
    let mut true_color = opts();
    true_color.palette.fg = ratatui::style::Color::Rgb(220, 220, 220);
    true_color.palette.bg = ratatui::style::Color::Rgb(20, 20, 20);
    let t = std::time::Instant::now();
    c.layout(120, 40, &true_color);
    eprintln!("first layout: {:?}", t.elapsed());
    let expanded = ChatOpts {
        expanded: true,
        ..true_color.clone()
    };
    let t = std::time::Instant::now();
    c.layout(120, 40, &expanded);
    eprintln!("ctrl-o: {:?} ({} lines)", t.elapsed(), c.total_lines());
    let t = std::time::Instant::now();
    c.layout(120, 40, &true_color);
    eprintln!("ctrl-o back: {:?}", t.elapsed());
    // Each kind of item on its own, expanded.
    let key = CacheKey {
        rev: 0,
        width: 120,
        expanded: true,
        thoughts: false,
        split_diffs: false,
        open_edit: false,
        icons: false,
        focused: None,
        linked: false,
        palette: expanded.palette,
        marker: Style::new(),
        images: None,
        ui: None,
    };
    for kind in ["u", "a", "c", "e"] {
        let t = std::time::Instant::now();
        for it in long_session(100)
            .iter()
            .filter(|i| i.r#ref.starts_with(kind))
        {
            render(it, 120, &key, None, None, true);
        }
        eprintln!("  {kind}: {:?}", t.elapsed());
    }
    let no_path: Vec<ViewItem> = long_session(100)
        .into_iter()
        .filter(|i| i.r#ref.starts_with('e'))
        .map(|mut i| {
            if let neo_protocol::Component::DiffComponent(d) = &mut i.component {
                d.path = None;
            }
            i
        })
        .collect();
    let t = std::time::Instant::now();
    for it in &no_path {
        render(it, 120, &key, None, None, true);
    }
    eprintln!("  e without a grammar: {:?}", t.elapsed());
    let rust = neo_renderer_ratatui::syntax::syntax_for("a.rs").unwrap();
    let t = std::time::Instant::now();
    for _ in 0..100 {
        let mut h = neo_renderer_ratatui::syntax::Highlighter::new(rust);
        for i in 0..80 {
            h.line(&format!("    let x{i} = \"s\".len() + {i};"));
        }
    }
    eprintln!("  syntect alone, 8000 lines: {:?}", t.elapsed());
}

#[test]
fn paths_in_the_session_directory_lose_its_prefix() {
    let mut c = Chat::new().in_dir("/w/site");
    c.apply_snapshot(snapshot(vec![
        item("u1", "user", "go"),
        edit("e1", "/w/site/src/a.rs", 1, 0),
        edit("e2", "/elsewhere/b.rs", 1, 0),
        item("a1", "agent", "Done."),
    ]));
    c.layout(80, 40, &opts());
    let shown = texts(&c);
    for line in ["edit src/a.rs  +1 −0", "  src/a.rs  +1 −0"] {
        assert!(shown.contains(&line.to_string()), "{line}: {shown:#?}");
    }
    assert!(
        shown.contains(&"  /elsewhere/b.rs  +1 −0".to_string()),
        "{shown:#?}"
    );
    // What is opened is still the whole path.
    c.pick(1);
    assert_eq!(c.picked_file(), Some("/elsewhere/b.rs"));
}

#[test]
fn the_mouse_selects_text_across_lines() {
    let mut c = many(3);
    // "line 0", "", "line 1", "", "line 2", all in view.
    assert_eq!(c.selected_text(), None);
    c.select_to(c.at_cell(2, 0), false);
    assert_eq!(c.selection(), None, "a click selects nothing");
    c.select_to(c.at_cell(3, 2), true);
    assert_eq!(c.selected_text().as_deref(), Some("ne 0\n\nline"));
    // Backwards is the same.
    c.select_to(c.at_cell(3, 2), false);
    c.select_to(c.at_cell(2, 0), true);
    assert_eq!(c.selected_text().as_deref(), Some("ne 0\n\nline"));
    c.clear_selection();
    assert_eq!(c.selected_text(), None);
}

#[test]
fn file_lines_are_picked_with_the_keys() {
    let mut c = Chat::new();
    c.apply_snapshot(snapshot(vec![
        item("u1", "user", "go"),
        edit("e1", "src/a.rs", 1, 0),
        edit("e2", "src/b.rs", 1, 0),
        item("a1", "agent", "Done."),
    ]));
    c.layout(80, 40, &opts());
    assert!(c.has_picks());
    // The two edits, the reply, then the two lines of the turn's changes.
    c.pick(-1);
    assert_eq!(c.picked_file(), Some("src/b.rs"));
    c.move_pick(-1);
    assert_eq!(c.picked_file(), Some("src/a.rs"));
    c.move_pick(-1);
    assert_eq!(c.picked_file(), None);
    assert_eq!(c.picked_reply(), Some("a1"));
    c.move_pick(-1);
    assert_eq!(c.picked_file(), Some("src/b.rs"));
    assert_eq!(c.picked_reply(), None);
    c.pick(0);
    assert_eq!(c.picked_file(), Some("src/a.rs"));
    c.move_pick(-1);
    assert_eq!(c.picked_file(), Some("src/a.rs"), "stays at the first");
}

#[test]
fn edits_show_what_they_changed_and_hide_edits_leaves_only_the_newest_open() {
    let mut c = Chat::new();
    c.apply_snapshot(snapshot(vec![
        item("u1", "user", "go"),
        edit("e1", "src/b.rs", 2, 0),
        edit("e2", "src/a.rs", 1, 1),
        item("a1", "agent", "Done."),
    ]));
    c.layout(100, 40, &opts());
    // Every edit is open.
    assert_eq!(texts(&c)[2..5], ["edit src/b.rs  +2 −0", "+new", "+new"]);
    // With hide-edits only the newest one is.
    let hide = ChatOpts {
        hide_edits: true,
        ..opts()
    };
    c.layout(100, 40, &hide);
    assert_eq!(
        texts(&c)[2..7],
        [
            "edit src/b.rs  +2 −0",
            "",
            "edit src/a.rs  +1 −1",
            "-old",
            "+new"
        ]
    );
}

#[test]
fn scrolling_up_pins_the_view_and_new_output_does_not_move_it() {
    let mut c = many(10);
    c.page_up();
    assert!(!c.following());
    let before = texts(&c);
    c.apply_patch(ViewPatch {
        agent: None,
        session: "s".into(),
        seq: 2,
        ops: vec![set(item("new", "agent", "fresh"))],
    });
    c.layout(40, 10, &opts());
    assert_eq!(texts(&c), before, "the view stays put");
    assert!(c.has_unseen());
    c.scroll_down(1000);
    assert!(c.following());
    assert!(!c.has_unseen());
    assert_eq!(texts(&c).last().unwrap(), "fresh");
}

#[test]
fn ctrl_o_leaves_what_a_read_read_out() {
    let long = |r: &str, label: &str| -> ViewItem {
        serde_json::from_value(json!({
            "ref": r, "kind": "tool", "label": label, "state": "done",
            "component": {"type":"code","code":"1\n2\n3\n4\n5\n6\n7\n8"}
        }))
        .unwrap()
    };
    let mut c = Chat::new();
    c.apply_snapshot(snapshot(vec![
        long("c-r", "read a.rs"),
        long("c-f", "fetch https://a"),
    ]));
    c.tool_reads("c-r");
    let expanded = ChatOpts {
        expanded: true,
        ..opts()
    };
    c.layout(80, 40, &expanded);
    let t = texts(&c);
    let fetch = t.iter().position(|l| l == "fetch https://a").unwrap();
    assert!(t[..fetch].len() <= 3, "the read stays its summary: {t:?}");
    assert!(t[fetch..].iter().any(|l| l.trim() == "8"), "{t:?}");
}

#[test]
fn ctrl_o_while_scrolled_up_keeps_the_line_at_the_top() {
    let mut items = vec![
        call("t1", "fetch https://a", "done"),
        call("t2", "fetch https://b", "done"),
        call("t3", "fetch https://c", "done"),
    ];
    items.extend((0..20).map(|i| item(&format!("a{i}"), "agent", &format!("line {i}"))));
    let mut c = Chat::new();
    c.apply_snapshot(snapshot(items));
    c.layout(40, 6, &opts());
    c.scroll_up(1000);
    c.scroll_down(4);
    let before = texts(&c)[0].clone();
    assert!(before.starts_with("line"), "{before}");
    let expanded = ChatOpts {
        expanded: true,
        ..opts()
    };
    c.keep_place();
    c.layout(40, 6, &expanded);
    assert_eq!(
        texts(&c)[0],
        before,
        "the unfolded calls above don't push it"
    );
    c.keep_place();
    c.layout(40, 6, &opts());
    assert_eq!(texts(&c)[0], before);
    assert!(!c.has_unseen());
}

#[test]
fn scrolled_up_the_items_through_the_view_are_known() {
    let mut c = many(10);
    assert_eq!(c.items_through_view(), None, "following: everything");
    c.page_up();
    c.layout(40, 10, &opts());
    let seen = c.items_through_view().unwrap();
    let last = &seen.last().unwrap().r#ref;
    assert!(
        texts(&c)
            .iter()
            .any(|l| *l == format!("line {}", &last[1..]))
    );
    assert!(seen.len() < 10);
}

#[test]
fn page_keys_clamp_at_the_ends() {
    let mut c = many(10);
    for _ in 0..10 {
        c.page_up();
    }
    assert_eq!(c.top(), 0);
    assert_eq!(texts(&c)[0], "line 0");
    c.half_page_down();
    assert_eq!(c.top(), 5);
    c.to_top();
    assert_eq!(c.top(), 0);
    for _ in 0..10 {
        c.page_down();
    }
    assert!(c.following());
}

#[test]
fn short_content_does_not_scroll() {
    let mut c = many(2);
    c.page_up();
    assert!(c.following());
    c.to_top();
    assert!(c.following());
}

#[test]
fn rendering_is_cached_until_an_item_changes() {
    let mut c = many(3);
    let first = Rc::as_ptr(&c.entries[0].lines);
    c.layout(
        40,
        10,
        &ChatOpts {
            expanded: false,
            ..opts()
        },
    );
    assert_eq!(Rc::as_ptr(&c.entries[0].lines), first);
    // A patch to the last item re-renders only that one.
    c.apply_patch(ViewPatch {
        agent: None,
        session: "s".into(),
        seq: 2,
        ops: vec![set(item("a2", "agent", "changed"))],
    });
    c.layout(40, 10, &opts());
    assert_eq!(Rc::as_ptr(&c.entries[0].lines), first);
    assert_eq!(texts(&c).last().unwrap(), "changed");
    // A new width re-renders everything.
    c.layout(30, 10, &opts());
    assert_ne!(Rc::as_ptr(&c.entries[0].lines), first);
}

#[test]
fn prompts_are_found_by_their_buttons() {
    let mut c = Chat::new();
    c.apply_snapshot(snapshot(vec![
        item("a", "agent", "hi"),
        prompt("p1", "done", &["Allow"]),
        prompt("p2", "pending", &["Allow", "Reject"]),
        prompt("p3", "pending", &["Allow"]),
    ]));
    assert_eq!(c.prompt_refs(), ["p1", "p2", "p3"]);
    assert_eq!(c.pending_prompts(), ["p2", "p3"], "oldest first");
    assert_eq!(c.buttons_of("p2").len(), 2);
    c.apply_patch(ViewPatch {
        agent: None,
        session: "s".into(),
        seq: 2,
        ops: vec![set(item("p2", "tool", "allowed"))],
    });
    assert_eq!(c.pending_prompts(), ["p3"]);
    c.apply_patch(ViewPatch {
        agent: None,
        session: "s".into(),
        seq: 3,
        ops: vec![set(item("p3", "tool", "allowed"))],
    });
    assert!(c.pending_prompts().is_empty());
    assert_eq!(c.prompt_refs(), ["p1"]);
}

fn picture(r: &str, src: &str) -> ViewItem {
    serde_json::from_value(json!({
        "ref": r, "kind": "agent",
        "component": {"type":"image","src":src,"alt":"pic"}
    }))
    .unwrap()
}

fn known(sizes: &[(&str, ImageStatus)]) -> Option<ChatImages> {
    Some(ChatImages {
        cell: (10, 20),
        status: Arc::new(
            sizes
                .iter()
                .map(|(s, st)| (s.to_string(), st.clone()))
                .collect(),
        ),
        charts: false,
    })
}

fn ready(width: u32, height: u32) -> ImageStatus {
    ImageStatus::Ready { width, height }
}

/// Three lines of text, a 600x240 picture (the pane's 60 columns, 12 rows), another line.
fn gallery(height: u16) -> Chat {
    let mut c = Chat::new();
    c.apply_snapshot(snapshot(vec![
        item("a", "agent", "before"),
        picture("p", "/tmp/p.png"),
        item("b", "agent", "after"),
    ]));
    c.set_images(known(&[("/tmp/p.png", ready(600, 240))]));
    c.layout(60, height, &opts());
    c
}

#[test]
fn images_are_placeholders_until_they_are_turned_on() {
    let mut c = Chat::new();
    c.apply_snapshot(snapshot(vec![picture("p", "/tmp/p.png")]));
    c.layout(60, 20, &opts());
    assert_eq!(texts(&c), ["[image pic]"]);
    assert!(c.placements().is_empty());
    assert_eq!(c.image_sources().collect::<Vec<_>>(), ["/tmp/p.png"]);
    // On, but the size not known yet.
    c.set_images(known(&[]));
    c.layout(60, 20, &opts());
    assert_eq!(texts(&c), ["[image pic] loading image…"]);
    assert!(c.placements().is_empty());
    // Known: the picture takes a slot.
    c.set_images(known(&[("/tmp/p.png", ready(600, 240))]));
    c.layout(60, 20, &opts());
    assert_eq!(c.placements().len(), 1);
    // And off again.
    c.set_images(None);
    c.layout(60, 20, &opts());
    assert_eq!(texts(&c), ["[image pic]"]);
}

#[test]
fn a_new_image_status_redraws_only_pictures() {
    let mut c = gallery(30);
    // The picture's status changing leaves the text items' lines alone.
    // (Held on to, so a new allocation can't take the same address.)
    let first_again = c.entries[0].lines.clone();
    let pic = c.entries[1].lines.clone();
    c.set_images(known(&[("/tmp/p.png", ready(400, 200))]));
    c.layout(60, 30, &opts());
    assert!(Rc::ptr_eq(&c.entries[0].lines, &first_again));
    assert!(!Rc::ptr_eq(&c.entries[1].lines, &pic));
}

#[test]
fn items_are_found_again_after_patches() {
    let mut c = gallery(30);
    assert_eq!(c.image_sources().count(), 1);
    c.apply_patch(ViewPatch {
        agent: None,
        session: "s".into(),
        seq: 2,
        ops: vec![set(item("p", "agent", "gone"))],
    });
    assert_eq!(c.image_sources().count(), 0);
    c.apply_patch(ViewPatch {
        agent: None,
        session: "s".into(),
        seq: 3,
        ops: vec![set(picture("q", "/tmp/q.png"))],
    });
    assert_eq!(c.image_sources().collect::<Vec<_>>(), ["/tmp/q.png"]);
}

fn set(item: ViewItem) -> neo_protocol::ViewOp {
    neo_protocol::ViewOp::SetViewOp(neo_protocol::SetViewOp {
        item,
        appended: None,
    })
}
