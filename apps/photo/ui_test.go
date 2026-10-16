package main

import (
	"context"
	"encoding/base64"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"
	"time"

	protocol "neoworks.dev/suite/protocol/go"
	"neoworks.dev/suite/sdk-go/sdktest"
)

// fakeTerm is a terminal made of a pipe for the keys and a file for the screen.
type fakeTerm struct {
	keysR, keysW, screen *os.File
	cols, rows           int
	mu                   sync.Mutex
	rawOn                bool
	restored             bool
}

func newFakeTerm(t *testing.T) *fakeTerm {
	t.Helper()
	r, w, err := os.Pipe()
	if err != nil {
		t.Fatal(err)
	}
	screen, err := os.Create(filepath.Join(t.TempDir(), "screen"))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { r.Close(); w.Close(); screen.Close() })
	return &fakeTerm{keysR: r, keysW: w, screen: screen, cols: 80, rows: 24}
}

func (f *fakeTerm) interactive() bool { return true }
func (f *fakeTerm) size() (int, int)  { return f.cols, f.rows }
func (f *fakeTerm) in() *os.File      { return f.keysR }
func (f *fakeTerm) out() *os.File     { return f.screen }
func (f *fakeTerm) type_(s string)    { f.keysW.WriteString(s) }
func (f *fakeTerm) raw() (func(), error) {
	f.mu.Lock()
	defer f.mu.Unlock()
	f.rawOn = true
	return func() { f.mu.Lock(); f.restored = true; f.mu.Unlock() }, nil
}

func (f *fakeTerm) output() string {
	data, _ := os.ReadFile(f.screen.Name())
	return string(data)
}

func (f *fakeTerm) state() (rawOn, restored bool) {
	f.mu.Lock()
	defer f.mu.Unlock()
	return f.rawOn, f.restored
}

func chatItem(t *testing.T) protocol.ViewItem {
	t.Helper()
	var it protocol.ViewItem
	data := `{"ref":"crop","kind":"tool","label":"photo-editor.crop","state":"done","component":{"type":"card","title":"Cropped image","children":[
		{"type":"image","src":"asset://img-7f3","width":1920,"height":1080},
		{"type":"row","children":[
			{"type":"button","label":"Open as layer","action":"photo-editor.open_layer","args":{"asset":"asset://img-7f3"},"key":"o"},
			{"type":"button","label":"Copy handle","action":"copy:asset://img-7f3","key":"c"}]}]}}`
	if err := json.Unmarshal([]byte(data), &it); err != nil {
		t.Fatal(err)
	}
	return it
}

func TestUI(t *testing.T) {
	d := sdktest.New(t)
	d.AddSession("s1", "chat", time.Now(),
		protocol.UserMessage{App: "cli", Text: "please crop the logo"},
		protocol.AgentMessageChunk{MessageID: "m1", Text: "Sure, cropping now."},
		protocol.TurnEnded{StopReason: protocol.StopReasonEndTurn})
	d.PatchView("s1", protocol.ViewOp{Value: protocol.SetViewOp{Item: chatItem(t)}})

	dir := t.TempDir()
	logo := writePNG(t, dir, "logo.png", 64, 32)
	second := writePNG(t, dir, "second.png", 20, 20)

	term := newFakeTerm(t)
	ctx, cancel := context.WithTimeout(context.Background(), 20*time.Second)
	defer cancel()
	done := make(chan error, 1)
	go func() {
		done <- run(ctx, config{
			Socket: d.Socket, Open: logo, Term: term, Stderr: os.Stderr,
			Getenv: func(k string) string { return map[string]string{"COLORTERM": "truecolor", "TERM": "xterm-256color"}[k] },
		})
	}()
	has := func(what string, wants ...string) {
		t.Helper()
		sdktest.Eventually(t, what, func() bool {
			out := term.output()
			for _, w := range wants {
				if !strings.Contains(out, w) {
					return false
				}
			}
			return true
		})
	}

	// The status line, the preview in half blocks and the shared chat.
	has("the first screen", "Photo editor · logo.png · 64×32 · asset://img-", "▀", "\x1b[38;2;", "please crop the logo", "Sure, cropping now.", "Open as layer")
	if rawOn, _ := term.state(); !rawOn {
		t.Error("the terminal was not put in raw mode")
	}
	p, ok := d.Presence("photo-editor")
	if !ok || p.Context["image"] != "logo.png" || p.Mcp == nil || p.Launch != nil && len(p.Launch.Command) == 0 || len(p.Templates) == 0 {
		t.Fatalf("presence: %+v", p)
	}
	if strings.Contains(term.output(), "agent is here") {
		t.Error("the agent is not here yet")
	}

	// The agent moves in: the status line says so.
	d.MoveAgentTo("photo-editor", protocol.FocusReasonAgent)
	has("the agent being here", "agent is here")

	// Typing sends session.input from this app; a backspace edits the line.
	term.type_("helloo\x7f agent\r")
	sdktest.Eventually(t, "the input", func() bool { return len(d.Inputs()) == 1 })
	if in := d.Inputs()[0]; in.Session != "s1" || in.App != "photo-editor" || in.Text != "hello agent" {
		t.Errorf("input %+v", in)
	}
	// A path-like line is text; // escapes a leading slash.
	term.type_("//usr/bin is missing\r")
	sdktest.Eventually(t, "the second input", func() bool { return len(d.Inputs()) == 2 })
	if in := d.Inputs()[1]; in.Text != "/usr/bin is missing" {
		t.Errorf("input %+v", in)
	}

	// Buttons: a plain action goes to the daemon, copy: to the clipboard.
	term.type_("/o\r")
	sdktest.Eventually(t, "the action", func() bool { return len(d.Actions()) == 1 })
	if a := d.Actions()[0]; a.Session != "s1" || a.Ref != "crop" || a.Action != "photo-editor.open_layer" {
		t.Errorf("action %+v", a)
	}
	term.type_("/c\r")
	has("the clipboard", "\x1b]52;c;"+base64.StdEncoding.EncodeToString([]byte("asset://img-7f3"))+"\x07")
	term.type_("/help\r")
	has("the help", "/o Open as layer  /c Copy handle")

	// /open loads another image into the preview and the presence.
	term.type_("/open " + second + "\r")
	has("the second image", "second.png · 20×20")
	sdktest.Eventually(t, "the presence context", func() bool {
		p, _ := d.Presence("photo-editor")
		return p.Context["image"] == "second.png"
	})
	term.type_("/open /no/such/file.png\r")
	has("the error", "no such file")

	// Arrow keys and other escape sequences are ignored, also when split.
	term.type_("\x1b[A")
	term.type_("\x1b")
	term.type_("[Bx\r")
	sdktest.Eventually(t, "the last input", func() bool { return len(d.Inputs()) == 3 })
	if in := d.Inputs()[2]; in.Text != "x" {
		t.Errorf("input %+v", in)
	}

	// q quits, restores the terminal and unregisters.
	term.type_("q\r")
	select {
	case err := <-done:
		if err != nil {
			t.Errorf("run: %v", err)
		}
	case <-time.After(5 * time.Second):
		t.Fatal("q did not quit")
	}
	if _, restored := term.state(); !restored {
		t.Error("the terminal was not restored")
	}
	if out := term.output(); !strings.HasSuffix(out, "\x1b[0m\x1b[?1049l") {
		t.Errorf("the alternate screen was not left: %q", out[max(len(out)-40, 0):])
	}
	sdktest.Eventually(t, "the app to unregister", func() bool {
		_, open := d.Presence("photo-editor")
		return !open
	})
}

