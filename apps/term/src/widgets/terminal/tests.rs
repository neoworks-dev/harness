
use super::*;
use neo_renderer_ratatui::plain_text;
use serde_json::json;

fn call(id: &str, kind: &str, command: &str) -> ToolCall {
    serde_json::from_value(json!({
        "toolCallId": id, "title": format!("bash: {command}"), "kind": kind,
        "status": "in_progress", "rawInput": {"command": command}
    }))
    .unwrap()
}

#[test]
fn the_agents_description_goes_in_front_of_the_command() {
    let mut t = TerminalState::default();
    let mut c = call("1", "execute", "cargo build");
    c.raw_input = Some(json!({"command": "cargo build", "description": "Build\n  term"}));
    t.on_call(&c);
    assert_eq!(
        shown(&t, 40, 10),
        ["● Build term  cargo build        running"]
    );
}

fn update(id: &str, status: Option<&str>, text: &str, exit: Option<i64>) -> ToolCallUpdate {
    let mut v = json!({"toolCallId": id, "content": [{"type": "text", "text": text}]});
    if let Some(s) = status {
        v["status"] = json!(s);
    }
    if let Some(code) = exit {
        v["rawOutput"] = json!({"exitCode": code});
    }
    serde_json::from_value(v).unwrap()
}

fn shown(t: &TerminalState, w: usize, h: usize) -> Vec<String> {
    plain_text(&t.render(w, h, Some("/w/site"), &Palette::default()))
}

#[test]
fn a_cd_home_is_left_out_and_a_long_command_cut_once_it_ran() {
    let mut t = TerminalState::default();
    let mut c = call(
        "1",
        "execute",
        "cd /w/site && gh issue create --body 'a\nb\nc\nd\ne'",
    );
    c.status = ToolCallStatus::Pending;
    t.on_call(&c);
    assert_eq!(
        shown(&t, 60, 10),
        [
            "? $ gh issue create --body 'a",
            "b",
            "c",
            "d",
            "e'",
            "  waiting for approval"
        ]
    );
    t.on_update(&update("1", Some("completed"), "https://x/42", None), 0);
    assert_eq!(
        shown(&t, 60, 10),
        ["✓ $ gh issue create --body 'a …", "│ https://x/42"]
    );
}

#[test]
fn the_header_says_how_long_it_took_and_how_it_ended() {
    let mut t = TerminalState::default();
    t.on_call_at(&call("1", "execute", "make"), 1_000);
    t.on_update(&update("1", Some("completed"), "", None), 1_800);
    t.on_call_at(&call("2", "execute", "make test"), 2_000);
    t.on_update(&update("2", Some("failed"), "", Some(2)), 74_000);
    assert_eq!(
        shown(&t, 30, 10),
        [
            "✓ $ make                  0.8s",
            "",
            "✗ $ make test  exit 2 · 1m 12s",
        ]
    );
}

#[test]
fn output_keeps_its_colors_and_long_lines_scroll_sideways() {
    let mut t = TerminalState::default();
    t.on_call(&call("1", "execute", "cargo test"));
    t.on_update(
        &update(
            "1",
            Some("failed"),
            "\u{1b}[31mFAILED\u{1b}[0m a_test_with_a_long_name",
            None,
        ),
        0,
    );
    let p = Palette::default();
    let lines = t.render(20, 10, None, &p);
    let row = &lines[1];
    assert_eq!(row.to_string(), "│ FAILED a_test_wit…");
    assert_eq!(row.spans[1].style.fg, Some(p.danger));
    t.key(KeyEvent::from(KeyCode::Right), 10);
    assert_eq!(shown(&t, 20, 10)[1], "│ _test_with_a_long…");
    t.key(KeyEvent::from(KeyCode::Char('0')), 10);
    assert_eq!(shown(&t, 20, 10)[1], "│ FAILED a_test_wit…");
}

