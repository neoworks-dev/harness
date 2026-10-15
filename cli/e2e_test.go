package main

import (
	"bytes"
	"context"
	"io"
	"net"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"sync"
	"testing"
	"time"

	protocol "neoworks.dev/suite/protocol/go"
	"neoworks.dev/suite/sdk-go/sdktest"
)

// syncBuffer is a bytes.Buffer that a test can read while run is writing.
type syncBuffer struct {
	mu sync.Mutex
	b  bytes.Buffer
}

func (s *syncBuffer) Write(p []byte) (int, error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.b.Write(p)
}

func (s *syncBuffer) String() string {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.b.String()
}

// cli is one invocation of run against a socket.
type cli struct {
	stdout, stderr syncBuffer
	interrupt      chan os.Signal
	done           chan int
}

// start launches run in the background; stdin may stay open.
func start(t *testing.T, socket string, stdin io.Reader, args ...string) *cli {
	t.Helper()
	c := &cli{interrupt: make(chan os.Signal, 1), done: make(chan int, 1)}
	ctx, cancel := context.WithTimeout(context.Background(), 20*time.Second)
	t.Cleanup(cancel)
	go func() {
		c.done <- run(ctx, args, cliEnv{
			Stdin:     stdin,
			Stdout:    &c.stdout,
			Stderr:    &c.stderr,
			Getenv:    func(k string) string { return map[string]string{"SUITE_SOCKET": socket}[k] },
			UID:       1000,
			Interrupt: c.interrupt,
			Now:       time.Now,
		})
	}()
	return c
}

// wait returns the exit code, failing the test if the command hangs.
func (c *cli) wait(t *testing.T) int {
	t.Helper()
	select {
	case code := <-c.done:
		return code
	case <-time.After(10 * time.Second):
		t.Fatalf("command did not exit; stdout so far:\n%s\nstderr:\n%s", c.stdout.String(), c.stderr.String())
		return -1
	}
}

// exec runs a command to completion with the given piped stdin.
func exec(t *testing.T, socket, stdin string, args ...string) (code int, stdout, stderr string) {
	t.Helper()
	c := start(t, socket, strings.NewReader(stdin), args...)
	code = c.wait(t)
	return code, c.stdout.String(), c.stderr.String()
}

func eventually(t *testing.T, what string, cond func() bool) {
	t.Helper()
	sdktest.Eventually(t, what, cond)
}

func TestStatus(t *testing.T) {
	d := sdktest.New(t)
	d.AddSession("s1", "one", time.Now())
	d.AddSession("s2", "two", time.Now())
	d.SetSessionState("s2", protocol.SessionStateRunning, 0)
	d.AddSession("s3", "three", time.Now())

	code, stdout, stderr := exec(t, d.Socket, "", "status")
	if code != 0 || stderr != "" {
		t.Fatalf("code %d, stderr %q", code, stderr)
	}
	for _, want := range []string{"pid 4242", "version test", "uptime:    1h 30m", "protocol:  1", "socket:    " + d.Socket, "3 total, 1 running"} {
		if !strings.Contains(stdout, want) {
			t.Errorf("status output missing %q:\n%s", want, stdout)
		}
	}
}

func TestSessionsTable(t *testing.T) {
	d := sdktest.New(t)
	now := time.Now()
	d.AddSession("old", "", now.Add(-3*time.Hour), protocol.UserMessage{App: "cli", Text: "x"})
	d.AddSession("recent", "Fix the bug", now.Add(-2*time.Minute), protocol.UserMessage{App: "cli", Text: "x"}, protocol.TurnEnded{StopReason: protocol.StopReasonEndTurn})
	d.AddSession("busy", "Busy one", now.Add(-10*time.Second))
	d.SetSessionState("busy", protocol.SessionStateRunning, 2)

	code, stdout, stderr := exec(t, d.Socket, "", "sessions")
	if code != 0 || stderr != "" {
		t.Fatalf("code %d, stderr %q", code, stderr)
	}
	lines := strings.Split(strings.TrimSpace(stdout), "\n")
	if len(lines) != 4 {
		t.Fatalf("want header and 3 rows, got:\n%s", stdout)
	}
	want := [][]string{
		{"ID", "STATE", "QUEUED", "EVENTS", "LAST", "ACTIVITY", "TITLE"},
		{"busy", "running", "2", "0", "10s", "ago", "Busy", "one"},
		{"recent", "idle", "0", "2", "2m", "ago", "Fix", "the", "bug"},
		{"old", "idle", "0", "1", "3h", "ago", "/work/old"},
	}
	for i, line := range lines {
		if got := strings.Fields(line); strings.Join(got, " ") != strings.Join(want[i], " ") {
			t.Errorf("row %d: got %q, want %q", i, got, want[i])
		}
	}
}

