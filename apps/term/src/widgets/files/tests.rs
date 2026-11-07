use super::*;
use crate::widgets::git::{GitEntry, parse_status};

fn status(entries: Vec<GitEntry>) -> GitStatus {
    GitStatus {
        head: Some("main".into()),
        ahead: 1,
        entries,
        ..GitStatus::default()
    }
}

fn entry(path: &str, change: Change) -> GitEntry {
    GitEntry {
        path: path.into(),
        orig: None,
        change,
    }
}

#[test]
fn untracked_files_alone_are_not_worth_a_column() {
    let mut f = FilesState::default();
    f.set_status(Some(status(vec![
        entry(".claude/", Change::Untracked),
        entry(".serena/", Change::Untracked),
    ])));
    assert!(!f.worth_showing());
    // An agent touching one makes it news.
    f.touch(".claude/settings.json", "claude", true);
    assert!(f.worth_showing());

    let mut s = status(vec![entry("src/a.rs", Change::Modified)]);
    s.lines.insert(
        "src/a.rs".into(),
        LineCount {
            added: 12,
            removed: 3,
        },
    );
    let mut f = FilesState::default();
    f.set_status(Some(s));
    assert!(f.worth_showing());
}

#[test]
fn a_folder_with_many_changed_files_starts_folded_until_opened() {
    let mut f = FilesState::default();
    let mut entries: Vec<_> = (0..6)
        .map(|i| entry(&format!("fixtures/t/{i}.txt"), Change::Modified))
        .collect();
    entries.push(entry("src/a.rs", Change::Modified));
    f.set_status(Some(status(entries)));
    let shown = |f: &FilesState| text(&f.render(40, 20, false, &Palette::default()));
    assert_eq!(shown(&f), ["▸ fixtures/t/ 6 files", "▾ src/", "  M a.rs"]);
    f.key(KeyEvent::from(KeyCode::Enter));
    assert_eq!(shown(&f).len(), 9, "opened by hand it stays open");
    f.key(KeyEvent::from(KeyCode::Enter));
    assert_eq!(shown(&f)[0], "▸ fixtures/t/ 6 files");
}

#[test]
fn a_tree_that_folds_and_is_walked_with_arrows() {
    let p = Palette::default();
    let mut f = FilesState::default();
    f.set_status(Some(status(vec![
        entry("apps/term/src/app.rs", Change::Modified),
        entry("apps/term/src/menu.rs", Change::Modified),
        entry("apps/term/README.md", Change::Modified),
        entry("Cargo.lock", Change::Modified),
    ])));
    // A folder holding only one folder is joined with it.
    assert_eq!(
        text(&f.render(30, 10, false, &p)),
        [
            "▾ apps/term/",
            "  ▾ src/",
            "    M app.rs",
            "    M menu.rs",
            "  M README.md",
            "M Cargo.lock",
        ]
    );
    let key = |c| KeyEvent::from(c);
    // On a folder Enter folds it; Right opens it again and then goes in.
    assert!(f.selected().is_none(), "apps/term/ is a folder");
    assert_eq!(f.key(key(KeyCode::Enter)), FileKey::Handled);
    assert_eq!(
        text(&f.render(30, 10, false, &p)),
        ["▸ apps/term/ 3 files", "M Cargo.lock"]
    );
    f.key(key(KeyCode::Right)); // opens apps/term/
    f.key(key(KeyCode::Right)); // into it: src/
    f.key(key(KeyCode::Right)); // into src/: app.rs
    assert_eq!(f.selected().unwrap().rel, "apps/term/src/app.rs");
    // Left goes up to the folder, then folds it; Enter on a file opens it.
    f.key(key(KeyCode::Left));
    f.key(key(KeyCode::Left));
    assert_eq!(text(&f.render(30, 10, false, &p))[1], "  ▸ src/ 2 files");
    f.key(key(KeyCode::Char('G')));
    assert_eq!(f.key(key(KeyCode::Enter)), FileKey::Open);
    // The fold survives a refresh.
    f.set_status(f.status.clone().map(|mut s| {
        s.ahead = 2;
        s
    }));
    assert_eq!(text(&f.render(30, 10, false, &p))[1], "  ▸ src/ 2 files");
}

fn text(lines: &[Line]) -> Vec<String> {
    lines
        .iter()
        .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
        .collect()
}

#[test]
fn git_entries_with_agent_tags_and_committed_ones_marked() {
    let mut f = FilesState::default();
    f.touch("src/a.ts", "agent", false);
    f.touch("src/a.ts", "claude", false);
    f.touch("gone.txt", "claude", true);
    assert_eq!(f.entries().len(), 2);
    assert_eq!(f.entries()[0].mark, Mark::Modified);
    assert_eq!(f.entries()[1].mark, Mark::Added);
    assert!(!f.in_repo());
    f.set_status(Some(status(vec![
        entry("src/a.ts", Change::Modified),
        entry("b.txt", Change::Untracked),
    ])));
    let got: Vec<_> = f
        .entries()
        .iter()
        .map(|e| (e.rel.as_str(), e.mark, e.tags.clone()))
        .collect();
    assert_eq!(
        got,
        [
            (
                "src/a.ts",
                Mark::Modified,
                vec!["agent".to_string(), "claude".to_string()]
            ),
            ("b.txt", Mark::Untracked, vec![]),
            // Touched, but git sees no change in it any more: committed.
            ("gone.txt", Mark::Committed, vec!["claude".to_string()]),
        ]
    );
    let p = Palette::default();
    let lines = f.render(40, 10, false, &p);
    assert_eq!(text(&lines)[2], "✓ gone.txt (claude, committed)");
    assert_eq!(text(&lines)[3], "▸ untracked 1 file");
    assert_eq!(f.branch().as_deref(), Some("main ↑1"));
    // Same status again is no change.
    assert!(!f.set_status(f.status.clone()));
}

