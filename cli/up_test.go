package main

import (
	"bytes"
	"context"
	"errors"
	"os"
	osexec "os/exec"
	"path/filepath"
	"runtime"
	"slices"
	"strconv"
	"strings"
	"syscall"
	"testing"
	"time"

	"neoworks.dev/suite/sdk-go/sdktest"
)

// upEnv is a CLI environment for up/down tests: a temp socket and data dir,
// and extra variables for both Getenv and the daemon's environment.
type upEnv struct {
	socket, dataDir string
	vars            map[string]string
	stdout, stderr  bytes.Buffer
	extra           []string // child-only environment
	timeout         time.Duration
}

func newUpEnv(t *testing.T) *upEnv {
	t.Helper()
	dir, err := os.MkdirTemp("", "suite")
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { os.RemoveAll(dir) })
	u := &upEnv{socket: filepath.Join(dir, "bus.sock"), dataDir: filepath.Join(dir, "data"), vars: map[string]string{}}
	u.vars["SUITE_SOCKET"] = u.socket
	u.vars["SUITE_DATA_DIR"] = u.dataDir
	// Never leak a stopped test's daemon: stop whatever answers on the socket.
	t.Cleanup(func() {
		pid, err := probe(context.Background(), u.socket)
		if err != nil || pid <= 0 {
			return
		}
		syscall.Kill(pid, syscall.SIGTERM)
		for range 100 {
			if _, err := probe(context.Background(), u.socket); err != nil {
				return
			}
			time.Sleep(50 * time.Millisecond)
		}
		syscall.Kill(pid, syscall.SIGKILL)
	})
	return u
}

func (u *upEnv) cli() cliEnv {
	return cliEnv{
		Stdin:  strings.NewReader(""),
		Stdout: &u.stdout, Stderr: &u.stderr,
		Getenv:     func(k string) string { return u.vars[k] },
		UID:        1000,
		Now:        time.Now,
		Environ:    func() []string { return append(os.Environ(), u.extra...) },
		UpTimeout:  u.timeout,
		Executable: func() (string, error) { return "/nonexistent/bin/suite", nil },
	}
}

// do runs one command and returns its exit code, stdout and stderr.
func (u *upEnv) do(t *testing.T, args ...string) (int, string, string) {
	t.Helper()
	u.stdout.Reset()
	u.stderr.Reset()
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()
	code := run(ctx, args, u.cli())
	return code, u.stdout.String(), u.stderr.String()
}

func repoRoot(t *testing.T) string {
	t.Helper()
	_, file, _, _ := runtime.Caller(0)
	for dir := filepath.Dir(file); dir != filepath.Dir(dir); dir = filepath.Dir(dir) {
		if _, err := os.Stat(filepath.Join(dir, "go.work")); err == nil {
			return dir
		}
	}
	t.Fatal("no go.work above " + file)
	return ""
}

func alive(pid int) bool { return syscall.Kill(pid, 0) == nil }

func writeScript(t *testing.T, body string) string {
	t.Helper()
	p := filepath.Join(t.TempDir(), "fake-daemon")
	if err := os.WriteFile(p, []byte("#!/bin/sh\n"+body+"\n"), 0o755); err != nil {
		t.Fatal(err)
	}
	return p
}

