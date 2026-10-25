//! Images attached to the next message: the clipboard's image (Ctrl-V), or image files pasted or
//! dropped as paths. The daemon takes attached images by path, so a clipboard image is saved to a
//! file first. The terminal only pastes text, so the clipboard is read with `wl-paste` (Wayland)
//! or `xclip` (X11), as the desktop app does.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

/// The image types taken, most wanted first, with the extension each is saved under.
const IMAGE_TYPES: &[(&str, &str)] = &[
    ("image/png", "png"),
    ("image/jpeg", "jpg"),
    ("image/webp", "webp"),
    ("image/gif", "gif"),
];

/// Saves the clipboard's image to a file and returns its path; none when it holds no image or
/// neither tool is installed.
pub fn paste_image() -> Option<PathBuf> {
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
    save(&std::env::temp_dir().join("neo-term"), &bytes, extension)
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

/// A command's standard output when it succeeds. Its stderr is dropped: it would land on the
/// screen.
fn run(argv: &[&str]) -> Option<Vec<u8>> {
    let (program, args) = argv.split_first()?;
    let output = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    output.status.success().then_some(output.stdout)
}

/// Writes the image to `<dir>/paste-<ms>.<extension>`.
fn save(dir: &Path, bytes: &[u8], extension: &str) -> Option<PathBuf> {
    std::fs::create_dir_all(dir).ok()?;
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_millis();
    let path = dir.join(format!("paste-{millis}.{extension}"));
    std::fs::write(&path, bytes).ok()?;
    Some(path)
}

/// Whether the agent gets the file as an image (PNG, JPEG, GIF, WebP), by its name.
pub fn is_image(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| {
            matches!(
                e.to_ascii_lowercase().as_str(),
                "png" | "jpg" | "jpeg" | "gif" | "webp"
            )
        })
}

/// Pasted text that is nothing but paths of image files that exist (what dropping files on the
/// terminal pastes): the absolute paths. Paths are separated by white space and may be quoted,
/// have escaped spaces, be `file://` URIs or start with `~/`; relative ones are taken from `cwd`.
/// None when anything else is in the text.
pub fn image_paths(text: &str, cwd: &str) -> Option<Vec<String>> {
    let words = split_words(text.trim())?;
    if words.is_empty() {
        return None;
    }
    words
        .into_iter()
        .map(|word| {
            let path = resolve(&word, cwd)?;
            (is_image(&path) && Path::new(&path).is_file()).then_some(path)
        })
        .collect()
}

fn resolve(word: &str, cwd: &str) -> Option<String> {
    let path = if let Some(rest) = word.strip_prefix("file://") {
        // file:///abs or file://host/abs; the host is dropped.
        let abs = &rest[rest.find('/')?..];
        percent_decode(abs)?
    } else if let Some(rest) = word.strip_prefix("~/") {
        format!("{}/{rest}", std::env::var("HOME").ok()?)
    } else if word.starts_with('/') {
        word.to_string()
    } else {
        Path::new(cwd).join(word).to_string_lossy().into_owned()
    };
    Some(path)
}

/// `%20` and friends; none for a broken escape or bytes that are not UTF-8.
fn percent_decode(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = s.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// Splits like a shell: white space separates, quotes and backslashes keep it. None for an
/// unclosed quote.
fn split_words(text: &str) -> Option<Vec<String>> {
    let mut words = Vec::new();
    let mut word: Option<String> = None;
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        match c {
            '\'' | '"' => {
                let w = word.get_or_insert_with(String::new);
                loop {
                    match chars.next()? {
                        q if q == c => break,
                        '\\' if c == '"' => w.push(chars.next()?),
                        ch => w.push(ch),
                    }
                }
            }
            '\\' => word.get_or_insert_with(String::new).push(chars.next()?),
            c if c.is_whitespace() => words.extend(word.take()),
            c => word.get_or_insert_with(String::new).push(c),
        }
    }
    words.extend(word);
    Some(words)
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
        let dir = std::env::temp_dir().join("neo-term-test-save");
        let path = save(&dir, b"\x89PNG", "png").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"\x89PNG");
        assert!(path.to_string_lossy().ends_with(".png"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn words_split_like_a_shell() {
        assert_eq!(
            split_words(r#"/a\ b.png '/c d.png' "/e \"f\".png" /g.png"#).unwrap(),
            ["/a b.png", "/c d.png", "/e \"f\".png", "/g.png"]
        );
        assert_eq!(split_words("'open"), None);
    }

    #[test]
    fn only_existing_image_files_are_attached() {
        let dir = std::env::temp_dir().join("neo-term-test-paths");
        std::fs::create_dir_all(dir.join("with space")).unwrap();
        let a = dir.join("with space/a.PNG");
        let b = dir.join("b.jpg");
        let txt = dir.join("notes.txt");
        for p in [&a, &b, &txt] {
            std::fs::write(p, b"x").unwrap();
        }
        let d = dir.to_string_lossy();
        let a_uri = format!("file://{d}/with%20space/a.PNG");

        assert_eq!(
            image_paths(&format!("{a_uri}\n'{d}/b.jpg'\n"), "/").unwrap(),
            [a.to_string_lossy(), b.to_string_lossy()]
        );
        assert_eq!(image_paths("b.jpg", &d).unwrap(), [b.to_string_lossy()]);
        assert_eq!(image_paths(&format!("{d}/notes.txt"), "/"), None);
        assert_eq!(image_paths(&format!("{d}/missing.png"), "/"), None);
        assert_eq!(image_paths(&format!("look at {d}/b.jpg"), "/"), None);
        assert_eq!(image_paths("  ", "/"), None);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
