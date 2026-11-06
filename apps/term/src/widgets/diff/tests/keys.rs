use super::*;

/// 40 numbered lines with a change after every 12th, as the split view shows them.
fn spread() -> DiffView {
    let mut text = String::new();
    for hunk in 0..3 {
        let at = 1 + hunk * 20;
        text.push_str(&format!("@@ -{at},12 +{at},12 @@ section {hunk}\n"));
        for i in 0..5 {
            text.push_str(&format!(" ctx {hunk}.{i}\n"));
        }
        text.push_str(&format!("-old {hunk}\n+new {hunk}\n"));
        for i in 5..10 {
            text.push_str(&format!(" ctx {hunk}.{i}\n"));
        }
    }
    DiffView::from_text("f", &text)
}

fn ctrl(v: &mut DiffView, c: char) -> bool {
    v.key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL))
}

#[test]
fn vim_keys_counts_and_line_jumps() {
    let mut v = spread();
    let mut t = Terminal::new(TestBackend::new(60, 10)).unwrap(); // unified, 9 rows
    draw_to(&v, &mut t);
    assert!(!v.view.get().split);
    let at = |v: &DiffView| (v.cursor.get(), v.scroll.get());
    ch(&mut v, '5');
    ch(&mut v, 'j');
    assert_eq!(at(&v), (5, 0));
    // Ctrl-E / Ctrl-Y move the view, not the cursor.
    ctrl(&mut v, 'e');
    assert_eq!(at(&v), (5, 1));
    ctrl(&mut v, 'y');
    ctrl(&mut v, 'y');
    assert_eq!(at(&v), (5, 0));
    // Half a page and a page move both.
    ctrl(&mut v, 'd');
    assert_eq!(at(&v), (9, 4));
    ctrl(&mut v, 'u');
    assert_eq!(at(&v), (5, 0));
    ctrl(&mut v, 'f');
    assert_eq!(at(&v), (13, 8));
    ctrl(&mut v, 'b');
    assert_eq!(at(&v), (5, 0));
    // H / M / L: the top, middle and bottom of the view.
    ch(&mut v, 'L');
    assert_eq!(at(&v), (8, 0));
    ch(&mut v, 'M');
    assert_eq!(at(&v), (4, 0));
    ch(&mut v, 'H');
    assert_eq!(at(&v), (0, 0));
    // `}` / `{` between hunks (each 13 unified rows: its `@@` and 12 lines); a count goes
    // further.
    assert_eq!(v.hunk_starts(false), [0, 13, 26]);
    ch(&mut v, '}');
    assert_eq!(at(&v), (13, 13));
    ch(&mut v, 'g');
    ch(&mut v, 'g');
    assert_eq!(v.cursor.get(), 0);
    ch(&mut v, '2');
    ch(&mut v, '}');
    assert_eq!(v.cursor.get(), 26);
    ch(&mut v, '{');
    assert_eq!(v.cursor.get(), 13);
    // `23G`: the row of the file's line 23 (in the second hunk, which starts at line 21).
    ch(&mut v, '2');
    ch(&mut v, '3');
    ch(&mut v, 'G');
    assert_eq!(v.numbers[v.cursor.get()], Some(23));
    // In a line: `$` to its end, `0` back; w / e / b by words, on into the next line.
    ch(&mut v, 'g');
    ch(&mut v, 'g');
    ch(&mut v, 'j');
    assert_eq!(v.row_body(1, false), "ctx 0.0");
    ch(&mut v, '$');
    assert_eq!(v.col, 6);
    ch(&mut v, '0');
    assert_eq!(v.col, 0);
    let mut word = |c: char| {
        ch(&mut v, c);
        (v.cursor.get(), v.col)
    };
    assert_eq!(word('w'), (1, 4));
    assert_eq!(word('w'), (1, 5));
    assert_eq!(word('w'), (1, 6));
    assert_eq!(word('w'), (2, 0));
    assert_eq!(word('e'), (2, 2));
    assert_eq!(word('b'), (2, 0));
    assert_eq!(word('b'), (1, 6));
}