func TestAttachNew(t *testing.T) {
	d := sdktest.New(t)
	// Blank lines are skipped; the second input is queued behind the first.
	code, stdout, stderr := exec(t, d.Socket, "hello\n\nworld\n", "attach", "--new", "--cwd", "/tmp/proj", "--title", "T", "--app", "tester")
	if code != 0 || stderr != "" {
		t.Fatalf("code %d, stderr %q", code, stderr)
	}
	want := "created session new1\n" +
		"-- attached to new1 --\n" +
		"\n» tester: hello\n\nyou said: hello\n" +
		"\n» tester: world\n\nyou said: world\n"
	if stdout != want {
		t.Errorf("stdout:\n%q\nwant:\n%q", stdout, want)
	}

	inputs := d.Inputs()
	if len(inputs) != 2 || inputs[0].Session != "new1" || inputs[0].App != "tester" || inputs[0].Text != "hello" || inputs[1].Text != "world" {
		t.Errorf("inputs = %+v", inputs)
	}
	s, _ := d.Session("new1")
	if s.Cwd != "/tmp/proj" || s.Title == nil || *s.Title != "T" {
		t.Errorf("session created as %+v", s)
	}
}

func TestAttachExistingReplaysHistory(t *testing.T) {
	d := sdktest.New(t)
	d.AddSession("abc123", "", time.Now(),
		protocol.UserMessage{App: "overlay", Text: "hi"},
		protocol.AgentThoughtChunk{MessageID: "t", Text: "thinking"},
		protocol.AgentMessageChunk{MessageID: "m", Text: "hello there"},
		protocol.TurnEnded{StopReason: protocol.StopReasonEndTurn},
	)
	d.AddSession("zzz", "", time.Now())

	code, stdout, stderr := exec(t, d.Socket, "", "attach", "abc")
	if code != 0 || stderr != "" {
		t.Fatalf("code %d, stderr %q", code, stderr)
	}
	want := "\n» overlay: hi\n\nhello there\n-- attached to abc123 --\n"
	if stdout != want {
		t.Errorf("stdout:\n%q\nwant:\n%q", stdout, want)
	}

	_, stdout, _ = exec(t, d.Socket, "", "attach", "--thoughts", "abc123")
	if !strings.Contains(stdout, "… thinking\n") {
		t.Errorf("--thoughts output missing reasoning:\n%s", stdout)
	}
	// Flags may follow the id.
	_, stdout, _ = exec(t, d.Socket, "", "attach", "abc123", "--thoughts")
	if !strings.Contains(stdout, "… thinking\n") {
		t.Errorf("trailing --thoughts ignored:\n%s", stdout)
	}
}

func TestAttachPrintsOtherClientsInput(t *testing.T) {
	d := sdktest.New(t)
	d.AddSession("s1", "", time.Now())
	stdin, stdinW := io.Pipe()
	c := start(t, d.Socket, stdin, "attach", "s1")
	eventually(t, "attach", func() bool { return strings.Contains(c.stdout.String(), "-- attached to s1 --") })

	d.Append("s1", protocol.UserMessage{App: "browser", Text: "from elsewhere"})
	eventually(t, "live user_message", func() bool { return strings.Contains(c.stdout.String(), "» browser: from elsewhere") })

	stdinW.Close()
	if code := c.wait(t); code != 0 {
		t.Errorf("exit %d", code)
	}
}