func TestUpDownRealDaemon(t *testing.T) {
	if _, err := osexec.LookPath("bun"); err != nil {
		t.Skip("bun is not installed; cannot start the real daemon")
	}
	u := newUpEnv(t)
	root := repoRoot(t)
	u.vars["SUITE_DAEMON_CMD"] = "bun run " + filepath.Join(root, "daemon", "src", "main.ts")
	u.extra = []string{"SUITE_AGENT=echo", "SUITE_SERVICES=none", "SUITE_GUESTS=none", "SUITE_PORT=0"}
	u.timeout = 20 * time.Second

	code, stdout, stderr := u.do(t, "up")
	logPath := filepath.Join(u.dataDir, "daemon.log")
	if code != 0 || stderr != "" {
		t.Fatalf("up: code %d, stderr %q", code, stderr)
	}
	prefix, suffix := "daemon started (pid ", ", log "+logPath+")\n"
	if !strings.HasPrefix(stdout, prefix) || !strings.HasSuffix(stdout, suffix) {
		t.Fatalf("up stdout = %q", stdout)
	}
	pid, err := strconv.Atoi(strings.TrimSuffix(strings.TrimPrefix(stdout, prefix), suffix))
	if err != nil || !alive(pid) {
		t.Fatalf("pid in %q: %v, alive %v", stdout, err, alive(pid))
	}
	if st, err := os.Stat(u.dataDir); err != nil || st.Mode().Perm() != 0o700 {
		t.Errorf("data dir: %v, %v", st, err)
	}
	if data, _ := os.ReadFile(logPath); !strings.Contains(string(data), "listening on") {
		t.Errorf("log lacks the daemon's output:\n%s", data)
	}
	if code, stdout, _ := u.do(t, "status"); code != 0 || !strings.Contains(stdout, "pid "+strconv.Itoa(pid)) {
		t.Errorf("status: code %d\n%s", code, stdout)
	}

	// A second up is a no-op.
	code, stdout, stderr = u.do(t, "up")
	if code != 0 || stderr != "" || !strings.Contains(stdout, "already running (pid "+strconv.Itoa(pid)+",") {
		t.Errorf("second up: code %d, stdout %q, stderr %q", code, stdout, stderr)
	}

	code, stdout, stderr = u.do(t, "down")
	if code != 0 || stderr != "" || !strings.Contains(stdout, "daemon stopped (pid "+strconv.Itoa(pid)+")") {
		t.Errorf("down: code %d, stdout %q, stderr %q", code, stdout, stderr)
	}
	if _, err := probe(context.Background(), u.socket); err == nil {
		t.Error("socket still answers after down")
	}
	eventually(t, "daemon process to exit", func() bool { return !alive(pid) })

	code, stdout, stderr = u.do(t, "down")
	if code != 0 || stdout != "daemon not running\n" || stderr != "" {
		t.Errorf("second down: code %d, stdout %q, stderr %q", code, stdout, stderr)
	}
}

func TestUpFailureShowsLog(t *testing.T) {
	u := newUpEnv(t)
	var body strings.Builder
	for i := 1; i <= 30; i++ {
		body.WriteString("echo line " + strconv.Itoa(i) + "\n")
	}
	body.WriteString("echo boom >&2\nexit 3")
	u.vars["SUITE_DAEMON_CMD"] = writeScript(t, body.String())
	u.timeout = 5 * time.Second

	code, stdout, stderr := u.do(t, "up")
	if code != 1 || stdout != "" {
		t.Fatalf("code %d, stdout %q", code, stdout)
	}
	for _, want := range []string{"daemon exited at startup", "daemon.log", "boom", "line 30", "line 12"} {
		if !strings.Contains(stderr, want) {
			t.Errorf("stderr missing %q:\n%s", want, stderr)
		}
	}
	if strings.Contains(stderr, "line 10\n") {
		t.Errorf("stderr has more than the last %d lines:\n%s", logTailLines, stderr)
	}
}

func TestUpTimeout(t *testing.T) {
	u := newUpEnv(t)
	u.vars["SUITE_DAEMON_CMD"] = writeScript(t, "echo starting\nexec sleep 60")
	u.timeout = 400 * time.Millisecond

	code, _, stderr := u.do(t, "up")
	if code != 1 || !strings.Contains(stderr, "did not answer within") || !strings.Contains(stderr, "starting") {
		t.Errorf("code %d, stderr %q", code, stderr)
	}
}

func TestUpCommandMissing(t *testing.T) {
	u := newUpEnv(t)
	u.vars["SUITE_DAEMON_CMD"] = "/nonexistent/daemon"
	code, _, stderr := u.do(t, "up")
	if code != 1 || !strings.Contains(stderr, "cannot start the daemon") {
		t.Errorf("code %d, stderr %q", code, stderr)
	}
	// And without a repo to find: the error names SUITE_ROOT.
	delete(u.vars, "SUITE_DAEMON_CMD")
	code, _, stderr = u.do(t, "up")
	if code != 1 || !strings.Contains(stderr, "SUITE_ROOT") {
		t.Errorf("code %d, stderr %q", code, stderr)
	}
}

