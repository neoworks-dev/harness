use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;

/// The daemon's Unix socket: `$NEO_SOCKET`, else `$XDG_RUNTIME_DIR/neo/bus.sock`, else
/// `/tmp/neo-<uid>/bus.sock`. The daemon and the other SDKs resolve it the same way.
pub fn default_socket_path() -> PathBuf {
    resolve(|name| std::env::var(name).ok(), current_uid())
}

fn resolve(getenv: impl Fn(&str) -> Option<String>, uid: u32) -> PathBuf {
    let set = |name| getenv(name).filter(|value| !value.is_empty());
    if let Some(socket) = set("NEO_SOCKET") {
        return socket.into();
    }
    if let Some(dir) = set("XDG_RUNTIME_DIR") {
        return PathBuf::from(dir).join("neo").join("bus.sock");
    }
    PathBuf::from(format!("/tmp/neo-{uid}")).join("bus.sock")
}

/// The effective user id. std has no getuid, and the SDK avoids a libc dependency for one
/// number: the owner of /proc/self is this process's user (Linux), and the home directory is
/// the next best thing elsewhere.
fn current_uid() -> u32 {
    [
        "/proc/self".into(),
        std::env::var("HOME").unwrap_or_default(),
    ]
    .iter()
    .find_map(|path| std::fs::metadata(path).ok())
    .map_or(0, |meta| meta.uid())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        |name| {
            pairs
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| value.to_string())
        }
    }

    #[test]
    fn socket_precedence() {
        let both = [("NEO_SOCKET", "/a/b.sock"), ("XDG_RUNTIME_DIR", "/run/u")];
        assert_eq!(resolve(env(&both), 7), PathBuf::from("/a/b.sock"));
        let xdg = [("XDG_RUNTIME_DIR", "/run/u")];
        assert_eq!(resolve(env(&xdg), 7), PathBuf::from("/run/u/neo/bus.sock"));
        assert_eq!(resolve(env(&[]), 7), PathBuf::from("/tmp/neo-7/bus.sock"));
        let empty = [("NEO_SOCKET", "")];
        assert_eq!(
            resolve(env(&empty), 7),
            PathBuf::from("/tmp/neo-7/bus.sock")
        );
    }
}
