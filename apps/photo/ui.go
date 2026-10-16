package main

import (
	"context"
	"encoding/base64"
	"fmt"
	"strings"
	"time"
	"unicode/utf8"

	protocol "neoworks.dev/suite/protocol/go"
	"neoworks.dev/suite/renderers/tui"
	sdk "neoworks.dev/suite/sdk-go"
)

const (
	tickPeriod    = 100 * time.Millisecond
	spinnerPeriod = 125 * time.Millisecond
	noteLifetime  = 8 * time.Second
	defaultCols   = 80
	defaultRows   = 24
)

// colorMode picks the color depth from the environment, like `suite attach`.
func colorMode(getenv func(string) string) tui.ColorMode {
	term := getenv("TERM")
	switch {
	case getenv("NO_COLOR") != "", term == "dumb":
		return tui.None
	case getenv("COLORTERM") == "truecolor" || getenv("COLORTERM") == "24bit":
		return tui.TrueColor
	case strings.Contains(term, "256color"):
		return tui.ANSI256
	}
	return tui.ANSI16
}

// ui is the full-screen terminal: image preview on top, the shared chat in
// the middle, an input line at the bottom. Everything runs on one goroutine
// (runUI), so there is no locking.
type ui struct {
	a     *app
	color tui.ColorMode
	now   func() time.Time

	session  string
	view     *sdk.ViewSubscription
	state    *sdk.ViewState
	focusApp string // app that has the agent, "" for none

	input    []rune
	rest     []byte // bytes of an incomplete key sequence
	note     string
	noteTill time.Time
	quit     bool

	previewKey   previewKey
	previewLines []string
	chatKey      chatKey
	chatLines    []string
	lastFrame    string
}

type previewKey struct {
	d          *doc
	cols, rows int
	mode       tui.ColorMode
}

type chatKey struct {
	seq    int64
	width  int
	rows   int
	frame  int
	nItems int
}

func (a *app) runUI(ctx context.Context) error {
	t := a.cfg.Term
	restore, err := t.raw()
	if err != nil {
		return fmt.Errorf("terminal: %w", err)
	}
	out := t.out()
	// Alternate screen, so the shell's scrollback is untouched.
	fmt.Fprint(out, "\x1b[?1049h\x1b[2J")
	defer func() {
		fmt.Fprint(out, "\x1b[0m\x1b[?1049l")
		restore()
	}()

	u := &ui{a: a, color: colorMode(a.cfg.Getenv), now: time.Now}
	keys := make(chan []byte, 16)
	go func() { // ends with the process; a blocked read cannot be cancelled
		buf := make([]byte, 256)
		for {
			n, err := t.in().Read(buf)
			if n > 0 {
				keys <- append([]byte(nil), buf[:n]...)
			}
			if err != nil {
				close(keys)
				return
			}
		}
	}()

	tick := time.NewTicker(tickPeriod)
	defer tick.Stop()
	u.refocus(ctx)
	for !u.quit {
		cols, rows := t.size()
		if cols <= 0 || rows <= 0 {
			cols, rows = defaultCols, defaultRows
		}
		u.draw(out, cols, rows)

		var updates <-chan sdk.ViewUpdate
		if u.view != nil {
			updates = u.view.Updates()
		}
		select {
		case b, ok := <-keys:
			if !ok {
				return nil // stdin closed
			}
			u.feed(ctx, b)
		case <-a.focused:
			u.refocus(ctx)
		case up, ok := <-updates:
			u.update(ctx, up, ok)
		case <-a.redraw:
		case <-tick.C:
		case <-ctx.Done():
			return nil
		case <-a.c.Done():
			return a.c.Err()
		}
		for _, n := range a.takeNotes() {
			u.setNote("%s", n)
		}
	}
	return nil
}

func (u *ui) setNote(format string, args ...any) {
	u.note = fmt.Sprintf(format, args...)
	u.noteTill = u.now().Add(noteLifetime)
}

// refocus follows the agent: the status shows where it is and the chat shows
// the current session.
func (u *ui) refocus(ctx context.Context) {
	f, ok := u.a.c.Focus()
	if !ok {
		return
	}
	u.focusApp = ""
	if f.App != nil {
		u.focusApp = *f.App
	}
	if f.Session == u.session && u.view != nil {
		return
	}
	if u.view != nil {
		u.view.Close()
		u.view, u.state = nil, nil
	}
	u.session = f.Session
	sctx, cancel := context.WithTimeout(ctx, 5*time.Second)
	defer cancel()
	sub, err := u.a.c.SubscribeView(sctx, f.Session)
	if err != nil {
		u.setNote("cannot follow session %s: %v", f.Session, err)
		return
	}
	u.view, u.state = sub, sub.State()
}