func TestUpBadArgs(t *testing.T) {
	u := newUpEnv(t)
	for _, cmd := range []string{"up", "down"} {
		if code, _, stderr := u.do(t, cmd, "extra"); code != 2 || !strings.Contains(stderr, "usage: suite "+cmd) {
			t.Errorf("%s extra: code %d, stderr %q", cmd, code, stderr)
		}
	}
}

func TestNoArgsNotTerminalPrintsUsage(t *testing.T) {
	d := sdktest.New(t)
	code, stdout, stderr := exec(t, d.Socket, "")
	if code != 2 || stdout != "" || stderr != usage {
		t.Errorf("code %d, stdout %q, stderr %q", code, stdout, stderr)
	}
	for _, want := range []string{"\n  up ", "\n  down "} {
		if !strings.Contains(usage, want) {
			t.Errorf("usage lacks %q", want)
		}
	}
}

// ttyIn is a character device, which is all isTerminal looks at.
func ttyIn(t *testing.T) *os.File {
	t.Helper()
	f, err := os.Open(os.DevNull)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { f.Close() })
	return f
}

func TestNoArgsTerminalExecsTermApp(t *testing.T) {
	d := sdktest.New(t)
	term := writeScript(t, "exit 0")
	u := newUpEnv(t)
	u.socket = d.Socket
	u.vars["SUITE_SOCKET"] = d.Socket
	u.vars["SUITE_TERM"] = term

	var gotPath string
	var gotArgv, gotEnv []string
	e := u.cli()
	e.Stdin, e.StdoutTTY = ttyIn(t), true
	e.Environ = func() []string { return []string{"A=1"} }
	e.Exec = func(path string, argv, env []string) error {
		gotPath, gotArgv, gotEnv = path, argv, env
		return nil
	}
	if code := run(context.Background(), nil, e); code != 1 {
		t.Errorf("code %d (a returning exec is a failure)", code)
	}
	if gotPath != term || !slices.Equal(gotArgv, []string{term}) || !slices.Equal(gotEnv, []string{"A=1"}) {
		t.Errorf("exec(%q, %q, %q)", gotPath, gotArgv, gotEnv)
	}
	if u.stdout.String() != "" {
		t.Errorf("a running daemon should be silent, got %q", u.stdout.String())
	}

	// Exec failing reports why.
	u.stderr.Reset()
	e.Exec = func(string, []string, []string) error { return errors.New("exec format error") }
	if code := run(context.Background(), nil, e); code != 1 || !strings.Contains(u.stderr.String(), "exec format error") {
		t.Errorf("code %d, stderr %q", code, u.stderr.String())
	}
}

func TestNoArgsTerminalStartsDaemonAndMissingApp(t *testing.T) {
	u := newUpEnv(t)
	e := u.cli()
	e.Stdin, e.StdoutTTY = ttyIn(t), true
	e.Exec = func(string, []string, []string) error { t.Error("exec must not run"); return nil }
	e.LookPath = func(string) (string, error) { return "", osexec.ErrNotFound }

	// No suite-term: a clear message, and no daemon started for nothing.
	u.vars["SUITE_DAEMON_CMD"] = writeScript(t, "exec sleep 60")
	if code := run(context.Background(), nil, e); code != 1 || !strings.Contains(u.stderr.String(), "suite-term isn't built: run make build, or use suite attach") {
		t.Errorf("code %d, stderr %q", code, u.stderr.String())
	}
	if _, err := os.Stat(filepath.Join(u.dataDir, "daemon.log")); err == nil {
		t.Error("daemon was started although the app is missing")
	}
}