#[test]
fn commands_show_with_their_output_as_it_comes() {
    let mut t = TerminalState::default();
    t.on_call(&call("a", "read", "x"));
    assert!(!t.has_any(), "only commands");
    t.on_call(&call("1", "execute", "cargo test\n  --all"));
    t.on_update(&update("1", None, "\u{1b}[32mok\u{1b}[0m 1", None), 0);
    assert_eq!(
        shown(&t, 30, 10),
        ["● $ cargo test …       running", "│ ok 1"]
    );
    t.on_update(&update("1", Some("failed"), "ok 1\nboom\n", Some(101)), 0);
    t.on_call(&call("2", "execute", "ls"));
    t.on_update(&update("2", Some("completed"), "50%\r100%", None), 0);
    assert_eq!(
        shown(&t, 30, 10),
        [
            "✗ $ cargo test …      exit 101",
            "│ ok 1",
            "│ boom",
            "",
            "✓ $ ls",
            "│ 100%"
        ]
    );
}

#[test]
fn scrolled_with_the_chat_the_commands_end_where_it_is() {
    let mut t = TerminalState::default();
    for (id, out) in [("1", "one"), ("2", "two")] {
        t.on_call(&call(id, "execute", "echo"));
        t.on_update(&update(id, Some("completed"), out, None), 0);
    }
    t.set_upto(Some(Some("1".into())));
    assert_eq!(shown(&t, 20, 10), ["✓ $ echo", "│ one"]);
    t.set_upto(Some(None));
    assert!(shown(&t, 20, 10).is_empty(), "no command yet there");
    t.set_upto(None);
    assert_eq!(shown(&t, 20, 10).last().unwrap(), "│ two");
}

#[test]
fn the_end_shows_until_scrolled_back() {
    let mut t = TerminalState::default();
    t.on_call(&call("1", "execute", "seq 20"));
    let numbers: Vec<String> = (1..=20).map(|n| n.to_string()).collect();
    t.on_update(
        &update("1", Some("completed"), &numbers.join("\n"), None),
        0,
    );
    // The command stays on top while its output scrolls under it.
    assert_eq!(shown(&t, 20, 3), ["✓ $ seq 20", "│ 19", "│ 20"]);
    t.key(KeyEvent::from(KeyCode::PageUp), 3);
    assert!(!t.following());
    assert_eq!(shown(&t, 20, 3), ["✓ $ seq 20", "│ 16", "│ 17"]);
    t.key(KeyEvent::from(KeyCode::Char('g')), 3);
    assert_eq!(shown(&t, 20, 3), ["✓ $ seq 20", "│ 1", "│ 2"]);
    t.key(KeyEvent::from(KeyCode::Char('G')), 3);
    assert!(t.following());
    assert!(t.key(KeyEvent::from(KeyCode::Esc), 3));
}

#[test]
fn the_command_on_top_is_the_one_whose_output_shows() {
    let mut t = TerminalState::default();
    t.on_call(&call("1", "execute", "seq 5"));
    t.on_update(&update("1", Some("completed"), "1\n2\n3\n4\n5", None), 0);
    t.on_call(&call("2", "execute", "seq 2"));
    t.on_update(&update("2", Some("completed"), "1\n2", None), 0);
    // On the blank row between them, nothing is pinned: the next command starts under it.
    assert_eq!(shown(&t, 10, 4), ["", "✓ $ seq 2", "│ 1", "│ 2"]);
    t.key(KeyEvent::from(KeyCode::Up), 4);
    t.key(KeyEvent::from(KeyCode::Up), 4);
    assert_eq!(shown(&t, 10, 4), ["✓ $ seq 5", "│ 5", "", "✓ $ seq 2"]);
}

#[test]
fn long_lines_are_cut_and_guests_are_named() {
    let mut t = TerminalState::default();
    let mut c = call("1", "execute", "echo");
    c.agent = Some("claude".into());
    t.on_call(&c);
    t.on_update(
        &update("1", Some("completed"), "```\nabcdefghij\n```", None),
        0,
    );
    // The output and the command that ran are cut to a row each.
    assert_eq!(shown(&t, 8, 10), ["✓ claud…", "│ abcde…"]);
    assert_eq!(shown(&t, 20, 10), ["✓ claude $ echo", "│ abcdefghij"]);
}

