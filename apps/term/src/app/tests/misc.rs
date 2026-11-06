use super::*;

/// Not a test: writes a frame of a busy session as HTML to `$PREVIEW_HTML` (in the theme
/// `$PREVIEW_THEME`, dark by default), to look at the design in a browser.
/// `PREVIEW_HTML=/tmp/term.html cargo test -p neo-term preview -- --ignored`
#[test]
#[ignore]
fn preview() {
    let Some(path) = std::env::var_os("PREVIEW_HTML") else {
        return;
    };
    let mut app = overview();
    if let Ok(theme) = std::env::var("PREVIEW_THEME") {
        type_str(&mut app, &format!("/theme {theme}"));
        app.handle_key(key(KeyCode::Enter));
    }
    app.handle_event(Event::Session(
        serde_json::from_value(json!({"session": "s1", "seq": 9, "ts": NOW, "event": {
        "type": "todos", "items": [
            {"id": "t1", "text": "Make the header sticky", "done": true, "by": "claude"},
            {"id": "t2", "text": "Check the login form on a phone", "done": false}
        ]}}))
        .unwrap(),
    ));
    type_str(&mut app, "and the footer too");
    let (w, h) = (140, 36);
    let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
    terminal.draw(|f| app.draw(f)).unwrap();
    let buf = terminal.backend().buffer();
    let css = |c: ratatui::style::Color, or: &str| match c {
        ratatui::style::Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        _ => or.to_string(),
    };
    let mut html = String::from(
        "<html><body style=\"margin:0;background:#000\"><pre style=\"margin:0;font:15px/1.3 'JetBrains Mono',monospace\">",
    );
    for y in 0..h {
        for x in 0..w {
            let cell = &buf[(x, y)];
            let m = cell.modifier;
            let (mut fg, mut bg) = (css(cell.fg, "#ccc"), css(cell.bg, "#000"));
            if m.contains(Modifier::REVERSED) {
                std::mem::swap(&mut fg, &mut bg);
            }
            let mut style = format!("color:{fg};background:{bg}");
            if m.contains(Modifier::BOLD) {
                style.push_str(";font-weight:bold");
            }
            if m.contains(Modifier::ITALIC) {
                style.push_str(";font-style:italic");
            }
            if m.contains(Modifier::DIM) {
                style.push_str(";opacity:.6");
            }
            if m.contains(Modifier::CROSSED_OUT) {
                style.push_str(";text-decoration:line-through");
            }
            let sym = match cell.symbol() {
                "<" => "&lt;",
                ">" => "&gt;",
                "&" => "&amp;",
                s => s,
            };
            html.push_str(&format!("<span style=\"{style}\">{sym}</span>"));
        }
        html.push('\n');
    }
    html.push_str("</pre></body></html>");
    std::fs::write(path, html).unwrap();
}

#[test]
fn an_expired_cache_is_a_band_over_the_input_box() {
    let mut app = scenario();
    app.cache_cold = true;
    let text = screen_text(&mut app, 100, 24);
    snapshot("cache-cold-100x24", &text);
    assert!(
        text.contains("⚠ cache expired · the next message re-reads"),
        "{text}"
    );
    // Said once: the status line leaves it out.
    assert_eq!(text.matches("cache expired").count(), 1, "{text}");
}

#[test]
fn a_background_command_ending_gets_one_card() {
    let mut app = app();
    let job = |id: &str, ended: i64, exit: i64| crate::widgets::JobRow {
        id: id.into(),
        agent: None,
        command: "cargo test\n  --all".into(),
        started_ms: NOW - 60_000,
        ended_ms: Some(ended),
        exit: Some(exit),
        last: None,
    };
    app.toast_jobs(&[job("j1", NOW, 0), job("j2", NOW, 101)], NOW);
    app.toast_jobs(&[job("j1", NOW, 0)], NOW + 100);
    // One that ended long before it was seen is not news.
    app.toast_jobs(&[job("j3", NOW - 60_000, 0)], NOW);
    let cards: Vec<_> = app
        .toasts
        .list()
        .iter()
        .map(|t| (t.title.as_str(), t.detail.as_str()))
        .collect();
    assert_eq!(
        cards,
        [
            ("Command finished", "cargo test --all"),
            ("Command failed", "exit 101 · cargo test --all"),
        ]
    );
}