// update applies a change to the followed view; if it does not fit, start
// over from a fresh snapshot.
func (u *ui) update(ctx context.Context, up sdk.ViewUpdate, ok bool) {
	if !ok {
		u.setNote("lost the view of session %s: %v", u.session, u.view.Err())
		u.view, u.state = nil, nil
		return
	}
	if err := up.ApplyTo(u.state); err != nil {
		u.view.Close()
		u.view, u.state, u.session = nil, nil, ""
		u.refocus(ctx)
	}
}

// feed handles bytes typed by the user.
func (u *ui) feed(ctx context.Context, b []byte) {
	u.rest = append(u.rest, b...)
	for len(u.rest) > 0 && !u.quit {
		c := u.rest[0]
		switch {
		case c == 0x1b:
			n := escapeLen(u.rest)
			if n == 0 {
				if len(u.rest) < 16 {
					return // wait for the rest of the sequence
				}
				n = len(u.rest)
			}
			u.rest = u.rest[n:]
		case c == 0x03: // Ctrl-C
			u.quit = true
		case c == 0x04: // Ctrl-D
			if len(u.input) == 0 {
				u.quit = true
			}
			u.rest = u.rest[1:]
		case c == '\r' || c == '\n':
			u.rest = u.rest[1:]
			u.submit(ctx)
		case c == 0x7f || c == 0x08:
			if len(u.input) > 0 {
				u.input = u.input[:len(u.input)-1]
			}
			u.rest = u.rest[1:]
		case c == 0x15: // Ctrl-U
			u.input = u.input[:0]
			u.rest = u.rest[1:]
		case c == 0x17: // Ctrl-W
			s := strings.TrimRight(string(u.input), " ")
			if i := strings.LastIndexByte(s, ' '); i >= 0 {
				s = s[:i+1]
			} else {
				s = ""
			}
			u.input = []rune(s)
			u.rest = u.rest[1:]
		case c < 0x20:
			u.rest = u.rest[1:] // Ctrl-L and other controls: the next draw is a full one anyway
			u.lastFrame = ""
		default:
			if !utf8.FullRune(u.rest) {
				return
			}
			r, size := utf8.DecodeRune(u.rest)
			u.rest = u.rest[size:]
			if r != utf8.RuneError {
				u.input = append(u.input, r)
			}
		}
	}
}

// escapeLen is the length of the escape sequence at the start of b, or 0 if
// it is not complete yet. A lone ESC counts as a one-byte sequence.
func escapeLen(b []byte) int {
	if len(b) == 1 {
		return 1
	}
	switch b[1] {
	case '[':
		for i := 2; i < len(b); i++ {
			if b[i] >= 0x40 && b[i] <= 0x7e {
				return i + 1
			}
		}
		return 0
	case 'O':
		if len(b) < 3 {
			return 0
		}
		return 3
	}
	return 2 // alt+key
}

// submit handles the input line: a command, a button, or text for the agent.
func (u *ui) submit(ctx context.Context) {
	line := strings.TrimSpace(string(u.input))
	u.input = u.input[:0]
	switch {
	case line == "":
	case line == "q" || line == "/q" || line == "/quit":
		u.quit = true
	case line == "/help":
		u.help()
	case line == "/clear":
		u.send(ctx, line) // the daemon starts a fresh session; the chat follows agent.focus
	case strings.HasPrefix(line, "/open "):
		u.open(ctx, strings.TrimSpace(strings.TrimPrefix(line, "/open ")))
	case isCommand(line):
		u.press(ctx, line[1:])
	default:
		if strings.HasPrefix(line, "//") {
			line = line[1:] // "//text" sends "/text"
		}
		u.send(ctx, line)
	}
}

// isCommand tells "/u" from a message such as "/usr/bin/env is missing".
func isCommand(s string) bool {
	return len(s) > 1 && s[0] == '/' && !strings.ContainsAny(s[1:], " \t/")
}

