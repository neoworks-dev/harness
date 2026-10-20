package main

import (
	"context"
	"encoding/json"
	"io"
	"os"
	"strings"
	"testing"
	"time"

	"neoworks.dev/suite/renderers/tui"

	protocol "neoworks.dev/suite/protocol/go"
	"neoworks.dev/suite/sdk-go/sdktest"
)

func TestAttachViewNew(t *testing.T) {
	d := sdktest.New(t)
	// Blank lines are skipped; the second input is queued behind the first.
	code, stdout, stderr := exec(t, d.Socket, "hello\n\nworld\n", "attach", "--new", "--app", "tester")
	if code != 0 || stderr != "" {
		t.Fatalf("code %d, stderr %q", code, stderr)
	}
	want := "created session new1\n" +
		"-- attached to new1 --\n" +
		"» tester\nhello\n\n" +
		"you said: hello\n\n" +
		"» tester\nworld\n\n" +
		"you said: world\n\n"
	if stdout != want {
		t.Errorf("stdout:\n%q\nwant:\n%q", stdout, want)
	}
	if got := d.ViewSubscribes(); len(got) != 1 || got[0] != "new1" {
		t.Errorf("view subscribes = %v", got)
	}
}

func TestAttachViewHistoryComesBeforeTheMarker(t *testing.T) {
	d := sdktest.New(t)
	d.AddSession("abc123", "", time.Now(),
		protocol.UserMessage{App: "overlay", Text: "hi"},
		protocol.AgentThoughtChunk{MessageID: "t", Text: "pondering"},
		protocol.AgentMessageChunk{MessageID: "m", Text: "hello there"},
		protocol.TurnEnded{StopReason: protocol.StopReasonEndTurn},
		protocol.ToolCall{ToolCallID: "c1", Title: "bash: ls", Status: protocol.ToolCallStatusInProgress},
		protocol.ToolCallUpdate{ToolCallID: "c1", Status: ptr(protocol.ToolCallStatusCompleted),
			Content: []protocol.ToolCallContent{{Value: protocol.TextContent{Text: "a.txt\nb.txt\n"}}}},
	)
	code, stdout, stderr := exec(t, d.Socket, "", "attach", "abc")
	if code != 0 || stderr != "" {
		t.Fatalf("code %d, stderr %q", code, stderr)
	}
	want := "» overlay\nhi\n\n" +
		"… thinking\n\n" +
		"hello there\n\n" +
		"✓ bash: ls\n    a.txt\n    b.txt\n\n" +
		"-- attached to abc123 --\n"
	if stdout != want {
		t.Errorf("stdout:\n%q\nwant:\n%q", stdout, want)
	}

	_, stdout, _ = exec(t, d.Socket, "", "attach", "--thoughts", "abc123")
	if !strings.Contains(stdout, "\npondering\n") || strings.Contains(stdout, "… thinking") {
		t.Errorf("--thoughts output:\n%s", stdout)
	}
}

func TestAttachViewFallsBackToRawWhenTheDaemonHasNoViews(t *testing.T) {
	d := sdktest.New(t)
	d.NoViews = true
	d.AddSession("s1", "", time.Now(), protocol.UserMessage{App: "overlay", Text: "hi"})
	code, stdout, stderr := exec(t, d.Socket, "", "attach", "s1")
	if code != 0 || !strings.Contains(stderr, "does not serve views") {
		t.Errorf("code %d, stderr %q", code, stderr)
	}
	if !strings.Contains(stdout, "» overlay: hi") {
		t.Errorf("not the raw rendering:\n%s", stdout)
	}
}