func TestUICtrlCQuits(t *testing.T) {
	d := sdktest.New(t)
	term := newFakeTerm(t)
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()
	done := make(chan error, 1)
	go func() {
		done <- run(ctx, config{Socket: d.Socket, Term: term, Getenv: func(string) string { return "" }})
	}()
	sdktest.Eventually(t, "the first screen", func() bool { return strings.Contains(term.output(), "no image") })
	if out := term.output(); strings.Contains(out, "▀") || !strings.Contains(out, "type /open <path>") {
		t.Errorf("an empty editor should explain itself: %q", out)
	}
	term.type_("typed but not sent\x03")
	select {
	case err := <-done:
		if err != nil {
			t.Error(err)
		}
	case <-time.After(5 * time.Second):
		t.Fatal("Ctrl-C did not quit")
	}
	if len(d.Inputs()) != 0 {
		t.Error("the half-typed line was sent")
	}
}

func TestFeedEditsTheLine(t *testing.T) {
	u := &ui{}
	ctx := context.Background()
	u.feed(ctx, []byte("héllo"))
	if string(u.input) != "héllo" {
		t.Fatalf("%q", string(u.input))
	}
	// A multi-byte character split between two reads.
	u.feed(ctx, []byte("日"[:2]))
	u.feed(ctx, []byte("日"[2:]))
	if string(u.input) != "héllo日" {
		t.Fatalf("%q", string(u.input))
	}
	u.feed(ctx, []byte("\x7f\x7f\x7f\x7f"))
	if string(u.input) != "hé" {
		t.Fatalf("%q", string(u.input))
	}
	u.feed(ctx, []byte(" one two\x17"))
	if string(u.input) != "hé one " {
		t.Fatalf("ctrl-w: %q", string(u.input))
	}
	u.feed(ctx, []byte("\x15"))
	if len(u.input) != 0 {
		t.Fatalf("ctrl-u: %q", string(u.input))
	}
	u.feed(ctx, []byte("a\x1b[1;5Cb\x1bOHc\x1bxd"))
	if string(u.input) != "abcd" {
		t.Fatalf("escape sequences leaked into the input: %q", string(u.input))
	}
	u.feed(ctx, []byte("\x04")) // Ctrl-D with text does not quit
	if u.quit {
		t.Error("Ctrl-D quit with text typed")
	}
	u.feed(ctx, []byte("\x15\x04"))
	if !u.quit {
		t.Error("Ctrl-D on an empty line should quit")
	}
}

func TestIsCommand(t *testing.T) {
	for s, want := range map[string]bool{"/u": true, "/10": true, "/": false, "/usr/bin/env": false, "/open x": false, "u": false} {
		if isCommand(s) != want {
			t.Errorf("isCommand(%q) = %v", s, !want)
		}
	}
}
