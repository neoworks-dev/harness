#![allow(dead_code)] // each test binary uses a different part

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use suite_sdk::Event;

/// A fresh scratch directory, removed on drop.
pub struct TempDir(pub PathBuf);

impl TempDir {
    pub fn new(label: &str) -> TempDir {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("suite-sdk-{label}-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        TempDir(dir)
    }

    pub fn join(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub const WAIT: Duration = Duration::from_secs(10);

/// Reads events until one satisfies `pick`, skipping the rest.
pub fn wait_for<T>(
    events: &async_channel::Receiver<Event>,
    what: &str,
    mut pick: impl FnMut(&Event) -> Option<T>,
) -> T {
    let deadline = std::time::Instant::now() + WAIT;
    loop {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        // The channel has no blocking-with-timeout receive; poll it.
        match events.try_recv() {
            Ok(event) => {
                if let Some(found) = pick(&event) {
                    return found;
                }
            }
            Err(async_channel::TryRecvError::Empty) => {
                assert!(!left.is_zero(), "timed out waiting for {what}");
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(async_channel::TryRecvError::Closed) => panic!("events closed waiting for {what}"),
        }
    }
}