func (u *ui) send(ctx context.Context, text string) {
	if u.session == "" {
		u.setNote("no session to talk to yet")
		return
	}
	sctx, cancel := context.WithTimeout(ctx, 10*time.Second)
	defer cancel()
	if err := u.a.c.Input(sctx, protocol.SessionInput{Session: u.session, App: appName, Text: text}); err != nil {
		u.setNote("%v", err)
	}
}

func (u *ui) open(ctx context.Context, path string) {
	if path == "" {
		u.setNote("usage: /open <path to a PNG, JPEG or GIF>")
		return
	}
	octx, cancel := context.WithTimeout(ctx, 30*time.Second)
	defer cancel()
	if res, err := u.a.ed.openPath(octx, path); err != nil {
		u.setNote("%v", err)
	} else {
		u.setNote("opened %s (%d×%d)", res.Name, res.Width, res.Height)
	}
}

// lastWithButtons is the most recent chat item that has buttons.
func (u *ui) lastWithButtons() (protocol.ViewItem, []tui.Button, bool) {
	if u.state == nil {
		return protocol.ViewItem{}, nil, false
	}
	items := u.state.Items()
	for i := len(items) - 1; i >= 0; i-- {
		if btns := tui.Buttons(items[i]); len(btns) > 0 {
			return items[i], btns, true
		}
	}
	return protocol.ViewItem{}, nil, false
}

func (u *ui) help() {
	_, btns, ok := u.lastWithButtons()
	if !ok {
		u.setNote("no buttons yet. /open <path> loads an image; /clear starts a new conversation; type to talk to the agent; q quits")
		return
	}
	parts := make([]string, len(btns))
	for i, b := range btns {
		parts[i] = "/" + b.Key + " " + b.Label
	}
	u.setNote("buttons: %s  (/clear: new conversation)", strings.Join(parts, "  "))
}

// press triggers the button with this key on the most recent item that has one.
func (u *ui) press(ctx context.Context, key string) {
	item, btns, ok := u.lastWithButtons()
	if !ok {
		u.setNote("no buttons in this session yet; to send %q to the agent, type //%s", "/"+key, key)
		return
	}
	for _, b := range btns {
		if b.Key != key {
			continue
		}
		if text, isCopy := b.CopyText(); isCopy {
			// OSC 52 puts it on the clipboard in most terminals, also over ssh.
			fmt.Fprint(u.a.cfg.Term.out(), "\x1b]52;c;"+base64.StdEncoding.EncodeToString([]byte(text))+"\x07")
			u.setNote("copied %s", text)
			return
		}
		sctx, cancel := context.WithTimeout(ctx, 10*time.Second)
		defer cancel()
		if err := u.a.c.Action(sctx, u.session, item.Ref, b.Action, b.Args); err != nil {
			u.setNote("%v", err)
			return
		}
		u.setNote("pressed [%s] %s", b.Key, b.Label)
		return
	}
	u.setNote("no button [%s] on the latest item with buttons; /help lists them", key)
}

// draw writes the screen if it differs from the last one.
func (u *ui) draw(out interface{ Write([]byte) (int, error) }, cols, rows int) {
	lines, cursor := u.frame(cols, rows)
	var b strings.Builder
	for i, l := range lines {
		fmt.Fprintf(&b, "\x1b[%d;1H%s\x1b[0m\x1b[K", i+1, l)
	}
	fmt.Fprintf(&b, "\x1b[%d;%dH", rows, cursor+1)
	if s := b.String(); s != u.lastFrame {
		u.lastFrame = s
		fmt.Fprint(out, s)
	}
}

// frame lays the screen out: exactly rows lines, and the cursor column on
// the last one (the input line).
func (u *ui) frame(cols, rows int) (lines []string, cursorCol int) {
	rows = max(rows, 6)
	avail := rows - 3 // status, divider, input
	previewRows := max(avail/2, 2)
	chatRows := avail - previewRows

	lines = append(lines, u.status(cols))
	lines = append(lines, u.previewArea(cols, previewRows)...)
	lines = append(lines, u.divider(cols))
	lines = append(lines, u.chatArea(cols, chatRows)...)
	input, cursorCol := u.inputLine(cols)
	return append(lines, input), cursorCol
}

func (u *ui) styled(code, s string) string {
	if u.color == tui.None {
		return s
	}
	return code + s + "\x1b[0m"
}

