package main

import (
	"bufio"
	"context"
	"errors"
	"fmt"
	"io"
	"os"
	"strings"
	"time"

	protocol "neoworks.dev/suite/protocol/go"
	sdk "neoworks.dev/suite/sdk-go"
)

// pollInterval is how often we ask whether the session went idle after stdin
// closed.
const pollInterval = 200 * time.Millisecond

func runAttach(ctx context.Context, args []string, e cliEnv) int {
	fs, socket := commandFlags("attach", e)
	create := fs.Bool("new", false, "create a new session and attach to it")
	cwd := fs.String("cwd", "", "working directory for --new (default: current directory)")
	title := fs.String("title", "", "title for --new")
	app := fs.String("app", "cli", "app name sent with each input")
	thoughts := fs.Bool("thoughts", false, "show the agent's reasoning")
	raw := fs.Bool("raw", false, "print the raw session events instead of the rendered view")
	positional, err := parseFlags(fs, args)
	if err != nil {
		return 2
	}
	switch {
	case *create && len(positional) > 0, !*create && len(positional) > 1:
		fmt.Fprint(e.Stderr, "usage: suite attach [flags] [id|prefix]\n       suite attach --new [--cwd DIR] [--title T] [flags]\n")
		return 2
	case !*create && (*cwd != "" || *title != ""):
		fmt.Fprintln(e.Stderr, "suite: --cwd and --title only apply with --new")
		return 2
	}

	c, _, ok := connect(ctx, *socket, e)
	if !ok {
		return 1
	}
	defer c.Close()

	var id string
	if *create {
		dir := *cwd
		if dir == "" {
			if dir, err = os.Getwd(); err != nil {
				fmt.Fprintf(e.Stderr, "suite: %v\n", err)
				return 1
			}
		}
		params := protocol.SessionCreateParams{Cwd: &dir}
		if *title != "" {
			params.Title = title
		}
		info, err := c.CreateSession(ctx, params)
		if err != nil {
			fmt.Fprintf(e.Stderr, "suite: create session: %v\n", err)
			return 1
		}
		id = info.ID
		fmt.Fprintf(e.Stdout, "created session %s\n", id)
	} else if len(positional) == 0 {
		focus, err := waitFocus(ctx, c)
		if err != nil {
			fmt.Fprintf(e.Stderr, "suite: no agent.focus from the daemon: %v\n", err)
			return 1
		}
		id = focus.Session
	} else {
		sessions, err := c.ListSessions(ctx)
		if err == nil {
			var s protocol.SessionInfo
			if s, err = resolveSession(sessions, positional[0]); err == nil {
				id = s.ID
			}
		}
		if err != nil {
			fmt.Fprintf(e.Stderr, "suite: %v\n", err)
			return 1
		}
	}

	if !*raw {
		if code, ok := attachView(ctx, c, id, *app, *thoughts, e); ok {
			return code
		}
	}

	sub, err := c.Subscribe(ctx, id, 1)
	if err != nil {
		fmt.Fprintf(e.Stderr, "suite: attach %s: %v\n", id, err)
		return 1
	}
	a := &attachment{
		c:           c,
		sub:         sub,
		id:          id,
		app:         *app,
		r:           newRenderer(e.Stdout, *thoughts),
		stderr:      e.Stderr,
		interactive: isTerminal(e.Stdin),
	}
	return a.run(ctx, e)
}

// attachView follows the session's resolved view and draws it with the TUI
// renderer. ok is false when the daemon refuses to serve views, so the caller
// can fall back to the raw events.
func attachView(ctx context.Context, c *sdk.Client, id, app string, thoughts bool, e cliEnv) (code int, ok bool) {
	view, err := c.SubscribeView(ctx, id)
	var refused *sdk.Error
	if errors.As(err, &refused) && refused.Code != protocol.ErrorCodeNotFound {
		fmt.Fprintf(e.Stderr, "suite: the daemon does not serve views (%v); showing raw events\n", err)
		return 0, false
	}
	if err != nil {
		fmt.Fprintf(e.Stderr, "suite: attach %s: %v\n", id, err)
		return 1, true
	}
	// The view says nothing of turns; the events do, and only their end matters.
	events, err := c.SubscribeLive(ctx, id)
	if err != nil {
		fmt.Fprintf(e.Stderr, "suite: attach %s: %v\n", id, err)
		return 1, true
	}
	// Give the theme a moment; it comes right after the handshake.
	themeCtx, cancel := context.WithTimeout(ctx, time.Second)
	c.WaitTheme(themeCtx)
	cancel()

	a := &viewAttachment{
		c: c, id: id, app: app,
		view: view, state: view.State(), events: events,
		out: e.Stdout, stderr: e.Stderr, getenv: e.Getenv, size: e.Size, now: e.Now,
		tty: e.StdoutTTY, interactive: isTerminal(e.Stdin), thoughts: thoughts,
		color: colorMode(e.Getenv, e.StdoutTTY),
	}
	return a.run(ctx, e), true
}

