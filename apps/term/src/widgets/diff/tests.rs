
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

fn snapshot(name: &str, actual: &str) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/snapshots")
        .join(format!("{name}.txt"));
    if std::env::var_os("UPDATE_SNAPSHOTS").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, format!("{actual}\n")).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&path).unwrap_or_else(|_| {
        panic!("missing snapshot {name} (UPDATE_SNAPSHOTS=1 to write)\n{actual}")
    });
    assert_eq!(actual, expected.trim_end_matches('\n'), "snapshot {name}");
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
fn windows_by_cells() {
    let text = |s: &str, skip, avail| -> String {
        let spans = windowed(
            s.chars().map(|c| (c, 0)),
            width(s),
            skip,
            avail,
            &[Style::new(); 3],
            true,
        );
        let out: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(width(&out), avail, "{s:?} {skip} {avail}");
        out
    };
    assert_eq!(text("hello", 0, 8), "hello   ");
    assert_eq!(text("hello world", 0, 8), "hello w…");
    assert_eq!(text("hello world", 6, 8), "world   ");
    assert_eq!(text("hello world", 3, 5), "lo w…");
    // A wide character cut by either edge leaves blanks, never half a character.
    assert_eq!(text("日本語です", 0, 5), "日本…");
    assert_eq!(text("日本語です", 1, 5), " 本… ");
    assert_eq!(text("日本語です", 2, 6), "本語… ");
    assert_eq!(text("日本語", 5, 4), "    ");
    assert_eq!(text("x", 4, 3), "   ");
    assert_eq!(text("abc", 0, 1), "…");
    assert!(
        windowed(
            "abc".chars().map(|c| (c, 0)),
            3,
            0,
            0,
            &[Style::new(); 3],
            true
        )
        .is_empty()
    );
}

