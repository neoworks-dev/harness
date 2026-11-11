//! Background commands: a command Claude Code runs in the background ends its call at once and
//! writes its output to a file of its own (`Output is being written to: /tmp/…/<id>.output`),
//! which ends with `[exited with code N]` (or `[killed]`). The apps read that file in parts as
//! it grows, and stop the command by signalling the processes that write it.

use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use serde::Serialize;

/// Bytes read from an output file at a time, so a flood doesn't stall the app.
pub const READ_CHUNK: u64 = 256 * 1024;

/// What a read from `offset` found.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Chunk {
    /// The bytes read, as text (invalid UTF-8 replaced).
    pub text: String,
    /// Where the next read starts.
    pub offset: u64,
    /// The file isn't there (not written yet, or cleaned up).
    pub missing: bool,
}

/// Up to [`READ_CHUNK`] bytes of `path` from `offset` on. A multi-byte character cut at the
/// end of the chunk is left for the next read.
pub fn read_from(path: &Path, offset: u64) -> Chunk {
    let Ok(mut file) = std::fs::File::open(path) else {
        return Chunk {
            text: String::new(),
            offset,
            missing: true,
        };
    };
    if file.seek(SeekFrom::Start(offset)).is_err() {
        return Chunk {
            text: String::new(),
            offset,
            missing: false,
        };
    }
    let mut bytes = Vec::new();
    let _ = file.take(READ_CHUNK).read_to_end(&mut bytes);
    let whole = whole_chars(&bytes);
    bytes.truncate(whole);
    Chunk {
        text: String::from_utf8_lossy(&bytes).into_owned(),
        offset: offset + whole as u64,
        missing: false,
    }
}

/// How many of the bytes end in a whole UTF-8 character: a character cut at the end is left out.
fn whole_chars(bytes: &[u8]) -> usize {
    for back in 1..=bytes.len().min(3) {
        let byte = bytes[bytes.len() - back];
        if byte & 0xC0 == 0x80 {
            continue; // a continuation byte: look further back for the start
        }
        let needs = match byte {
            b if b >= 0xF0 => 4,
            b if b >= 0xE0 => 3,
            b if b >= 0xC0 => 2,
            _ => 1,
        };
        return if needs > back {
            bytes.len() - back
        } else {
            bytes.len()
        };
    }
    bytes.len()
}

/// Whether `path` is one a background command writes to: an absolute `….output` file. The apps
/// read and stop only those.
pub fn is_output_file(path: &Path) -> bool {
    path.is_absolute() && path.extension().is_some_and(|ext| ext == "output")
}

/// Stops the command writing `path`: SIGTERM to the process groups writing it (Claude Code runs
/// each in a group of its own and then ends the file with the exit code). The number of groups
/// signalled, or why there was none.
pub fn stop(path: &Path) -> Result<usize, String> {
    let groups = writers(path)?;
    if groups.is_empty() {
        return Err("no process writes its output".into());
    }
    for pgid in &groups {
        std::process::Command::new("kill")
            .args(["-TERM", "--", &format!("-{pgid}")])
            .status()
            .map_err(|e| format!("kill: {e}"))?;
    }
    Ok(groups.len())
}

/// The process groups of the processes whose output (fd 1 or 2) goes to `path`, found in
/// `/proc`; never this app's own group.
pub fn writers(path: &Path) -> Result<Vec<i64>, String> {
    let procs = std::fs::read_dir("/proc").map_err(|_| "no /proc to find its process in")?;
    let own = pgid_of("self");
    let mut groups = Vec::new();
    for entry in procs.flatten() {
        let name = entry.file_name();
        let Some(pid) = name
            .to_str()
            .filter(|n| n.bytes().all(|b| b.is_ascii_digit()))
        else {
            continue;
        };
        let writes = ["1", "2"].iter().any(|fd| {
            std::fs::read_link(format!("/proc/{pid}/fd/{fd}")).is_ok_and(|target| target == path)
        });
        if let Some(pgid) = pgid_of(pid).filter(|&g| writes && g > 1 && Some(g) != own)
            && !groups.contains(&pgid)
        {
            groups.push(pgid);
        }
    }
    Ok(groups)
}

/// A process's group, from `/proc/<pid>/stat` (its fifth field, after the name in parens).
fn pgid_of(pid: &str) -> Option<i64> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let (_, rest) = stat.rsplit_once(')')?;
    rest.split_whitespace().nth(2)?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("neo-jobs-{}-{name}.output", std::process::id()))
    }

    #[test]
    fn a_file_is_read_on_from_where_the_last_read_ended() {
        let path = temp("read");
        std::fs::write(&path, "one\ntwo").unwrap();
        let first = read_from(&path, 0);
        assert_eq!(
            (first.text.as_str(), first.offset, first.missing),
            ("one\ntwo", 7, false)
        );
        std::fs::write(&path, "one\ntwo\nthree\n").unwrap();
        assert_eq!(read_from(&path, first.offset).text, "\nthree\n");
        std::fs::remove_file(&path).unwrap();
        assert!(read_from(&path, 0).missing);
    }

    #[test]
    fn a_character_cut_at_the_end_waits_for_the_next_read() {
        let bytes = "aé".as_bytes();
        assert_eq!(whole_chars(&bytes[..2]), 1, "é's first byte alone");
        assert_eq!(whole_chars(bytes), 3);
        assert_eq!(whole_chars(b"abc"), 3);
    }

    #[test]
    fn only_absolute_output_files_are_jobs() {
        assert!(is_output_file(Path::new("/tmp/claude/tasks/b1.output")));
        assert!(!is_output_file(Path::new("tasks/b1.output")));
        assert!(!is_output_file(Path::new("/etc/passwd")));
    }

    #[test]
    fn a_file_nobody_writes_has_nothing_to_stop() {
        let path = temp("stop");
        std::fs::write(&path, "x").unwrap();
        assert!(stop(&path).is_err());
        std::fs::remove_file(&path).unwrap();
    }
}
