use super::*;
use crate::widgets::testutil::dump;
use ratatui::Terminal;
use ratatui::backend::TestBackend;

const SAMPLE: &str = "diff --git a/src/a.ts b/src/a.ts\nindex 111..222 100644\n--- a/src/a.ts\n+++ b/src/a.ts\n@@ -1,3 +1,3 @@\n keep\n-old line\n+new line\n--- dashes\n\\ No newline at end of file\n";

/// Three hunks: a changed line, an uneven run with a tab, a pure addition, a long line and a
/// file without a final newline.
const BIG: &str = "diff --git a/src/app.ts b/src/app.ts
index 1111111..2222222 100644
--- a/src/app.ts
+++ b/src/app.ts
@@ -1,6 +1,6 @@ imports
 import { readFile } from \"node:fs\";
-import { join } from \"node:path\";
+import { join, resolve } from \"node:path\";
 
 const limit = 10;
 const name = \"app\";
@@ -20,8 +20,9 @@ export function load(path: string) {
 \tconst text = readFile(path);
-\tif (!text) return null;
-\tconst parsed = JSON.parse(text);
-\treturn parsed;
+\tif (!text) {
+\t\treturn null;
+\t}
+\treturn JSON.parse(text);
 }
 
 export const version = 3;
@@ -40,4 +41,7 @@ export function save(path: string, data: unknown) {
 \twriteFile(path, JSON.stringify(data));
+\t// Keep a backup next to the file.
+\tcopyFile(path, path + \".bak\");
+\tlog(\"saved\");
 }
-export const long = \"a very long line that does not fit into a column of the split view, so it is cut and can be scrolled sideways\";
+export const long = \"a very long line that does not fit into a column of the split view, so it is cut and can be scrolled sideways!\";
-last
\\ No newline at end of file
+last line
\\ No newline at end of file
";

fn press(v: &mut DiffView, code: KeyCode) -> bool {
    v.key(KeyEvent::from(code))
}

fn ch(v: &mut DiffView, c: char) -> bool {
    press(v, KeyCode::Char(c))
}

fn draw_to(v: &DiffView, t: &mut Terminal<TestBackend>) -> String {
    t.draw(|f| v.draw(f, f.area(), &Palette::default()))
        .unwrap();
    dump(t.backend().buffer())
}

fn render(v: &DiffView, w: u16, h: u16) -> String {
    draw_to(v, &mut Terminal::new(TestBackend::new(w, h)).unwrap())
}

/// Rows as `old | new`: line number, sign, text; `∅` for a blank side, `¬` after a line
/// without a final newline.
fn show(a: &Aligned) -> Vec<String> {
    let side = |s: &Option<Side>, sign: char| match s {
        Some(s) => format!(
            "{}{sign}{}{}",
            s.no,
            s.text,
            if s.no_eol { " ¬" } else { "" }
        ),
        None => "∅".to_string(),
    };
    a.rows
        .iter()
        .map(|r| match r {
            Row::Hunk(h) => format!("@ {h}"),
            Row::Note(n) => format!("! {n}"),
            Row::Line {
                left,
                right,
                changed,
            } => {
                let (l, r) = if *changed { ('-', '+') } else { (' ', ' ') };
                format!("{} | {}", side(left, l), side(right, r))
            }
        })
        .collect()
}

#[test]
fn classifies_lines_and_counts() {
    let (lines, adds, dels) = parse_diff(SAMPLE);
    let kinds: Vec<Kind> = lines.iter().map(|l| l.kind).collect();
    use Kind::*;
    assert_eq!(
        kinds,
        [
            Header, Header, Header, Header, Hunk, Context, Del, Add, Del, Header
        ]
    );
    assert_eq!((adds, dels), (1, 2));
}

#[test]
fn tabs_expand_and_controls_vanish() {
    let (lines, ..) = parse_diff("@@ -1 +1 @@\n+\tx\x1b[0m\n");
    assert_eq!(lines[1].text, "+   x[0m");
    let a = align("@@ -1 +1 @@\n+\tx\x1b[0m\n");
    assert_eq!(show(&a)[1], "∅ | 1+    x[0m");
}

#[test]
fn aligns_rows() {
    let cases: &[(&str, &str, &[&str])] = &[
        (
            "modify",
            "@@ -1,3 +1,3 @@\n keep\n-old\n+new\n tail\n",
            &[
                "@ @@ -1,3 +1,3 @@",
                "1 keep | 1 keep",
                "2-old | 2+new",
                "3 tail | 3 tail",
            ],
        ),
        (
            "pure add",
            "@@ -0,0 +1,2 @@\n+a\n+b\n",
            &["@ @@ -0,0 +1,2 @@", "∅ | 1+a", "∅ | 2+b"],
        ),
        (
            "pure delete",
            "@@ -3,2 +2,0 @@\n-x\n-y\n",
            &["@ @@ -3,2 +2,0 @@", "3-x | ∅", "4-y | ∅"],
        ),
        (
            "two hunks",
            "@@ -1,2 +1,2 @@ fn a\n c\n-d\n+D\n@@ -10,2 +10,3 @@ fn b\n e\n+f\n g\n",
            &[
                "@ @@ -1,2 +1,2 @@ fn a",
                "1 c | 1 c",
                "2-d | 2+D",
                "@ @@ -10,2 +10,3 @@ fn b",
                "10 e | 10 e",
                "∅ | 11+f",
                "11 g | 12 g",
            ],
        ),
        (
            "more removed than added",
            "@@ -1,3 +1 @@\n-a\n-b\n-c\n+X\n",
            &["@ @@ -1,3 +1 @@", "1-a | 1+X", "2-b | ∅", "3-c | ∅"],
        ),
        (
            "more added than removed",
            "@@ -1,2 +1,4 @@\n-a\n-b\n+1\n+2\n+3\n+4\n z\n",
            &[
                "@ @@ -1,2 +1,4 @@",
                "1-a | 1+1",
                "2-b | 2+2",
                "∅ | 3+3",
                "∅ | 4+4",
                "3 z | 5 z",
            ],
        ),
        (
            "two runs between context",
            "@@ -1,5 +1,5 @@\n-a\n+A\n m\n-b\n-c\n+B\n+C\n",
            &[
                "@ @@ -1,5 +1,5 @@",
                "1-a | 1+A",
                "2 m | 2 m",
                "3-b | 3+B",
                "4-c | 4+C",
            ],
        ),
        (
            "no newline at the end of the old file",
            "@@ -1 +1 @@\n-old\n\\ No newline at end of file\n+new\n",
            &["@ @@ -1 +1 @@", "1-old ¬ | 1+new"],
        ),
        (
            "no newline at the end of the new file",
            "@@ -1 +1 @@\n-old\n+new\n\\ No newline at end of file\n",
            &["@ @@ -1 +1 @@", "1-old | 1+new ¬"],
        ),
        (
            "no newline after context",
            "@@ -1,2 +1,2 @@\n same\n\\ No newline at end of file\n",
            &["@ @@ -1,2 +1,2 @@", "1 same ¬ | 1 same ¬"],
        ),
        (
            "an empty context line",
            "@@ -1,3 +1,3 @@\n a\n\n-b\n+B\n",
            &["@ @@ -1,3 +1,3 @@", "1 a | 1 a", "2  | 2 ", "3-b | 3+B"],
        ),
        (
            "a removed line that looks like a header",
            "@@ -1,2 +1,2 @@\n--- dashes\n+++ plus\n",
            &["@ @@ -1,2 +1,2 @@", "1--- dashes | 1+++ plus"],
        ),
    ];
    for (name, diff, want) in cases {
        let full = format!("diff --git a/f b/f\nindex 1..2 100644\n--- a/f\n+++ b/f\n{diff}");
        assert_eq!(show(&align(&full)), *want, "{name}");
    }
}

#[test]
fn aligned_knows_hunks_flags_and_widths() {
    let a = align(BIG);
    assert_eq!(a.hunks.len(), 3);
    assert!(a.hunks.iter().all(|&i| matches!(a.rows[i], Row::Hunk(_))));
    assert_eq!(a.gutter, 3);
    assert!(a.max_w > 100);
    assert!(!a.new_file && !a.deleted);
    let new = align(
        "diff --git a/n b/n\nnew file mode 100644\nindex 0..1\n--- /dev/null\n+++ b/n\n@@ -0,0 +1 @@\n+x\n",
    );
    assert!(new.new_file && !new.deleted);
    let gone = align(
        "diff --git a/n b/n\ndeleted file mode 100644\n--- a/n\n+++ /dev/null\n@@ -1 +0,0 @@\n-x\n",
    );
    assert!(gone.deleted && !gone.new_file);
    // Line numbers beyond 999 widen the gutter.
    assert_eq!(align("@@ -1200,1 +1200,1 @@\n a\n").gutter, 4);
    // A rename without edits has no rows to pair.
    let renamed = align("diff --git a/x b/y\nsimilarity index 100%\nrename from x\nrename to y\n");
    assert!(!renamed.has_lines());
}

#[test]
fn marks_the_changed_part_of_a_pair() {
    let parts = |old: &str, new: &str| {
        let (a, b) = intra(old, new);
        let cut = |s: &str, r: Option<(usize, usize)>| {
            r.map(|(x, y)| s.chars().skip(x).take(y - x).collect::<String>())
        };
        (cut(old, a), cut(new, b))
    };
    let some = |a: &str, b: &str| (Some(a.to_string()), Some(b.to_string()));
    assert_eq!(parts("foo(a)", "foo(b, c)"), some("a", "b, c"));
    assert_eq!(parts("hello world", "hello there"), some("world", "there"));
    // Whole words, not letters.
    assert_eq!(
        parts("x = getName(1)", "x = getTitle(1)"),
        some("getName", "getTitle")
    );
    // Nothing in common or nothing different: no mark.
    assert_eq!(parts("a b", "x y"), (None, None));
    assert_eq!(parts("same", "same"), (None, None));
    assert_eq!(parts("getName", "getTitle"), (None, None));
    // A pure insertion marks only the new side.
    assert_eq!(parts("abc", "abc d"), (None, Some(" d".to_string())));
    // The pairs of a diff carry the marks; unpaired lines and context do not.
    let a = align("@@ -1,3 +1,2 @@\n let x = foo(a);\n-let y = foo(a);\n-gone\n+let y = foo(b);\n");
    let Row::Line { left, right, .. } = &a.rows[2] else {
        panic!()
    };
    assert_eq!(left.as_ref().unwrap().hl, Some((12, 13)));
    assert_eq!(right.as_ref().unwrap().hl, Some((12, 13)));
    let Row::Line { left, right, .. } = &a.rows[3] else {
        panic!()
    };
    assert_eq!((left.as_ref().unwrap().hl, right.is_none()), (None, true));
}

#[test]
fn long_lines_wrap_so_there_is_nothing_to_scroll_sideways_and_w_cuts_them_instead() {
    let line = "0123456789abcdefghijklmnopqrstuvwxyz";
    let mut v = DiffView::from_text("f", &format!("@@ -1,2 +1,2 @@\n+{line}\n ok\n"));
    let mut t = Terminal::new(TestBackend::new(20, 6)).unwrap();
    draw_to(&v, &mut t);
    assert!(v.wrap);
    ch(&mut v, 'j');
    ch(&mut v, '$');
    assert_eq!(v.hscroll, 0);
    ch(&mut v, '0');
    ch(&mut v, ' ');
    ch(&mut v, 'w');
    assert!(!v.wrap);
}

#[test]
fn wrapped_rows_still_let_the_end_fill_the_screen() {
    let mut text = String::from("@@ -1,20 +1,20 @@\n");
    for i in 0..20 {
        text.push_str(&format!(" {i:02} {}\n", "x".repeat(30)));
    }
    let mut v = DiffView::from_text("f", &text);
    let mut t = Terminal::new(TestBackend::new(24, 9)).unwrap();
    draw_to(&v, &mut t);
    ch(&mut v, 'G');
    let out = draw_to(&v, &mut t);
    // Each line takes two rows: the last four lines fill the 8 rows.
    assert!(out.lines().nth(1).unwrap().contains(" 16 "), "{out}");
    assert!(
        out.lines().last().unwrap().trim_start().starts_with('x'),
        "{out}"
    );
}

#[test]
fn unified_view_scrolls_sideways_with_the_mark_in_place() {
    let mut v = DiffView::from_text("f", "@@ -1 +1 @@\n+0123456789abcdef\n");
    v.wrap = false;
    let mut t = Terminal::new(TestBackend::new(10, 4)).unwrap();
    assert_eq!(draw_to(&v, &mut t).lines().nth(2).unwrap(), "+01234567…");
    // The view follows the cursor sideways, the sign staying put.
    ch(&mut v, 'j');
    for _ in 0..6 {
        ch(&mut v, 'l');
    }
    assert_eq!(draw_to(&v, &mut t).lines().nth(2).unwrap(), "+01234567…");
    ch(&mut v, 'l');
    assert_ne!(draw_to(&v, &mut t).lines().nth(2).unwrap(), "+01234567…");
    // Not past the end.
    ch(&mut v, '$');
    assert_eq!(draw_to(&v, &mut t).lines().nth(2).unwrap(), "+89abcdef");
    ch(&mut v, 'l');
    assert_eq!(draw_to(&v, &mut t).lines().nth(2).unwrap(), "+89abcdef");
    ch(&mut v, '0');
    assert_eq!(draw_to(&v, &mut t).lines().nth(2).unwrap(), "+01234567…");
}

#[test]
fn narrow_terminals_fall_back_to_the_unified_view() {
    let v = DiffView::from_text("src/app.ts", BIG);
    let out = render(&v, 60, 14);
    assert!(out.contains("diff --git"));
    assert!(!out.contains("HEAD"));
    // Just wide enough for the split.
    assert!(render(&v, 90, 6).contains("working tree"));
    assert!(!render(&v, 89, 6).contains("working tree"));
}

#[test]
fn headers_name_both_sides() {
    let new =
        "diff --git a/n b/n\nnew file mode 100644\n--- /dev/null\n+++ b/n\n@@ -0,0 +1 @@\n+x\n";
    let out = render(&DiffView::from_text("n", new), 100, 6);
    assert!(
        out.lines().nth(1).unwrap().starts_with(" (new file)"),
        "{out}"
    );
    assert!(out.contains("│ working tree"));
    let gone =
        "diff --git a/n b/n\ndeleted file mode 100644\n--- a/n\n+++ /dev/null\n@@ -1 +0,0 @@\n-x\n";
    let out = render(&DiffView::from_text("n", gone), 100, 6);
    assert!(out.lines().nth(1).unwrap().contains("│ (deleted)"), "{out}");
    let mut renamed = DiffView::from_text("old.rs → new.rs", "@@ -1 +1 @@\n-a\n+b\n");
    renamed.req = Some(DiffRequest {
        rel: "new.rs".into(),
        orig: Some("old.rs".into()),
        mark: Mark::Renamed,
        in_repo: true,
    });
    let out = render(&renamed, 100, 6);
    assert!(out.lines().next().unwrap().starts_with("old.rs → new.rs"));
    assert!(
        out.lines().nth(1).unwrap().starts_with(" HEAD · old.rs"),
        "{out}"
    );
    assert!(out.contains("│ working tree · new.rs"));
}

impl DiffView {
    /// Waits until rows `from..to` of both views are highlighted.
    fn wait_for_syntax(&mut self, from: usize, to: usize) {
        for lazy in [&mut self.lit_uni, &mut self.lit_split]
            .into_iter()
            .flatten()
        {
            lazy.wait(from, to);
        }
    }
}

#[test]
fn a_huge_file_is_highlighted_where_it_is_looked_at() {
    let n = 200_000;
    let mut text = format!("--- a/big.rs\n+++ b/big.rs\n@@ -1,{n} +1,{n} @@\n");
    for i in 0..n {
        text.push_str(&format!(
            " fn f{i}(x: u32) -> u32 {{ x * {i} + \"s\".len() as u32 }} // {i}\n"
        ));
    }
    let mut v = DiffView::from_text("big.rs", &text);
    assert_eq!(v.lines.len(), n + 3, "nothing cut");
    let mut t = Terminal::new(TestBackend::new(70, 12)).unwrap();
    draw_to(&v, &mut t);
    // The end, far from where any parse is.
    ch(&mut v, 'G');
    draw_to(&v, &mut t);
    let started = std::time::Instant::now();
    v.wait_for_syntax(n - 10, n + 3);
    let took = started.elapsed();
    eprintln!("the end of {n} lines highlighted in {took:?}");
    assert!(took < std::time::Duration::from_secs(5), "{took:?}");
    let lazy = v.lit_uni.as_ref().unwrap();
    assert!(lazy.get(n).unwrap().1.contains(&Class::Keyword));
    assert!(lazy.get(n / 2).is_none(), "the middle was never looked at");
}

#[test]
fn scrolls_clamped_and_closes() {
    let text: String = std::iter::once("@@ -1 +1 @@\n".to_string())
        .chain((0..40).map(|i| format!("+line {i}\n")))
        .collect();
    let mut v = DiffView::from_text("f", &text);
    // Narrow, so unified: 10 rows of text under the title.
    let mut t = Terminal::new(TestBackend::new(30, 11)).unwrap();
    let line = |out: &str, n: usize| out.lines().nth(n).unwrap().to_string();
    draw_to(&v, &mut t); // learns the height
    // Down moves the cursor; the view only once it would leave it.
    assert!(!press(&mut v, KeyCode::Down));
    let out = draw_to(&v, &mut t);
    assert_eq!(v.cursor.get(), 1);
    assert!(line(&out, 0).ends_with("lines 1–10 of 41"));
    for _ in 0..9 {
        press(&mut v, KeyCode::Down);
    }
    let out = draw_to(&v, &mut t);
    assert!(line(&out, 1).starts_with("  1 +line 0"));
    assert!(line(&out, 0).ends_with("lines 2–11 of 41"));
    press(&mut v, KeyCode::Char('G'));
    let out = draw_to(&v, &mut t);
    assert!(out.lines().last().unwrap().starts_with(" 40 +line 39"));
    assert!(line(&out, 0).ends_with("lines 32–41 of 41"));
    press(&mut v, KeyCode::Down);
    assert_eq!(draw_to(&v, &mut t).lines().last().unwrap(), " 40 +line 39");
    press(&mut v, KeyCode::PageUp);
    assert!(
        draw_to(&v, &mut t)
            .lines()
            .last()
            .unwrap()
            .starts_with(" 31 +line 30")
    );
    v.key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
    assert!(
        draw_to(&v, &mut t)
            .lines()
            .last()
            .unwrap()
            .starts_with(" 26 +line 25")
    );
    v.key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL));
    assert!(
        draw_to(&v, &mut t)
            .lines()
            .last()
            .unwrap()
            .starts_with(" 31 +line 30")
    );
    press(&mut v, KeyCode::PageDown);
    assert!(
        draw_to(&v, &mut t)
            .lines()
            .last()
            .unwrap()
            .starts_with(" 40 +line 39")
    );
    ch(&mut v, 'g');
    ch(&mut v, 'g');
    assert!(line(&draw_to(&v, &mut t), 1).starts_with("    @@"));
    ch(&mut v, 'j');
    ch(&mut v, 'j');
    ch(&mut v, 'k');
    assert_eq!(v.cursor.get(), 1);
    assert!(line(&draw_to(&v, &mut t), 1).starts_with("    @@"));
    assert!(press(&mut v, KeyCode::Esc));
    assert!(ch(&mut v, 'q'));
}