func TestResolveTerm(t *testing.T) {
	bin := t.TempDir()
	exe := filepath.Join(bin, "suite")
	nextTo := filepath.Join(bin, "suite-term")
	other := filepath.Join(t.TempDir(), "custom-term")
	onPath := filepath.Join(t.TempDir(), "suite-term")
	for _, p := range []string{exe, nextTo, other, onPath} {
		if err := os.WriteFile(p, []byte("#!/bin/sh\n"), 0o755); err != nil {
			t.Fatal(err)
		}
	}
	notExec := filepath.Join(t.TempDir(), "plain")
	os.WriteFile(notExec, nil, 0o644)

	look := func(name string) (string, error) {
		switch name {
		case "suite-term":
			return onPath, nil
		case "custom":
			return other, nil
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
	// A symlinked executable resolves to the directory of its target too.
	link := filepath.Join(t.TempDir(), "suite")
	if err := os.Symlink(exe, link); err != nil {
		t.Fatal(err)
	}

	tests := []struct {
		name    string
		getenv  func(string) string
		exe     string
		look    func(string) (string, error)
		want    string
		wantErr string
	}{
		{"SUITE_TERM path wins", env("SUITE_TERM=" + other), exe, look, other, ""},
		{"SUITE_TERM name via PATH", env("SUITE_TERM=custom"), exe, look, other, ""},
		{"SUITE_TERM missing", env("SUITE_TERM=/nonexistent/x"), exe, look, "", "SUITE_TERM=/nonexistent/x"},
		{"SUITE_TERM not executable", env("SUITE_TERM=" + notExec), exe, look, "", "SUITE_TERM="},
		{"SUITE_TERM unknown name", env("SUITE_TERM=nope"), exe, look, "", "SUITE_TERM=nope"},
		{"next to the executable", env(""), exe, look, nextTo, ""},
		{"next to a symlink target", env(""), link, none, nextTo, ""},
		{"PATH", env(""), filepath.Join(t.TempDir(), "suite"), look, onPath, ""},
		{"missing", env(""), filepath.Join(t.TempDir(), "suite"), none, "", errTermMissing.Error()},
	}
	for _, tc := range tests {
		got, err := resolveTerm(tc.getenv, tc.exe, tc.look)
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

func TestDaemonCommandAndRoot(t *testing.T) {
	root := t.TempDir()
	if err := os.MkdirAll(filepath.Join(root, "daemon", "src"), 0o755); err != nil {
		t.Fatal(err)
	}
	os.WriteFile(filepath.Join(root, daemonEntry), nil, 0o644)
	os.MkdirAll(filepath.Join(root, "bin"), 0o755)
	exe := filepath.Join(root, "bin", "suite")
	os.WriteFile(exe, nil, 0o755)
	// Installed elsewhere through a symlink: the real path decides.
	link := filepath.Join(t.TempDir(), "suite")
	if err := os.Symlink(exe, link); err != nil {
		t.Fatal(err)
	}
	want := []string{"bun", "run", filepath.Join(root, daemonEntry)}
	getenv := func(m map[string]string) func(string) string { return func(k string) string { return m[k] } }
	at := func(p string) func() (string, error) { return func() (string, error) { return p, nil } }

	for name, tc := range map[string]struct {
		env  map[string]string
		exe  string
		want []string
	}{
		"walks up from bin/suite": {nil, exe, want},
		"follows symlinks":        {nil, link, want},
		"SUITE_ROOT overrides":    {map[string]string{"SUITE_ROOT": root}, "/nowhere/suite", want},
		"SUITE_DAEMON_CMD wins":   {map[string]string{"SUITE_DAEMON_CMD": "  node  x.js\t--flag "}, exe, []string{"node", "x.js", "--flag"}},
	} {
		got, err := daemonCommand(getenv(tc.env), at(tc.exe))
		if err != nil || !slices.Equal(got, tc.want) {
			t.Errorf("%s: got %q, %v; want %q", name, got, err, tc.want)
		}
	}
	if _, err := daemonCommand(getenv(nil), at(filepath.Join(t.TempDir(), "suite"))); err == nil {
		t.Error("expected an error when no repo is found")
	}
}

func TestDataDir(t *testing.T) {
	for _, tc := range []struct {
		env  map[string]string
		want string
	}{
		{map[string]string{"SUITE_DATA_DIR": "/d", "XDG_DATA_HOME": "/x", "HOME": "/h"}, "/d"},
		{map[string]string{"XDG_DATA_HOME": "/x", "HOME": "/h"}, "/x/suite"},
		{map[string]string{"HOME": "/h"}, "/h/.local/share/suite"},
	} {
		if got := dataDir(func(k string) string { return tc.env[k] }); got != tc.want {
			t.Errorf("%v: got %q, want %q", tc.env, got, tc.want)
		}
	}
}
