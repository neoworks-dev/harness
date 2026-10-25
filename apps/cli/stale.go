package main

import (
	"context"
	"io/fs"
	"path/filepath"
	"strings"
	"time"

	protocol "neoworks.dev/suite/protocol/go"
	sdk "neoworks.dev/suite/sdk-go"
)

// sourceDirs hold the code a running daemon has loaded (relative to the repo
// root). Apps and services run as their own processes, so they don't count.
var sourceDirs = []string{
	filepath.Join("daemon", "src"),
	filepath.Join("neo", "src"),
	filepath.Join("protocol", "gen"),
	filepath.Join("protocol", "schemas"),
	filepath.Join("protocol", "core-tools.json"),
}

// newestSource is the latest modification time of the daemon's TypeScript
// and JSON sources under root (zero if none are found).
func newestSource(root string) time.Time {
	var newest time.Time
	for _, dir := range sourceDirs {
		filepath.WalkDir(filepath.Join(root, dir), func(path string, d fs.DirEntry, err error) error {
			if err != nil {
				return nil
			}
			if d.IsDir() {
				if d.Name() == "node_modules" {
					return filepath.SkipDir
				}
				return nil
			}
			if !strings.HasSuffix(path, ".ts") && !strings.HasSuffix(path, ".json") {
				return nil
			}
			if info, err := d.Info(); err == nil && info.ModTime().After(newest) {
				newest = info.ModTime()
			}
			return nil
		})
	}
	return newest
}

// daemonState is what a running daemon says about itself.
type daemonState struct {
	pid       int
	startedAt time.Time
	// busy: a session's turn is running, so a restart would cut it off.
	busy bool
}

func inspect(ctx context.Context, socket string) (daemonState, error) {
	ctx, cancel := context.WithTimeout(ctx, 2*time.Second)
	defer cancel()
	c, err := sdk.Dial(ctx, sdk.Options{Socket: socket, Client: "cli"})
	if err != nil {
		return daemonState{}, err
	}
	defer c.Close()
	w := c.Welcome()
	st := daemonState{pid: int(w.PID), startedAt: time.UnixMilli(w.StartedAt)}
	sessions, err := c.ListSessions(ctx)
	if err != nil {
		return daemonState{}, err
	}
	for _, s := range sessions {
		if s.State == protocol.SessionStateRunning {
			st.busy = true
		}
	}
	return st, nil
}