/// A request's diff line: `c`ontext, `-`, `+`, with its old and new numbers.
fn req_line(kind: char, text: &str, old: Option<i64>, new: Option<i64>) -> neo_protocol::DiffLine {
    use neo_protocol::DiffLineKind as K;
    let kind = match kind {
        '-' => K::Remove,
        '+' => K::Add,
        '@' => K::Hunk,
        _ => K::Context,
    };
    neo_protocol::DiffLine {
        kind,
        text: text.into(),
        old,
        new,
    }
}

/// A directory of its own with `rel` holding `text` (none: no such file).
fn dir_with(name: &str, rel: &str, text: Option<&str>) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("neo-term-proposed-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    if let Some(text) = text {
        std::fs::write(dir.join(rel), text).unwrap();
    }
    dir
}

#[test]
fn a_proposed_edit_is_shown_in_its_whole_file_written_or_not() {
    let lines = [
        req_line('@', "@@ -2,3 +2,3 @@", None, None),
        req_line(' ', "b", Some(2), Some(2)),
        req_line('-', "c", Some(3), None),
        req_line('+', "C", None, Some(3)),
        req_line(' ', "d", Some(4), Some(4)),
    ];
    let want = "--- a/a.txt\n+++ b/a.txt\n@@ -1,5 +1,5 @@\n a\n b\n-c\n+C\n d\n e\n";
    // Asked about: the file is the old version.
    let dir = dir_with("old", "a.txt", Some("a\nb\nc\nd\ne\n"));
    assert_eq!(proposed_diff(&dir, "a.txt", &lines).unwrap(), want);
    // Review mode wrote it already: the file is the new one.
    let dir = dir_with("new", "a.txt", Some("a\nb\nC\nd\ne\n"));
    assert_eq!(proposed_diff(&dir, "a.txt", &lines).unwrap(), want);
    // Changed since: the request's diff as it is.
    let dir = dir_with("other", "a.txt", Some("zzz\n"));
    let text = proposed_diff(&dir, "a.txt", &lines).unwrap();
    assert!(text.ends_with("@@ -2 +2 @@\n b\n-c\n+C\n d\n"), "{text}");
}

