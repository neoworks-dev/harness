//! What the workspace card asks of the machine around the window: its name, the code editors
//! installed on it and the file manager. Editors are looked up in a fixed list, never run by a
//! name the UI makes up.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::{Value, json};
use svelte_gpui::{CommandError, Commands};

/// An editor that can be found on `PATH`: the id the UI remembers, its name and the commands it
/// installs under (the first one found is used).
struct Editor {
    id: &'static str,
    name: &'static str,
    commands: &'static [&'static str],
}

const EDITORS: &[Editor] = &[
    Editor {
        id: "zed",
        name: "Zed",
        commands: &["zed", "zeditor"],
    },
    Editor {
        id: "code",
        name: "VS Code",
        commands: &["code"],
    },
    Editor {
        id: "cursor",
        name: "Cursor",
        commands: &["cursor"],
    },
    Editor {
        id: "windsurf",
        name: "Windsurf",
        commands: &["windsurf"],
    },
    Editor {
        id: "idea",
        name: "IntelliJ IDEA",
        commands: &["idea"],
    },
    Editor {
        id: "subl",
        name: "Sublime Text",
        commands: &["subl"],
    },
];

pub fn register(commands: Commands) -> Commands {
    commands
        .command("hostname", |_| Ok::<_, CommandError>(json!(hostname())))
        .command("editors", |_| {
            let path = std::env::var_os("PATH").unwrap_or_default();
            let found = installed(&path)
                .into_iter()
                .map(|(editor, _)| json!({ "id": editor.id, "name": editor.name }));
            Ok::<_, CommandError>(Value::Array(found.collect()))
        })
        .command("open_in_editor", |args| {
            let editor = args["editor"].as_str().unwrap_or("");
            let path = args["path"].as_str().unwrap_or("");
            let search = std::env::var_os("PATH").unwrap_or_default();
            open_in_editor(&search, editor, path).map(|()| Value::Null)
        })
        .command("open_folder", |args| {
            open_folder(args["path"].as_str().unwrap_or("")).map(|()| Value::Null)
        })
        .command("open_url", |args| {
            open_url(args["url"].as_str().unwrap_or("")).map(|()| Value::Null)
        })
}

/// This machine's name, empty when it cannot be found.
pub fn hostname() -> String {
    let files = ["/proc/sys/kernel/hostname", "/etc/hostname"];
    let read = files
        .iter()
        .find_map(|file| std::fs::read_to_string(file).ok())
        .map(|text| text.trim().to_string())
        .filter(|name| !name.is_empty());
    let env = || {
        ["HOSTNAME", "COMPUTERNAME"]
            .iter()
            .find_map(|key| std::env::var(key).ok())
            .filter(|name| !name.is_empty())
    };
    let command = || {
        let out = Command::new("hostname")
            .stderr(Stdio::null())
            .output()
            .ok()?;
        let name = String::from_utf8(out.stdout).ok()?.trim().to_string();
        (out.status.success() && !name.is_empty()).then_some(name)
    };
    read.or_else(env).or_else(command).unwrap_or_default()
}