// The fake daemon has no templates, so tests hand it an item with buttons.
func cropItem(t *testing.T) protocol.ViewItem {
	t.Helper()
	var it protocol.ViewItem
	data := `{"ref":"crop","kind":"tool","label":"photo-editor.crop","state":"done","component":{"type":"card","title":"Cropped image","children":[
		{"type":"image","src":"asset://img-7f3","width":1920,"height":1080},
		{"type":"row","children":[
			{"type":"button","label":"Open as layer","action":"photo-editor.open_layer","args":{"asset":"asset://img-7f3"},"key":"o"},
			{"type":"button","label":"Copy handle","action":"copy:asset://img-7f3","key":"c"},
			{"type":"button","label":"Undo","action":"photo-editor.undo"}]}]}}`
	if err := json.Unmarshal([]byte(data), &it); err != nil {
		t.Fatal(err)
	}
	return it
}

func TestAttachViewButtons(t *testing.T) {
	d := sdktest.New(t)
	d.AddSession("s1", "", time.Now(), protocol.UserMessage{App: "cli", Text: "crop it"})
	d.PatchView("s1", protocol.ViewOp{Value: protocol.SetViewOp{Item: cropItem(t)}})
	stdin, stdinW := io.Pipe()
	c := start(t, d.Socket, stdin, "attach", "s1")
	eventually(t, "item drawn", func() bool {
		return strings.Contains(c.stdout.String(), "[o] Open as layer  [c] Copy handle  [3] Undo")
	})

	send := func(line string, wait string) {
		t.Helper()
		before := len(c.stdout.String())
		stdinW.Write([]byte(line + "\n"))
		eventually(t, "reaction to "+line, func() bool { return strings.Contains(c.stdout.String()[before:], wait) })
	}
	send("/help", "/o  Open as layer  (photo-editor.open_layer)")
	send("/help", "/c  Copy handle  (copy asset://img-7f3)")
	send("/o", "pressed [o] Open as layer")
	send("/3", "pressed [3] Undo")
	send("/c", "copy: asset://img-7f3") // no terminal to copy to: the text is printed
	send("/x", "no button [x]")

	// Anything that isn't "/" and one character goes to the daemon.
	want := []string{"/usr/bin/env is missing", "/plain", "/model", "/resume abc", "/mode neo plan"}
	stdinW.Write([]byte("/usr/bin/env is missing\n//plain\n/model\n/resume abc\n/mode neo plan\n"))
	eventually(t, "inputs", func() bool { return len(d.Inputs()) == len(want) })
	for i, in := range d.Inputs() {
		if in.Text != want[i] {
			t.Errorf("input %d = %q, want %q", i, in.Text, want[i])
		}
	}

	actions := d.Actions()
	if len(actions) != 2 {
		t.Fatalf("actions = %+v", actions)
	}
	if a := actions[0]; a.Session != "s1" || a.Ref != "crop" || a.Action != "photo-editor.open_layer" || string(a.Args) != `{"asset":"asset://img-7f3"}` {
		t.Errorf("first action = %+v", a)
	}
	if a := actions[1]; a.Action != "photo-editor.undo" || a.Args != nil {
		t.Errorf("second action = %+v", a)
	}
	stdinW.Close()
	if code := c.wait(t); code != 0 {
		t.Errorf("exit %d", code)
	}
}

func TestAttachViewButtonsUseTheLatestItemWithButtons(t *testing.T) {
	d := sdktest.New(t)
	d.AddSession("s1", "", time.Now())
	older := cropItem(t)
	older.Ref = "older"
	newer := cropItem(t)
	newer.Ref = "newer"
	for _, it := range []protocol.ViewItem{older, newer} {
		d.PatchView("s1", protocol.ViewOp{Value: protocol.SetViewOp{Item: it}})
	}
	d.Append("s1", protocol.UserMessage{App: "cli", Text: "later, without buttons"})
	code, stdout, stderr := exec(t, d.Socket, "/o\n", "attach", "s1")
	if code != 0 || stderr != "" {
		t.Fatalf("code %d, stderr %q\n%s", code, stderr, stdout)
	}
	if got := d.Actions(); len(got) != 1 || got[0].Ref != "newer" {
		t.Errorf("actions = %+v", got)
	}
}

