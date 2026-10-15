package main

import (
	"context"
	"encoding/base64"
	"errors"
	"fmt"
	"io"
	"strconv"
	"strings"
	"time"
	"unicode/utf8"

	"neoworks.dev/suite/renderers/tui"

	protocol "neoworks.dev/suite/protocol/go"
	sdk "neoworks.dev/suite/sdk-go"
)

const (
	// flushInterval caps redraws of the live tail at 20 a second.
	flushInterval = 50 * time.Millisecond
	// quietPeriod is how long the last item must stay unchanged after the
	// turn ended before it counts as settled. The turn_ended event and the
	// last view patch travel on separate queues; the pause keeps the item
	// from being printed for good before its final text arrived.
	quietPeriod = 150 * time.Millisecond
	// spinnerPeriod is how long the spinner of a running tool shows a frame.
	spinnerPeriod = 125 * time.Millisecond
	// catchUpQuiet is how long the queues must be empty at exit before
	// everything the connection delivered is taken as seen.
	catchUpQuiet = 30 * time.Millisecond
)

// colorMode picks the color depth from the environment: none for NO_COLOR or
// anything that is not a terminal, else the best the terminal advertises.
func colorMode(getenv func(string) string, tty bool) tui.ColorMode {
	switch term := getenv("TERM"); {
	case !tty, getenv("NO_COLOR") != "", term == "dumb":
		return tui.None
	case getenv("COLORTERM") == "truecolor" || getenv("COLORTERM") == "24bit":
		return tui.TrueColor
	case strings.Contains(term, "256color"):
		return tui.ANSI256
	}
	return tui.ANSI16
}

// viewAttachment is `suite attach` drawing the session's resolved view.
//
// A terminal only appends, so the chat is split in two. Items that are
// settled are printed once and scroll up. The live tail, from the first item
// still changing to the end, is drawn below them and redrawn in place: move
// up over the rows it took, clear to the end of the screen, draw it again.
// Without a terminal nothing is ever redrawn: each item is printed once it
// is settled, and the rest at exit.
//
// Everything runs on the goroutine in run, so there is no locking.
type viewAttachment struct {
	c       *sdk.Client
	id, app string
	view    *sdk.ViewSubscription
	state   *sdk.ViewState
	events  *sdk.Subscription // live only; says whether a turn is running
	out     io.Writer
	stderr  io.Writer
	getenv  func(string) string
	size    func() (cols, rows int)
	now     func() time.Time

	tty         bool // stdout is a terminal: color and the live tail
	interactive bool // stdin is a terminal
	thoughts    bool
	color       tui.ColorMode

	running  bool // a turn is in progress, from user_message/turn_ended
	draining bool // stdin closed; waiting for the session to go idle

	printed    map[string]bool // refs of items already in the scrollback
	tailRows   int             // terminal rows below the scrollback to erase before drawing
	tail       string          // what those rows show
	notes      []string        // lines to print above the tail on the next flush
	dirty      bool
	lastFlush  time.Time
	lastChange time.Time // last view update or change of running
	settleAt   time.Time // when the quiet period of the last change ends
	animated   bool      // the tail has a spinner
	frame      int

	done bool
	code int
}

func (a *viewAttachment) finish(code int) { a.done, a.code = true, code }

// width is the width to draw in: $COLUMNS, else the terminal's, else 80.
func (a *viewAttachment) width() int {
	if n, err := strconv.Atoi(a.getenv("COLUMNS")); err == nil && n > 0 {
		return n
	}
	if a.tty && a.size != nil {
		if cols, _ := a.size(); cols > 0 {
			return cols
		}
	}
	return 80
}

func (a *viewAttachment) theme() protocol.Theme {
	if t, ok := a.c.Theme(); ok {
		return t
	}
	return tui.DefaultTheme()
}

// note queues a line for stdout, printed above the tail at the next flush.
func (a *viewAttachment) note(format string, args ...any) {
	a.notes = append(a.notes, fmt.Sprintf(format, args...))
	a.dirty = true
}

// warnf reports a problem. On a terminal it goes through the same output as
// the chat, since a stray line from another stream would throw off the
// tail's row count.
func (a *viewAttachment) warnf(format string, args ...any) {
	if a.tty {
		a.note("suite: "+format, args...)
		return
	}
	fmt.Fprintf(a.stderr, "suite: "+format+"\n", args...)
}