#[test]
fn slash_searches_and_n_goes_on_with_smart_case() {
    let mut v = spread();
    let mut t = Terminal::new(TestBackend::new(60, 10)).unwrap();
    draw_to(&v, &mut t);
    ch(&mut v, '/');
    for c in "new".chars() {
        ch(&mut v, c);
    }
    assert!(draw_to(&v, &mut t).starts_with("/new"));
    press(&mut v, KeyCode::Enter);
    let at = v.scroll.get();
    assert!(v.lines[at].text.contains("new 0"), "{}", v.lines[at].text);
    ch(&mut v, 'n');
    assert!(v.lines[v.scroll.get()].text.contains("new 1"));
    ch(&mut v, 'N');
    assert!(v.lines[v.scroll.get()].text.contains("new 0"));
    // Upper case makes it exact: nothing, and the title says so.
    ch(&mut v, '/');
    for c in "NEW".chars() {
        ch(&mut v, c);
    }
    press(&mut v, KeyCode::Enter);
    assert!(v.lines[v.scroll.get()].text.contains("new 0"));
    assert!(
        draw_to(&v, &mut t)
            .lines()
            .next()
            .unwrap()
            .contains("no match for NEW")
    );
    // Esc while typing gives up without closing.
    ch(&mut v, '/');
    assert!(!press(&mut v, KeyCode::Esc));
    assert!(v.typing.is_none());
}

#[test]
fn leader_e_asks_for_the_file_in_the_editor_at_the_cursor() {
    let dir = std::env::temp_dir();
    let mut v = file_view(&dir, "@@ -10,3 +10,3 @@\n a\n-b\n+B\n c\n");
    // On the `@@`: the first line in view.
    ch(&mut v, ' ');
    ch(&mut v, 'e');
    assert_eq!(v.take_edit(), Some((dir.join("src/a.rs"), Some(10))));
    assert_eq!(v.take_edit(), None);
    // On `+B`, line 11; `\` is the leader too.
    for _ in 0..3 {
        ch(&mut v, 'j');
    }
    ch(&mut v, '\\');
    ch(&mut v, 'e');
    assert_eq!(v.take_edit(), Some((dir.join("src/a.rs"), Some(11))));
    // A pull request's diff has no file here.
    let mut pr = DiffView::from_text("pr", "@@ -1 +1 @@\n-a\n+b\n");
    ch(&mut pr, ' ');
    ch(&mut pr, 'e');
    assert_eq!(pr.take_edit(), None);
    assert_eq!(pr.message.as_deref(), Some("not a file here"));
}

/// The diff of `src/a.rs` in `dir`, as the files panel opens it.
fn file_view(dir: &Path, diff: &str) -> DiffView {
    let mut v = DiffView::blank("src/a.rs".into());
    v.cwd = dir.to_path_buf();
    v.req = Some(DiffRequest {
        rel: "src/a.rs".into(),
        orig: None,
        mark: Mark::Modified,
        in_repo: true,
    });
    v.finish(Ok(diff.into()));
    v
}

fn typed(v: &mut DiffView, text: &str) {
    for c in text.chars() {
        ch(v, c);
    }
}