func TestAttachViewNoButtons(t *testing.T) {
	d := sdktest.New(t)
	d.AddSession("s1", "", time.Now())
	code, stdout, _ := exec(t, d.Socket, "/help\n/1\n", "attach", "s1")
	if code != 0 || strings.Count(stdout, "no buttons in this session yet") != 2 {
		t.Errorf("code %d:\n%s", code, stdout)
	}
	if len(d.Actions()) != 0 || len(d.Inputs()) != 0 {
		t.Errorf("sent %+v %+v", d.Actions(), d.Inputs())
	}
}

func TestAttachViewRefusedActionIsOnlyAWarning(t *testing.T) {
	d := sdktest.New(t)
	d.AddSession("s1", "", time.Now())
	d.PatchView("s1", protocol.ViewOp{Value: protocol.SetViewOp{Item: cropItem(t)}})
	stdin, stdinW := io.Pipe()
	c := start(t, d.Socket, stdin, "attach", "s1")
	eventually(t, "attach", func() bool { return strings.Contains(c.stdout.String(), "[o] Open as layer") })
	d.Forget("s1") // the daemon refuses ui.action for a session it does not know
	stdinW.Write([]byte("/o\n"))
	eventually(t, "warning", func() bool { return strings.Contains(c.stderr.String(), "no session s1") })
	c.interrupt <- os.Interrupt // still attached: not running, so Ctrl-C exits
	if code := c.wait(t); code != 0 {
		t.Errorf("exit %d: a refusal must not end the session", code)
	}
}

func TestColorMode(t *testing.T) {
	env := func(kv ...string) func(string) string {
		m := map[string]string{}
		for i := 0; i < len(kv); i += 2 {
			m[kv[i]] = kv[i+1]
		}
		return func(k string) string { return m[k] }
	}
	tests := []struct {
		name string
		env  func(string) string
		tty  bool
		want tui.ColorMode
	}{
		{"not a terminal", env("COLORTERM", "truecolor"), false, tui.None},
		{"NO_COLOR", env("NO_COLOR", "1", "COLORTERM", "truecolor"), true, tui.None},
		{"dumb", env("TERM", "dumb"), true, tui.None},
		{"truecolor", env("COLORTERM", "truecolor", "TERM", "xterm-256color"), true, tui.TrueColor},
		{"24bit", env("COLORTERM", "24bit"), true, tui.TrueColor},
		{"256", env("TERM", "xterm-256color"), true, tui.ANSI256},
		{"16", env("TERM", "xterm"), true, tui.ANSI16},
		{"nothing set", env(), true, tui.ANSI16},
	}
	for _, tc := range tests {
		if got := colorMode(tc.env, tc.tty); got != tc.want {
			t.Errorf("%s: %v, want %v", tc.name, got, tc.want)
		}
	}
}

func TestWidth(t *testing.T) {
	size := func() (int, int) { return 100, 30 }
	tests := []struct {
		name string
		cols string
		tty  bool
		size func() (int, int)
		want int
	}{
		{"terminal", "", true, size, 100},
		{"COLUMNS wins", "50", true, size, 50},
		{"bad COLUMNS", "wide", true, size, 100},
		{"no terminal", "", false, size, 80},
		{"no terminal, COLUMNS", "120", false, size, 120},
		{"size unknown", "", true, func() (int, int) { return 0, 0 }, 80},
		{"no size func", "", true, nil, 80},
	}
	for _, tc := range tests {
		a := &viewAttachment{tty: tc.tty, size: tc.size, getenv: func(k string) string {
			if k == "COLUMNS" {
				return tc.cols
			}
			return ""
		}}
		if got := a.width(); got != tc.want {
			t.Errorf("%s: %d, want %d", tc.name, got, tc.want)
		}
	}
}

