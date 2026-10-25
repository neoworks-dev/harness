//! An image on the system clipboard, for pasting into the composer: svelte-gpui's clipboard
//! carries text only. The image is read with `wl-paste` (Wayland) or `xclip` (X11) and saved to a
//! file, since the daemon takes attached images by path.

use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::json;
use svelte_gpui::{CommandError, Commands};

/// The image types taken, most wanted first, with the extension each is saved under.
const IMAGE_TYPES: &[(&str, &str)] = &[
    ("image/png", "png"),
    ("image/jpeg", "jpg"),
    ("image/webp", "webp"),
    ("image/gif", "gif"),
];

pub fn register(commands: Commands) -> Commands {
    commands.command("clipboard_image", |_| {
        Ok::<_, CommandError>(json!(paste_image().map(|path| path.display().to_string())))
    })
}

/// Saves the clipboard's image to a file and returns its path; none when it holds no image or
/// neither tool is installed.
fn paste_image() -> Option<PathBuf> {
    let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some();
    let (extension, bytes) = if wayland {
        read_with(&["wl-paste", "--list-types"], |mime| {
            vec!["wl-paste", "--no-newline", "--type", mime]
        })?
    } else {
        read_with(
            &["xclip", "-selection", "clipboard", "-t", "TARGETS", "-o"],
            |mime| vec!["xclip", "-selection", "clipboard", "-t", mime, "-o"],
        )?
    };
    save(&bytes, extension)
}

/// Lists the clipboard's types with `list`, picks an image type and reads it with `read`.
fn read_with(list: &[&str], read: impl Fn(&str) -> Vec<&str>) -> Option<(&'static str, Vec<u8>)> {
    let types = run(list)?;
    let (mime, extension) = pick_type(&String::from_utf8_lossy(&types))?;
    let bytes = run(&read(mime))?;
    (!bytes.is_empty()).then_some((extension, bytes))
}

/// The most wanted image type among the clipboard's types, one per line.
fn pick_type(types: &str) -> Option<(&'static str, &'static str)> {
    let offered: Vec<&str> = types.lines().map(str::trim).collect();
    IMAGE_TYPES
        .iter()
        .find(|(mime, _)| offered.contains(mime))
        .copied()
}

/// A command's standard output when it succeeds.
fn run(argv: &[&str]) -> Option<Vec<u8>> {
    let (program, args) = argv.split_first()?;
    let output = Command::new(program).args(args).output().ok()?;
    output.status.success().then_some(output.stdout)
}

/// Writes the image to `$TMPDIR/neo-gui/paste-<ms>.<extension>`.
fn save(bytes: &[u8], extension: &str) -> Option<PathBuf> {
    let dir = std::env::temp_dir().join("neo-gui");
    std::fs::create_dir_all(&dir).ok()?;
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_millis();
    let path = dir.join(format!("paste-{millis}.{extension}"));
    std::fs::write(&path, bytes).ok()?;
    Some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_is_preferred_and_text_alone_is_no_image() {
        assert_eq!(
            pick_type("text/html\nimage/jpeg\nimage/png\n"),
            Some(("image/png", "png"))
        );
        assert_eq!(pick_type("image/webp"), Some(("image/webp", "webp")));
        assert_eq!(pick_type("text/plain\nUTF8_STRING"), None);
    }

    #[test]
    fn saved_images_keep_their_bytes() {
        let path = save(b"\x89PNG", "png").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"\x89PNG");
        assert!(path.to_string_lossy().ends_with(".png"));
        std::fs::remove_file(path).unwrap();
    }
}