func TestAttachErrors(t *testing.T) {
	d := sdktest.New(t)
	d.AddSession("abc1", "", time.Now())
	d.AddSession("abc2", "", time.Now())

	tests := []struct {
		args   []string
		code   int
		stderr string
	}{
		{[]string{"attach", "abc"}, 1, "ambiguous"},
		{[]string{"attach", "nope"}, 1, "no session matches"},
		{[]string{"attach"}, 2, "usage"},
		{[]string{"attach", "--new", "abc1"}, 2, "usage"},
		{[]string{"attach", "--cwd", "/x", "abc1"}, 2, "only apply with --new"},
		{[]string{"bogus"}, 2, "unknown command"},
	}
	for _, tc := range tests {
		code, stdout, stderr := exec(t, d.Socket, "", tc.args...)
		if code != tc.code || !strings.Contains(stderr, tc.stderr) || stdout != "" {
			t.Errorf("%v: code %d stdout %q stderr %q", tc.args, code, stdout, stderr)
		}
	}
}

func TestInterrupt(t *testing.T) {
	d := sdktest.New(t)
	d.StepDelay = 300 * time.Millisecond
	d.AddSession("s1", "", time.Now())
	stdin, stdinW := io.Pipe()
	c := start(t, d.Socket, stdin, "attach", "s1")
	eventually(t, "attach", func() bool { return strings.Contains(c.stdout.String(), "-- attached") })

	// Idle: Ctrl-C exits without cancelling anything.
	idleIn, idleInW := io.Pipe()
	defer idleInW.Close()
	c2 := start(t, d.Socket, idleIn, "attach", "s1")
	eventually(t, "second attach", func() bool { return strings.Contains(c2.stdout.String(), "-- attached") })
	c2.interrupt <- os.Interrupt
	if code := c2.wait(t); code != 0 {
		t.Errorf("idle Ctrl-C exit %d", code)
	}
	if cancels := d.Cancels(); len(cancels) != 0 {
		t.Errorf("idle Ctrl-C sent cancel: %v", cancels)
	}

	// Running: Ctrl-C cancels the turn and keeps the client attached.
	stdinW.Write([]byte("work\n"))
	eventually(t, "turn start", func() bool { return strings.Contains(c.stdout.String(), "» cli: work") })
	c.interrupt <- os.Interrupt
	eventually(t, "cancel", func() bool { return slices.Equal(d.Cancels(), []string{"s1"}) })
	select {
	case code := <-c.done:
		t.Fatalf("exited with %d after cancel", code)
	default:
	}
	stdinW.Close()
	if code := c.wait(t); code != 0 {
		t.Errorf("exit %d", code)
	}
}

func TestConnectionLost(t *testing.T) {
	d := sdktest.New(t)
	d.AddSession("s1", "", time.Now())
	stdin, stdinW := io.Pipe()
	defer stdinW.Close()
	c := start(t, d.Socket, stdin, "attach", "s1")
	eventually(t, "attach", func() bool { return strings.Contains(c.stdout.String(), "-- attached") })

	d.DropConnections()
	if code := c.wait(t); code != 1 {
		t.Errorf("exit %d, want 1", code)
	}
	if !strings.Contains(c.stderr.String(), "connection to daemon lost") {
		t.Errorf("stderr %q", c.stderr.String())
	}
}

func TestDaemonNotRunning(t *testing.T) {
	dir, err := os.MkdirTemp("", "suite")
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { os.RemoveAll(dir) })

	// A socket file with nobody listening refuses; a missing one doesn't exist.
	stale := filepath.Join(dir, "stale.sock")
	l, err := net.Listen("unix", stale)
	if err != nil {
		t.Fatal(err)
	}
	l.(*net.UnixListener).SetUnlinkOnClose(false)
	l.Close()

	for _, sock := range []string{filepath.Join(dir, "missing.sock"), stale} {
		for _, cmd := range []string{"status", "sessions", "attach"} {
			args := []string{cmd}
			if cmd == "attach" {
				args = append(args, "x")
			}
			code, stdout, stderr := exec(t, sock, "", args...)
			want := "suite: daemon not running (socket " + sock + "); start it with: bun run daemon\n"
			if code != 1 || stdout != "" || stderr != want {
				t.Errorf("%v: code %d stdout %q stderr %q", args, code, stdout, stderr)
			}
		}
	}
}
