package main

import (
	"context"
	"errors"
	"fmt"
	"os"
	osexec "os/exec"
	"path/filepath"
	"strings"

	sdk "neoworks.dev/suite/sdk-go"
)

// guiBundle is the desktop app's UI bundle relative to the repo root (bun apps/gui/build.mjs).
var guiBundle = filepath.Join("apps", "gui", "dist", "gui.js")

// errGuiMissing means the desktop app isn't built or installed.
var errGuiMissing = errors.New("suite-gui isn't built: run make build, or use suite attach")

// resolveGui finds the desktop app: SUITE_GUI, else suite-gui next to the running executable,
// else on PATH. The same search as the terminal app's, for another name.
func resolveGui(getenv func(string) string, exe string, lookPath func(string) (string, error)) (string, error) {
	return resolveApp("SUITE_GUI", "suite-gui", errGuiMissing, getenv, exe, lookPath)
}

// guiEnviron is base with the session choice (and the socket, when it was given) for the window.
func guiEnviron(base []string, session string, newSession bool, socket string) []string {
	env := make([]string, 0, len(base)+3)
	for _, kv := range base {
		key, _, _ := strings.Cut(kv, "=")
		if key == "SUITE_GUI_SESSION" || key == "SUITE_GUI_NEW" {
			continue
		}
		if key == "SUITE_SOCKET" && socket != "" {
			continue
		}
		env = append(env, kv)
	}
	if session != "" {
		env = append(env, "SUITE_GUI_SESSION="+session)
	}
	if newSession {
		env = append(env, "SUITE_GUI_NEW=1")
	}
	if socket != "" {
		env = append(env, "SUITE_SOCKET="+socket)
	}
	return env
}

// runGui is `suite gui`: start the daemon if needed (saying so only then), then open the desktop
// app on the session asked for and leave it running on its own, so the terminal is free.
func runGui(ctx context.Context, args []string, e cliEnv) int {
	fs, socket := commandFlags("gui", e)
	session := fs.String("session", "", "open this session (id or prefix)")
	newSession := fs.Bool("new", false, "start a new session in the current directory")
	positional, err := parseFlags(fs, args)
	if err != nil {
		return 2
	}
	if len(positional) > 0 || (*newSession && *session != "") {
		fmt.Fprint(e.Stderr, "usage: suite gui [--session ID | --new]\n")
		return 2
	}

	exe := ""
	if e.Executable != nil {
		exe, _ = e.Executable()
	} else {
		exe, _ = os.Executable()
	}
	lookPath := e.LookPath
	if lookPath == nil {
		lookPath = osexec.LookPath
	}
	// Resolve first so a missing app doesn't start a daemon for nothing.
	gui, err := resolveGui(e.Getenv, exe, lookPath)
	if err != nil {
		fmt.Fprintf(e.Stderr, "suite: %v\n", err)
		return 1
	}
	root, err := daemonRoot(e.Getenv, e.Executable)
	if err != nil {
		fmt.Fprintf(e.Stderr, "suite: %v\n", err)
		return 1
	}
	bundle := filepath.Join(root, guiBundle)
	if st, err := os.Stat(bundle); err != nil || st.IsDir() {
		fmt.Fprintf(e.Stderr, "suite: the desktop app's bundle is missing (%s): run bun apps/gui/build.mjs, or make build\n", bundle)
		return 1
	}

	path := sdk.SocketPath(*socket, e.Getenv, e.UID)
	pid, started, logPath, err := ensureDaemon(ctx, path, e)
	if err != nil {
		fmt.Fprintf(e.Stderr, "suite: %v\n", err)
		return 1
	}
	if started {
		fmt.Fprintf(e.Stdout, "daemon started (pid %d, log %s)\n", pid, logPath)
	}

	argv := []string{gui, "--bundle", bundle}
	env := guiEnviron(e.environ(), *session, *newSession, *socket)
	spawn := e.Spawn
	if spawn == nil {
		spawn = func(path string, argv, env []string) error { return spawnDetached(path, argv, env, dataDir(e.Getenv)) }
	}
	if err := spawn(gui, argv, env); err != nil {
		fmt.Fprintf(e.Stderr, "suite: cannot run %s: %v\n", gui, err)
		return 1
	}
	return 0
}

// spawnDetached starts argv in a session of its own, with its output in gui.log in the data
// directory, and does not wait for it: it outlives this process and the terminal.
func spawnDetached(path string, argv, env []string, logDir string) error {
	if err := os.MkdirAll(logDir, 0o700); err != nil {
		return err
	}
	logf, err := os.OpenFile(filepath.Join(logDir, "gui.log"), os.O_WRONLY|os.O_CREATE|os.O_APPEND, 0o600)
	if err != nil {
		return err
	}
	defer logf.Close()
	devnull, err := os.Open(os.DevNull)
	if err != nil {
		return err
	}
	defer devnull.Close()

	cmd := osexec.Command(path, argv[1:]...)
	cmd.Stdin, cmd.Stdout, cmd.Stderr = devnull, logf, logf
	cmd.Env = env
	detach(cmd)
	if err := cmd.Start(); err != nil {
		return err
	}
	return cmd.Process.Release()
}
