//! Every `fixtures/<name>/resolved.json` is drawn with the default theme and compared with
//! snapshots next to it: `ratatui.txt` (80 columns, plain text), `ratatui-40.txt`,
//! `ratatui-expanded.txt` (80 columns, tool results in full, thoughts shown) and
//! `ratatui-styled.txt` (80 columns with the styles written out). `UPDATE_SNAPSHOTS=1`
//! rewrites them.

mod common;

use std::fs;

use common::*;
use neo_renderer_ratatui::{Options, Palette};

fn snapshots(name: &str) -> Vec<(&'static str, String)> {
    let items = fixture_items(name);
    let opts = Options::default();
    let expanded = Options {
        expanded: true,
        show_thoughts: true,
        ..Options::default()
    };
    vec![
        ("ratatui.txt", plain(&render_all(&items, 80, &opts))),
        ("ratatui-40.txt", plain(&render_all(&items, 40, &opts))),
        (
            "ratatui-expanded.txt",
            plain(&render_all(&items, 80, &expanded)),
        ),
        (
            "ratatui-styled.txt",
            styled(&render_all(&items, 80, &opts), &Palette::default()),
        ),
    ]
}

#[test]
fn fixtures_match_their_snapshots() {
    let names = fixture_names();
    assert!(names.len() >= 8, "fixtures missing: {names:?}");
    let update = std::env::var_os("UPDATE_SNAPSHOTS").is_some();
    let mut failures = Vec::new();
    for name in names {
        for (file, got) in snapshots(&name) {
            let path = fixtures_dir().join(&name).join(file);
            if update {
                fs::write(&path, &got).expect("write snapshot");
                continue;
            }
            match fs::read_to_string(&path) {
                Ok(want) if want == got => {}
                Ok(_) => failures.push(format!(
                    "{} differs (UPDATE_SNAPSHOTS=1 rewrites it)",
                    path.display()
                )),
                Err(err) => failures.push(format!(
                    "{}: {err} (run with UPDATE_SNAPSHOTS=1 to create it)",
                    path.display()
                )),
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn rendering_is_deterministic() {
    for name in fixture_names() {
        assert_eq!(snapshots(&name), snapshots(&name), "{name}");
    }
}

/// No line is ever wider than the width asked for, whatever the width, in either mode.
#[test]
fn no_line_overflows_at_any_width() {
    let modes = [
        Options::default(),
        Options {
            expanded: true,
            show_thoughts: true,
            focused_button: Some(0),
            ..Options::default()
        },
    ];
    for name in fixture_names() {
        let items = fixture_items(&name);
        for opts in &modes {
            for width in [1u16, 2, 3, 4, 5, 7, 10, 13, 20, 33, 40, 80, 200] {
                for it in &items {
                    for (n, line) in neo_renderer_ratatui::render_item(it, width, opts)
                        .iter()
                        .enumerate()
                    {
                        assert!(
                            line.width() <= usize::from(width),
                            "{name}/{} width {width} line {n} is {} wide: {line:?}",
                            it.r#ref,
                            line.width()
                        );
                    }
                }
            }
        }
    }
}