// screen plays terminal output on a screen and returns its rows: enough of
// the protocol for the live tail (cursor up, carriage return, clear to the
// end of the screen), ignoring colors and OSC sequences.
func screen(out string) []string {
	rows := [][]rune{{}}
	row, col := 0, 0
	put := func(r rune) {
		for len(rows[row]) < col {
			rows[row] = append(rows[row], ' ')
		}
		if col < len(rows[row]) {
			rows[row][col] = r
		} else {
			rows[row] = append(rows[row], r)
		}
		col++
	}
	rs := []rune(out)
	for i := 0; i < len(rs); i++ {
		switch r := rs[i]; {
		case r == '\n':
			row, col = row+1, 0
			if row == len(rows) {
				rows = append(rows, nil)
			}
		case r == '\r':
			col = 0
		case r == 0x1b && i+1 < len(rs) && rs[i+1] == ']': // OSC ... BEL
			for i < len(rs) && rs[i] != 0x07 {
				i++
			}
		case r == 0x1b && i+1 < len(rs) && rs[i+1] == '[':
			j := i + 2
			n := 0
			for j < len(rs) && rs[j] >= '0' && rs[j] <= ';' {
				if rs[j] != ';' {
					n = n*10 + int(rs[j]-'0')
				}
				j++
			}
			switch rs[j] {
			case 'A':
				row = max(row-max(n, 1), 0)
			case 'J':
				if col < len(rows[row]) {
					rows[row] = rows[row][:col]
				}
				rows = rows[:row+1]
			}
			i = j
		default:
			put(r)
		}
	}
	lines := make([]string, len(rows))
	for i, r := range rows {
		lines[i] = strings.TrimRight(string(r), " ")
	}
	return lines
}

func TestScreenEmulator(t *testing.T) {
	// Draw two rows, go back up over them, draw one: only that remains.
	got := screen("keep\nold 1\nold 2\n\x1b[2A\r\x1b[Jnew\n")
	if strings.Join(got, "|") != "keep|new|" {
		t.Errorf("screen = %q", got)
	}
}

// startTTY runs attach as if stdout were a terminal of the given width.
func startTTY(t *testing.T, socket string, stdin io.Reader, env map[string]string, args ...string) *cli {
	t.Helper()
	c := &cli{interrupt: make(chan os.Signal, 1), done: make(chan int, 1)}
	ctx, cancel := context.WithTimeout(context.Background(), 20*time.Second)
	t.Cleanup(cancel)
	env["SUITE_SOCKET"] = socket
	go func() {
		c.done <- run(ctx, args, cliEnv{
			Stdin: stdin, Stdout: &c.stdout, Stderr: &c.stderr,
			Getenv:    func(k string) string { return env[k] },
			UID:       1000,
			Interrupt: c.interrupt,
			Now:       time.Now,
			StdoutTTY: true,
			Size:      func() (int, int) { return 50, 24 },
		})
	}()
	return c
}