// isTerminal reports whether r is a character device such as a tty.
func isTerminal(r io.Reader) bool {
	f, ok := r.(*os.File)
	if !ok {
		return false
	}
	st, err := f.Stat()
	return err == nil && st.Mode()&os.ModeCharDevice != 0
}

// attachment is the state of one attached session. Everything runs on the
// goroutine in run, so there is no locking. Requests are synchronous: events
// queue up in the subscription meanwhile.
type attachment struct {
	c           *sdk.Client
	sub         *sdk.Subscription
	id, app     string
	r           *renderer
	stderr      io.Writer
	interactive bool // stdin is a terminal

	running  bool // a turn is in progress, from user_message/turn_ended
	draining bool // stdin closed; waiting for the session to go idle

	done bool
	code int
}

func (a *attachment) finish(code int) { a.done, a.code = true, code }

func (a *attachment) warnf(format string, args ...any) {
	fmt.Fprintf(a.stderr, "suite: "+format+"\n", args...)
}

// failed reports a failed request. A refusal from the daemon is only a
// warning; anything else means the connection is gone, which ends the session.
func (a *attachment) failed(err error) {
	a.warnf("%v", err)
	var refused *sdk.Error
	if !errors.As(err, &refused) {
		a.finish(1)
	}
}

func (a *attachment) run(ctx context.Context, e cliEnv) int {
	for _, rec := range a.sub.Replay {
		a.render(rec)
	}
	a.running = a.sub.Info.State == protocol.SessionStateRunning
	a.r.line("-- attached to %s --", a.id)

	stdinC := readLines(e.Stdin)
	var tick <-chan time.Time
	ticker := time.NewTicker(pollInterval)
	defer ticker.Stop()

	for !a.done {
		select {
		case rec, ok := <-a.sub.Events():
			if !ok {
				a.warnf("%v", a.sub.Err())
				return 1
			}
			a.render(rec)
		case line, ok := <-stdinC:
			switch {
			case ok:
				a.sendInput(ctx, line)
			case a.interactive:
				a.finish(0)
			default:
				// Piped input: let the agent finish so its output is printed.
				stdinC, tick, a.draining = nil, ticker.C, true
				a.poll(ctx)
			}
		case <-tick:
			a.poll(ctx)
		case <-e.Interrupt:
			if !a.running {
				a.finish(0)
			} else if err := a.c.Cancel(ctx, a.id); err != nil {
				a.failed(err)
			}
		case <-ctx.Done():
			return 1
		}
	}
	return a.code
}

func (a *attachment) render(rec protocol.SessionEventRecord) {
	switch rec.Event.Value.(type) {
	case protocol.UserMessage:
		a.running = true
	case protocol.TurnEnded:
		a.running = false
	}
	a.r.Render(rec.Event)
}

func (a *attachment) sendInput(ctx context.Context, line string) {
	if strings.TrimSpace(line) == "" {
		return
	}
	if err := a.c.Input(ctx, protocol.SessionInput{Session: a.id, App: a.app, Text: line}); err != nil {
		a.failed(err)
	}
}

// poll finishes the attachment once stdin is closed and the session has
// nothing left to do. Inputs are acked before poll runs, and an ack means the
// session already shows as running or queued; the daemon sends turn_ended
// before reporting idle, so all output has been printed by then.
func (a *attachment) poll(ctx context.Context) {
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

// readLines delivers stdin lines on the returned channel and closes it at
// EOF. It uses a Reader rather than a Scanner so long lines are not an error.
func readLines(r io.Reader) <-chan string {
	lines := make(chan string)
	go func() {
		defer close(lines)
		br := bufio.NewReader(r)
		for {
			line, err := br.ReadString('\n')
			line = strings.TrimRight(line, "\r\n")
			// A blank line is kept (the view needs it to count rows the
			// terminal echoed); only the empty remainder at EOF is not a line.
			if line != "" || err == nil {
				lines <- line
			}
			if err != nil {
				return
			}
		}
	}()
	return lines
}
