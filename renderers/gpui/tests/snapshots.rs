//! Every `fixtures/<name>/resolved.json` goes through the layout model with the default theme and
//! is compared with `fixtures/<name>/gpui.txt`. `UPDATE_SNAPSHOTS=1` rewrites the files.

use std::fs;
use std::path::{Path, PathBuf};

use neo_protocol::ViewItem;
use neo_renderer_gpui::{ChatTheme, dump_items, layout_items};

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures")
}

fn fixture_names() -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(fixtures_dir())
        .expect("fixtures dir")
        .filter_map(Result::ok)
        .filter(|entry| entry.path().join("resolved.json").is_file())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

fn render(name: &str) -> String {
    let path = fixtures_dir().join(name).join("resolved.json");
    let data = fs::read_to_string(&path).expect("resolved.json");
    let items: Vec<ViewItem> =
        serde_json::from_str(&data).unwrap_or_else(|err| panic!("{name}: {err}"));
    let theme = ChatTheme::default();
    dump_items(&layout_items(&items, &theme), &theme)
}

#[test]
fn fixtures_match_their_snapshots() {
    let names = fixture_names();
    assert!(names.len() >= 8, "fixtures missing: {names:?}");
    let update = std::env::var_os("UPDATE_SNAPSHOTS").is_some();
    let mut failures = Vec::new();
    for name in names {
        let got = render(&name);
        let file = fixtures_dir().join(&name).join("gpui.txt");
        if update {
            fs::write(&file, &got).expect("write snapshot");
            continue;
        }
        match fs::read_to_string(&file) {
            Ok(want) if want == got => {}
            Ok(_) => failures.push(format!(
                "{} differs (UPDATE_SNAPSHOTS=1 rewrites it)",
                file.display()
            )),
            Err(err) => failures.push(format!(
                "{}: {err} (run with UPDATE_SNAPSHOTS=1 to create it)",
                file.display()
            )),
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn every_fixture_lays_out_deterministically() {
    for name in fixture_names() {
        assert_eq!(render(&name), render(&name), "{name}");
    }
}