// failed reports a failed request. A refusal from the daemon is only a
// warning; anything else means the connection is gone, which ends the session.
func (a *viewAttachment) failed(err error) {
	a.warnf("%v", err)
	var refused *sdk.Error
	if !errors.As(err, &refused) {
		a.finish(1)
	}
}

func (a *viewAttachment) run(ctx context.Context, e cliEnv) int {
	a.running = a.events.Info.State == protocol.SessionStateRunning
	a.printed = map[string]bool{}

	// History first, then the marker, then whatever is still changing.
	a.flush(false)
	a.note("-- attached to %s --", a.id)
	a.flush(false)

	stdinC := readLines(e.Stdin)
	var poll <-chan time.Time
	ticker := time.NewTicker(flushInterval)
	defer ticker.Stop()
	pollTicker := time.NewTicker(pollInterval)
	defer pollTicker.Stop()

	for !a.done {
		select {
		case u, ok := <-a.view.Updates():
			if !ok {
				return a.lost(a.view.Err())
			}
			a.update(ctx, u)
		case rec, ok := <-a.events.Events():
			if !ok {
				return a.lost(a.events.Err())
			}
			a.event(rec)
		case line, ok := <-stdinC:
			switch {
			case ok:
				a.input(ctx, line)
			case a.interactive:
				a.finish(0)
			default:
				// Piped input: let the agent finish so its output is printed.
				stdinC, poll, a.draining = nil, pollTicker.C, true
				a.pollIdle(ctx)
			}
		case <-poll:
			a.pollIdle(ctx)
		case <-ticker.C:
			a.animate()
		case <-e.Interrupt:
			if !a.running {
				a.finish(0)
			} else if err := a.c.Cancel(ctx, a.id); err != nil {
				a.failed(err)
			}
		case <-ctx.Done():
			a.flush(true)
			return 1
		}
		a.maybeFlush()
	}
	a.catchUp()
	a.flush(true)
	return a.code
}

// lost ends the session when the connection did, showing what arrived.
func (a *viewAttachment) lost(err error) int {
	a.flush(true)
	a.warnf("%v", err)
	a.flush(true) // the warning, on a terminal
	return 1
}

// update applies a view change. If it does not fit the state, the state has
// drifted from the daemon's, so start over from a fresh snapshot.
func (a *viewAttachment) update(ctx context.Context, u sdk.ViewUpdate) {
	a.touch()
	if err := u.ApplyTo(a.state); err != nil {
		a.warnf("%v; refreshing the view", err)
		a.view.Close()
		sub, err := a.c.SubscribeView(ctx, a.id)
		if err != nil {
			a.failed(err)
			a.finish(1)
			return
		}
		a.view, a.state = sub, sub.State()
	}
}

func (a *viewAttachment) event(rec protocol.SessionEventRecord) {
	switch rec.Event.Value.(type) {
	case protocol.UserMessage:
		a.running = true
		a.touch()
	case protocol.TurnEnded:
		a.running = false
		a.touch()
	}
}

// touch notes that something changed: redraw, and settle once it is quiet.
func (a *viewAttachment) touch() {
	a.dirty = true
	a.lastChange = a.now()
	a.settleAt = a.lastChange.Add(quietPeriod)
}

// animate runs on every tick: it redraws for the end of the quiet period and,
// on a terminal, for the next frame of a spinner.
func (a *viewAttachment) animate() {
	now := a.now()
	if !a.settleAt.IsZero() && !now.Before(a.settleAt) {
		a.settleAt = time.Time{}
		a.dirty = true
	}
	if a.animated {
		if f := int(now.UnixMilli() / int64(spinnerPeriod/time.Millisecond)); f != a.frame {
			a.frame, a.dirty = f, true
		}
	}
}

// maybeFlush draws if something changed. A terminal is held to 20 draws a
// second; the tick catches what a skipped draw left.
func (a *viewAttachment) maybeFlush() {
	if !a.dirty {
		return
	}
	if a.tty && a.now().Sub(a.lastFlush) < flushInterval {
		return
	}
	a.flush(false)
}