#[test]
fn a_chat_opened_with_its_latest_items_pages_in_older_ones() {
    let mut app = app();
    app.handle_event(Event::ViewSnapshot(ViewSnapshot {
        before: Some(2),
        agent: None,
        session: "s1".into(),
        seq: 9,
        items: vec![item("a3", "agent", "third")],
    }));
    screen_text(&mut app, 80, 24);
    let fx = app.tick(NOW);
    assert!(
        matches!(&fx[..], [Effect::Page { session, before, .. }] if session == "s1" && before == "a3"),
        "{fx:?}"
    );
    // One at a time.
    assert!(
        !app.tick(NOW)
            .iter()
            .any(|e| matches!(e, Effect::Page { .. }))
    );
    let page: neo_protocol::ViewPage = serde_json::from_value(json!({
        "session": "s1",
        "items": [
            {"ref": "a1", "kind": "agent", "component": {"type": "text", "blocks": [
                {"type": "paragraph", "spans": [{"text": "first"}]}]}},
            {"ref": "a2", "kind": "agent", "component": {"type": "text", "blocks": [
                {"type": "paragraph", "spans": [{"text": "second"}]}]}}
        ]
    }))
    .unwrap();
    app.handle_msg(Msg::Page {
        session: "s1".into(),
        page: Ok(page),
    });
    let s = screen_text(&mut app, 80, 24);
    let at = |t: &str| s.find(t).unwrap_or_else(|| panic!("{t} in {s}"));
    assert!(at("first") < at("second") && at("second") < at("third"));
    // Nothing older is left.
    assert!(
        !app.tick(NOW)
            .iter()
            .any(|e| matches!(e, Effect::Page { .. }))
    );
}

#[test]
fn system_prompt_alone_puts_the_words_there_now_in_the_editor() {
    let mut app = app();
    let settings: neo_protocol::UserSettings = serde_json::from_value(json!({
        "showDontTell": false, "splitDiffs": false, "promptAddition": "Be brief.", "work": "coding"
    }))
    .unwrap();
    app.handle_event(Event::UserSettings(settings));
    type_str(&mut app, "/system-prompt");
    let fx = app.handle_key(key(KeyCode::Enter));
    assert!(fx.is_empty(), "{fx:?}");
    assert_eq!(app.editor_text(), "/system-prompt Be brief.");
    // Sent again (changed or not), it goes to the daemon.
    let fx = app.handle_key(key(KeyCode::Enter));
    assert!(
        matches!(&fx[..], [Effect::Input { text, .. }] if text == "/system-prompt Be brief."),
        "{fx:?}"
    );
}