#[test]
fn c_comments_on_a_line_and_the_prompt_gets_it() {
    let mut v = file_view(&std::env::temp_dir(), "@@ -10,3 +10,3 @@\n a\n-b\n+B\n c\n");
    let mut t = Terminal::new(TestBackend::new(60, 12)).unwrap();
    draw_to(&v, &mut t);
    // On `+B` (line 11): the block opens above it, the file split there.
    for _ in 0..3 {
        ch(&mut v, 'j');
    }
    typed(&mut v, "c");
    typed(&mut v, "use a const");
    let out = draw_to(&v, &mut t);
    let rows: Vec<&str> = out.lines().collect();
    let head = rows
        .iter()
        .position(|r| r.contains("╭ @src/a.rs:11"))
        .expect(&out);
    assert!(rows[head + 1].starts_with("│ use a const"), "{out}");
    assert!(rows[head + 2].contains("Enter Save"), "{out}");
    assert!(rows[head + 3].contains("+B"), "{out}");
    // Keys go to the comment while it is written: j is a letter.
    ch(&mut v, 'j');
    press(&mut v, KeyCode::Backspace);
    press(&mut v, KeyCode::Enter);
    assert_eq!(
        v.take_comment_changes(),
        [(None, Some("@src/a.rs:11 use a const".to_string()))]
    );
    let out = draw_to(&v, &mut t);
    assert!(out.contains("╭ @src/a.rs:11"), "{out}");
    assert!(out.contains("│ use a const"), "{out}");
    assert!(!out.contains("Enter Save"), "{out}");
    // On the commented line again: edits it; the prompt's copy is replaced.
    typed(&mut v, "c");
    assert_eq!(
        v.writing.as_ref().map(|w| w.text.as_str()),
        Some("use a const")
    );
    typed(&mut v, "ant");
    press(&mut v, KeyCode::Enter);
    assert_eq!(
        v.take_comment_changes(),
        [(
            Some("@src/a.rs:11 use a const".to_string()),
            Some("@src/a.rs:11 use a constant".to_string())
        )]
    );
    // Emptied: it goes.
    typed(&mut v, "c");
    ctrl(&mut v, 'u');
    press(&mut v, KeyCode::Enter);
    assert_eq!(
        v.take_comment_changes(),
        [(Some("@src/a.rs:11 use a constant".to_string()), None)]
    );
    assert!(v.comments.is_empty());
    // Esc gives up and leaves nothing.
    typed(&mut v, "cnever mind");
    assert!(!press(&mut v, KeyCode::Esc), "the view stays open");
    assert!(v.writing.is_none());
    assert!(v.take_comment_changes().is_empty());
}

#[test]
fn y_copies_the_selection_or_lines_without_their_signs() {
    let mut v = file_view(
        &std::env::temp_dir(),
        "@@ -1,3 +1,3 @@\n alpha\n-beta\n+gamma one\n",
    );
    let mut t = Terminal::new(TestBackend::new(60, 12)).unwrap();
    draw_to(&v, &mut t);
    // `V` and `y`: whole lines; the cursor goes back to the first.
    typed(&mut v, "jVjjy");
    assert_eq!(v.take_copy().as_deref(), Some("alpha\nbeta\ngamma one"));
    assert!(v.visual.get().is_none());
    assert_eq!(v.cursor.get(), 1);
    // `v` and `y`: from the char it started on to the cursor's, both in.
    typed(&mut v, "lvjjey");
    assert_eq!(v.take_copy().as_deref(), Some("lpha\nbeta\ngamma"));
    assert_eq!((v.cursor.get(), v.col), (1, 1));
    // `yy`: the cursor's line; `2yy` two of them.
    typed(&mut v, "yy");
    assert_eq!(v.take_copy().as_deref(), Some("alpha"));
    typed(&mut v, "2yy");
    assert_eq!(v.take_copy().as_deref(), Some("alpha\nbeta"));
    assert_eq!(v.take_copy(), None);
}

#[test]
fn visual_selections_comment_on_their_lines() {
    let mut v = file_view(
        &std::env::temp_dir(),
        "@@ -10,4 +10,4 @@\n a\n-b\n+B\n c\n d\n",
    );
    let mut t = Terminal::new(TestBackend::new(60, 12)).unwrap();
    draw_to(&v, &mut t);
    // `V` from `a` (10) down to `c` (12): the removed line has no new number.
    ch(&mut v, 'j');
    ch(&mut v, 'V');
    typed(&mut v, "jjj");
    assert_eq!(v.marks(2, false).sel, Some((0, 1)));
    typed(&mut v, "c");
    assert!(v.visual.get().is_none(), "the comment ends the selection");
    typed(&mut v, "why");
    press(&mut v, KeyCode::Enter);
    assert_eq!(
        v.take_comment_changes(),
        [(None, Some("@src/a.rs:10-12 why".to_string()))]
    );
    // `v` selects characters: from the cursor's to the one it moves to.
    ch(&mut v, 'G');
    ch(&mut v, '0');
    ch(&mut v, 'v');
    assert_eq!(v.marks(5, false).sel, Some((0, 1)));
    ch(&mut v, 'k');
    assert_eq!(v.marks(4, false).sel, Some((0, 1)));
    assert_eq!(v.marks(5, false).sel, Some((0, 1)));
    // Esc leaves visual mode first, then the view.
    assert!(!press(&mut v, KeyCode::Esc));
    assert!(v.visual.get().is_none());
    assert!(press(&mut v, KeyCode::Esc));
    // Only the old side has a removed line: its comment counts in the old numbers.
    let mut w = file_view(&std::env::temp_dir(), "@@ -10,2 +10,1 @@\n a\n-b\n");
    draw_to(&w, &mut t);
    typed(&mut w, "jjcgone?");
    press(&mut w, KeyCode::Enter);
    assert!(w.comments[0].old);
    assert_eq!(w.comments[0].start, 11);
}