// live reports whether item i may still change, so it cannot go to the
// scrollback yet: a tool that has not finished, or the last item while the
// agent is still writing it.
func (a *viewAttachment) live(items []protocol.ViewItem, i int) bool {
	it := items[i]
	switch it.Kind {
	case protocol.ItemKindTool:
		return it.State != nil && (*it.State == protocol.ItemStatePending || *it.State == protocol.ItemStateRunning)
	case protocol.ItemKindAgent, protocol.ItemKindThought:
		return i == len(items)-1 && (a.running || a.now().Sub(a.lastChange) < quietPeriod)
	}
	return false
}

// flush draws what changed. With final set everything counts as settled and
// the tail goes to the scrollback, for the exit.
func (a *viewAttachment) flush(final bool) {
	a.dirty = false
	a.lastFlush = a.now()
	items := a.state.Items()
	opts := tui.Options{Width: a.width(), Color: a.color, Frame: a.frame, Thoughts: a.thoughts}
	theme := a.theme()

	// The settled prefix goes to the scrollback; the first live item starts the tail.
	var settled strings.Builder
	i := 0
	for ; i < len(items); i++ {
		if a.printed[items[i].Ref] {
			continue
		}
		if !final && a.live(items, i) {
			break
		}
		settled.WriteString(tui.RenderItem(items[i], theme, opts) + "\n\n")
		a.printed[items[i].Ref] = true
	}

	tail := ""
	a.animated = false
	if a.tty && !final {
		var pending []protocol.ViewItem
		for _, it := range items[i:] {
			if a.printed[it.Ref] {
				continue
			}
			pending = append(pending, it)
			a.animated = a.animated || (it.State != nil && *it.State == protocol.ItemStateRunning)
		}
		if len(pending) > 0 {
			tail = a.fit(tui.Render(pending, theme, opts) + "\n")
		}
	}
	notes := ""
	if len(a.notes) > 0 {
		notes = strings.Join(a.notes, "\n") + "\n"
		a.notes = nil
	}

	if settled.Len() == 0 && notes == "" && tail == a.tail && a.tailRows == strings.Count(tail, "\n") {
		return
	}
	var b strings.Builder
	if a.tty && a.tailRows > 0 {
		fmt.Fprintf(&b, "\x1b[%dA\r\x1b[J", a.tailRows)
	}
	b.WriteString(notes)
	b.WriteString(settled.String())
	b.WriteString(tail)
	a.tail, a.tailRows = tail, strings.Count(tail, "\n")
	io.WriteString(a.out, b.String()) // a broken stdout has nowhere to be reported
}

// fit keeps the last lines of a tail that is taller than the terminal: rows
// that scrolled off cannot be redrawn. The whole item reaches the scrollback
// when it settles.
func (a *viewAttachment) fit(tail string) string {
	if a.size == nil {
		return tail
	}
	_, rows := a.size()
	lines := strings.SplitAfter(tail, "\n")
	lines = lines[:len(lines)-1] // SplitAfter leaves an empty last element
	if rows < 2 || len(lines) <= rows-1 {
		return tail
	}
	return strings.Join(lines[len(lines)-(rows-1):], "")
}

// catchUp takes in what the connection already delivered but the loop has not
// read yet, so the final draw has the end of the last message. It stops once
// the queues stay quiet for a moment.
func (a *viewAttachment) catchUp() {
	ctx, cancel := context.WithTimeout(context.Background(), time.Second)
	defer cancel()
	quiet := time.NewTimer(catchUpQuiet)
	defer quiet.Stop()
	updates, events := a.view.Updates(), a.events.Events()
	for updates != nil || events != nil {
		select {
		case u, ok := <-updates:
			if !ok {
				updates = nil
				continue
			}
			if err := u.ApplyTo(a.state); err != nil {
				return
			}
		case rec, ok := <-events:
			if !ok {
				events = nil
				continue
			}
			a.event(rec)
		case <-quiet.C:
			return
		case <-ctx.Done():
			return
		}
		quiet.Reset(catchUpQuiet)
	}
}

// pollIdle finishes the attachment once stdin is closed and the session has
// nothing left to do. Inputs are acked before this runs, and an ack means the
// session already shows as running or queued; the daemon sends turn_ended
// before reporting idle, and catchUp then collects the output still in the
// queues.
func (a *viewAttachment) pollIdle(ctx context.Context) {
	if !a.draining {
		return
	}
	sessions, err := a.c.ListSessions(ctx)
	if err != nil {
		a.warnf("%v", err)
		a.finish(1)
		return
	}
	for _, s := range sessions {
		if s.ID == a.id {
			if s.State == protocol.SessionStateIdle && s.Queued == 0 {
				a.finish(0)
			}
			return
		}
	}
	a.warnf("session %s disappeared", a.id)
	a.finish(1)
}

