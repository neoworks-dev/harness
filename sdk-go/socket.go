package sdk

import (
	"fmt"
	"os"
	"path/filepath"
)

// SocketPath picks the daemon socket: override, then $SUITE_SOCKET, then the
// per-user runtime dir, then a per-uid directory under /tmp. getenv and uid
// are parameters so callers (and tests) need not touch the process.
func SocketPath(override string, getenv func(string) string, uid int) string {
	if override != "" {
		return override
	}
	if p := getenv("SUITE_SOCKET"); p != "" {
		return p
	}
	if dir := getenv("XDG_RUNTIME_DIR"); dir != "" {
		return filepath.Join(dir, "suite", "bus.sock")
	}
	return filepath.Join(fmt.Sprintf("/tmp/suite-%d", uid), "bus.sock")
}

// DefaultSocketPath is SocketPath for the current process.
func DefaultSocketPath() string {
	return SocketPath("", os.Getenv, os.Getuid())
}