#[test]
fn jumps_between_hunks_and_pages_in_the_split_view() {
    let mut v = spread();
    let mut t = Terminal::new(TestBackend::new(120, 10)).unwrap(); // 8 rows
    let out = draw_to(&v, &mut t);
    assert!(v.view.get().split);
    assert_eq!(v.aligned.hunks, [0, 12, 24]);
    assert!(
        out.lines()
            .nth(2)
            .unwrap()
            .starts_with("⋯ @@ -1,12 +1,12 @@ section 0")
    );
    assert!(out.lines().next().unwrap().ends_with("lines 1–8 of 36"));
    ch(&mut v, 'n');
    assert_eq!(v.scroll.get(), 12);
    assert!(
        draw_to(&v, &mut t)
            .lines()
            .nth(2)
            .unwrap()
            .contains("section 1")
    );
    ch(&mut v, 'n');
    assert_eq!(v.scroll.get(), 24);
    // The last hunk is further than the view can scroll: clamped, and n stays put.
    ch(&mut v, 'n');
    assert_eq!(v.scroll.get(), 24);
    ch(&mut v, 'p');
    assert_eq!(v.scroll.get(), 12);
    ch(&mut v, 'p');
    assert_eq!(v.scroll.get(), 0);
    ch(&mut v, 'p');
    assert_eq!(v.scroll.get(), 0);
    press(&mut v, KeyCode::PageDown);
    assert_eq!(v.scroll.get(), 7);
    press(&mut v, KeyCode::PageDown);
    press(&mut v, KeyCode::PageDown);
    press(&mut v, KeyCode::PageDown);
    press(&mut v, KeyCode::PageDown);
    // 36 rows, 8 shown.
    assert_eq!(v.scroll.get(), 28);
    assert!(
        draw_to(&v, &mut t)
            .lines()
            .next()
            .unwrap()
            .ends_with("lines 29–36 of 36")
    );
    // The mouse wheel is Ctrl-Y / Ctrl-E: the view moves, the cursor (at the end) with it
    // only as far as it has to.
    assert_eq!(v.cursor.get(), 35);
    ctrl(&mut v, 'y');
    ctrl(&mut v, 'y');
    assert_eq!(v.scroll.get(), 26);
    assert_eq!(v.cursor.get(), 33);
    press(&mut v, KeyCode::End);
    assert_eq!(v.scroll.get(), 28);
    press(&mut v, KeyCode::Home);
    assert_eq!(v.scroll.get(), 0);
}

#[test]
fn scrolls_both_sides_sideways() {
    let mut v = DiffView::from_text("src/app.ts", BIG);
    v.wrap = false;
    let mut t = Terminal::new(TestBackend::new(100, 30)).unwrap();
    let out = draw_to(&v, &mut t);
    let at = |out: &str, needle: &str| out.lines().position(|l| l.contains(needle)).unwrap();
    let long = at(&out, "export const long");
    let cut = out.lines().nth(long).unwrap();
    assert_eq!(cut.matches('…').count(), 2, "{cut}");
    assert!(!cut.contains("sideways"));
    // The cursor on the long line (the new side's): `$` takes both sides to its end.
    ch(&mut v, '/');
    for c in "const long".chars() {
        ch(&mut v, c);
    }
    press(&mut v, KeyCode::Enter);
    ch(&mut v, '$');
    let out = draw_to(&v, &mut t);
    let cut = out.lines().nth(long).unwrap();
    // The end of both lines is in view, and no `…` is left.
    assert_eq!(cut.matches("sideways").count(), 2, "{cut}");
    assert!(cut.trim_end().ends_with("sideways!\";"), "{cut}");
    assert!(v.hscroll > 0);
    // The line numbers stay.
    assert!(out.lines().nth(2).unwrap().starts_with("⋯"));
    // Back a few columns stays where it is; `0` goes back to the left edge.
    let at = v.hscroll;
    press(&mut v, KeyCode::Left);
    assert_eq!(v.hscroll, at);
    ch(&mut v, '0');
    assert_eq!(v.hscroll, 0);
}