/// Claude Code's result for a command it runs in the background.
fn backgrounded(path: &std::path::Path) -> String {
    format!(
        "Command running in background with ID: b1. Output is being written to: {}. You \
             will be notified when it completes.",
        path.display()
    )
}

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("neo-term-jobs-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

#[test]
fn a_background_commands_file_is_tailed_until_it_ends() {
    let path = temp("live.output");
    std::fs::write(&path, "tick 1\n").unwrap();
    let mut t = TerminalState::default();
    t.on_call(&call("1", "execute", "for i in 1 2; do echo tick $i; done"));
    t.on_update(
        &update("1", Some("completed"), &backgrounded(&path), None),
        1_000,
    );
    assert!(t.jobs(1_000).is_empty(), "not before its file was read");
    assert!(t.poll_jobs(1_250));
    let jobs = t.jobs(1_250);
    assert_eq!(jobs.len(), 1);
    assert_eq!(
        (jobs[0].exit, jobs[0].last.as_deref()),
        (None, Some("tick 1"))
    );
    // Among all commands it shows what its file holds.
    assert_eq!(
        shown(&t, 60, 10),
        [
            "● $ for i in 1 2; do echo tick $i; done              running",
            "│ tick 1"
        ]
    );

    // A half-written line waits for its end.
    let mut f = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap();
    use std::io::Write;
    write!(f, "tick").unwrap();
    assert!(!t.poll_jobs(1_500));
    write!(f, " 2\n\n[exited with code 3]\n").unwrap();
    assert!(t.poll_jobs(1_750));
    let jobs = t.jobs(1_750);
    assert_eq!((jobs[0].exit, jobs[0].ended_ms), (Some(3), Some(1_750)));
    t.set_view(Some("1".into()));
    assert_eq!(t.view(), Some("1"));
    assert_eq!(
        shown(&t, 60, 10),
        [
            "✗ $ for i in 1 2; do echo tick $i; done        exit 3 · 0.8s",
            "│ tick 1",
            "│ tick 2"
        ]
    );
    // It lingers a moment, then leaves the list.
    assert!(t.jobs(1_750 + JOB_LINGER_MS).is_empty());
    std::fs::remove_file(&path).unwrap();
}

#[cfg(target_os = "linux")]
#[test]
fn stopping_signals_the_group_that_writes_the_file() {
    use std::os::unix::process::CommandExt;
    let path = temp("stop.output");
    let out = std::fs::File::create(&path).unwrap();
    // Like Claude Code: a group of its own, its output into the file.
    let mut child = std::process::Command::new("sh")
        .args(["-c", "echo up; sleep 30"])
        .stdout(out)
        .process_group(0)
        .spawn()
        .unwrap();
    let mut t = TerminalState::default();
    t.on_call(&call("1", "execute", "sleep 30"));
    t.on_update(
        &update("1", Some("completed"), &backgrounded(&path), None),
        0,
    );
    assert_eq!(t.stop_job("1"), Ok(1));
    let status = child.wait().unwrap();
    assert!(!status.success(), "{status:?}");
    assert!(t.stop_job("nope").is_err());
    std::fs::remove_file(&path).unwrap();
}

#[test]
fn a_replayed_background_command_that_ended_is_not_listed() {
    let ended = temp("ended.output");
    std::fs::write(&ended, "ok\n\n[exited with code 0]\n").unwrap();
    let mut t = TerminalState::default();
    t.on_call(&call("1", "execute", "make"));
    t.on_update(
        &update("1", Some("completed"), &backgrounded(&ended), None),
        0,
    );
    // And one whose file was cleaned up.
    t.on_call(&call("2", "execute", "make again"));
    let gone = temp("gone.output");
    t.on_update(
        &update("2", Some("completed"), &backgrounded(&gone), None),
        0,
    );
    t.poll_jobs(60_000);
    assert!(t.jobs(60_000).is_empty());
    // Among all commands it still shows what its file held.
    let all = shown(&t, 200, 20);
    assert!(
        all[0].starts_with("✓ $ make ") && all[0].ends_with("1m 0s"),
        "{all:?}"
    );
    assert_eq!(all[1], "│ ok");
    std::fs::remove_file(&ended).unwrap();
}