#[test]
fn long_lines_wrap_under_their_number_and_w_cuts_them_instead() {
    let line = "0123456789abcdefghijklmnopqrstuvwxyz";
    let mut v = DiffView::from_text("f", &format!("@@ -1,2 +1,2 @@\n+{line}\n ok\n"));
    let mut t = Terminal::new(TestBackend::new(20, 6)).unwrap();
    let out = draw_to(&v, &mut t);
    let rows: Vec<&str> = out.lines().collect();
    // The gutter is 4 cells, the sign 1: 15 cells of text a row.
    assert_eq!(rows[2], "  1 +0123456789abcde");
    assert_eq!(rows[3], "     fghijklmnopqrst");
    assert_eq!(rows[4], "     uvwxyz");
    assert_eq!(rows[5], "  2  ok");
    // The title counts lines of the diff, not rows of the screen.
    assert!(rows[0].ends_with("1–3/3"), "{}", rows[0]);
    // Nothing to scroll sideways while wrapping; `<leader>w` cuts again.
    ch(&mut v, 'j');
    ch(&mut v, '$');
    assert_eq!(v.hscroll, 0);
    ch(&mut v, '0');
    ch(&mut v, ' ');
    ch(&mut v, 'w');
    let cut = draw_to(&v, &mut t);
    assert_eq!(cut.lines().nth(2).unwrap(), "  1 +0123456789abcd…");
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
fn draws_the_unified_view_when_narrow() {
    let mut v = DiffView::from_text("src/a.ts", SAMPLE);
    v.wrap = false;
    let mut t = Terminal::new(TestBackend::new(24, 8)).unwrap();
    let out = draw_to(&v, &mut t);
    let first = out.lines().next().unwrap();
    assert_eq!(first, "src/a.ts  +1 −2   1–7/10");
    // Headers get an empty gutter, so the code lines up under them.
    assert_eq!(out.lines().nth(1).unwrap(), "    diff --git a/src/a.…");
    assert_eq!(out.lines().count(), 8);
    // Colors.
    let p = Palette::default();
    let buf = t.backend().buffer();
    assert_eq!(buf[(0, 5)].fg, p.muted); // @@
    assert_eq!(buf[(0, 7)].fg, p.muted); // its line number
    assert_eq!(buf[(4, 7)].fg, p.danger); // -old line
    assert_eq!(buf[(10, 0)].fg, p.success); // +1 in the title
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
fn draws_the_split_view() {
    let v = DiffView::from_text("src/app.ts", BIG);
    let out = render(&v, 120, 30);
    snapshot("diff-split-120x30", &out);
    let lines: Vec<&str> = out.lines().collect();
    assert!(lines[0].starts_with("src/app.ts  +"));
    assert!(lines[0].ends_with("lines 1–23 of 23"));
    assert!(lines[1].starts_with(" HEAD"));
    assert!(lines[1].contains("│ working tree"));
    // The separator is in the same column on every row, hunk bands excepted.
    let sep = lines[1].chars().position(|c| c == '│').unwrap();
    for l in &lines[2..] {
        if !l.starts_with('⋯') {
            assert_eq!(l.chars().nth(sep), Some('│'), "{l}");
        }
    }
}

#[test]
fn narrow_terminals_fall_back_to_the_unified_view() {
    let v = DiffView::from_text("src/app.ts", BIG);
    let out = render(&v, 60, 14);
    snapshot("diff-unified-60x14", &out);
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

/// The cells showing `text` on row `y`, by where it is found.
fn cells_of(buf: &ratatui::buffer::Buffer, y: u16, text: &str) -> Vec<(Color, Color)> {
    let row: String = (0..buf.area.width)
        .map(|x| buf[(x, y)].symbol().to_string())
        .collect();
    let at = row.find(text).expect("the text on the row");
    let x = row[..at].chars().count() as u16;
    (x..x + text.chars().count() as u16)
        .map(|x| (buf[(x, y)].fg, buf[(x, y)].bg))
        .collect()
}

#[test]
fn code_is_colored_by_its_grammar_on_the_change_tint() {
    let diff = "--- a/src/m.rs\n+++ b/src/m.rs\n@@ -1,2 +1,2 @@\n fn keep() {}\n-let a = \"x\";\n+let a = \"y\";\n";
    let mut v = DiffView::from_text("src/m.rs", diff);
    v.wait_for_syntax(0, 10);
    let mut t = Terminal::new(TestBackend::new(60, 8)).unwrap();
    draw_to(&v, &mut t);
    let buf = t.backend().buffer();
    let colors = code_colors(&Palette::default()).unwrap();
    let keyword = colors[Class::Keyword as usize].fg.unwrap();
    let string = colors[Class::String as usize].fg.unwrap();
    // Unified at this width: under the title, the headers and the hunk, the context line,
    // then the removal and the addition.
    assert!(cells_of(buf, 4, "fn").iter().all(|(fg, _)| *fg == keyword));
    let removed = cells_of(buf, 5, "let");
    assert!(
        removed
            .iter()
            .all(|(fg, bg)| *fg == keyword && *bg != Color::Reset)
    );
    assert!(
        cells_of(buf, 6, "\"y\"")
            .iter()
            .all(|(fg, _)| *fg == string)
    );
    // The sign keeps the change's color.
    assert_eq!(buf[(4, 6)].fg, Palette::default().success);
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
fn paints_changes_and_blank_halves() {
    let p = Palette::default();
    let v = DiffView::from_text(
        "f",
        "@@ -1,2 +1,3 @@\n same\n-a = foo(1)\n+a = foo(2)\n+extra\n",
    );
    let mut t = Terminal::new(TestBackend::new(100, 8)).unwrap();
    draw_to(&v, &mut t);
    let buf = t.backend().buffer();
    let (del_bg, add_bg) = (buf[(10, 4)].bg, buf[(60, 4)].bg);
    assert_ne!(del_bg, Color::Reset);
    assert_ne!(add_bg, Color::Reset);
    assert_eq!(buf[(10, 4)].fg, p.danger);
    assert_eq!(buf[(60, 4)].fg, p.success);
    assert_eq!(buf[(10, 3)].bg, Color::Reset); // context
    // The changed digit has the stronger mark, the rest of the line the row's.
    assert_ne!(buf[(6 + 8, 4)].bg, del_bg);
    assert_eq!(buf[(6 + 2, 4)].bg, del_bg);
    // The blank half is shaded, the line numbers are muted, signs say which side.
    assert_eq!(buf[(10, 5)].bg, p.surface);
    assert_eq!(buf[(2, 3)].fg, p.muted);
    assert_eq!((buf[(4, 4)].symbol(), buf[(54, 4)].symbol()), ("-", "+"));
    assert_eq!(buf[(4, 5)].symbol(), " ");
    // Without colors, the mark is reversed and nothing is painted.
    let plain = crate::style::plain_palette();
    t.draw(|f| v.draw(f, f.area(), &plain)).unwrap();
    let buf = t.backend().buffer();
    assert_eq!(buf[(10, 4)].bg, Color::Reset);
    assert!(buf[(6 + 8, 4)].modifier.contains(Modifier::REVERSED));
    assert!(!buf[(6 + 2, 4)].modifier.contains(Modifier::REVERSED));
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
    assert!(rows[head + 2].contains("enter keeps it"), "{out}");
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
    assert!(!out.contains("enter keeps it"), "{out}");
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
