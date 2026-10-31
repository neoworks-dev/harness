//! Speech to text: holding Space in the editor records from the microphone, letting go stops,
//! and the daemon writes out what was said (`speech.transcribe`), which goes into the editor at
//! the cursor. A held Space is told from typed ones by the terminal's key repeat: kitty's
//! keyboard protocol says so (a repeat event, and a release when it is let go); elsewhere
//! Spaces coming faster than anyone types are a held one, and their stopping is letting go. The
//! Spaces it typed before that was clear are taken back out. The
//! recording is 16 kHz mono WAV from PipeWire's `pw-record`, else PulseAudio's `parecord`, else
//! ALSA's `arecord`, in `$TMPDIR/neo-term/voice-<ms>.wav`, deleted once read.

use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// The recorders tried in turn: the program and its arguments before the file.
const RECORDERS: [(&str, &[&str]); 3] = [
    (
        "pw-record",
        &["--rate", "16000", "--channels", "1", "--format", "s16"],
    ),
    (
        "parecord",
        &["--rate=16000", "--channels=1", "--file-format=wav"],
    ),
    (
        "arecord",
        &["-q", "-f", "S16_LE", "-r", "16000", "-c", "1", "-t", "wav"],
    ),
];
/// How long a recorder gets to finish its file once told to stop.
const STOP_WAIT: Duration = Duration::from_secs(3);
/// Shorter than this (a WAV header and a few milliseconds), nothing was said.
const MIN_BYTES: usize = 3200;

/// A recording in progress.
pub struct Recording {
    child: Child,
    path: PathBuf,
}

impl Recording {
    /// Starts recording with the first recorder that is installed.
    pub fn start() -> Result<Recording, String> {
        let dir = std::env::temp_dir().join("neo-term");
        std::fs::create_dir_all(&dir).map_err(|e| format!("can't record: {e}"))?;
        let ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis());
        let path = dir.join(format!("voice-{ms}.wav"));
        for (program, args) in RECORDERS {
            let spawned = Command::new(program)
                .args(args)
                .arg(&path)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn();
            if let Ok(child) = spawned {
                return Ok(Recording { child, path });
            }
        }
        Err(
            "no recorder: install pw-record (PipeWire), parecord (PulseAudio) or arecord (ALSA)"
                .into(),
        )
    }

    /// Stops recording (SIGINT, so the recorder finishes the WAV header) and gives the file's
    /// bytes; None when nothing was recorded.
    pub fn stop(mut self) -> Result<Option<Vec<u8>>, String> {
        let _ = Command::new("kill")
            .arg("-INT")
            .arg(self.child.id().to_string())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let deadline = Instant::now() + STOP_WAIT;
        while self.child.try_wait().ok().flatten().is_none() {
            if Instant::now() > deadline {
                let _ = self.child.kill();
                let _ = self.child.wait();
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let bytes = std::fs::read(&self.path);
        let _ = std::fs::remove_file(&self.path);
        match bytes {
            Ok(bytes) if bytes.len() >= MIN_BYTES => Ok(Some(bytes)),
            Ok(_) => Ok(None),
            Err(e) => Err(format!("the recording is gone: {e}")),
        }
    }
}

impl Drop for Recording {
    /// A recording left running (term quits) stops with it.
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Spaces this close together are one run: the key repeat's first delay fits in it.
const RUN_GAP_MS: i64 = 800;
/// Spaces this close together are the key repeating, not typing.
const REPEAT_GAP_MS: i64 = 120;
/// Repeats in a row that make a held Space.
const REPEATS: usize = 2;
/// No repeat for this long while recording: Space was let go.
pub const RELEASE_MS: i64 = 400;

/// What a Space press is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpaceKey {
    /// Typed: it goes into the editor.
    Type,
    /// Held down: recording starts, and the `typed` Spaces of this run come back out.
    Hold { typed: usize },
}

/// The Spaces just pressed in the editor, to tell a held one from typed ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SpaceRun {
    last_ms: Option<i64>,
    typed: usize,
    repeats: usize,
}

impl SpaceRun {
    /// A Space pressed at `now_ms`; `repeat` when the terminal said it repeats (kitty).
    pub fn press(&mut self, now_ms: i64, repeat: bool) -> SpaceKey {
        match self.last_ms.map(|last| now_ms - last) {
            Some(gap) if gap < RUN_GAP_MS => {
                if gap < REPEAT_GAP_MS {
                    self.repeats += 1;
                }
            }
            _ => *self = SpaceRun::default(),
        }
        self.last_ms = Some(now_ms);
        if repeat || self.repeats >= REPEATS {
            let typed = self.typed;
            *self = SpaceRun {
                last_ms: Some(now_ms),
                ..SpaceRun::default()
            };
            return SpaceKey::Hold { typed };
        }
        self.typed += 1;
        SpaceKey::Type
    }

    /// Space still held (a repeat while recording).
    pub fn held(&mut self, now_ms: i64) {
        self.last_ms = Some(now_ms);
    }

    /// Whether Space was let go: no repeat came for a while.
    pub fn released(&self, now_ms: i64) -> bool {
        self.last_ms.is_none_or(|last| now_ms - last > RELEASE_MS)
    }
}

/// What the voice input is doing, for the row over the editor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Voice {
    #[default]
    Off,
    /// Recording since then (ms).
    Recording(i64),
    /// The daemon is writing it out.
    Transcribing,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_spaces_stay_typed() {
        let mut run = SpaceRun::default();
        // A word, a space, a word, two spaces typed quickly but not repeat-fast.
        for at in [0, 400, 1000, 1150] {
            assert_eq!(run.press(at, false), SpaceKey::Type);
        }
    }

    #[test]
    fn a_held_space_is_told_by_its_repeats() {
        let mut run = SpaceRun::default();
        assert_eq!(run.press(0, false), SpaceKey::Type);
        // The key repeat's delay, then a repeat every 33 ms.
        assert_eq!(run.press(500, false), SpaceKey::Type);
        assert_eq!(run.press(533, false), SpaceKey::Type);
        assert_eq!(run.press(566, false), SpaceKey::Hold { typed: 3 });
        run.held(600);
        assert!(!run.released(700));
        assert!(run.released(1001));
    }

    #[test]
    fn kitty_says_it_repeats() {
        let mut run = SpaceRun::default();
        assert_eq!(run.press(0, false), SpaceKey::Type);
        assert_eq!(run.press(500, true), SpaceKey::Hold { typed: 1 });
    }
}