func TestAttachViewLiveTail(t *testing.T) {
	d := sdktest.New(t)
	d.StepDelay = 120 * time.Millisecond
	ok := protocol.ToolCallStatusCompleted
	d.AgentReply = func(text string) []protocol.SessionEventVariant {
		return []protocol.SessionEventVariant{
			protocol.AgentMessageChunk{MessageID: "m1", Text: "alpha "},
			protocol.AgentMessageChunk{MessageID: "m1", Text: "beta\n\ngamma"},
			protocol.ToolCall{ToolCallID: "c1", Title: "run it", Status: protocol.ToolCallStatusInProgress},
			protocol.ToolCallUpdate{ToolCallID: "c1", Status: &ok,
				Content: []protocol.ToolCallContent{{Value: protocol.TextContent{Text: "output"}}}},
			protocol.TurnEnded{StopReason: protocol.StopReasonEndTurn},
		}
	}
	d.AddSession("s1", "", time.Now())
	stdin, stdinW := io.Pipe()
	c := startTTY(t, d.Socket, stdin, map[string]string{"TERM": "xterm"}, "attach", "s1")
	eventually(t, "attach", func() bool { return strings.Contains(c.stdout.String(), "-- attached to s1 --") })
	stdinW.Write([]byte("go\n"))
	shown := func() string { return strings.Join(screen(c.stdout.String()), "\n") }
	eventually(t, "tool running", func() bool { return strings.Contains(shown(), " run it") && !strings.Contains(shown(), "✓") })

	// Mid-turn the screen shows the settled part and the live tail once each.
	if mid := shown(); strings.Count(mid, "alpha beta") != 1 || strings.Count(mid, "run it") != 1 {
		t.Errorf("mid-turn screen:\n%s", mid)
	}
	eventually(t, "tool done", func() bool { return strings.Contains(shown(), "✓ run it") })
	stdinW.Close()
	if code := c.wait(t); code != 0 {
		t.Fatalf("exit %d, stderr %q", code, c.stderr.String())
	}

	out := c.stdout.String()
	want := strings.Join([]string{
		"-- attached to s1 --",
		"» cli",
		"go",
		"",
		"alpha beta",
		"",
		"gamma",
		"",
		"✓ run it",
		"    output",
		"",
		"",
	}, "\n")
	if got := strings.Join(screen(out), "\n"); got != want {
		t.Errorf("final screen:\n%s\n--- want ---\n%s", got, want)
	}
	if !strings.Contains(out, "A\r\x1b[J") {
		t.Error("the tail was never redrawn in place")
	}
	if strings.Contains(out, "\x1b[") && !strings.Contains(out, "\x1b[0m") {
		t.Error("colored output without resets")
	}
}

func TestAttachViewTailIsLimitedToTheTerminalHeight(t *testing.T) {
	d := sdktest.New(t)
	d.AddSession("s1", "", time.Now(), protocol.ToolCall{ToolCallID: "c1", Title: "long", Status: protocol.ToolCallStatusInProgress})
	lines := make([]string, 40)
	for i := range lines {
		lines[i] = "line"
	}
	d.Append("s1", protocol.ToolCallUpdate{ToolCallID: "c1", Content: []protocol.ToolCallContent{{Value: protocol.TextContent{Text: strings.Join(lines, "\n")}}}})
	stdin, stdinW := io.Pipe()
	defer stdinW.Close()
	c := startTTY(t, d.Socket, stdin, map[string]string{"TERM": "xterm"}, "attach", "s1")
	eventually(t, "tail drawn", func() bool { return strings.Count(c.stdout.String(), "line") >= 10 })
	time.Sleep(100 * time.Millisecond)
	// Below the "attached" line come the tail and the row the cursor is on.
	if n := len(screen(c.stdout.String())) - 2; n > 23 {
		t.Errorf("the tail is %d rows on a 24-row terminal; the cursor cannot move back over it", n)
	}
}

func TestInputCountsEchoedRows(t *testing.T) {
	a := &viewAttachment{tty: true, interactive: true, getenv: func(k string) string {
		if k == "COLUMNS" {
			return "20"
		}
		return ""
	}}
	a.input(context.Background(), strings.Repeat(" ", 45)) // blank: nothing is sent
	if a.tailRows != 3 || !a.dirty {
		t.Errorf("tailRows %d dirty %v, want 3 rows to erase", a.tailRows, a.dirty)
	}
	a.tailRows = 0
	a.input(context.Background(), " ") // fits one row
	if a.tailRows != 1 {
		t.Errorf("tailRows %d, want 1", a.tailRows)
	}
}

func TestIsCommand(t *testing.T) {
	for line, want := range map[string]bool{
		"/o":       true,
		"/3":       true,
		"/é":       true,
		"/":        false,
		"//":       false,
		"//x":      false,
		"/model":   false,
		"/resume":  false,
		"/o x":     false,
		"/usr/bin": false,
		"text":     false,
		"/\xff":    false,
	} {
		if got := isCommand(line); got != want {
			t.Errorf("isCommand(%q) = %v, want %v", line, got, want)
		}
	}
}