#[test]
fn a_proposed_edit_of_a_snippet_is_found_in_its_file() {
    // Claude Code's edits: the replaced text and its replacement, numbered from their own top.
    let lines = [
        req_line('-', "c", Some(1), None),
        req_line('-', "d", Some(2), None),
        req_line('+', "D", None, Some(1)),
    ];
    let want = "--- a/a.txt\n+++ b/a.txt\n@@ -1,5 +1,4 @@\n a\n b\n-c\n-d\n+D\n e\n";
    let dir = dir_with("snippet-old", "a.txt", Some("a\nb\nc\nd\ne\n"));
    assert_eq!(proposed_diff(&dir, "a.txt", &lines).unwrap(), want);
    let dir = dir_with("snippet-new", "a.txt", Some("a\nb\nD\ne\n"));
    assert_eq!(proposed_diff(&dir, "a.txt", &lines).unwrap(), want);
}

#[test]
fn a_proposed_new_file_and_a_change_without_context() {
    let dir = dir_with("create", "n.txt", None);
    let lines = [
        req_line('+', "x", None, Some(1)),
        req_line('+', "y", None, Some(2)),
    ];
    assert_eq!(
        proposed_diff(&dir, "n.txt", &lines).unwrap(),
        "--- a/n.txt\n+++ b/n.txt\n@@ -1,0 +1,2 @@\n+x\n+y\n"
    );
    // An addition is placed by its number when no context comes before it.
    let dir = dir_with("insert", "a.txt", Some("1\n2\n3\n4\n"));
    let lines = [req_line('+', "new", None, Some(3))];
    assert_eq!(
        proposed_diff(&dir, "a.txt", &lines).unwrap(),
        "--- a/a.txt\n+++ b/a.txt\n@@ -1,4 +1,5 @@\n 1\n 2\n+new\n 3\n 4\n"
    );
    // The view starts on it, with nothing to reload.
    let mut v = DiffView::proposed(dir, "a.txt", &lines);
    let text = render(&v, 60, 8);
    assert!(text.contains("proposed edit"), "{text}");
    ch(&mut v, 'f');
    assert!(render(&v, 60, 8).contains("the edit is shown in its whole file"));
}

// The keys: moving, searching, selecting, commenting, reloading.
mod keys;