/// The file `command` names in one of the directories of `path`, if it can be run.
fn find_on_path(path: &OsStr, command: &str) -> Option<PathBuf> {
    std::env::split_paths(path)
        .map(|dir| dir.join(command))
        .find(|candidate| is_executable(candidate))
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

/// The known editors found in `path`, each with the file that runs it.
fn installed(path: &OsStr) -> Vec<(&'static Editor, PathBuf)> {
    EDITORS
        .iter()
        .filter_map(|editor| {
            let found = editor
                .commands
                .iter()
                .find_map(|name| find_on_path(path, name))?;
            Some((editor, found))
        })
        .collect()
}

/// Starts `program` on `path` and leaves it running on its own: the window does not wait for it.
fn launch(program: &OsStr, path: &str) -> Result<(), String> {
    let mut child = Command::new(program)
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("could not start {}: {error}", program.to_string_lossy()))?;
    // Reaped on a thread of its own, so a launcher that stays attached leaves no zombie.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

/// Opens `path` (a folder) in the editor with this id.
fn open_in_editor(search: &OsStr, id: &str, path: &str) -> Result<(), CommandError> {
    if !Path::new(path).exists() {
        return Err(format!("{path} does not exist").into());
    }
    let Some((_, program)) = installed(search).into_iter().find(|(e, _)| e.id == id) else {
        return Err(format!("{id} is not installed").into());
    };
    launch(program.as_os_str(), path).map_err(Into::into)
}

/// Shows a folder in the system's file manager.
fn open_folder(path: &str) -> Result<(), CommandError> {
    if !Path::new(path).is_dir() {
        return Err(format!("{path} is not a folder").into());
    }
    let program = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(windows) {
        "explorer"
    } else {
        "xdg-open"
    };
    launch(OsStr::new(program), path).map_err(Into::into)
}

/// Opens a web or mail address in the default browser or mail app. Anything else is refused: the
/// address comes from a model's document.
fn open_url(url: &str) -> Result<(), CommandError> {
    let program = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(windows) {
        "explorer"
    } else {
        "xdg-open"
    };
    open_url_with(OsStr::new(program), url)
}

fn open_url_with(program: &OsStr, url: &str) -> Result<(), CommandError> {
    let allowed = ["http://", "https://", "mailto:"];
    let lower = url.to_ascii_lowercase();
    if !allowed.iter().any(|scheme| lower.starts_with(scheme)) || url.chars().any(char::is_control)
    {
        return Err(format!("will not open {url}").into());
    }
    launch(program, url).map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("suite-gui-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[cfg(unix)]
    fn script(dir: &Path, name: &str, body: &str) {
        use std::os::unix::fs::PermissionsExt;
        let file = dir.join(name);
        std::fs::write(&file, body).unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn editors_are_the_known_ones_found_on_path() {
        let bin = temp("editors-bin");
        script(&bin, "code", "#!/bin/sh\n");
        // Another name for Zed, and a file that is not executable.
        script(&bin, "zeditor", "#!/bin/sh\n");
        std::fs::write(bin.join("cursor"), "not runnable").unwrap();
        // Not in the list: never offered.
        script(&bin, "vim", "#!/bin/sh\n");

        let found = installed(bin.as_os_str());
        let ids: Vec<&str> = found.iter().map(|(editor, _)| editor.id).collect();
        assert_eq!(ids, ["zed", "code"]);
        assert!(installed(OsStr::new("/nonexistent")).is_empty());
        let _ = std::fs::remove_dir_all(&bin);
    }

    #[cfg(unix)]
    #[test]
    fn an_editor_is_started_with_the_folder() {
        let bin = temp("editor-run-bin");
        let folder = temp("editor-run-folder");
        let log = bin.join("log");
        let body = format!("#!/bin/sh\necho \"$1\" > {}\n", log.display());
        script(&bin, "subl", &body);

        open_in_editor(bin.as_os_str(), "subl", &folder.to_string_lossy()).unwrap();
        let mut seen = String::new();
        for _ in 0..100 {
            seen = std::fs::read_to_string(&log).unwrap_or_default();
            if !seen.is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_eq!(seen.trim(), folder.to_string_lossy());

        let missing = open_in_editor(bin.as_os_str(), "zed", &folder.to_string_lossy());
        assert!(missing.unwrap_err().message.contains("not installed"));
        let nowhere = open_in_editor(bin.as_os_str(), "subl", "/nonexistent-folder");
        assert!(nowhere.is_err());
        let _ = std::fs::remove_dir_all(&bin);
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn only_a_folder_is_shown_in_the_file_manager() {
        assert!(open_folder("/nonexistent-folder").is_err());
        assert!(open_folder("").is_err());
    }

    #[test]
    fn this_machine_has_a_name() {
        assert!(!hostname().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn only_web_and_mail_addresses_are_opened() {
        let bin = temp("open-url-bin");
        let log = bin.join("log");
        let body = format!("#!/bin/sh\necho \"$1\" > {}\n", log.display());
        script(&bin, "opener", &body);
        let opener = bin.join("opener");

        for refused in [
            "file:///etc/passwd",
            "javascript:alert(1)",
            "ssh://host",
            "/etc/passwd",
            "https://a.example/\nb",
            "",
        ] {
            assert!(open_url_with(opener.as_os_str(), refused).is_err(), "{refused}");
        }
        assert!(!log.exists());

        open_url_with(opener.as_os_str(), "https://example.com/a?b=1").unwrap();
        let mut seen = String::new();
        for _ in 0..100 {
            seen = std::fs::read_to_string(&log).unwrap_or_default();
            if !seen.is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_eq!(seen.trim(), "https://example.com/a?b=1");
        let _ = std::fs::remove_dir_all(&bin);
    }
}