#[test]
fn untracked_directory_takes_the_tags_of_files_in_it() {
    let mut f = FilesState::default();
    f.touch("new/x.rs", "agent", true);
    f.set_status(Some(status(vec![entry("new/", Change::Untracked)])));
    assert_eq!(f.entries().len(), 1);
    assert_eq!(f.entries()[0].tags, ["agent"]);
}

#[test]
fn paths_are_relative_to_the_session_directory() {
    let mut f = FilesState::default();
    let mut s = parse_status(b"# branch.head main\0? src/x.rs\0? top.md\0");
    s.prefix = "src/".into();
    f.set_status(Some(s));
    let rels: Vec<_> = f.entries().iter().map(|e| e.rel.as_str()).collect();
    assert_eq!(rels, ["x.rs", "../top.md"]);
}

#[test]
fn rows_carry_their_mark_and_untracked_ones_group() {
    let p = Palette::default();
    let mut f = FilesState::default();
    let mut old = entry("new name.rs", Change::Renamed);
    old.orig = Some("old.rs".into());
    let all = vec![
        entry("a.ts", Change::Modified),
        entry("b.ts", Change::Added),
        entry("c.ts", Change::Deleted),
        old,
        entry("d.md", Change::Untracked),
        entry("e.txt", Change::Conflicted),
    ];
    f.set_status(Some(status(all)));
    f.touch("a.ts", "agent", false);
    // The untracked group opened.
    f.key(KeyEvent::from(KeyCode::Char('G')));
    f.key(KeyEvent::from(KeyCode::Enter));
    let lines = f.render(30, 20, false, &p);
    assert_eq!(
        text(&lines),
        [
            "M a.ts (agent)",
            "A b.ts",
            "D c.ts",
            "U e.txt",
            "R old.rs → new name.rs",
            "▾ untracked",
            "  ? d.md",
        ]
    );
}

#[test]
fn untracked_files_no_agent_touched_fold_into_one_row() {
    let p = Palette::default();
    let mut f = FilesState::default();
    f.set_status(Some(status(vec![
        entry(".claude/", Change::Untracked),
        entry("atlas/", Change::Untracked),
        entry("notes/todo.md", Change::Untracked),
        entry("src/a.rs", Change::Modified),
    ])));
    f.touch("atlas/x.rs", "agent", true);
    assert_eq!(
        text(&f.render(40, 10, false, &p)),
        [
            "▾ src/",
            "  M a.rs",
            "? atlas/ (agent)",
            "▸ untracked 2 files"
        ],
        "one an agent touched stays in the tree"
    );
    f.key(KeyEvent::from(KeyCode::Char('G')));
    f.key(KeyEvent::from(KeyCode::Enter));
    assert_eq!(
        text(&f.render(40, 10, false, &p)),
        [
            "▾ src/",
            "  M a.rs",
            "? atlas/ (agent)",
            "▾ untracked",
            "  ▾ notes/",
            "    ? todo.md",
            "  ? .claude/",
        ]
    );
}

#[test]
fn selection_moves_and_scrolls_with_focus() {
    let p = Palette::default();
    let mut f = FilesState::default();
    f.set_status(Some(status(
        (0..6)
            .map(|i| entry(&format!("f{i}"), Change::Modified))
            .collect(),
    )));
    let key = |c| KeyEvent::from(c);
    assert_eq!(f.key(key(KeyCode::Up)), FileKey::Handled);
    assert_eq!(f.selected().unwrap().rel, "f0");
    f.key(key(KeyCode::Char('j')));
    f.key(key(KeyCode::Down));
    assert_eq!(f.selected().unwrap().rel, "f2");
    f.key(key(KeyCode::Char('k')));
    assert_eq!(f.selected().unwrap().rel, "f1");
    f.key(key(KeyCode::Char('G')));
    assert_eq!(f.selected().unwrap().rel, "f5");
    f.key(key(KeyCode::Down));
    assert_eq!(f.selected().unwrap().rel, "f5");
    // Window follows the selection when focused; the selected row is highlighted.
    let lines = f.render(10, 4, true, &p);
    let trimmed: Vec<_> = text(&lines)
        .iter()
        .map(|l| l.trim_end().to_string())
        .collect();
    assert_eq!(trimmed, ["M f3", "M f4", "M f5", "… 3 more"]);
    assert_eq!(text(&f.render(10, 4, false, &p))[0], "M f0");
    f.key(key(KeyCode::Char('g')));
    assert_eq!(f.key(key(KeyCode::Enter)), FileKey::Open);
    assert_eq!(f.key(key(KeyCode::Esc)), FileKey::Leave);
}

#[test]
fn selection_survives_a_refresh_by_path() {
    let mut f = FilesState::default();
    f.set_status(Some(status(vec![
        entry("a", Change::Modified),
        entry("b", Change::Modified),
        entry("c", Change::Modified),
    ])));
    f.key(KeyEvent::from(KeyCode::Char('G')));
    f.set_status(Some(status(vec![
        entry("0", Change::Modified),
        entry("a", Change::Modified),
        entry("c", Change::Modified),
    ])));
    assert_eq!(f.selected().unwrap().rel, "c");
    f.set_status(Some(status(vec![entry("0", Change::Modified)])));
    assert_eq!(f.selected().unwrap().rel, "0");
    f.set_status(Some(status(vec![])));
    assert!(f.selected().is_none());
    assert_eq!(f.key(KeyEvent::from(KeyCode::Enter)), FileKey::Handled);
}
