package main

import (
	"fmt"
	"path/filepath"
)

// socketPath picks the daemon socket: --socket, then $SUITE_SOCKET, then the
// per-user runtime dir, then a per-uid directory under /tmp.
func socketPath(flagValue string, getenv func(string) string, uid int) string {
	if flagValue != "" {
		return flagValue
	}
	if p := getenv("SUITE_SOCKET"); p != "" {
		return p
	}
	if dir := getenv("XDG_RUNTIME_DIR"); dir != "" {
		return filepath.Join(dir, "suite", "bus.sock")
	}
	return filepath.Join(fmt.Sprintf("/tmp/suite-%d", uid), "bus.sock")
}