#[test]
fn s_switches_between_split_and_unified() {
    let mut v = spread();
    let mut t = Terminal::new(TestBackend::new(120, 10)).unwrap();
    draw_to(&v, &mut t);
    ch(&mut v, 'n');
    ch(&mut v, 'n');
    assert!(v.view.get().split);
    ch(&mut v, 's');
    let out = draw_to(&v, &mut t);
    assert!(!v.view.get().split);
    assert!(!out.contains("working tree"));
    // The top row is the same hunk.
    assert!(
        out.lines()
            .nth(1)
            .unwrap()
            .starts_with("    @@ -41,12 +41,12 @@ section 2"),
        "{out}"
    );
    ch(&mut v, 'p');
    assert!(
        draw_to(&v, &mut t)
            .lines()
            .nth(1)
            .unwrap()
            .starts_with("    @@ -21,12")
    );
    ch(&mut v, 's');
    let out = draw_to(&v, &mut t);
    assert!(v.view.get().split);
    assert!(out.lines().nth(2).unwrap().contains("section 1"));
    // Too narrow for two columns whatever `s` says.
    let narrow = render(&v, 24, 8);
    assert!(!narrow.contains("working tree"));
}

#[test]
fn no_row_is_wider_than_the_overlay() {
    let mut text = String::from("@@ -1,6 +1,6 @@ 日本語のヘッダー\n");
    text.push_str(" 日本語\tの文字列と\tlatin text that goes on and on and on and on\n");
    text.push_str("-\t\told\t日本語の長い行がここにありますそしてさらに続きますよ\n");
    text.push_str("+\t\tnew\t日本語の長い行がここにあります、そしてさらに続きますよ\n");
    text.push_str("\\ No newline at end of file\n");
    text.push_str("+extra 😀 emoji 😀 line with a combining e\u{301} accent and more text\n");
    text.push_str(&format!("-{}\n", "x".repeat(300)));
    text.push_str(" ok\n");
    let mut v = DiffView::from_text("日本語/パス/名前/とても/長い/ファイル名.rs", &text);
    let p = Palette::default();
    let look = Look::new(&p);
    for (force, wrap) in [None, Some(true), Some(false)]
        .into_iter()
        .flat_map(|f| [(f, true), (f, false)])
    {
        v.force = force;
        v.wrap = wrap;
        for w in 1..=140usize {
            for h in [1u16, 3, 9] {
                for hscroll in [0, 3, 21] {
                    v.hscroll = hscroll;
                    // The rows themselves.
                    match v.split_geom(w) {
                        Some(g) => {
                            let (rows, _) = v.split_lines(&g, 0, 12, hscroll, &look);
                            for l in rows {
                                assert_eq!(l.width(), w, "split {w} {hscroll}: {l:?}");
                            }
                        }
                        None => {
                            let (rows, _) = v.unified_lines(w, 0, 12, hscroll, &look);
                            for l in rows {
                                assert!(l.width() <= w, "unified {w} {hscroll}: {l:?}");
                            }
                        }
                    }
                    // And through the terminal, with every key.
                    for key in ['n', 'l', 'j', 'G', 'p', 'h', 's'] {
                        ch(&mut v, key);
                        // (a fresh terminal each time: the test backend leaves half
                        // of a replaced wide character behind)
                        let out = render(&v, w as u16, h);
                        assert_eq!(out.split('\n').count(), h as usize);
                    }
                }
            }
        }
    }
}

#[test]
fn empty_diff_says_so() {
    let v = DiffView::from_text("f", "");
    assert_eq!(v.lines()[0].text, "no changes");
    assert_eq!(v.counts(), (0, 0));
    // A note is shown in the unified view at any width.
    assert!(render(&v, 120, 4).contains("no changes"));
    assert!(!render(&v, 120, 4).contains("HEAD"));
    let failed = DiffView::from_text("f", "").lines().len();
    assert_eq!(failed, 1);
}