#[test]
fn the_first_start_asks_what_neo_is_for_once() {
    let settings = |work: Option<&str>| {
        let mut value = json!({"showDontTell": false, "splitDiffs": false});
        if let Some(work) = work {
            value["work"] = json!(work);
        }
        Event::UserSettings(serde_json::from_value(value).unwrap())
    };
    // Chosen before: not asked.
    let mut app = app();
    app.handle_event(settings(Some("computer")));
    assert_ne!(app.overlay, Overlay::Setup);

    // Not yet: the walkthrough. Its picks go to the daemon once the summary is confirmed.
    let mut app = self::app();
    app.handle_event(settings(None));
    assert_eq!(app.overlay, Overlay::Setup);
    app.handle_key(key(KeyCode::Right));
    assert!(app.handle_key(key(KeyCode::Enter)).is_empty());
    assert!(app.handle_key(key(KeyCode::Enter)).is_empty());
    let fx = app.handle_key(key(KeyCode::Enter));
    let sent: Vec<&str> = fx
        .iter()
        .filter_map(|e| match e {
            Effect::Input { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(sent, ["/settings work computer", "/settings show-dont-tell on"]);
    assert_eq!(app.overlay, Overlay::None);
    // The daemon's settings again, before it has the choice: not asked twice.
    app.handle_event(settings(None));
    assert_eq!(app.overlay, Overlay::None);

    // Left on the first start: coding, so it doesn't ask again.
    let mut app = self::app();
    app.handle_event(settings(None));
    let fx = app.handle_key(key(KeyCode::Esc));
    assert!(
        matches!(&fx[..], [Effect::Input { text, .. }] if text == "/settings work coding"),
        "{fx:?}"
    );
    // Done once on this machine, it isn't asked again, whatever the daemon kept.
    let file = std::env::temp_dir().join(format!("neo-term-setup-{}.json", std::process::id()));
    let _ = std::fs::remove_file(&file);
    let mut first = self::app();
    first.prefs = crate::theme::Prefs::new(Some(file.clone()));
    first.handle_event(settings(None));
    first.handle_key(key(KeyCode::Esc));
    let mut next = self::app();
    next.prefs = crate::theme::Prefs::new(Some(file.clone()));
    next.handle_event(settings(None));
    assert_eq!(next.overlay, Overlay::None);
    let _ = std::fs::remove_file(&file);
    // /setup runs it again; left, nothing changes.
    type_str(&mut app, "/setup");
    app.handle_key(key(KeyCode::Enter));
    assert_eq!(app.overlay, Overlay::Setup);
    assert!(app.handle_key(key(KeyCode::Esc)).is_empty());
    assert_eq!(app.overlay, Overlay::None);
}

#[test]
fn a_draft_waits_for_its_chat_to_be_shown_again() {
    let mut app = app();
    app.set_sessions(vec![info("s1", "One", 2), info("s2", "Two", 1)]);
    type_str(&mut app, "half a thought");
    app.begin_session("s2", "/home/me/site");
    assert_eq!(app.editor_text(), "");
    type_str(&mut app, "for two");
    app.begin_session("s1", "/home/me/site");
    assert_eq!(app.editor_text(), "half a thought");
    app.begin_session("s2", "/home/me/site");
    assert_eq!(app.editor_text(), "for two");
}

/// Not a test: how long switching to a long session takes in term, from a dump of its view and
/// events (`sdk-rs/examples/switch_timing.rs <session> <dir>`).
/// `SWITCH_DUMP=/tmp/dir cargo test --release -p neo-term switch_timing -- --ignored --nocapture`
#[test]
#[ignore = "a timing, needs SWITCH_DUMP"]
fn switch_timing() {
    let dir = std::env::var("SWITCH_DUMP").expect("SWITCH_DUMP");
    let t = std::time::Instant::now();
    let mut view: ViewSnapshot =
        serde_json::from_str(&std::fs::read_to_string(format!("{dir}/view.json")).unwrap())
            .unwrap();
    let records: Vec<neo_protocol::SessionEventRecord> =
        serde_json::from_str(&std::fs::read_to_string(format!("{dir}/events.json")).unwrap())
            .unwrap();
    eprintln!("parse dumps: {:?}", t.elapsed());
    let mut app = app();
    view.session = "s1".into();
    let t = std::time::Instant::now();
    app.handle_event(Event::ViewSnapshot(view));
    eprintln!("snapshot: {:?}", t.elapsed());
    let t = std::time::Instant::now();
    let n = records.len();
    for mut r in records {
        r.session = "s1".into();
        app.handle_event(Event::Session(r));
    }
    eprintln!("{n} records: {:?}", t.elapsed());
    for i in 0..3 {
        let t = std::time::Instant::now();
        screen(&mut app, 160, 50);
        eprintln!("frame {i}: {:?}", t.elapsed());
    }
}