// fit cuts s to at most w cells, ending in … when it had to.
func fit(s string, w int) string {
	if w <= 0 {
		return ""
	}
	if tui.Width(s) <= w {
		return s
	}
	for tui.Width(s) > w-1 && s != "" {
		_, size := utf8.DecodeLastRuneInString(s)
		s = s[:len(s)-size]
	}
	return s + "…"
}

func (u *ui) status(cols int) string {
	parts := []string{"Photo editor"}
	if d, _ := u.a.ed.current(); d != nil {
		b := d.img.Bounds()
		parts = append(parts, d.asset.Name, fmt.Sprintf("%d×%d", b.Dx(), b.Dy()), d.asset.Handle)
	} else {
		parts = append(parts, "no image")
	}
	switch {
	case u.focusApp == appName:
		parts = append(parts, "agent is here")
	case u.focusApp != "":
		parts = append(parts, "agent is in "+u.focusApp)
	}
	text := " " + strings.Join(parts, " · ")
	text = fit(text, cols)
	text += strings.Repeat(" ", max(cols-tui.Width(text), 0))
	return u.styled("\x1b[7m", text)
}

func (u *ui) previewArea(cols, rows int) []string {
	d, _ := u.a.ed.current()
	var shown []string
	if d == nil {
		msg := "No image open. Ask the agent to open one, or type /open <path>."
		shown = []string{u.styled("\x1b[2m", strings.Repeat(" ", max((cols-tui.Width(msg))/2, 0))+fit(msg, cols))}
	} else {
		key := previewKey{d, cols, rows, u.color}
		if key != u.previewKey || u.previewLines == nil {
			u.previewKey = key
			u.previewLines = renderPreview(d.img, cols, rows, u.color)
		}
		w, _ := previewSize(d.img.Bounds().Dx(), d.img.Bounds().Dy(), cols, rows)
		pad := strings.Repeat(" ", max((cols-w)/2, 0))
		for _, l := range u.previewLines {
			shown = append(shown, pad+l)
		}
	}
	top := max((rows-len(shown))/2, 0)
	out := make([]string, 0, rows)
	for range top {
		out = append(out, "")
	}
	out = append(out, shown...)
	for len(out) < rows {
		out = append(out, "")
	}
	return out[:rows]
}

func (u *ui) divider(cols int) string {
	text := "q quits · /help lists buttons · /open <path>"
	if u.note != "" && u.now().Before(u.noteTill) {
		text = u.note
	}
	text = fit(" "+text+" ", cols-4)
	line := "── " + text + strings.Repeat("─", max(cols-tui.Width(text)-3, 0))
	return u.styled("\x1b[2m", line)
}

// chatArea shows the last rows lines of the followed session.
func (u *ui) chatArea(cols, rows int) []string {
	if u.state == nil {
		return pad(nil, rows)
	}
	frame := int(u.now().UnixMilli() / int64(spinnerPeriod/time.Millisecond))
	items := u.state.Items()
	animated := false
	for _, it := range items {
		animated = animated || (it.State != nil && *it.State == protocol.ItemStateRunning)
	}
	if !animated {
		frame = 0
	}
	key := chatKey{u.state.Seq, cols, rows, frame, len(items)}
	if key != u.chatKey || u.chatLines == nil {
		u.chatKey = key
		theme := tui.DefaultTheme()
		if t, ok := u.a.c.Theme(); ok {
			theme = t
		}
		// Each item takes at least a line, so the last `rows` items are enough.
		tail := items[max(len(items)-rows, 0):]
		text := tui.Render(tail, theme, tui.Options{Width: cols, Color: u.color, Frame: frame})
		u.chatLines = strings.Split(strings.TrimRight(text, "\n"), "\n")
	}
	lines := u.chatLines
	if len(lines) > rows {
		lines = lines[len(lines)-rows:]
	}
	return pad(lines, rows)
}

// pad extends lines with empty ones to n entries, anchoring them at the bottom.
func pad(lines []string, n int) []string {
	out := make([]string, 0, n)
	for range max(n-len(lines), 0) {
		out = append(out, "")
	}
	return append(out, lines...)
}

func (u *ui) inputLine(cols int) (string, int) {
	prompt := "> "
	text := string(u.input)
	room := max(cols-tui.Width(prompt)-1, 1)
	for tui.Width(text) > room {
		_, size := utf8.DecodeRuneInString(text)
		text = text[size:]
	}
	return prompt + text, tui.Width(prompt) + tui.Width(text)
}