#[test]
fn a_rename_without_edits_shows_git_header() {
    let v = DiffView::from_text(
        "x → y",
        "diff --git a/x b/y\nsimilarity index 100%\nrename from x\nrename to y\n",
    );
    let out = render(&v, 120, 6);
    assert!(out.contains("rename from x"));
    assert!(!out.contains("working tree"));
}

#[test]
fn big_diffs_are_cut() {
    let text: String = std::iter::once("@@ -1 +1 @@\n".to_string())
        .chain((0..MAX_LINES + 5).map(|i| format!("+l{i}\n")))
        .collect();
    let v = DiffView::from_text("f", &text);
    assert_eq!(v.lines().len(), MAX_LINES + 1);
    assert!(matches!(v.aligned.rows.last(), Some(Row::Note(n)) if n.contains("cut after")));
    assert!(render(&v, 120, 5).contains("lines 1–3 of"));
}

fn git_run(dir: &Path, args: &[&str]) {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}");
}

#[test]
fn f_reloads_with_the_whole_file_and_keeps_the_place() {
    if std::process::Command::new("git")
        .arg("--version")
        .output()
        .is_err()
    {
        eprintln!("git not found, skipping");
        return;
    }
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("neo-term-diff-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    git_run(&dir, &["init", "-q"]);
    let lines = |mid: &str| -> String {
        (1..=60)
            .map(|i| {
                if i == 30 {
                    format!("{mid}\n")
                } else {
                    format!("line {i}\n")
                }
            })
            .collect()
    };
    std::fs::write(dir.join("f.txt"), lines("thirty")).unwrap();
    git_run(&dir, &["add", "."]);
    git_run(
        &dir,
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "-q",
            "-m",
            "init",
        ],
    );
    std::fs::write(dir.join("f.txt"), lines("THIRTY")).unwrap();
    let req = DiffRequest {
        rel: "f.txt".into(),
        orig: None,
        mark: Mark::Modified,
        in_repo: true,
    };
    let wait = |v: &mut DiffView| {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !v.is_loaded() {
            assert!(std::time::Instant::now() < deadline, "timed out");
            v.poll();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    };
    let mut v = DiffView::open(dir.clone(), req);
    wait(&mut v);
    let mut t = Terminal::new(TestBackend::new(120, 20)).unwrap();
    let out = draw_to(&v, &mut t);
    // Five lines of context each way: 25 to 35.
    assert_eq!(v.aligned.rows.len(), 1 + 11);
    assert!(out.contains("@@ -25,11 +25,11 @@"), "{out}");
    assert!(out.contains("working tree"));
    ch(&mut v, 'f');
    assert!(!v.is_loaded());
    wait(&mut v);
    let out = draw_to(&v, &mut t);
    assert!(out.lines().next().unwrap().contains("full file"));
    assert!(matches!(&v.aligned.rows[0], Row::Hunk(h) if h == "@@ -1,60 +1,60 @@"));
    assert_eq!(v.aligned.rows.len(), 1 + 60);
    // The line at the top (25) is still at the top.
    let first = out.lines().nth(2).unwrap();
    assert!(first.contains("line 25"), "{out}");
    // And back.
    ch(&mut v, 'f');
    wait(&mut v);
    let out = draw_to(&v, &mut t);
    assert!(!out.lines().next().unwrap().contains("full file"));
    assert!(matches!(&v.aligned.rows[0], Row::Hunk(h) if h.starts_with("@@ -25,11")));
    assert!(
        out.lines().nth(2).unwrap().starts_with("⋯ @@ -25,11"),
        "{out}"
    );

    // Opened whole (from the files view), it starts five lines above the change.
    let req = DiffRequest {
        rel: "f.txt".into(),
        orig: None,
        mark: Mark::Modified,
        in_repo: true,
    };
    let mut v = DiffView::open_file(dir.clone(), req);
    wait(&mut v);
    let out = draw_to(&v, &mut t);
    // Unified: one header row.
    let first = out.lines().nth(1).unwrap();
    assert!(first.contains("line 25"), "{out}");
    assert!(out.contains("THIRTY"), "{out}");
    let _ = std::fs::remove_dir_all(&dir);
}