// input handles one line from stdin: a button (/key, or /help for the list)
// or text for the agent.
func (a *viewAttachment) input(ctx context.Context, line string) {
	if a.tty && a.interactive {
		// The terminal echoed the line below the tail; it is erased with it.
		a.tailRows += max(1, (tui.Width(line)+a.width()-1)/a.width())
		a.dirty = true
	}
	trimmed := strings.TrimSpace(line)
	switch {
	case trimmed == "":
	case trimmed == "/help":
		a.help()
	case isCommand(trimmed):
		a.press(ctx, trimmed[1:])
	default:
		if strings.HasPrefix(trimmed, "//") {
			// "//text" sends "/text": a way to start a message with a slash.
			line = strings.Replace(line, "/", "", 1)
		}
		a.send(ctx, line)
	}
}

// isCommand tells "/o" from a message such as "/usr/bin/env is missing".
func isCommand(s string) bool {
	return len(s) > 1 && s[0] == '/' && !strings.ContainsAny(s[1:], " \t/")
}

func (a *viewAttachment) send(ctx context.Context, text string) {
	if err := a.c.Input(ctx, protocol.SessionInput{Session: a.id, App: a.app, Text: text}); err != nil {
		a.failed(err)
	}
}

// lastWithButtons is the most recent item that has buttons.
func (a *viewAttachment) lastWithButtons() (protocol.ViewItem, []tui.Button, bool) {
	items := a.state.Items()
	for i := len(items) - 1; i >= 0; i-- {
		if btns := tui.Buttons(items[i]); len(btns) > 0 {
			return items[i], btns, true
		}
	}
	return protocol.ViewItem{}, nil, false
}

func itemName(it protocol.ViewItem) string {
	if it.Label != nil && *it.Label != "" {
		return *it.Label
	}
	return it.Ref
}

func (a *viewAttachment) help() {
	it, btns, ok := a.lastWithButtons()
	if !ok {
		a.note("no buttons in this session yet; type text to talk to the agent")
		return
	}
	a.note("buttons of the latest item with any (%s); type /<key> to press one:", itemName(it))
	for _, b := range btns {
		what := b.Action
		if text, isCopy := b.CopyText(); isCopy {
			what = "copy " + shorten(text, 40)
		}
		a.note("  /%s  %s  (%s)", b.Key, b.Label, what)
	}
}

// press triggers the button with this key on the most recent item that has one.
func (a *viewAttachment) press(ctx context.Context, key string) {
	it, btns, ok := a.lastWithButtons()
	if !ok {
		a.note("no buttons in this session yet; to send %q to the agent, type //%s", "/"+key, key)
		return
	}
	for _, b := range btns {
		if b.Key != key {
			continue
		}
		if text, isCopy := b.CopyText(); isCopy {
			a.copyText(text)
			return
		}
		if err := a.c.Action(ctx, a.id, it.Ref, b.Action, b.Args); err != nil {
			a.failed(err)
			return
		}
		a.note("pressed [%s] %s", b.Key, b.Label)
		return
	}
	a.note("no button [%s] on the latest item with buttons (%s); /help lists them", key, itemName(it))
}

// copyText puts text on the clipboard with the OSC 52 escape, which most
// terminals (also over ssh) pass on. Without a terminal there is nothing to
// copy to, so the text is printed instead.
func (a *viewAttachment) copyText(text string) {
	if !a.tty {
		a.note("copy: %s", text)
		return
	}
	io.WriteString(a.out, "\x1b]52;c;"+base64.StdEncoding.EncodeToString([]byte(text))+"\x07")
	a.note("copied: %s", shorten(text, max(a.width()-8, 10)))
}

// shorten cuts s to at most w cells, ending in … when it had to.
func shorten(s string, w int) string {
	s = strings.Join(strings.Fields(s), " ")
	if tui.Width(s) <= w {
		return s
	}
	for tui.Width(s) > w-1 {
		_, size := utf8.DecodeLastRuneInString(s)
		s = s[:len(s)-size]
	}
	return s + "…"
}
