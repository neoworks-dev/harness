package main

import (
	"bufio"
	"context"
	"fmt"
	"io"
	"os"
	"strings"
	"time"

	protocol "neoworks.dev/suite/protocol/go"
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
	positional, err := parseFlags(fs, args)
	if err != nil {
		return 2
	}
	switch {
	case *create && len(positional) > 0, !*create && len(positional) != 1:
		fmt.Fprint(e.Stderr, "usage: suite attach [flags] <id|prefix>\n       suite attach --new [--cwd DIR] [--title T] [flags]\n")
		return 2
	case !*create && (*cwd != "" || *title != ""):
		fmt.Fprintln(e.Stderr, "suite: --cwd and --title only apply with --new")
		return 2
	}

	b, _, ok := connect(ctx, *socket, e)
	if !ok {
		return 1
	}
	defer b.close()

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
		reply, err := b.call(ctx, func(id string) protocol.EnvelopeVariant {
			return protocol.SessionCreate{ID: id, Payload: params}
		})
		if err != nil {
			fmt.Fprintf(e.Stderr, "suite: create session: %v\n", err)
			return 1
		}
		info, ok := reply.(protocol.SessionInfoMessage)
		if !ok {
			fmt.Fprintf(e.Stderr, "suite: unexpected reply %T to session.create\n", reply)
			return 1
		}
		id = info.Payload.ID
		fmt.Fprintf(e.Stdout, "created session %s\n", id)
	} else {
		sessions, err := b.listSessions(ctx)
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

	a := &attachment{
		b:           b,
		id:          id,
		app:         *app,
		r:           newRenderer(e.Stdout, *thoughts),
		stderr:      e.Stderr,
		interactive: isTerminal(e.Stdin),
		pending:     map[string]func(protocol.EnvelopeVariant){},
	}
	return a.run(ctx, e)
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
// goroutine in run, so there is no locking; replies are matched to requests
// through pending.
type attachment struct {
	b           *bus
	id, app     string
	r           *renderer
	stderr      io.Writer
	interactive bool // stdin is a terminal

	pending map[string]func(protocol.EnvelopeVariant) // request id -> reply handler

	subscribed    bool // replay is done
	running       bool // a turn is in progress, from user_message/turn_ended
	unackedInputs int
	pollInFlight  bool
	draining      bool // stdin closed; waiting for the session to go idle

	done bool
	code int
}

func (a *attachment) finish(code int) { a.done, a.code = true, code }

func (a *attachment) warnf(format string, args ...any) {
	fmt.Fprintf(a.stderr, "suite: "+format+"\n", args...)
}

// request sends a message built for a fresh id and registers onReply for the
// reply, which may be an ack, a bus.error or a typed result.
func (a *attachment) request(ctx context.Context, build func(id string) protocol.EnvelopeVariant, onReply func(protocol.EnvelopeVariant)) {
	id := a.b.newID()
	a.pending[id] = onReply
	if err := a.b.send(ctx, build(id)); err != nil {
		a.warnf("connection to daemon lost: %v", err)
		a.finish(1)
	}
}

func (a *attachment) run(ctx context.Context, e cliEnv) int {
	a.request(ctx, func(id string) protocol.EnvelopeVariant {
		from := int64(1)
		return protocol.SessionSubscribe{ID: id, Payload: protocol.SubscribeParams{Session: a.id, FromSeq: &from}}
	}, a.onSubscribed)

	lines := readLines(e.Stdin)
	var stdinC <-chan string // stays nil until the replay is printed
	var tick <-chan time.Time
	ticker := time.NewTicker(pollInterval)
	defer ticker.Stop()

	for !a.done {
		select {
		case f, ok := <-a.b.in:
			if !ok || f.err != nil {
				a.warnf("connection to daemon lost")
				return 1
			}
			a.handle(f.value)
			if a.subscribed && !a.draining && stdinC == nil {
				stdinC = lines
			}
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
			if a.running {
				a.request(ctx, func(id string) protocol.EnvelopeVariant {
					return protocol.SessionCancel{ID: id, Payload: protocol.SessionRef{Session: a.id}}
				}, a.onResult)
			} else {
				a.finish(0)
			}
		case <-ctx.Done():
			return 1
		}
	}
	return a.code
}

func (a *attachment) handle(v protocol.EnvelopeVariant) {
	if id, ok := replyID(v); ok {
		if onReply := a.pending[id]; onReply != nil {
			delete(a.pending, id)
			onReply(v)
		}
		return
	}
	switch m := v.(type) {
	case protocol.SessionEvents:
		rec := m.Payload
		if rec.Session != a.id {
			return
		}
		switch rec.Event.Value.(type) {
		case protocol.UserMessage:
			a.running = true
		case protocol.TurnEnded:
			a.running = false
		}
		a.r.Render(rec.Event)
	case protocol.BusError: // without an id
		a.warnf("daemon error: %s", m.Payload.Message)
	}
}

func (a *attachment) onSubscribed(v protocol.EnvelopeVariant) {
	switch m := v.(type) {
	case protocol.SessionSubscribed:
		a.subscribed = true
		a.running = m.Payload.Session.State == protocol.SessionStateRunning
		a.r.line("-- attached to %s --", a.id)
	case protocol.BusError:
		a.warnf("attach %s: %s", a.id, m.Payload.Message)
		a.finish(1)
	}
}

// onResult reports a failed request; successful acks need no handling.
func (a *attachment) onResult(v protocol.EnvelopeVariant) {
	if m, ok := v.(protocol.BusError); ok {
		a.warnf("%s", m.Payload.Message)
	}
}

func (a *attachment) sendInput(ctx context.Context, line string) {
	if strings.TrimSpace(line) == "" {
		return
	}
	a.unackedInputs++
	a.request(ctx, func(id string) protocol.EnvelopeVariant {
		return protocol.SessionInputMessage{ID: &id, Payload: protocol.SessionInput{Session: a.id, App: a.app, Text: line}}
	}, func(v protocol.EnvelopeVariant) {
		a.unackedInputs--
		a.onResult(v)
	})
}

// poll finishes the attachment once stdin is closed and the session has
// nothing left to do. It waits for input acks first: after an ack, the list
// already shows the session running or queued, and the daemon sends turn_ended
// before reporting idle, so all output has been printed by then.
func (a *attachment) poll(ctx context.Context) {
	if !a.draining || a.unackedInputs > 0 || a.pollInFlight {
		return
	}
	a.pollInFlight = true
	a.request(ctx, func(id string) protocol.EnvelopeVariant { return protocol.SessionList{ID: id} }, func(v protocol.EnvelopeVariant) {
		a.pollInFlight = false
		res, ok := v.(protocol.SessionListResult)
		if !ok {
			a.onResult(v)
			a.finish(1)
			return
		}
		for _, s := range res.Payload.Sessions {
			if s.ID == a.id {
				if s.State == protocol.SessionStateIdle && s.Queued == 0 {
					a.finish(0)
				}
				return
			}
		}
		a.warnf("session %s disappeared", a.id)
		a.finish(1)
	})
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
			if line = strings.TrimRight(line, "\r\n"); line != "" {
				lines <- line
			}
			if err != nil {
				return
			}
		}
	}()
	return lines
}
