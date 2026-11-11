package main

import (
	"context"
	"errors"
	"fmt"
	"os"
	osexec "os/exec"
	"path/filepath"
	"strings"

	sdk "neoworks.dev/neo/sdk-go"
)

// guiBundle is the desktop app's UI bundle relative to the repo root (bun apps/gui/build.mjs).
var guiBundle = filepath.Join("apps", "gui", "dist", "gui.js")

// webBuild is the same UI built for a browser tab, relative to the repo root (bun apps/gui/build.mjs --web).
var webBuild = filepath.Join("apps", "gui", "dist", "web")

// guiTarget is what `neo gui` and `neo web` do differently: both start neo-gui on a build of the UI.
type guiTarget struct {
	command string
	// built is a file of the build, relative to the repo root; the build is missing without it.
	built string
	// missing says what to run when it is.
	missing string
	// argv is neo-gui's command line, given the built file's path.
	argv func(gui, built string) []string
}

var (
	// The window: `neo-gui --bundle gui.js`.
	desktopTarget = guiTarget{
		command: "gui",
		built:   guiBundle,
		missing: "the desktop app's bundle is missing (%s): run bun apps/gui/build.mjs, or task build",
		argv:    func(gui, built string) []string { return []string{gui, "--bundle", built} },
	}
	// A browser tab: `neo-gui --web dist/web`, which serves the build, opens the browser on it and
	// stays running as long as the tab needs it.
	browserTarget = guiTarget{
		command: "web",
		built:   filepath.Join(webBuild, "index.html"),
		missing: "the desktop app's web build is missing (%s): run bun apps/gui/build.mjs --web",
		argv:    func(gui, built string) []string { return []string{gui, "--web", filepath.Dir(built)} },
	}
)

// errGuiMissing means the desktop app isn't built or installed.
var errGuiMissing = errors.New("neo-gui isn't built: run task build, or use neo attach")

// resolveGui finds the desktop app: NEO_GUI, else neo-gui next to the running executable,
// else on PATH. The same search as the terminal app's, for another name.
func resolveGui(getenv func(string) string, exe string, lookPath func(string) (string, error)) (string, error) {
	return resolveApp("NEO_GUI", "neo-gui", errGuiMissing, getenv, exe, lookPath)
}

// guiEnviron is base with the session choice (and the socket, when it was given) for the window.
func guiEnviron(base []string, session string, newSession bool, socket string) []string {
	env := make([]string, 0, len(base)+3)
	for _, kv := range base {
		key, _, _ := strings.Cut(kv, "=")
		if key == "NEO_GUI_SESSION" || key == "NEO_GUI_NEW" {
			continue
		}
		if key == "NEO_SOCKET" && socket != "" {
			continue
		}
		env = append(env, kv)
	}
	if session != "" {
		env = append(env, "NEO_GUI_SESSION="+session)
	}
	if newSession {
		env = append(env, "NEO_GUI_NEW=1")
	}
	if socket != "" {
		env = append(env, "NEO_SOCKET="+socket)
	}
	return env
}

// runGui is `neo gui`: start the daemon if needed (saying so only then), then open the desktop
// app on the session asked for and leave it running on its own, so the terminal is free.
func runGui(ctx context.Context, args []string, e cliEnv) int {
	return launchGui(ctx, args, e, desktopTarget)
}

// runWeb is `neo web`: the same app for a browser. It starts the daemon if needed, then neo-gui
// in web mode, which opens the default browser on the page with the daemon's token in the URL
// fragment (so it reaches no log or referrer) and keeps serving the page, in the background.
func runWeb(ctx context.Context, args []string, e cliEnv) int {
	return launchGui(ctx, args, e, browserTarget)
}

func launchGui(ctx context.Context, args []string, e cliEnv, target guiTarget) int {
	fs, socket := commandFlags(target.command, e)
	session := fs.String("session", "", "open this session (id or prefix)")
	newSession := fs.Bool("new", false, "start a new session in the current directory")
	positional, err := parseFlags(fs, args)
	if err != nil {
		return 2
	}
	if len(positional) > 0 || (*newSession && *session != "") {
		fmt.Fprintf(e.Stderr, "usage: neo %s [--session ID | --new]\n", target.command)
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
		fmt.Fprintf(e.Stderr, "neo: %v\n", err)
		return 1
	}
	root, err := daemonRoot(e.Getenv, e.Executable)
	if err != nil {
		fmt.Fprintf(e.Stderr, "neo: %v\n", err)
		return 1
	}
	built := filepath.Join(root, target.built)
	if st, err := os.Stat(built); err != nil || st.IsDir() {
		fmt.Fprintf(e.Stderr, "neo: "+target.missing+"\n", built)
		return 1
	}

	path := sdk.SocketPath(*socket, e.Getenv, e.UID)
	pid, started, logPath, err := ensureDaemon(ctx, path, e)
	if err != nil {
		fmt.Fprintf(e.Stderr, "neo: %v\n", err)
		return 1
	}
	if started {
		fmt.Fprintf(e.Stdout, "daemon started (pid %d, log %s)\n", pid, logPath)
	}

	argv := target.argv(gui, built)
	env := guiEnviron(e.environ(), *session, *newSession, *socket)
	spawn := e.Spawn
	if spawn == nil {
		spawn = func(path string, argv, env []string) error { return spawnDetached(path, argv, env, dataDir(e.Getenv)) }
	}
	if err := spawn(gui, argv, env); err != nil {
		fmt.Fprintf(e.Stderr, "neo: cannot run %s: %v\n", gui, err)
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
