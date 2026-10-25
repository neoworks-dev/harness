package main

import (
	"context"
	"os"
	osexec "os/exec"
	"path/filepath"
	"slices"
	"strings"
	"testing"
	"time"

	"neoworks.dev/suite/sdk-go/sdktest"
)

// guiFixture is a repo with the desktop app's bundle, and an executable standing in for suite-gui.
type guiFixture struct {
	root, bundle, gui string
}

func newGuiFixture(t *testing.T) guiFixture {
	t.Helper()
	root := t.TempDir()
	bundle := filepath.Join(root, "apps", "gui", "dist", "gui.js")
	if err := os.MkdirAll(filepath.Dir(bundle), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(bundle, []byte("// bundle"), 0o644); err != nil {
		t.Fatal(err)
	}
	return guiFixture{root: root, bundle: bundle, gui: writeScript(t, "exit 0")}
}

// spawned is what runGui asked to start.
type spawned struct {
	path      string
	argv, env []string
	calls     int
	fail      error
}

func (s *spawned) spawn(path string, argv, env []string) error {
	s.calls++
	s.path, s.argv, s.env = path, argv, env
	return s.fail
}

// guiEnv is an environment for `suite gui` against a running test daemon.
func guiEnv(t *testing.T, f guiFixture) *upEnv {
	t.Helper()
	d := sdktest.New(t)
	u := newUpEnv(t)
	u.socket = d.Socket
	u.vars["SUITE_SOCKET"] = d.Socket
	u.vars["SUITE_ROOT"] = f.root
	u.vars["SUITE_GUI"] = f.gui
	// A daemon already answers; without this the daemon's age is checked against the repo's sources.
	u.vars["SUITE_DAEMON_CMD"] = "true"
	return u
}

func runGuiWith(t *testing.T, u *upEnv, s *spawned, environ []string, args ...string) int {
	t.Helper()
	u.stdout.Reset()
	u.stderr.Reset()
	e := u.cli()
	e.Spawn = s.spawn
	e.Environ = func() []string { return environ }
	return run(context.Background(), append([]string{"gui"}, args...), e)
}

func TestGuiStartsTheWindowOnTheBundleAndLeavesTheTerminalFree(t *testing.T) {
	f := newGuiFixture(t)
	s := &spawned{}
	u := guiEnv(t, f)

	if code := runGuiWith(t, u, s, []string{"A=1"}); code != 0 {
		t.Fatalf("code %d, stderr %q", code, u.stderr.String())
	}
	if s.path != f.gui || !slices.Equal(s.argv, []string{f.gui, "--bundle", f.bundle}) {
		t.Errorf("spawn(%q, %q)", s.path, s.argv)
	}
	// No session asked for: the window picks (the agent's session, else the latest here).
	if !slices.Equal(s.env, []string{"A=1"}) {
		t.Errorf("env %q", s.env)
	}
	if u.stdout.String() != "" {
		t.Errorf("a running daemon and a started window should be silent, got %q", u.stdout.String())
	}
}

func TestGuiPassesTheSessionChoiceOnAsEnvironmentVariables(t *testing.T) {
	f := newGuiFixture(t)
	s := &spawned{}
	u := guiEnv(t, f)
	// What a parent shell left behind must not choose the session.
	stale := []string{"A=1", "SUITE_GUI_SESSION=old", "SUITE_GUI_NEW=1"}

	if code := runGuiWith(t, u, s, stale, "--session", "s_ab"); code != 0 {
		t.Fatalf("code %d, stderr %q", code, u.stderr.String())
	}
	if !slices.Equal(s.env, []string{"A=1", "SUITE_GUI_SESSION=s_ab"}) {
		t.Errorf("--session: env %q", s.env)
	}

	if code := runGuiWith(t, u, s, stale, "--new"); code != 0 {
		t.Fatalf("code %d, stderr %q", code, u.stderr.String())
	}
	if !slices.Equal(s.env, []string{"A=1", "SUITE_GUI_NEW=1"}) {
		t.Errorf("--new: env %q", s.env)
	}

	// A socket given on the command line is the window's too.
	if code := runGuiWith(t, u, s, []string{"SUITE_SOCKET=/other"}, "--socket", u.socket); code != 0 {
		t.Fatalf("code %d, stderr %q", code, u.stderr.String())
	}
	if !slices.Equal(s.env, []string{"SUITE_SOCKET=" + u.socket}) {
		t.Errorf("--socket: env %q", s.env)
	}
}

func TestGuiArgumentsAreChecked(t *testing.T) {
	f := newGuiFixture(t)
	s := &spawned{}
	u := guiEnv(t, f)
	for _, args := range [][]string{
		{"--new", "--session", "s_a"},
		{"s_a"},
		{"--bogus"},
	} {
		if code := runGuiWith(t, u, s, nil, args...); code != 2 {
			t.Errorf("%q: code %d, want 2 (stderr %q)", args, code, u.stderr.String())
		}
	}
	if s.calls != 0 {
		t.Errorf("a bad command line started the window %d times", s.calls)
	}
	if !strings.Contains(usage, "\n  gui ") {
		t.Error("usage does not list gui")
	}
}

func TestGuiSaysWhatIsMissingAndStartsNoDaemonForIt(t *testing.T) {
	f := newGuiFixture(t)
	s := &spawned{}
	u := guiEnv(t, f)
	u.vars["SUITE_DAEMON_CMD"] = writeScript(t, "exec sleep 60")
	u.vars["SUITE_SOCKET"] = filepath.Join(t.TempDir(), "none.sock")
	u.socket = u.vars["SUITE_SOCKET"]
	u.vars["SUITE_DATA_DIR"] = u.dataDir

	// No suite-gui anywhere.
	delete(u.vars, "SUITE_GUI")
	e := u.cli()
	e.Spawn = s.spawn
	e.LookPath = func(string) (string, error) { return "", osexec.ErrNotFound }
	if code := run(context.Background(), []string{"gui"}, e); code != 1 || !strings.Contains(u.stderr.String(), errGuiMissing.Error()) {
		t.Errorf("code %d, stderr %q", code, u.stderr.String())
	}

	// No bundle.
	u.stderr.Reset()
	u.vars["SUITE_GUI"] = f.gui
	if err := os.Remove(f.bundle); err != nil {
		t.Fatal(err)
	}
	if code := run(context.Background(), []string{"gui"}, e); code != 1 || !strings.Contains(u.stderr.String(), "bundle is missing") {
		t.Errorf("code %d, stderr %q", code, u.stderr.String())
	}
	if _, err := os.Stat(filepath.Join(u.dataDir, "daemon.log")); err == nil {
		t.Error("the daemon was started although the window can't open")
	}
	if s.calls != 0 {
		t.Error("the window was started")
	}
}

func TestGuiReportsAWindowThatWillNotStart(t *testing.T) {
	f := newGuiFixture(t)
	s := &spawned{fail: os.ErrPermission}
	u := guiEnv(t, f)
	if code := runGuiWith(t, u, s, nil); code != 1 || !strings.Contains(u.stderr.String(), "cannot run "+f.gui) {
		t.Errorf("code %d, stderr %q", code, u.stderr.String())
	}
}

func TestResolveGui(t *testing.T) {
	bin := t.TempDir()
	exe := filepath.Join(bin, "suite")
	nextTo := filepath.Join(bin, "suite-gui")
	custom := filepath.Join(t.TempDir(), "custom-gui")
	onPath := filepath.Join(t.TempDir(), "suite-gui")
	for _, p := range []string{exe, nextTo, custom, onPath} {
		if err := os.WriteFile(p, []byte("#!/bin/sh\n"), 0o755); err != nil {
			t.Fatal(err)
		}
	}
	look := func(name string) (string, error) {
		if name == "suite-gui" {
			return onPath, nil
		}
		return "", osexec.ErrNotFound
	}
	none := func(string) (string, error) { return "", osexec.ErrNotFound }
	env := func(kv string) func(string) string {
		return func(k string) string {
			if name, v, _ := strings.Cut(kv, "="); k == name {
				return v
			}
			return ""
		}
	}

	tests := []struct {
		name    string
		getenv  func(string) string
		exe     string
		look    func(string) (string, error)
		want    string
		wantErr string
	}{
		{"SUITE_GUI path wins", env("SUITE_GUI=" + custom), exe, look, custom, ""},
		{"SUITE_GUI missing", env("SUITE_GUI=/nonexistent/x"), exe, look, "", "SUITE_GUI=/nonexistent/x"},
		{"next to the executable", env(""), exe, look, nextTo, ""},
		{"PATH", env(""), filepath.Join(t.TempDir(), "suite"), look, onPath, ""},
		{"missing", env(""), filepath.Join(t.TempDir(), "suite"), none, "", errGuiMissing.Error()},
	}
	for _, tc := range tests {
		got, err := resolveGui(tc.getenv, tc.exe, tc.look)
		if tc.wantErr != "" {
			if err == nil || !strings.Contains(err.Error(), tc.wantErr) {
				t.Errorf("%s: err = %v, want %q", tc.name, err, tc.wantErr)
			}
			continue
		}
		if err != nil || got != tc.want {
			t.Errorf("%s: got %q, %v; want %q", tc.name, got, err, tc.want)
		}
	}
}

func TestSpawnDetachedWritesTheLogAndDoesNotWait(t *testing.T) {
	dir := t.TempDir()
	logDir := filepath.Join(dir, "data")
	script := writeScript(t, `echo "args: $*"; echo "session: $SUITE_GUI_SESSION"`)
	if err := spawnDetached(script, []string{script, "--bundle", "x.js"}, []string{"SUITE_GUI_SESSION=s_1"}, logDir); err != nil {
		t.Fatal(err)
	}
	var got string
	for range 100 {
		data, _ := os.ReadFile(filepath.Join(logDir, "gui.log"))
		got = string(data)
		if strings.Contains(got, "session: s_1") {
			break
		}
		<-timeAfter(50)
	}
	if !strings.Contains(got, "args: --bundle x.js") || !strings.Contains(got, "session: s_1") {
		t.Errorf("gui.log %q", got)
	}
}

func timeAfter(ms int) <-chan time.Time { return time.After(time.Duration(ms) * time.Millisecond) }
